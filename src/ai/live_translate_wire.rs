//! Gemini Live Translate caption stream. Audio replies are never played or retained.
use super::{
    native_caption::NativeCaption,
    speech::{MAX_TRANSCRIPT_BYTES, SpeechError, SpeechLanguage, StreamIdentity, TranscriptKind},
    translation::QualityPolicy,
};
use crate::domain::{SourceLanguage, TranslationQuality};
use serde_json::Value;
use std::time::Instant;

pub(crate) struct LiveTranslateWire {
    identity: StreamIdentity,
    selected: SpeechLanguage,
    quality: TranslationQuality,
    sequence: u64,
    revision: u64,
    text: String,
    dirty: bool,
    receipt: Instant,
    emitted: Instant,
}
impl LiveTranslateWire {
    pub fn new(
        identity: StreamIdentity,
        language: SourceLanguage,
        quality: TranslationQuality,
    ) -> Result<Self, SpeechError> {
        if quality == TranslationQuality::High {
            return Err(SpeechError::NativeHighUnavailable);
        }
        Ok(Self {
            identity,
            selected: SpeechLanguage::selected(language).ok_or(SpeechError::UnsupportedLanguage)?,
            quality,
            sequence: 1,
            revision: 0,
            text: String::new(),
            dirty: false,
            receipt: Instant::now(),
            emitted: Instant::now(),
        })
    }
    pub fn receive(&mut self, message: &Value) -> Result<Option<NativeCaption>, SpeechError> {
        if !message.is_object() {
            return Err(SpeechError::InvalidData);
        }
        let Some(content) = message.get("serverContent") else {
            return Ok(None);
        };
        if !content.is_object() {
            return Err(SpeechError::InvalidData);
        }
        if let Some(interrupted) = content.get("interrupted")
            && interrupted.as_bool().ok_or(SpeechError::InvalidData)?
        {
            return Err(SpeechError::GenerationInterrupted);
        }
        if let Some(input) = content.get("inputTranscription") {
            if !input.is_object() {
                return Err(SpeechError::InvalidData);
            }
            if let Some(text) = input.get("text")
                && text
                    .as_str()
                    .is_none_or(|text| text.len() > MAX_TRANSCRIPT_BYTES)
            {
                return Err(SpeechError::InvalidData);
            }
            if let Some(code) = input.get("languageCode") {
                let code = code.as_str().ok_or(SpeechError::InvalidData)?;
                if SpeechLanguage::from_code(code) != Some(self.selected) {
                    return Err(SpeechError::UnsupportedLanguage);
                }
            }
        }
        let mut finished = false;
        for flag in ["turnComplete", "generationComplete"] {
            if let Some(value) = content.get(flag) {
                finished |= value.as_bool().ok_or(SpeechError::InvalidData)?;
            }
        }
        if let Some(output) = content.get("outputTranscription") {
            if !output.is_object() {
                return Err(SpeechError::InvalidData);
            }
            if let Some(code) = output.get("languageCode") {
                let code = code.as_str().ok_or(SpeechError::InvalidData)?;
                if !matches!(
                    code.to_ascii_lowercase().as_str(),
                    "zh" | "zh-hant" | "zh-tw" | "zh-hans" | "zh-cn"
                ) {
                    return Err(SpeechError::UnsupportedLanguage);
                }
            }
            if let Some(value) = output.get("finished") {
                finished |= value.as_bool().ok_or(SpeechError::InvalidData)?;
            }
            if let Some(value) = output.get("text") {
                let text = value.as_str().ok_or(SpeechError::InvalidData)?;
                if self
                    .text
                    .len()
                    .checked_add(text.len())
                    .is_none_or(|size| size > MAX_TRANSCRIPT_BYTES)
                {
                    return Err(SpeechError::InvalidData);
                }
                if !text.is_empty() {
                    self.text.push_str(text);
                    self.dirty = true;
                    self.receipt = Instant::now();
                }
            }
        }
        // modelTurn.inlineData is intentionally ignored within the transport's frame cap.
        // No base64 decode, PCM allocation, speaker output, or persistence occurs here.
        if finished && !self.text.is_empty() {
            self.receipt = Instant::now();
            return self.emit(true);
        }
        self.flush()
    }
    pub fn flush(&mut self) -> Result<Option<NativeCaption>, SpeechError> {
        self.flush_at(Instant::now())
    }
    fn flush_at(&mut self, now: Instant) -> Result<Option<NativeCaption>, SpeechError> {
        if !self.dirty
            || now.saturating_duration_since(self.emitted)
                < QualityPolicy::for_quality(self.quality).debounce
        {
            return Ok(None);
        }
        self.emit(false)
    }
    fn emit(&mut self, finished: bool) -> Result<Option<NativeCaption>, SpeechError> {
        if self.text.is_empty() {
            return Ok(None);
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(SpeechError::InvalidData)?;
        let event = NativeCaption {
            identity: self.identity,
            sequence: self.sequence,
            revision: self.revision,
            kind: if finished {
                TranscriptKind::Final
            } else {
                TranscriptKind::Partial
            },
            text: self.text.clone().into(),
            received: self.receipt,
        };
        self.dirty = false;
        self.emitted = Instant::now();
        if finished {
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or(SpeechError::InvalidData)?;
            self.revision = 0;
            self.text.clear();
        }
        Ok(Some(event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::capture::SessionId;
    use serde_json::json;
    fn wire(quality: TranslationQuality) -> LiveTranslateWire {
        LiveTranslateWire::new(
            StreamIdentity {
                session: SessionId(98),
                generation: 2,
            },
            SourceLanguage::English,
            quality,
        )
        .unwrap()
    }
    #[test]
    fn phase11_native_partial_snapshots_continue_without_a_generation_completion() {
        use crate::session::live_translation::LiveTranslation;
        let mut wire = wire(TranslationQuality::Standard);
        let mut flow =
            LiveTranslation::native(wire.identity, TranslationQuality::Standard, None).unwrap();
        for fragment in [" ", "第一句", "下一句", "持續翻譯"] {
            wire.emitted += std::time::Duration::from_secs(60);
            wire.receive(&json!({"serverContent":{"outputTranscription":{"text":fragment}}}))
                .unwrap();
            let caption = wire
                .flush_at(wire.emitted + std::time::Duration::from_secs(1))
                .unwrap()
                .unwrap();
            assert!(
                flow.ingest_native(caption).unwrap(),
                "a complete native snapshot must replace the previous partial"
            );
            let item = flow.timeline().items().last().unwrap();
            assert!(item.display_translation().ends_with(fragment));
            assert_eq!(
                item.display_stability(),
                crate::subtitle::timeline::Stability::Tentative
            );
            assert!(item.source.text.is_empty() && item.source.start_us.is_none());
        }
        assert_eq!(flow.active_requests(), 0);
        assert_eq!(flow.pending_requests(), 0);
    }
    #[test]
    fn phase11_live_translate_fragments_coalesce_and_real_completion_closes_exactly_one_caption() {
        let mut wire = wire(TranslationQuality::Standard);
        wire.emitted += std::time::Duration::from_secs(60);
        wire.receive(&json!({"serverContent":{"inputTranscription":{"text":"synthetic source","languageCode":"en-US"},"outputTranscription":{"text":"计程车","languageCode":"zh-Hant"},"modelTurn":{"parts":[{"inlineData":{"data":"not-decoded-as-audio","mimeType":"audio/pcm;rate=24000"}}]}}})).unwrap();
        wire.receive(&json!({"serverContent":{"outputTranscription":{"text":"软件"}}}))
            .unwrap();
        let event = wire
            .flush_at(wire.emitted + std::time::Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(event.text.as_ref(), "计程车软件");
        assert_eq!(event.kind, TranscriptKind::Partial);
        assert_eq!((event.sequence, event.revision), (1, 1));
        let final_event = wire
            .receive(&json!({"serverContent":{"generationComplete":true}}))
            .unwrap()
            .unwrap();
        assert_eq!((final_event.sequence, final_event.revision), (1, 2));
        assert_eq!(final_event.kind, TranscriptKind::Final);
        assert!(
            wire.receive(&json!({"serverContent":{"turnComplete":true}}))
                .unwrap()
                .is_none()
        );
        let next = wire
            .receive(
                &json!({"serverContent":{"outputTranscription":{"text":"下一段","finished":true}}}),
            )
            .unwrap()
            .unwrap();
        assert_eq!((next.sequence, next.revision), (2, 1));
    }
    #[test]
    fn phase11_live_translate_quality_changes_real_coalescing_and_unsupported_modes_are_explicit() {
        for quality in [TranslationQuality::Fast, TranslationQuality::Standard] {
            let mut wire = wire(quality);
            wire.text = "合成譯文".into();
            wire.dirty = true;
            let now = wire.emitted + std::time::Duration::from_millis(100);
            assert_eq!(
                wire.flush_at(now).unwrap().is_some(),
                quality == TranslationQuality::Fast
            );
        }
        let id = StreamIdentity {
            session: SessionId(1),
            generation: 1,
        };
        assert!(matches!(
            LiveTranslateWire::new(id, SourceLanguage::Auto, TranslationQuality::Standard),
            Err(SpeechError::UnsupportedLanguage)
        ));
        assert!(matches!(
            LiveTranslateWire::new(id, SourceLanguage::English, TranslationQuality::High),
            Err(SpeechError::NativeHighUnavailable)
        ));
    }
    #[test]
    fn phase11_live_translate_rejects_foreign_languages_malformed_flags_and_capacity_without_private_errors()
     {
        for message in [
            json!({"serverContent":{"inputTranscription":{"languageCode":"es","text":"synthetic private text"}}}),
            json!({"serverContent":{"outputTranscription":{"languageCode":"en","text":"synthetic private text"}}}),
            json!({"serverContent":{"turnComplete":"synthetic private text"}}),
            json!({"serverContent":{"interrupted":true,"turnComplete":true}}),
            json!({"serverContent":{"outputTranscription":{"text":"x".repeat(MAX_TRANSCRIPT_BYTES+1)}}}),
        ] {
            let error = wire(TranslationQuality::Standard)
                .receive(&message)
                .err()
                .unwrap();
            assert!(!error.to_string().contains("synthetic private text"));
        }
    }
}
