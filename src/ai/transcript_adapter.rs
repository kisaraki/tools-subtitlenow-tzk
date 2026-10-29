//! Bounded provider wire adapters. OpenAI uses submitted-audio correlation;
//! Gemini uses ordered server utterances with unknown capture timing. Response
//! arrival time is never a media/capture timestamp. No network or content logs.
use super::{
    provider::SpeechProvider,
    speech::{
        EVENT_QUEUE_ITEMS, MAX_TRANSCRIPT_BYTES, SpeechError, SpeechLanguage, StreamIdentity,
        TranscriptEvent, TranscriptKind,
    },
};
use serde_json::Value;

/// A wire message is capped before JSON allocation/parsing. Transport must also
/// enforce this limit before buffering a full WebSocket frame.
pub const MAX_WIRE_BYTES: usize = 64 * 1024;
pub(super) const MAX_ITEM_KEY_BYTES: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct UtteranceContext {
    pub utterance: u64,
    pub sequence: u64,
    pub start_us: Option<u64>,
    pub duration_us: Option<u64>,
    pub language: Option<SpeechLanguage>,
}
struct Item {
    key: Box<str>,
    context: UtteranceContext,
    revision: u64,
    text: Box<str>,
    finalized: bool,
}
pub struct TranscriptAdapter {
    provider: SpeechProvider,
    identity: StreamIdentity,
    items: Vec<Item>,
}
impl TranscriptAdapter {
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.items.iter().any(|item| item.key.as_ref() == key)
    }
    pub fn new(provider: SpeechProvider, identity: StreamIdentity) -> Self {
        Self {
            provider,
            identity,
            items: Vec::new(),
        }
    }
    /// OpenAI keys correlate submitted audio. Gemini keys identify ordered server
    /// utterances only; uncorrelated capture timing must remain None.
    pub fn register(&mut self, key: &str, context: UtteranceContext) -> Result<(), SpeechError> {
        if key.is_empty()
            || key.len() > MAX_ITEM_KEY_BYTES
            || context
                .start_us
                .zip(context.duration_us)
                .is_some_and(|(start, duration)| start.checked_add(duration).is_none())
        {
            return Err(SpeechError::InvalidData);
        }
        if let Some(item) = self.items.iter().find(|item| item.key.as_ref() == key) {
            return if item.context == context {
                Ok(())
            } else {
                Err(SpeechError::InvalidData)
            };
        }
        if self.items.len() >= EVENT_QUEUE_ITEMS {
            return Err(SpeechError::Overloaded);
        }
        if self.items.iter().any(|item| {
            item.context.utterance == context.utterance || item.context.sequence == context.sequence
        }) {
            return Err(SpeechError::InvalidData);
        }
        self.items.push(Item {
            key: key.into(),
            context,
            revision: 0,
            text: "".into(),
            finalized: false,
        });
        Ok(())
    }
    /// Owner retires finalized correlations after downstream acceptance. Late
    /// responses then fail correlation instead of silently becoming a new turn.
    pub fn retire(&mut self, key: &str) -> Result<(), SpeechError> {
        let index = self
            .items
            .iter()
            .position(|item| item.key.as_ref() == key && item.finalized)
            .ok_or(SpeechError::InvalidData)?;
        self.items.remove(index);
        Ok(())
    }
    pub fn set_duration(&mut self, key: &str, duration_us: u64) -> Result<(), SpeechError> {
        let item = self
            .items
            .iter_mut()
            .find(|item| item.key.as_ref() == key)
            .ok_or(SpeechError::InvalidData)?;
        if item.finalized
            || item
                .context
                .start_us
                .is_some_and(|start| start.checked_add(duration_us).is_none())
        {
            return Err(SpeechError::InvalidData);
        }
        item.context.duration_us = Some(duration_us);
        Ok(())
    }
    pub fn decode(
        &mut self,
        payload: &[u8],
        gemini_key: Option<&str>,
    ) -> Result<Vec<TranscriptEvent>, SpeechError> {
        if payload.len() > MAX_WIRE_BYTES {
            return Err(SpeechError::InvalidData);
        }
        let message: Value =
            serde_json::from_slice(payload).map_err(|_| SpeechError::InvalidData)?;
        if !message.is_object() {
            return Err(SpeechError::InvalidData);
        }
        if let Some(error) = message.get("error") {
            return Err(match error.get("code").and_then(Value::as_str) {
                Some("invalid_api_key" | "authentication_error" | "permission_denied") => {
                    SpeechError::Credential
                }
                Some("rate_limit_exceeded") => SpeechError::RateLimited,
                _ => SpeechError::InvalidData,
            });
        }
        match self.provider {
            SpeechProvider::OpenAi => {
                let Some(kind) = message.get("type").and_then(Value::as_str) else {
                    return Err(SpeechError::InvalidData);
                };
                let (field, kind, append) = match kind {
                    "conversation.item.input_audio_transcription.delta" => {
                        ("delta", TranscriptKind::Partial, true)
                    }
                    "conversation.item.input_audio_transcription.completed" => {
                        ("transcript", TranscriptKind::Final, false)
                    }
                    "conversation.item.input_audio_transcription.failed" => {
                        return Err(SpeechError::InvalidData);
                    }
                    _ => return Ok(Vec::new()),
                };
                if message.get("content_index").and_then(Value::as_u64) != Some(0) {
                    return Err(SpeechError::InvalidData);
                }
                let key = message
                    .get("item_id")
                    .and_then(Value::as_str)
                    .ok_or(SpeechError::InvalidData)?;
                let text = message
                    .get(field)
                    .and_then(Value::as_str)
                    .ok_or(SpeechError::InvalidData)?;
                Ok(self.update(key, kind, text, append)?.into_iter().collect())
            }
            SpeechProvider::Gemini => {
                let Some(content) = message.get("serverContent") else {
                    return Ok(Vec::new());
                };
                if !content.is_object() {
                    return Err(SpeechError::InvalidData);
                }
                let mut result = Vec::with_capacity(2);
                // Validate both branches before mutating revision state.
                let updates: Result<Vec<_>, _> = [
                    ("interimInputTranscription", TranscriptKind::Partial),
                    ("inputTranscription", TranscriptKind::Final),
                ]
                .into_iter()
                .filter_map(|(field, kind)| {
                    content.get(field).map(|value| {
                        value
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|text| text.len() <= MAX_TRANSCRIPT_BYTES)
                            .map(|text| (kind, text))
                            .ok_or(SpeechError::InvalidData)
                    })
                })
                .collect();
                let updates = updates?;
                if updates.is_empty() {
                    return Ok(result);
                }
                let key = gemini_key.ok_or(SpeechError::InvalidData)?;
                for (kind, text) in updates {
                    if let Some(event) = self.update(key, kind, text, false)? {
                        result.push(event);
                    }
                }
                Ok(result)
            }
        }
    }
    fn update(
        &mut self,
        key: &str,
        kind: TranscriptKind,
        text: &str,
        append: bool,
    ) -> Result<Option<TranscriptEvent>, SpeechError> {
        let item = self
            .items
            .iter_mut()
            .find(|item| item.key.as_ref() == key)
            .ok_or(SpeechError::InvalidData)?;
        if item.finalized {
            return Ok(None);
        }
        let new_len = if append {
            item.text.len().checked_add(text.len())
        } else {
            Some(text.len())
        }
        .filter(|len| *len <= MAX_TRANSCRIPT_BYTES)
        .ok_or(SpeechError::InvalidData)?;
        let revision = item
            .revision
            .checked_add(1)
            .ok_or(SpeechError::InvalidData)?;
        let mut replacement = String::with_capacity(new_len);
        if append {
            replacement.push_str(&item.text);
        }
        replacement.push_str(text);
        item.text = replacement.into_boxed_str();
        item.revision = revision;
        item.finalized = kind == TranscriptKind::Final;
        let context = item.context;
        Ok(Some(TranscriptEvent {
            identity: self.identity,
            utterance: context.utterance,
            revision,
            sequence: context.sequence,
            start_us: context.start_us,
            duration_us: context.duration_us,
            language: context.language,
            kind,
            text: item.text.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ai::speech::TranscriptLedger, audio::capture::SessionId};
    use serde_json::json;
    fn identity() -> StreamIdentity {
        StreamIdentity {
            session: SessionId(41),
            generation: 3,
        }
    }
    fn context(sequence: u64) -> UtteranceContext {
        UtteranceContext {
            utterance: sequence,
            sequence,
            start_us: Some(sequence * 100000),
            duration_us: Some(100000),
            language: None,
        }
    }
    fn openai(key: &str, final_text: bool, text: &str) -> Vec<u8> {
        let kind = if final_text {
            "conversation.item.input_audio_transcription.completed"
        } else {
            "conversation.item.input_audio_transcription.delta"
        };
        let mut value = json!({"type":kind,"item_id":key,"content_index":0});
        value[if final_text { "transcript" } else { "delta" }] = text.into();
        serde_json::to_vec(&value).unwrap()
    }
    #[test]
    fn openai_deltas_accumulate_final_corrects_and_out_of_order_turns_keep_source_order() {
        let mut adapter = TranscriptAdapter::new(SpeechProvider::OpenAi, identity());
        adapter.register("first", context(1)).unwrap();
        adapter.register("second", context(2)).unwrap();
        assert_eq!(
            adapter
                .decode(&openai("first", false, "Test "), None)
                .unwrap()[0]
                .text
                .as_ref(),
            "Test "
        );
        assert_eq!(
            adapter
                .decode(&openai("first", false, "partial"), None)
                .unwrap()[0]
                .text
                .as_ref(),
            "Test partial"
        );
        let mut ledger = TranscriptLedger::new(identity());
        for event in adapter
            .decode(&openai("second", true, "Second final"), None)
            .unwrap()
        {
            assert!(ledger.apply(event));
        }
        for event in adapter
            .decode(&openai("first", true, "Corrected final"), None)
            .unwrap()
        {
            assert!(ledger.apply(event));
        }
        assert_eq!(ledger.entries()[0].text.as_ref(), "Corrected final");
        assert_eq!(ledger.entries()[1].sequence, 2);
        assert!(
            adapter
                .decode(&openai("first", false, "late"), None)
                .unwrap()
                .is_empty()
        );
        adapter.retire("first").unwrap();
        assert!(
            adapter
                .decode(&openai("first", true, "late"), None)
                .is_err()
        );
    }
    #[test]
    fn gemini_interim_replaces_instead_of_appending_and_final_uses_explicit_correlation() {
        let mut adapter = TranscriptAdapter::new(SpeechProvider::Gemini, identity());
        adapter.register("audio-turn", context(1)).unwrap();
        let interim = |text| {
            serde_json::to_vec(
                &json!({"serverContent":{"interimInputTranscription":{"text":text}}}),
            )
            .unwrap()
        };
        assert!(adapter.decode(&interim("long hypothesis"), None).is_err());
        assert_eq!(
            adapter
                .decode(&interim("long hypothesis"), Some("audio-turn"))
                .unwrap()[0]
                .text
                .as_ref(),
            "long hypothesis"
        );
        let partial = adapter
            .decode(&interim("short"), Some("audio-turn"))
            .unwrap();
        assert_eq!(partial[0].text.as_ref(), "short");
        assert_eq!(partial[0].revision, 2);
        let final_message = serde_json::to_vec(&json!({"serverContent":{"interimInputTranscription":{"text":"tail"},"inputTranscription":{"text":"final"}}})).unwrap();
        let updates = adapter.decode(&final_message, Some("audio-turn")).unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[1].kind, TranscriptKind::Final);
        assert_eq!(updates[1].identity, identity());
        assert!(updates[1].language.is_none());
    }
    #[test]
    fn unknown_items_oversized_payloads_and_malformed_updates_do_not_mutate_valid_state() {
        let mut adapter = TranscriptAdapter::new(SpeechProvider::OpenAi, identity());
        adapter.register("known", context(1)).unwrap();
        assert!(
            adapter
                .decode(&openai("unknown", false, "private"), None)
                .is_err()
        );
        assert!(
            adapter
                .decode(&vec![b' '; MAX_WIRE_BYTES + 1], None)
                .is_err()
        );
        assert!(
            adapter
                .decode(
                    &openai("known", false, &"x".repeat(MAX_TRANSCRIPT_BYTES + 1)),
                    None
                )
                .is_err()
        );
        assert_eq!(
            adapter
                .decode(&openai("known", false, "valid"), None)
                .unwrap()[0]
                .revision,
            1
        );
        assert!(adapter.decode(b"[]", None).is_err());
        assert!(adapter.decode(br#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"known","content_index":1,"transcript":"wrong"}"#, None).is_err());
    }
    #[test]
    fn correlation_storage_is_bounded_and_never_overwrites_another_source() {
        let mut adapter = TranscriptAdapter::new(SpeechProvider::OpenAi, identity());
        for index in 0..EVENT_QUEUE_ITEMS {
            adapter
                .register(&index.to_string(), context(index as u64))
                .unwrap();
        }
        assert_eq!(
            adapter.register("overflow", context(100)),
            Err(SpeechError::Overloaded)
        );
        assert_eq!(
            adapter.register("0", context(1)),
            Err(SpeechError::InvalidData)
        );
        assert!(adapter.retire("0").is_err());
    }
    #[test]
    fn service_errors_are_typed_and_do_not_expose_payload() {
        let mut adapter = TranscriptAdapter::new(SpeechProvider::OpenAi, identity());
        let error = adapter
            .decode(
                br#"{"error":{"code":"invalid_api_key","message":"PRIVATE_SECRET"}}"#,
                None,
            )
            .err()
            .unwrap();
        assert_eq!(error, SpeechError::Credential);
        assert!(!error.to_string().contains("PRIVATE_SECRET"));
        assert_eq!(
            adapter
                .decode(br#"{"error":{"code":"rate_limit_exceeded"}}"#, None)
                .err(),
            Some(SpeechError::RateLimited)
        );
    }
}
