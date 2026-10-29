//! One bounded RAM display history fed by current-session snapshots, shared by both views.
//! Historical rows never become backend work or alignment evidence.
use crate::{
    ai::speech::StreamIdentity,
    domain::DisplayMode,
    subtitle::timeline::{Stability, TranscriptTimeline},
};
use std::sync::Arc;

pub const MAX_SEARCH_BYTES: usize = 256;
#[derive(Clone, PartialEq, Eq)]
pub struct DisplayRow {
    pub identity: StreamIdentity,
    historical: bool,
    pub sequence: u64,
    pub utterance: u64,
    pub timestamp_us: Option<u64>,
    pub source: Box<str>,
    pub translation: Box<str>,
    pub stability: Stability,
    pub origin: crate::resolver::contracts::SubtitleSource,
}
pub struct TimelineSnapshot {
    identity: StreamIdentity,
    version: u64,
    rows: Vec<DisplayRow>,
}
impl TimelineSnapshot {
    pub(crate) fn latest_origin(&self) -> Option<crate::resolver::contracts::SubtitleSource> {
        self.rows.last().map(|row| row.origin)
    }
    pub fn from_timeline(timeline: &TranscriptTimeline, version: u64) -> Self {
        let now = std::time::Instant::now();
        Self {
            identity: timeline.identity(),
            version,
            rows: timeline
                .items()
                .iter()
                .map(|item| {
                    let display = item.display_at(now);
                    DisplayRow {
                        identity: timeline.identity(),
                        historical: false,
                        sequence: item.source.sequence,
                        utterance: item.source.utterance,
                        timestamp_us: display.timestamp_us,
                        source: clean(display.original).into(),
                        translation: clean(display.translated).into(),
                        stability: display.stability,
                        origin: display.origin,
                    }
                })
                .collect(),
        }
    }
}
pub struct Presentation {
    pub mode: DisplayMode,
    pub original_in_overlay: bool,
    pub auto_follow: bool,
    identity: Option<StreamIdentity>,
    version: Option<u64>,
    latest_sequence: Option<u64>,
    cleared_through: Option<u64>,
    history: Vec<DisplayRow>,
    clear_epoch: u64,
    search: String,
}
impl Presentation {
    pub fn new(mode: DisplayMode) -> Self {
        Self {
            mode,
            original_in_overlay: false,
            auto_follow: true,
            identity: None,
            version: None,
            latest_sequence: None,
            cleared_through: None,
            history: Vec::new(),
            clear_epoch: 0,
            search: String::new(),
        }
    }
    /// Only the session owner can change identity; a delayed snapshot cannot start a session.
    pub fn select_session(&mut self, identity: StreamIdentity) {
        if self.identity == Some(identity) {
            return;
        }
        self.identity = Some(identity);
        self.version = None;
        self.latest_sequence = None;
        self.cleared_through = None;
    }
    pub fn stop(&mut self) {
        self.identity = None;
        self.version = None;
        self.latest_sequence = None;
        self.cleared_through = None;
    }
    /// User-owned display clearing. Backend jobs and their private context are unchanged.
    pub fn clear_subtitles(&mut self) {
        self.history.clear();
        self.cleared_through = self.latest_sequence;
        self.clear_epoch = self.clear_epoch.saturating_add(1);
        self.search.clear();
        self.auto_follow = true;
    }
    pub(crate) fn clear_epoch(&self) -> u64 {
        self.clear_epoch
    }
    /// Hidden/cleared rows must never be counted as a fresh UI presentation.
    pub(crate) fn latest_caption_visible(&self) -> bool {
        self.history.iter().any(|row| {
            Some(row.identity) == self.identity
                && Some(row.sequence) == self.latest_sequence
                && !row.historical
                && !row.translation.is_empty()
        })
    }
    pub fn apply(&mut self, snapshot: TimelineSnapshot) -> bool {
        self.apply_shared(Arc::new(snapshot))
    }
    pub fn apply_shared(&mut self, snapshot: Arc<TimelineSnapshot>) -> bool {
        if Some(snapshot.identity) != self.identity
            || self
                .version
                .is_some_and(|previous| snapshot.version <= previous)
        {
            return false;
        }
        self.version = Some(snapshot.version);
        if let Some(sequence) = snapshot.rows.last().map(|row| row.sequence) {
            self.latest_sequence = Some(
                self.latest_sequence
                    .map_or(sequence, |previous| previous.max(sequence)),
            );
        }
        for row in &snapshot.rows {
            if Some(row.identity) != self.identity
                || self
                    .cleared_through
                    .is_some_and(|sequence| row.sequence <= sequence)
            {
                continue;
            }
            if let Some(existing) = self.history.iter_mut().find(|existing| {
                existing.identity == row.identity
                    && existing.sequence == row.sequence
                    && existing.utterance == row.utterance
            }) {
                if row.translation.is_empty() && !existing.translation.is_empty() {
                    // A withdrawn live source must not erase what the user read.
                    // Preserve the prior pair as history, never as a live adoption.
                    existing.historical = true;
                } else if existing != row {
                    *existing = row.clone();
                }
                continue;
            }
            if self.history.len() == crate::subtitle::timeline::MAX_TIMELINE_ITEMS {
                if self.history.first().is_some_and(|oldest| {
                    oldest.identity == row.identity && row.sequence <= oldest.sequence
                }) {
                    continue;
                }
                self.history.remove(0);
            }
            let position = self
                .history
                .iter()
                .position(|existing| {
                    existing.identity == row.identity && existing.sequence > row.sequence
                })
                .unwrap_or(self.history.len());
            self.history.insert(position, row.clone());
        }
        true
    }
    pub fn identity(&self) -> Option<StreamIdentity> {
        self.identity
    }
    pub(crate) fn is_historical(&self, row: &DisplayRow) -> bool {
        row.historical || Some(row.identity) != self.identity
    }
    pub fn searching(&self) -> bool {
        !self.search.is_empty()
    }
    pub fn set_search(&mut self, query: &str) -> bool {
        if query.len() > MAX_SEARCH_BYTES || query.chars().any(char::is_control) {
            return false;
        }
        self.search = query.to_lowercase();
        if !query.is_empty() {
            self.auto_follow = false;
        }
        true
    }
    pub fn return_to_live(&mut self) {
        self.search.clear();
        self.auto_follow = true;
    }
    pub fn rows(&self) -> impl Iterator<Item = &DisplayRow> {
        self.history.iter().filter(|row| {
            self.search.is_empty()
                || row.source.to_lowercase().contains(&self.search)
                || row.translation.to_lowercase().contains(&self.search)
        })
    }
    pub fn side_text(&self) -> String {
        super::history::SideHistory::from_view(self).text
    }
    /// SideChat search/scrollback never changes the current overlay subtitle.
    pub fn overlay_text(&self) -> String {
        let Some(row) = self
            .history
            .iter()
            .rev()
            .find(|row| !row.translation.is_empty())
            .or_else(|| self.history.last())
        else {
            return String::new();
        };
        let text = if self.original_in_overlay && !row.source.is_empty() {
            // Keep zh-TW visible first when the renderer has a small line budget.
            format!("{}\n{}", row.translation, row.source)
        } else {
            row.translation.to_string()
        };
        if self.is_historical(row) {
            format!("（先前字幕）{text}")
        } else {
            text
        }
    }
}
fn clean(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ai::speech::{TranscriptEvent, TranscriptKind},
        ai::translation::{TranslationPass, TranslationUpdate},
        audio::capture::SessionId,
    };
    fn identity(id: u64) -> StreamIdentity {
        StreamIdentity {
            session: SessionId(id),
            generation: 1,
        }
    }
    fn timeline(id: u64, text: &str) -> TranscriptTimeline {
        let mut timeline = TranscriptTimeline::new(identity(id));
        timeline
            .ingest(TranscriptEvent {
                identity: identity(id),
                utterance: 1,
                sequence: 1,
                revision: 1,
                start_us: Some(1_234_000),
                duration_us: None,
                language: None,
                kind: TranscriptKind::Partial,
                text: text.into(),
            })
            .unwrap();
        timeline
            .apply(&TranslationUpdate {
                job_id: 0,
                identity: identity(id),
                utterance: 1,
                source_revision: 1,
                pass: TranslationPass::Draft,
                revision: 1,
                text: "測試翻譯".into(),
                stability: Stability::Tentative,
                stable_prefix_bytes: 0,
                complete: false,
            })
            .unwrap();
        timeline
    }
    #[test]
    fn mode_and_search_share_one_snapshot_and_do_not_change_overlay() {
        let mut view = Presentation::new(DisplayMode::SideChat);
        view.select_session(identity(1));
        assert!(view.apply(TimelineSnapshot::from_timeline(&timeline(1, "source"), 1)));
        for mode in [
            DisplayMode::SideChat,
            DisplayMode::BottomSubtitle,
            DisplayMode::Dual,
        ] {
            view.mode = mode;
            assert_eq!(view.rows().count(), 1);
            assert_eq!(view.overlay_text(), "測試翻譯");
        }
        assert!(view.set_search("missing"));
        assert_eq!(view.rows().count(), 0);
        assert_eq!(view.overlay_text(), "測試翻譯");
        assert!(!view.auto_follow);
        view.return_to_live();
        assert_eq!(view.rows().count(), 1);
        assert!(view.auto_follow);
        view.original_in_overlay = true;
        assert_eq!(view.overlay_text(), "測試翻譯\nsource");
    }
    #[test]
    fn source_switch_stop_retains_history_but_old_snapshots_cannot_mutate_it() {
        let mut view = Presentation::new(DisplayMode::Dual);
        view.select_session(identity(1));
        assert!(view.apply(TimelineSnapshot::from_timeline(&timeline(1, "source"), 2)));
        assert!(!view.apply(TimelineSnapshot::from_timeline(&timeline(1, "older"), 1)));
        view.select_session(identity(2));
        assert_eq!(view.rows().count(), 1);
        assert!(!view.latest_caption_visible());
        assert!(view.side_text().contains("先前字幕"));
        assert!(!view.apply(TimelineSnapshot::from_timeline(
            &timeline(1, "old session"),
            3
        )));
        assert!(view.apply(TimelineSnapshot::from_timeline(&timeline(2, "new"), 1)));
        view.stop();
        assert_eq!(view.rows().count(), 2);
        assert!(!view.apply(TimelineSnapshot::from_timeline(&timeline(2, "late"), 2)));
        assert!(!view.side_text().contains("late"));
        view.clear_subtitles();
        assert_eq!(view.rows().count(), 0);
    }
    #[test]
    fn search_is_bounded_and_source_controls_are_never_rendered() {
        let mut view = Presentation::new(DisplayMode::SideChat);
        view.select_session(identity(1));
        view.apply(TimelineSnapshot::from_timeline(
            &timeline(1, "SOURCE\0\u{1b}"),
            1,
        ));
        assert!(view.set_search("source"));
        assert_eq!(view.rows().count(), 1);
        assert!(!view.set_search(&"x".repeat(MAX_SEARCH_BYTES + 1)));
        assert!(!view.set_search("bad\nquery"));
        assert_eq!(view.rows().count(), 1);
        assert!(!view.side_text().contains('\0'));
        assert!(view.side_text().contains("00:01.234"));
    }
    #[test]
    fn phase11_untranslated_tail_keeps_readable_caption_in_both_views_without_losing_source_history()
     {
        let mut timeline = timeline(1, "previous source");
        let pending = TranscriptEvent {
            identity: identity(1),
            utterance: 2,
            sequence: 2,
            revision: 1,
            start_us: None,
            duration_us: None,
            language: None,
            kind: TranscriptKind::Partial,
            text: "pending synthetic source".into(),
        };
        timeline.ingest(pending).unwrap();
        let mut view = Presentation::new(DisplayMode::Dual);
        view.select_session(identity(1));
        view.apply(TimelineSnapshot::from_timeline(&timeline, 2));
        assert_eq!(view.rows().count(), 2); // Source stays in the one shared snapshot.
        assert_eq!(view.overlay_text(), "測試翻譯");
        assert!(view.side_text().contains("測試翻譯"));
        assert!(view.side_text().contains("pending synthetic source"));
        view.original_in_overlay = true;
        assert_eq!(view.overlay_text(), "測試翻譯\nprevious source");
        view.set_search("pending");
        assert!(view.side_text().contains("pending synthetic source"));
        assert_eq!(view.overlay_text(), "測試翻譯\nprevious source");
        view.return_to_live();
        timeline
            .apply(&TranslationUpdate {
                identity: identity(1),
                utterance: 2,
                source_revision: 1,
                job_id: 1,
                pass: TranslationPass::Draft,
                revision: 1,
                text: "新段合成中文".into(),
                stability: Stability::Tentative,
                stable_prefix_bytes: 0,
                complete: false,
            })
            .unwrap();
        view.apply(TimelineSnapshot::from_timeline(&timeline, 3));
        assert_eq!(
            view.overlay_text(),
            "新段合成中文\npending synthetic source"
        );
        assert!(view.side_text().contains("pending synthetic source"));
        view.stop();
        assert!(view.overlay_text().contains("先前字幕"));
        view.clear_subtitles();
        assert!(view.overlay_text().is_empty());
        assert!(view.side_text().is_empty());
    }
    #[test]
    fn phase11_manual_clear_does_not_stop_session_or_allow_snapshot_refill() {
        let mut view = Presentation::new(DisplayMode::Dual);
        let mut source = timeline(1, "original fixture");
        view.select_session(identity(1));
        view.apply(TimelineSnapshot::from_timeline(&source, 1));
        let epoch = view.clear_epoch();
        view.clear_subtitles();
        assert_eq!(view.identity(), Some(identity(1)));
        assert!(view.clear_epoch() > epoch);
        assert!(view.overlay_text().is_empty());
        assert!(!view.latest_caption_visible());
        assert!(!view.apply(TimelineSnapshot::from_timeline(&source, 1)));
        assert!(view.apply(TimelineSnapshot::from_timeline(&source, 2)));
        assert_eq!(view.rows().count(), 0);
        source
            .ingest(TranscriptEvent {
                identity: identity(1),
                utterance: 2,
                sequence: 2,
                revision: 1,
                start_us: None,
                duration_us: None,
                language: None,
                kind: TranscriptKind::Partial,
                text: "next fixture".into(),
            })
            .unwrap();
        source
            .apply(&TranslationUpdate {
                identity: identity(1),
                utterance: 2,
                source_revision: 1,
                job_id: 2,
                pass: TranslationPass::Draft,
                revision: 1,
                text: "新段合成譯文".into(),
                stability: Stability::Tentative,
                stable_prefix_bytes: 0,
                complete: false,
            })
            .unwrap();
        view.apply(TimelineSnapshot::from_timeline(&source, 3));
        assert_eq!(view.rows().count(), 1);
        assert_eq!(view.overlay_text(), "新段合成譯文");
        assert!(view.latest_caption_visible());
        view.clear_subtitles();
        view.stop();
        view.select_session(identity(2));
        view.apply(TimelineSnapshot::from_timeline(
            &timeline(2, "new session fixture"),
            1,
        ));
        assert_eq!(view.rows().count(), 1);
    }
    #[test]
    fn phase11_retained_history_is_bounded_and_repeated_session_row_ids_are_distinct() {
        let mut view = Presentation::new(DisplayMode::Dual);
        for session in 1..=80 {
            view.select_session(identity(session));
            view.apply(TimelineSnapshot::from_timeline(
                &timeline(session, &format!("fixture {session}")),
                1,
            ));
            view.stop();
        }
        assert_eq!(
            view.rows().count(),
            crate::subtitle::timeline::MAX_TIMELINE_ITEMS
        );
        assert_eq!(view.rows().next().unwrap().identity, identity(17));
        assert_eq!(view.rows().last().unwrap().identity, identity(80));
        assert!(!view.apply(TimelineSnapshot::from_timeline(&timeline(80, "late"), 3)));
        view.select_session(identity(81));
        view.apply(TimelineSnapshot::from_timeline(&timeline(81, "new"), 1));
        view.apply(TimelineSnapshot::from_timeline(&timeline(81, "updated"), 2));
        assert_eq!(view.rows().count(), 64);
        assert_eq!(view.rows().last().unwrap().source.as_ref(), "updated");
        assert!(!view.apply(TimelineSnapshot::from_timeline(&timeline(80, "late"), 100)));
        assert_eq!(view.rows().last().unwrap().source.as_ref(), "updated");
    }
    #[test]
    fn phase11_withdrawn_display_text_is_retained_as_history_until_fresh_translation_arrives() {
        let mut view = Presentation::new(DisplayMode::Dual);
        view.select_session(identity(1));
        let old = timeline(1, "paired old source");
        view.apply(TimelineSnapshot::from_timeline(&old, 1));
        let mut withdrawn = TimelineSnapshot::from_timeline(&old, 2);
        withdrawn.rows[0].source = "replacement without translation".into();
        withdrawn.rows[0].translation = "".into();
        view.apply(withdrawn);
        assert!(view.side_text().contains("paired old source"));
        assert!(!view.side_text().contains("replacement without translation"));
        assert!(view.overlay_text().contains("測試翻譯"));
        assert!(view.overlay_text().starts_with("（先前字幕）"));
        assert!(!view.latest_caption_visible());
        let mut fresh = TimelineSnapshot::from_timeline(&old, 3);
        fresh.rows[0].source = "replacement with translation".into();
        fresh.rows[0].translation = "新的有效譯文".into();
        view.apply(fresh);
        assert_eq!(view.overlay_text(), "新的有效譯文");
        assert!(view.side_text().contains("replacement with translation"));
        assert!(view.latest_caption_visible());
    }
}
