//! Bounded subtitle lifecycle only; no locations, metadata or cue payloads enter UI.
use crate::ui::content::{ContentDisclosure, ContentTransport};

pub use crate::ui::lyrics::UiLyricsState as UiSubtitleState;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct UiSubtitleStatus {
    pub disclosure: Option<ContentDisclosure>,
    pub state: UiSubtitleState,
}
impl UiSubtitleStatus {
    pub fn with_pipeline(
        mut self,
        state: crate::session::live_text::TextPipelineStatus,
        adopted: bool,
    ) -> Self {
        // Both text kinds consume the same bounded pipeline state projection.
        self.state = crate::ui::lyrics::UiLyricsStatus {
            disclosure: self.disclosure,
            state: self.state,
        }
        .with_pipeline(state, adopted)
        .state;
        self
    }
    pub fn stop(&mut self) {
        self.state = if self.disclosure.is_some() {
            UiSubtitleState::Stopped
        } else {
            UiSubtitleState::Disabled
        };
    }
    pub fn label(self) -> String {
        let Some(disclosure) = self.disclosure else {
            return "字幕查詢：尚未設定服務".into();
        };
        let state = match self.state {
            UiSubtitleState::Disabled => "尚未啟動".into(),
            UiSubtitleState::Waiting => "等待可靠電影／影集確認，字幕仍由即時翻譯提供".into(),
            UiSubtitleState::LookingUp => {
                "正在讀取已確認電影／影集的字幕，字幕仍由即時翻譯提供".into()
            }
            UiSubtitleState::Available => {
                "已取得字幕候選，尚未定位或採用，字幕仍由即時翻譯提供".into()
            }
            UiSubtitleState::Verifying => "正在核對字幕與最終辨識，字幕仍由即時翻譯提供".into(),
            UiSubtitleState::Localizing => {
                "已核對字幕片段，正在本機處理臺灣用語，字幕仍由即時翻譯提供".into()
            }
            UiSubtitleState::Preparing => {
                "已命中字幕片段，正使用所選翻譯服務準備，字幕仍由即時翻譯提供".into()
            }
            UiSubtitleState::UsingExisting => "正在顯示已定位字幕片段；缺口仍由即時翻譯提供".into(),
            UiSubtitleState::Unmatched => {
                "目前沒有可靠的字幕定位或片段已播放完，字幕仍由即時翻譯提供".into()
            }
            UiSubtitleState::PreparationFailed(error) => format!("{error} 字幕仍由即時翻譯提供。"),
            UiSubtitleState::Unavailable => "目前沒有可用字幕，字幕仍由即時翻譯提供".into(),
            UiSubtitleState::Failed(error) => format!("{error} 字幕仍由即時翻譯提供。"),
            UiSubtitleState::Stopped => "已停止".into(),
        };
        format!(
            "字幕查詢：{}（{}）· {state}",
            disclosure.service,
            match disclosure.transport {
                ContentTransport::Local => "本機處理",
                ContentTransport::Remote => match self.state {
                    UiSubtitleState::Stopped => "遠端字幕服務，已停止讀取",
                    _ => "僅讀取已設定的字幕位置",
                },
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{resolver::contracts::ResolverError, session::live_text::TextPipelineStatus};
    fn status(state: UiSubtitleState) -> UiSubtitleStatus {
        UiSubtitleStatus {
            disclosure: Some(ContentDisclosure {
                service: "明確測試字幕服務",
                transport: ContentTransport::Remote,
            }),
            state,
        }
    }
    #[test]
    fn subtitle_labels_describe_only_configured_location_reads_and_live_fallback() {
        for (state, expected) in [
            (UiSubtitleState::Disabled, "尚未啟動"),
            (UiSubtitleState::Waiting, "等待可靠電影／影集確認"),
            (UiSubtitleState::LookingUp, "正在讀取已確認電影／影集的字幕"),
            (UiSubtitleState::Available, "尚未定位或採用"),
            (UiSubtitleState::Verifying, "正在核對字幕與最終辨識"),
            (UiSubtitleState::Localizing, "本機處理臺灣用語"),
            (UiSubtitleState::Preparing, "所選翻譯服務"),
            (UiSubtitleState::Unmatched, "目前沒有可靠的字幕定位"),
            (UiSubtitleState::Unavailable, "目前沒有可用字幕"),
            (
                UiSubtitleState::Failed(ResolverError::Timeout),
                "內容辨識服務逾時",
            ),
        ] {
            let label = status(state).label();
            assert!(label.contains("字幕查詢：明確測試字幕服務"));
            assert!(label.contains("僅讀取已設定的字幕位置"));
            assert!(label.contains(expected));
            assert!(!label.contains("送至"));
            assert!(!label.contains("歌詞"));
            if state != UiSubtitleState::Disabled {
                assert!(label.contains("字幕仍由即時翻譯提供"));
            }
        }
    }
    #[test]
    fn subtitle_local_disabled_and_stopped_states_never_claim_remote_activity() {
        let mut disabled = UiSubtitleStatus::default();
        disabled.stop();
        assert_eq!(disabled.state, UiSubtitleState::Disabled);
        assert_eq!(disabled.label(), "字幕查詢：尚未設定服務");
        let mut remote = status(UiSubtitleState::Available);
        remote.stop();
        assert_eq!(remote.state, UiSubtitleState::Stopped);
        assert!(remote.label().contains("已停止讀取"));
        assert!(!remote.label().contains("僅讀取已設定"));
        let mut local = status(UiSubtitleState::LookingUp);
        local.disclosure.as_mut().unwrap().transport = ContentTransport::Local;
        assert!(local.label().contains("本機處理"));
        assert!(!local.label().contains("字幕位置"));
        local.stop();
        assert!(local.label().contains("已停止"));
    }
    #[test]
    fn subtitle_pipeline_projection_shares_lyrics_state_without_premature_adoption() {
        let failure = crate::session::live_text::TextPipelineError::Preparation(
            crate::session::text_preparation::TextPreparationError::Lookup(ResolverError::Timeout),
        );
        for (pipeline, expected) in [
            (TextPipelineStatus::Verifying, UiSubtitleState::Verifying),
            (TextPipelineStatus::Localizing, UiSubtitleState::Localizing),
            (TextPipelineStatus::Preparing, UiSubtitleState::Preparing),
            (TextPipelineStatus::Ready, UiSubtitleState::Unmatched),
            (TextPipelineStatus::Unmatched, UiSubtitleState::Unmatched),
            (
                TextPipelineStatus::Failed(failure),
                UiSubtitleState::PreparationFailed(failure),
            ),
        ] {
            assert_eq!(
                status(UiSubtitleState::Available)
                    .with_pipeline(pipeline, false)
                    .state,
                expected
            );
            let adopted = status(UiSubtitleState::Available).with_pipeline(pipeline, true);
            assert_eq!(adopted.state, UiSubtitleState::UsingExisting);
            assert!(adopted.label().contains("缺口仍由即時翻譯提供"));
            assert_eq!(
                status(UiSubtitleState::Waiting)
                    .with_pipeline(pipeline, true)
                    .state,
                UiSubtitleState::Waiting
            );
        }
    }
}
