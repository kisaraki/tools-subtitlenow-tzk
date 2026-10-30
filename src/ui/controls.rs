//! UI commands and bounded source choices. No native handles or cloud clients.
use crate::{
    ai::{credentials::ApiKey, provider::SpeechProvider},
    audio::sessions::{
        ApplicationIcon, AudioSource, AudioSourceId, MAX_SESSIONS, ProcessScope, SIGNAL_THRESHOLD,
        ScanReport, SessionState, normalized_level,
    },
    domain::{SourceLanguage, TranslationQuality},
    error::Result,
    ui::bridge::UiInbox,
};
use std::sync::Arc;

#[derive(Clone, PartialEq, Eq)]
pub struct SourceChoice {
    pub id: AudioSourceId,
    pub pid: u32,
    pub created: u64,
    pub label: String,
    pub icon: Option<Arc<ApplicationIcon>>,
    pub activity: SourceActivity,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SourceActivity {
    pub state: SessionState,
    pub peak_permille: Option<u16>,
    pub signal: Option<bool>,
    pub volume_percent: Option<u8>,
    pub muted: Option<bool>,
}
impl SourceActivity {
    fn from_source(source: &AudioSource) -> Self {
        let peak = source.peak.and_then(normalized_level);
        Self {
            state: source.state,
            peak_permille: peak.map(|peak| (peak * 1000.0).round() as u16),
            signal: if source.state == SessionState::Inactive {
                Some(false)
            } else {
                peak.map(|peak| peak > SIGNAL_THRESHOLD)
            },
            volume_percent: source
                .volume
                .and_then(normalized_level)
                .map(|volume| (volume * 100.0).round() as u8),
            muted: source.muted,
        }
    }
    pub fn text(self, icon_available: bool) -> String {
        let state = match self.state {
            SessionState::Active => "串流執行中",
            SessionState::Inactive => "閒置",
        };
        let signal = match self.signal {
            Some(true) => "有訊號",
            Some(false) => "無訊號",
            None => "訊號未知",
        };
        let peak = self
            .peak_permille
            .map(|peak| format!("{}.{:01}%", peak / 10, peak % 10))
            .unwrap_or_else(|| "未知".into());
        let volume = self
            .volume_percent
            .map(|volume| format!("{volume}%"))
            .unwrap_or_else(|| "未知".into());
        let mute = match self.muted {
            Some(true) => "是",
            Some(false) => "否",
            None => "未知",
        };
        format!(
            "所選工作階段：{state} · {signal} · 峰值 {peak}\r\n音量 {volume} · 靜音 {mute} · 圖示{}",
            if icon_available {
                "已取得"
            } else {
                "無法取得"
            }
        )
    }
}
impl SourceChoice {
    pub fn same_source(&self, other: &Self) -> bool {
        self.id == other.id && self.pid == other.pid && self.created == other.created
    }
    /// Meters/icons can change without rebuilding the native choice list or its indices.
    pub fn same_option(&self, other: &Self) -> bool {
        self.same_source(other) && self.label == other.label
    }
}
const MAX_CHOICE_LABEL_CHARS: usize = 160;
pub fn choices(report: &ScanReport) -> Vec<SourceChoice> {
    report
        .sources
        .iter()
        .filter(|source| {
            source.process_scope == ProcessScope::SingleProcess
                && source.process_id != 0
                && source.process_id != std::process::id()
                && source.process_created.is_some_and(|created| created != 0)
        })
        .take(MAX_SESSIONS)
        .map(|source| {
            let clean = |text: &str| {
                text.chars()
                    .take(MAX_CHOICE_LABEL_CHARS)
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .collect::<String>()
            };
            SourceChoice {
                id: source.id.clone(),
                pid: source.process_id,
                created: source.process_created.unwrap_or_default(),
                icon: source.icon.as_ref().filter(|icon| icon.valid()).cloned(),
                activity: SourceActivity::from_source(source),
                label: format!(
                    "{} · PID {} · {}",
                    clean(source.application_name.as_deref().unwrap_or("名稱未提供")),
                    source.process_id,
                    clean(source.window_title.as_deref().unwrap_or("無可見視窗"))
                ),
            }
        })
        .collect()
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ControlPreferences {
    pub provider: Option<SpeechProvider>,
    pub live_model: Option<crate::ai::provider::LiveSpeechModel>,
    pub language: SourceLanguage,
    pub quality: TranslationQuality,
}
pub struct StartRequest {
    pub source: SourceChoice,
    pub provider: SpeechProvider,
    pub live_model: Option<crate::ai::provider::LiveSpeechModel>,
    pub language: SourceLanguage,
    pub quality: TranslationQuality,
    pub subtitle_source: Option<super::subtitle_source::SubtitleSourceSettings>,
}
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ControlUpdate {
    pub sources: Arc<Vec<SourceChoice>>,
    pub busy: bool,
    pub notice: Option<String>,
}
/// Implementations may spawn work, but never run network or Core Audio on UI callbacks.
pub trait UiController {
    fn start_prepared(
        &mut self,
        _request: StartRequest,
        _key: Option<ApiKey>,
        _source: crate::subtitle::fuzzy_worker::AlignmentSource,
    ) -> Result<UiInbox> {
        Err(crate::ui::bridge::UiBridgeError::InvalidState.into())
    }
    fn poll(&mut self) -> ControlUpdate;
    fn start(&mut self, request: StartRequest, key: Option<ApiKey>) -> Result<UiInbox>;
    fn request_stop(&mut self);
    /// Explicit preference save; no key fields, source identity or content are included.
    fn save_preferences(&mut self, _preferences: ControlPreferences) -> Result<()> {
        Err(crate::ui::bridge::UiBridgeError::InvalidState.into())
    }
    /// Explicit user action only. Verify write/readback before success. No network/capture.
    fn save_key(&mut self, _provider: SpeechProvider, _key: ApiKey) -> Result<()> {
        Err(crate::ai::credentials::CredentialError::Unavailable.into())
    }
    /// Explicit selected-provider check; return no secret material to the view.
    fn check_key(&mut self, _provider: SpeechProvider) -> Result<()> {
        Err(crate::ai::credentials::CredentialError::Unavailable.into())
    }
}
#[cfg(test)]
pub(crate) fn mock_source(pid: u32) -> crate::audio::sessions::AudioSource {
    use crate::audio::sessions::{AudioSource, SessionState};
    AudioSource {
        id: AudioSourceId {
            endpoint: "mock-endpoint".into(),
            instance: format!("mock-{pid}"),
        },
        endpoint_name: None,
        process_id: pid,
        process_created: Some(100),
        process_scope: ProcessScope::SingleProcess,
        application_name: Some("測試程式".into()),
        window_title: Some("測試標題".into()),
        icon: None,
        state: SessionState::Active,
        peak: None,
        volume: None,
        muted: None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_choices_exclude_unverifiable_system_and_cross_process_sources() {
        let mut sources = vec![mock_source(42)];
        for scope in [ProcessScope::SystemSounds, ProcessScope::MultipleProcesses] {
            let mut source = mock_source(43);
            source.process_scope = scope;
            sources.push(source);
        }
        let mut unknown = mock_source(44);
        unknown.process_created = None;
        sources.push(unknown);
        sources.push(mock_source(0));
        sources.push(mock_source(std::process::id()));
        let result = choices(&ScanReport {
            sources,
            ..Default::default()
        });
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pid, 42);
    }
    #[test]
    fn source_choices_are_bounded_and_clean_control_characters() {
        let mut sources = Vec::new();
        for _ in 0..MAX_SESSIONS + 1 {
            let mut source = mock_source(42);
            source.application_name = Some("test\0\u{1b}".into());
            sources.push(source);
        }
        let result = choices(&ScanReport {
            sources,
            ..Default::default()
        });
        assert_eq!(result.len(), MAX_SESSIONS);
        assert!(!result[0].label.contains('\0'));
        assert!(!result[0].label.contains('\u{1b}'));
    }
    #[test]
    fn measured_activity_is_distinct_from_running_and_invalid_levels_are_unknown() {
        let mut source = mock_source(42);
        source.peak = Some(0.0);
        source.volume = Some(0.5);
        source.muted = Some(false);
        let activity = SourceActivity::from_source(&source);
        assert!(
            activity
                .text(false)
                .contains("串流執行中 · 無訊號 · 峰值 0.0%")
        );
        assert!(activity.text(false).contains("音量 50% · 靜音 否"));
        source.peak = Some(f32::NAN);
        source.volume = Some(1.5);
        let activity = SourceActivity::from_source(&source);
        assert_eq!(activity.peak_permille, None);
        assert_eq!(activity.signal, None);
        assert_eq!(activity.volume_percent, None);
        source.peak = Some(0.25);
        assert_eq!(SourceActivity::from_source(&source).signal, Some(true));
    }
    #[test]
    fn icon_shape_alpha_and_identity_remain_bounded_when_metadata_changes() {
        let valid = Arc::new(ApplicationIcon {
            width: 1,
            height: 1,
            bgra: vec![0, 0, 255, 255],
        });
        assert!(valid.valid());
        for invalid in [
            ApplicationIcon {
                width: u32::MAX,
                height: 1,
                bgra: vec![],
            },
            ApplicationIcon {
                width: 1,
                height: 1,
                bgra: vec![255, 0, 0, 1],
            },
            ApplicationIcon {
                width: 1,
                height: 1,
                bgra: vec![],
            },
        ] {
            assert!(!invalid.valid());
        }
        let mut source = mock_source(42);
        source.icon = Some(valid.clone());
        let old = choices(&ScanReport {
            sources: vec![source.clone()],
            ..Default::default()
        })
        .remove(0);
        source.peak = Some(0.7);
        let next = choices(&ScanReport {
            sources: vec![source],
            ..Default::default()
        })
        .remove(0);
        assert!(old.same_option(&next));
        assert!(Arc::ptr_eq(next.icon.as_ref().unwrap(), &valid));
        assert_ne!(old.activity.peak_permille, next.activity.peak_permille);
    }
}
