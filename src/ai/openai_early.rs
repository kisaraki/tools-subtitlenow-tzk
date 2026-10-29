//! Bounded, uncorrelated replies. Only a real commit acknowledgement may release them.
use super::{
    speech::{EVENT_QUEUE_ITEMS, MAX_TRANSCRIPT_BYTES, SpeechError, SpeechLanguage, SpeechQueue},
    transcript_adapter::MAX_ITEM_KEY_BYTES,
};
use crate::domain::SourceLanguage;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

struct EarlyItem {
    key: Box<str>,
    partial: Option<Value>,
    final_message: Option<Value>,
    since: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(key: &str, text: &str, final_message: bool) -> Value {
        let kind = if final_message {
            "conversation.item.input_audio_transcription.completed"
        } else {
            "conversation.item.input_audio_transcription.delta"
        };
        let field = if final_message { "transcript" } else { "delta" };
        json!({"type":kind,"item_id":key,"content_index":0,field:text})
    }
    #[test]
    fn phase11_openai_early_coalesces_only_bounded_fields_and_requires_matching_key() {
        let mut held = EarlyTranscripts::default();
        let mut first = event("one", "first ", false);
        first["unrelated_private_payload"] = json!("private-fixture");
        held.hold(&first, SourceLanguage::English).unwrap();
        held.hold(&event("one", "prefix", false), SourceLanguage::English)
            .unwrap();
        held.hold(&event("one", "final text", true), SourceLanguage::English)
            .unwrap();
        assert!(held.take("unmatched").iter().all(Option::is_none));
        let [partial, final_message] = held.take("one");
        let partial = partial.unwrap();
        assert_eq!(partial["delta"], "first prefix");
        assert!(partial.get("unrelated_private_payload").is_none());
        assert_eq!(final_message.unwrap()["transcript"], "final text");
        assert!(held.items.is_empty());
    }
    #[test]
    fn phase11_openai_early_validates_before_retaining_and_rejects_overflow_without_truncation() {
        let mut held = EarlyTranscripts::default();
        for mut invalid in [
            event("", "text", false),
            event(&"x".repeat(MAX_ITEM_KEY_BYTES + 1), "text", false),
            event("one", &"x".repeat(MAX_TRANSCRIPT_BYTES + 1), false),
        ] {
            assert_eq!(
                held.hold(&invalid, SourceLanguage::English),
                Err(SpeechError::InvalidData)
            );
            invalid["content_index"] = json!(1);
            assert_eq!(
                held.hold(&invalid, SourceLanguage::English),
                Err(SpeechError::InvalidData)
            );
            assert!(held.items.is_empty());
        }
        let mut foreign = event("one", "text", false);
        foreign["languages"] = json!([{"code":"es"}]);
        assert_eq!(
            held.hold(&foreign, SourceLanguage::English),
            Err(SpeechError::UnsupportedLanguage)
        );
        let full = "x".repeat(MAX_TRANSCRIPT_BYTES);
        held.hold(&event("one", &full, false), SourceLanguage::English)
            .unwrap();
        assert_eq!(
            held.hold(&event("one", "x", false), SourceLanguage::English),
            Err(SpeechError::InvalidData)
        );
        assert_eq!(held.take("one")[0].as_ref().unwrap()["delta"], full);
    }
    #[test]
    fn phase11_openai_early_queue_and_original_wait_budget_remain_bounded() {
        let mut held = EarlyTranscripts::default();
        for index in 0..EVENT_QUEUE_ITEMS {
            held.hold(
                &event(&format!("item-{index}"), "x", false),
                SourceLanguage::English,
            )
            .unwrap();
        }
        assert_eq!(
            held.hold(&event("overflow", "x", false), SourceLanguage::English),
            Err(SpeechError::queue_full(
                SpeechQueue::EarlyTranscripts,
                EVENT_QUEUE_ITEMS
            ))
        );
        let budget = Duration::from_secs(8);
        held.items[0].since = Instant::now() - budget;
        held.hold(&event("item-0", "y", false), SourceLanguage::English)
            .unwrap();
        assert!(held.expired(Instant::now(), budget));
        held.take("item-0");
        assert!(!held.expired(Instant::now(), budget));
        held.hold(&event("item-1", "done", true), SourceLanguage::English)
            .unwrap();
        assert_eq!(
            held.hold(&event("item-1", "late", false), SourceLanguage::English),
            Err(SpeechError::InvalidData)
        );
    }
}
#[derive(Default)]
pub(super) struct EarlyTranscripts {
    items: Vec<EarlyItem>,
}
impl EarlyTranscripts {
    pub fn is_event(message: &Value) -> bool {
        matches!(
            message["type"].as_str(),
            Some(
                "conversation.item.input_audio_transcription.delta"
                    | "conversation.item.input_audio_transcription.completed"
            )
        )
    }
    pub fn hold(&mut self, message: &Value, selected: SourceLanguage) -> Result<(), SpeechError> {
        let kind = message["type"].as_str().ok_or(SpeechError::InvalidData)?;
        let final_message = match kind {
            "conversation.item.input_audio_transcription.delta" => false,
            "conversation.item.input_audio_transcription.completed" => true,
            _ => return Err(SpeechError::InvalidData),
        };
        let field = if final_message { "transcript" } else { "delta" };
        let key = message["item_id"]
            .as_str()
            .filter(|key| !key.is_empty() && key.len() <= MAX_ITEM_KEY_BYTES)
            .ok_or(SpeechError::InvalidData)?;
        let text = message[field]
            .as_str()
            .filter(|text| text.len() <= MAX_TRANSCRIPT_BYTES)
            .ok_or(SpeechError::InvalidData)?;
        if message["content_index"].as_u64() != Some(0) {
            return Err(SpeechError::InvalidData);
        }
        if let Some(codes) = message.get("languages") {
            for code in codes.as_array().ok_or(SpeechError::InvalidData)? {
                let language = SpeechLanguage::from_code(
                    code["code"].as_str().ok_or(SpeechError::InvalidData)?,
                )
                .ok_or(SpeechError::UnsupportedLanguage)?;
                if SpeechLanguage::selected(selected).is_some_and(|selected| selected != language) {
                    return Err(SpeechError::UnsupportedLanguage);
                }
            }
        }
        // Copy only bounded transcript fields; never retain unrelated server payload.
        let mut canonical = json!({"type":kind, "item_id":key, "content_index":0, field:text});
        if let Some(codes) = message.get("languages") {
            canonical["languages"] = codes.clone();
        }
        let index = if let Some(index) = self.items.iter().position(|item| item.key.as_ref() == key)
        {
            index
        } else {
            if self.items.len() >= EVENT_QUEUE_ITEMS {
                return Err(SpeechError::queue_full(
                    SpeechQueue::EarlyTranscripts,
                    EVENT_QUEUE_ITEMS,
                ));
            }
            self.items.push(EarlyItem {
                key: key.into(),
                partial: None,
                final_message: None,
                since: Instant::now(),
            });
            self.items.len() - 1
        };
        let item = &mut self.items[index];
        if item.final_message.is_some() {
            return Err(SpeechError::InvalidData);
        }
        if final_message {
            item.final_message = Some(canonical);
        } else {
            if let Some(previous) = &item.partial {
                if previous.get("languages") != canonical.get("languages") {
                    return Err(SpeechError::InvalidData);
                }
                let prefix = previous["delta"].as_str().ok_or(SpeechError::InvalidData)?;
                if prefix
                    .len()
                    .checked_add(text.len())
                    .is_none_or(|len| len > MAX_TRANSCRIPT_BYTES)
                {
                    return Err(SpeechError::InvalidData);
                }
                let mut combined = prefix.to_owned();
                combined.push_str(text);
                canonical["delta"] = Value::String(combined);
            }
            item.partial = Some(canonical);
        }
        // Do not renew `since` on subsequent deltas or final replies.
        Ok(())
    }
    pub fn take(&mut self, key: &str) -> [Option<Value>; 2] {
        let Some(index) = self.items.iter().position(|item| item.key.as_ref() == key) else {
            return [None, None];
        };
        let item = self.items.remove(index);
        [item.partial, item.final_message]
    }
    pub fn expired(&self, now: Instant, budget: Duration) -> bool {
        self.items.iter().any(|item| {
            now.checked_duration_since(item.since)
                .is_some_and(|age| age >= budget)
        })
    }
}
