//! Provider-specific streaming setup. Shared domain types never expose vendor JSON.
use crate::{
    ai::speech::SpeechLanguage,
    domain::{SourceLanguage, TranslationQuality},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SpeechProvider {
    OpenAi,
    Gemini,
}
/// A Live model is separate from the vendor used for credentials and Prepared work.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LiveSpeechModel {
    GptLiveTranscribe,
    GeminiTranscribeLive,
    GeminiLiveTranslate,
}
impl LiveSpeechModel {
    pub const fn default_for(provider: SpeechProvider) -> Self {
        match provider {
            SpeechProvider::OpenAi => Self::GptLiveTranscribe,
            SpeechProvider::Gemini => Self::GeminiTranscribeLive,
        }
    }
    pub const fn provider(self) -> SpeechProvider {
        match self {
            Self::GptLiveTranscribe => SpeechProvider::OpenAi,
            _ => SpeechProvider::Gemini,
        }
    }
    pub const fn model(self) -> &'static str {
        match self {
            Self::GptLiveTranscribe => "gpt-live-transcribe",
            Self::GeminiTranscribeLive => "gemini-3.5-transcribe-live",
            Self::GeminiLiveTranslate => "gemini-3.5-live-translate-preview",
        }
    }
    pub const fn native_translation(self) -> bool {
        matches!(self, Self::GeminiLiveTranslate)
    }
    /// App-supported constrained five-language detection, not the vendor's full language set.
    pub const fn supports_auto_detection(self) -> bool {
        matches!(self, Self::GeminiTranscribeLive)
    }
    pub fn validate_live(
        self,
        source: SourceLanguage,
        quality: TranslationQuality,
    ) -> Result<(), crate::ai::speech::SpeechError> {
        if source == SourceLanguage::Auto && !self.supports_auto_detection() {
            return Err(crate::ai::speech::SpeechError::ExplicitLanguageRequired);
        }
        if self.native_translation() && quality == TranslationQuality::High {
            return Err(crate::ai::speech::SpeechError::NativeHighUnavailable);
        }
        Ok(())
    }
    pub fn plan(self, source: SourceLanguage, quality: TranslationQuality) -> ProviderPlan {
        let mut plan = self.provider().plan(source);
        plan.model = self.model();
        if self == Self::GptLiveTranscribe {
            plan.setup["session"]["audio"]["input"]["transcription"]["delay"] =
                json!(match quality {
                    TranslationQuality::Fast => "minimal",
                    TranslationQuality::Standard => "low",
                    TranslationQuality::High => "medium",
                });
        } else if self.native_translation() {
            plan.setup = json!({"setup":{"model":"models/gemini-3.5-live-translate-preview",
                "generationConfig":{"responseModalities":["AUDIO"],"translationConfig":{"targetLanguageCode":"zh-Hant","echoTargetLanguage":false}},
                "inputAudioTranscription":{},"outputAudioTranscription":{}}});
        }
        plan
    }
}

pub struct ProviderPlan {
    pub provider: SpeechProvider,
    pub model: &'static str,
    pub pcm_sample_rate: u32,
    pub setup: Value,
}
impl SpeechProvider {
    pub fn plan(self, source: SourceLanguage) -> ProviderPlan {
        let languages: Vec<_> = SpeechLanguage::selected(source)
            .map_or_else(|| SpeechLanguage::ALL.to_vec(), |language| vec![language]);
        match self {
            Self::OpenAi => ProviderPlan {
                provider: self,
                model: "gpt-live-transcribe",
                pcm_sample_rate: 24000,
                setup: json!({"type":"session.update", "session":{"type":"transcription", "audio":{"input":{"format":{"type":"audio/pcm","rate":24000}, "transcription":{"model":"gpt-live-transcribe","languages":languages.iter().map(|language| language.iso_code()).collect::<Vec<_>>()},"turn_detection":null}}}}),
            },
            Self::Gemini => ProviderPlan {
                provider: self,
                model: "gemini-3.5-transcribe-live",
                pcm_sample_rate: 16000,
                // Empty hints enable actual service detection. The client still
                // validates every reported language against the five-language enum.
                setup: json!({"setup":{"model":"models/gemini-3.5-transcribe-live", "generationConfig":{"responseModalities":["TEXT"]}, "inputAudioTranscription":{"languageCodes":SpeechLanguage::selected(source).into_iter().map(|language| language.locale()).collect::<Vec<_>>(), "mode":"VERBATIM"}, "realtimeInputConfig":{"automaticActivityDetection":{"disabled":false}}}}),
            },
        }
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;
    #[test]
    fn phase11_live_model_plans_use_requested_models_target_and_real_openai_delay_settings() {
        assert!(LiveSpeechModel::GeminiTranscribeLive.supports_auto_detection());
        assert!(!LiveSpeechModel::GeminiLiveTranslate.supports_auto_detection());
        assert!(!LiveSpeechModel::GptLiveTranscribe.supports_auto_detection());
        for (quality, delay) in [
            (TranslationQuality::Fast, "minimal"),
            (TranslationQuality::Standard, "low"),
            (TranslationQuality::High, "medium"),
        ] {
            let plan = LiveSpeechModel::GptLiveTranscribe.plan(SourceLanguage::English, quality);
            assert_eq!(plan.model, "gpt-live-transcribe");
            assert_eq!(
                plan.setup["session"]["audio"]["input"]["transcription"]["delay"],
                delay
            );
            assert!(plan.setup["session"]["audio"]["input"]["turn_detection"].is_null());
        }
        let native = LiveSpeechModel::GeminiLiveTranslate
            .plan(SourceLanguage::English, TranslationQuality::Standard);
        assert_eq!(native.model, "gemini-3.5-live-translate-preview");
        assert_eq!(native.pcm_sample_rate, 16000);
        assert_eq!(
            native.setup["setup"]["generationConfig"]["translationConfig"]["targetLanguageCode"],
            "zh-Hant"
        );
        assert_eq!(
            native.setup["setup"]["generationConfig"]["responseModalities"],
            json!(["AUDIO"])
        );
        assert!(native.setup["setup"]["inputAudioTranscription"].is_object());
        assert!(native.setup["setup"]["outputAudioTranscription"].is_object());
        assert!(native.setup["setup"].get("systemInstruction").is_none());
        assert!(
            LiveSpeechModel::GeminiLiveTranslate
                .validate_live(SourceLanguage::Auto, TranslationQuality::Standard)
                .is_err()
        );
        assert!(
            LiveSpeechModel::GptLiveTranscribe
                .validate_live(SourceLanguage::Auto, TranslationQuality::Standard)
                .is_err()
        );
        assert!(
            LiveSpeechModel::GeminiTranscribeLive
                .validate_live(SourceLanguage::Auto, TranslationQuality::High)
                .is_ok()
        );
    }
}
