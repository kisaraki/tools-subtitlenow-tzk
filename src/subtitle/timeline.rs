//! Single RAM-only timeline. Source and translation revisions have independent gates.
use super::source_arbiter::{SourceDecision, SourceLease, SourceSelection};
use crate::{
    ai::{
        speech::{StreamIdentity, TranscriptEvent, TranscriptKind},
        translation::{TranslationError, TranslationPass, TranslationUpdate},
    },
    resolver::contracts::SubtitleSource,
};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimelineBinding {
    pub identity: StreamIdentity,
    pub utterance: u64,
    pub sequence: u64,
    pub revision: u64,
}
impl TimelineBinding {
    pub fn from_event(event: &TranscriptEvent) -> Self {
        Self {
            identity: event.identity,
            utterance: event.utterance,
            sequence: event.sequence,
            revision: event.revision,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SourceAdoptionError {
    #[error("{0}")]
    Arbitration(#[from] super::source_arbiter::ArbitrationError),
    #[error("{0}")]
    Translation(#[from] TranslationError),
    #[error("字幕候選與目前辨識片段或時間範圍不符，未變更字幕來源。")]
    InvalidBinding,
}
/// Provenance and display strings are bounded Session-only data, never Debug/Serialize.
pub struct ExistingDisplay {
    origin: SubtitleSource,
    content_id: Box<str>,
    document_cue_index: usize,
    media_start_us: u64,
    capture_start_us: u64,
    capture_end_us: u64,
    original: Box<str>,
    translated: Box<str>,
    lease: SourceLease,
}
impl ExistingDisplay {
    pub fn origin(&self) -> SubtitleSource {
        self.origin
    }
    pub fn content_id(&self) -> &str {
        &self.content_id
    }
    pub fn document_cue_index(&self) -> usize {
        self.document_cue_index
    }
    pub fn media_start_us(&self) -> u64 {
        self.media_start_us
    }
}

pub const MAX_TIMELINE_ITEMS: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stability {
    Tentative,
    Stable,
    Final,
}
pub struct TimelineItem {
    pub source: TranscriptEvent,
    pub translated: Box<str>,
    pub stability: Stability,
    pub stable_prefix_bytes: usize,
    pub(crate) translation_order: Option<(u64, u64, TranslationPass, u64)>,
    existing: Option<ExistingDisplay>,
}
impl TimelineItem {
    fn active_existing(&self, now: Instant) -> Option<&ExistingDisplay> {
        self.existing
            .as_ref()
            .filter(|text| text.lease.valid(self.source.identity, now))
    }
    pub(crate) fn display_at(&self, now: Instant) -> TimelineDisplay<'_> {
        match self.active_existing(now) {
            Some(text) => TimelineDisplay {
                original: &text.original,
                translated: &text.translated,
                stability: Stability::Final,
                timestamp_us: Some(text.capture_start_us),
                origin: text.origin,
            },
            None => TimelineDisplay {
                original: &self.source.text,
                translated: &self.translated,
                stability: self.stability,
                timestamp_us: self.source.start_us,
                origin: SubtitleSource::LiveAi,
            },
        }
    }
    pub fn display_original(&self) -> &str {
        self.active_existing(Instant::now())
            .map_or(&self.source.text, |text| &text.original)
    }
    pub fn display_translation(&self) -> &str {
        self.active_existing(Instant::now())
            .map_or(&self.translated, |text| &text.translated)
    }
    pub fn display_stability(&self) -> Stability {
        if self.active_existing(Instant::now()).is_some() {
            Stability::Final
        } else {
            self.stability
        }
    }
    pub fn display_timestamp_us(&self) -> Option<u64> {
        self.active_existing(Instant::now())
            .map_or(self.source.start_us, |text| Some(text.capture_start_us))
    }
    pub fn display_origin(&self) -> SubtitleSource {
        self.active_existing(Instant::now())
            .map_or(SubtitleSource::LiveAi, |text| text.origin)
    }
    pub fn existing_display(&self) -> Option<&ExistingDisplay> {
        self.active_existing(Instant::now())
    }
}
pub(crate) struct TimelineDisplay<'a> {
    pub original: &'a str,
    pub translated: &'a str,
    pub stability: Stability,
    pub timestamp_us: Option<u64>,
    pub origin: SubtitleSource,
}
pub struct TranscriptTimeline {
    identity: StreamIdentity,
    items: Vec<TimelineItem>,
    retired_through: Option<u64>,
}

impl TranscriptTimeline {
    pub fn new(identity: StreamIdentity) -> Self {
        Self {
            identity,
            items: Vec::new(),
            retired_through: None,
        }
    }
    pub fn items(&self) -> &[TimelineItem] {
        &self.items
    }
    pub fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub fn retired_through(&self) -> Option<u64> {
        self.retired_through
    }
    pub fn ingest(&mut self, event: TranscriptEvent) -> Result<bool, TranslationError> {
        if event.identity != self.identity
            || self
                .retired_through
                .is_some_and(|sequence| event.sequence <= sequence)
        {
            return Ok(false);
        }
        if event.text.len() > crate::ai::speech::MAX_TRANSCRIPT_BYTES
            || event
                .start_us
                .zip(event.duration_us)
                .is_some_and(|(start, duration)| start.checked_add(duration).is_none())
        {
            return Err(TranslationError::InvalidData);
        }
        if let Some(item) = self
            .items
            .iter_mut()
            .find(|item| item.source.utterance == event.utterance)
        {
            if item.source.kind == TranscriptKind::Final || event.revision <= item.source.revision {
                return Ok(false);
            }
            if item.source.sequence != event.sequence || item.source.start_us != event.start_us {
                return Err(TranslationError::InvalidData);
            }
            item.source = event;
            item.existing = None;
            // Keep the existing display while new translation is pending. Only its tail may change.
            item.stability = Stability::Tentative;
            return Ok(true);
        }
        if self
            .items
            .iter()
            .any(|item| item.source.sequence == event.sequence)
        {
            return Err(TranslationError::InvalidData);
        }
        self.items.push(TimelineItem {
            source: event,
            translated: "".into(),
            stability: Stability::Tentative,
            stable_prefix_bytes: 0,
            translation_order: None,
            existing: None,
        });
        self.items.sort_by_key(|item| item.source.sequence);
        if self.items.len() > MAX_TIMELINE_ITEMS {
            let retired = self.items.remove(0).source.sequence;
            self.retired_through = Some(retired);
        }
        Ok(true)
    }
    pub fn apply(&mut self, update: &TranslationUpdate) -> Result<bool, TranslationError> {
        if update.identity != self.identity {
            return Ok(false);
        }
        let Some(item) = self
            .items
            .iter_mut()
            .find(|item| item.source.utterance == update.utterance)
        else {
            return Ok(false);
        };
        if item.source.revision != update.source_revision || item.stability == Stability::Final {
            return Ok(false);
        }
        let order = (
            update.source_revision,
            update.job_id,
            update.pass,
            update.revision,
        );
        if item
            .translation_order
            .is_some_and(|previous| order <= previous)
        {
            return Ok(false);
        }
        if update.text.len() > crate::ai::speech::MAX_TRANSCRIPT_BYTES
            || update.stable_prefix_bytes > update.text.len()
            || !update.text.is_char_boundary(update.stable_prefix_bytes)
            || update.stable_prefix_bytes < item.stable_prefix_bytes
            || !update
                .text
                .starts_with(&item.translated[..item.stable_prefix_bytes])
            || (update.stability == Stability::Final && item.source.kind != TranscriptKind::Final)
            || (update.stability == Stability::Final
                && (!update.complete || update.stable_prefix_bytes != update.text.len()))
        {
            return Err(TranslationError::InvalidData);
        }
        // A new source revision starts its full translation from the beginning.
        // Keep the previous readable draft until that replacement completes;
        // first translations still stream immediately. Never mark it as stable.
        if !update.complete
            && !item.translated.is_empty()
            && item
                .translation_order
                .is_some_and(|previous| previous.0 != update.source_revision)
        {
            return Ok(false);
        }
        item.translated = update.text.clone();
        item.stability = update.stability;
        item.stable_prefix_bytes = update.stable_prefix_bytes;
        item.translation_order = Some(order);
        Ok(true)
    }
    /// Explicitly commit a verified prefix, on a UTF-8 boundary, while retaining a mutable tail.
    pub fn commit_prefix(&mut self, utterance: u64, bytes: usize) -> Result<(), TranslationError> {
        let item = self
            .items
            .iter_mut()
            .find(|item| item.source.utterance == utterance)
            .ok_or(TranslationError::InvalidData)?;
        if bytes < item.stable_prefix_bytes
            || bytes > item.translated.len()
            || !item.translated.is_char_boundary(bytes)
        {
            return Err(TranslationError::InvalidData);
        }
        item.stable_prefix_bytes = bytes;
        if item.stability != Stability::Final {
            item.stability = Stability::Stable;
        }
        Ok(())
    }
    pub(crate) fn validate_binding(
        &self,
        binding: TimelineBinding,
    ) -> Result<usize, SourceAdoptionError> {
        if binding.identity != self.identity {
            return Err(SourceAdoptionError::InvalidBinding);
        }
        self.items
            .iter()
            .position(|item| {
                TimelineBinding::from_event(&item.source) == binding
                    && item.source.kind == TranscriptKind::Final
                    && item
                        .source
                        .start_us
                        .zip(item.source.duration_us)
                        .is_some_and(|(start, duration)| {
                            duration > 0 && start.checked_add(duration).is_some()
                        })
            })
            .ok_or(SourceAdoptionError::InvalidBinding)
    }
    /// Only the caller-owned Session flow can apply an arbiter-minted decision.
    pub(crate) fn adopt(
        &mut self,
        binding: TimelineBinding,
        decision: SourceDecision<'_>,
        now: Instant,
    ) -> Result<bool, SourceAdoptionError> {
        let index = self.validate_binding(binding)?;
        if decision.identity != self.identity {
            return Err(SourceAdoptionError::InvalidBinding);
        }
        let SourceSelection::Existing { source, cue } = decision.selection else {
            return Ok(self.items[index].existing.take().is_some());
        };
        let item = &self.items[index];
        let (Some(start), Some(duration)) = (item.source.start_us, item.source.duration_us) else {
            return Err(SourceAdoptionError::InvalidBinding);
        };
        if !cue.lease.valid(self.identity, now)
            || start < cue.capture_start_us
            || start
                .checked_add(duration)
                .is_none_or(|end| end > cue.capture_end_us)
            || self.items.iter().enumerate().any(|(other_index, item)| {
                other_index != index
                    && item.existing.as_ref().is_some_and(|existing| {
                        existing.content_id.as_ref() == cue.content_id
                            && existing.document_cue_index == cue.document_cue_index
                            && existing.capture_start_us == cue.capture_start_us
                            && existing.capture_end_us == cue.capture_end_us
                    })
            })
        {
            return Err(SourceAdoptionError::InvalidBinding);
        }
        if item.existing.as_ref().is_some_and(|existing| {
            existing.origin == source
                && existing.content_id.as_ref() == cue.content_id
                && existing.document_cue_index == cue.document_cue_index
                && existing.capture_start_us == cue.capture_start_us
                && existing.capture_end_us == cue.capture_end_us
                && existing.original.as_ref() == cue.original
                && existing.translated.as_ref() == cue.translated
                && existing.lease.same_candidate(&cue.lease)
        }) {
            return Ok(false);
        }
        self.items[index].existing = Some(ExistingDisplay {
            origin: source,
            content_id: cue.content_id.into(),
            document_cue_index: cue.document_cue_index,
            media_start_us: cue.media_start_us,
            capture_start_us: cue.capture_start_us,
            capture_end_us: cue.capture_end_us,
            original: cue.original.into(),
            translated: cue.translated.into(),
            lease: cue.lease,
        });
        Ok(true)
    }
    pub(crate) fn release_invalid_sources(&mut self, now: Instant) -> Vec<u64> {
        let mut released = Vec::new();
        for item in &mut self.items {
            if item
                .existing
                .as_ref()
                .is_some_and(|existing| !existing.lease.valid(self.identity, now))
            {
                item.existing = None;
                released.push(item.source.utterance);
            }
        }
        released
    }
}

#[cfg(test)]
mod revision_display_tests {
    use super::*;
    use crate::{ai::speech::SpeechLanguage, audio::capture::SessionId};
    fn source(revision: u64) -> TranscriptEvent {
        TranscriptEvent {
            identity: StreamIdentity {
                session: SessionId(9500),
                generation: 1,
            },
            utterance: 1,
            sequence: 1,
            revision,
            start_us: None,
            duration_us: None,
            language: Some(SpeechLanguage::English),
            kind: TranscriptKind::Partial,
            text: "explicit synthetic source".into(),
        }
    }
    fn update(
        source_revision: u64,
        revision: u64,
        text: &str,
        complete: bool,
    ) -> TranslationUpdate {
        TranslationUpdate {
            identity: source(1).identity,
            utterance: 1,
            source_revision,
            job_id: source_revision,
            pass: TranslationPass::Draft,
            revision,
            text: text.into(),
            stability: Stability::Tentative,
            stable_prefix_bytes: 0,
            complete,
        }
    }
    #[test]
    fn phase11_long_partial_replacement_keeps_readable_chinese_until_new_draft_completes() {
        let mut timeline = TranscriptTimeline::new(source(1).identity);
        timeline.ingest(source(1)).unwrap();
        assert!(timeline.apply(&update(1, 1, "第一", false)).unwrap());
        assert!(
            timeline
                .apply(&update(1, 2, "第一段完整而可讀的合成譯文。", true))
                .unwrap()
        );
        timeline.ingest(source(2)).unwrap();
        for (revision, text) in [(1, "新"), (2, "新的較長譯文")] {
            assert!(!timeline.apply(&update(2, revision, text, false)).unwrap());
            assert_eq!(
                timeline.items()[0].translated.as_ref(),
                "第一段完整而可讀的合成譯文。"
            );
            assert_eq!(timeline.items()[0].stability, Stability::Tentative);
        }
        assert!(
            timeline
                .apply(&update(2, 3, "新的完整合成譯文。", true))
                .unwrap()
        );
        assert_eq!(
            timeline.items()[0].translated.as_ref(),
            "新的完整合成譯文。"
        );
        assert!(!timeline.apply(&update(1, 4, "過期內容", true)).unwrap());
    }
    #[test]
    fn phase11_replacement_validation_is_not_bypassed_while_preserving_previous_text() {
        let mut timeline = TranscriptTimeline::new(source(1).identity);
        timeline.ingest(source(1)).unwrap();
        timeline.apply(&update(1, 1, "既有合成譯文", true)).unwrap();
        timeline.ingest(source(2)).unwrap();
        let mut invalid = update(2, 1, "新", false);
        invalid.stable_prefix_bytes = 1; // Inside a UTF-8 character.
        assert_eq!(timeline.apply(&invalid), Err(TranslationError::InvalidData));
        assert_eq!(timeline.items()[0].translated.as_ref(), "既有合成譯文");
    }
}
