//! Bounded lookup status only; no lyrics, identifiers or provider payloads enter the UI.
use crate::{resolver::contracts::ResolverError, ui::content::ContentDisclosure};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiLyricsState {
    #[default]
    Disabled,
    Waiting,
    LookingUp,
    Available,
    Verifying,
    Localizing,
    Preparing,
    UsingExisting,
    Unmatched,
    PreparationFailed(crate::session::live_text::TextPipelineError),
    Unavailable,
    Failed(ResolverError),
    Stopped,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct UiLyricsStatus {
    pub disclosure: Option<ContentDisclosure>,
    pub state: UiLyricsState,
}
impl UiLyricsStatus {
    pub fn with_pipeline(
        mut self,
        state: crate::session::live_text::TextPipelineStatus,
        adopted: bool,
    ) -> Self {
        use crate::session::live_text::TextPipelineStatus;
        if self.state == UiLyricsState::Available {
            self.state = if adopted {
                UiLyricsState::UsingExisting
            } else {
                match state {
                    TextPipelineStatus::Verifying => UiLyricsState::Verifying,
                    TextPipelineStatus::Localizing => UiLyricsState::Localizing,
                    TextPipelineStatus::Preparing => UiLyricsState::Preparing,
                    TextPipelineStatus::Unmatched | TextPipelineStatus::Ready => {
                        UiLyricsState::Unmatched
                    }
                    TextPipelineStatus::Failed(error) => UiLyricsState::PreparationFailed(error),
                    _ => UiLyricsState::Available,
                }
            };
        }
        self
    }
    pub fn stop(&mut self) {
        self.state = if self.disclosure.is_some() {
            UiLyricsState::Stopped
        } else {
            UiLyricsState::Disabled
        };
    }
    pub fn label(self) -> String {
        let Some(disclosure) = self.disclosure else {
            return "歌詞查詢：尚未設定服務".into();
        };
        let state = match self.state {
            UiLyricsState::Disabled => "尚未啟動".into(),
            UiLyricsState::Waiting => "等待可靠歌曲確認，字幕仍由即時翻譯提供".into(),
            UiLyricsState::LookingUp => "正在查詢已確認歌曲的歌詞，字幕仍由即時翻譯提供".into(),
            UiLyricsState::Available => {
                "已取得歌詞候選，尚未定位或採用，字幕仍由即時翻譯提供".into()
            }
            UiLyricsState::Verifying => "正在核對歌詞與最終辨識，字幕仍由即時翻譯提供".into(),
            UiLyricsState::Localizing => {
                "已核對歌詞片段，正在本機處理臺灣用語，字幕仍由即時翻譯提供".into()
            }
            UiLyricsState::Preparing => {
                "已命中歌詞片段，正使用所選翻譯服務準備，字幕仍由即時翻譯提供".into()
            }
            UiLyricsState::UsingExisting => "正在顯示已定位歌詞片段；缺口仍由即時翻譯提供".into(),
            UiLyricsState::Unmatched => {
                "目前沒有可靠的歌詞定位或片段已播放完，字幕仍由即時翻譯提供".into()
            }
            UiLyricsState::PreparationFailed(error) => format!("{error} 字幕仍由即時翻譯提供。"),
            UiLyricsState::Unavailable => "目前沒有可用歌詞，字幕仍由即時翻譯提供".into(),
            UiLyricsState::Failed(error) => format!("{error} 字幕仍由即時翻譯提供。"),
            UiLyricsState::Stopped => "已停止".into(),
        };
        format!(
            "歌詞查詢：{}（{}）· {state}",
            disclosure.service,
            match disclosure.transport {
                crate::ui::content::ContentTransport::Local => "本機處理",
                crate::ui::content::ContentTransport::Remote => match self.state {
                    UiLyricsState::Stopped => "雲端服務，已停止傳送",
                    _ => "歌曲資料將送至此服務",
                },
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::content::ContentTransport;
    fn status(state: UiLyricsState) -> UiLyricsStatus {
        UiLyricsStatus {
            disclosure: Some(ContentDisclosure {
                service: "明確測試歌詞服務",
                transport: ContentTransport::Remote,
            }),
            state,
        }
    }
    #[test]
    fn lookup_labels_keep_candidates_separate_from_live_translation() {
        for (state, expected) in [
            (UiLyricsState::Disabled, "尚未啟動"),
            (UiLyricsState::Waiting, "等待可靠歌曲確認"),
            (UiLyricsState::LookingUp, "正在查詢已確認歌曲的歌詞"),
            (UiLyricsState::Available, "尚未定位或採用"),
            (UiLyricsState::Unavailable, "目前沒有可用歌詞"),
            (
                UiLyricsState::Failed(ResolverError::Timeout),
                "內容辨識服務逾時",
            ),
        ] {
            let label = status(state).label();
            assert!(label.contains("歌詞查詢：明確測試歌詞服務"));
            assert!(label.contains("歌曲資料將送至此服務"));
            assert!(label.contains(expected));
            if state != UiLyricsState::Disabled {
                assert!(label.contains("字幕仍由即時翻譯提供"));
            }
        }
    }
    #[test]
    fn unconfigured_and_stopped_lookups_do_not_claim_active_transmission() {
        let mut disabled = UiLyricsStatus::default();
        assert!(disabled.disclosure.is_none());
        assert_eq!(disabled.state, UiLyricsState::Disabled);
        disabled.stop();
        assert_eq!(disabled.state, UiLyricsState::Disabled);
        assert_eq!(disabled.label(), "歌詞查詢：尚未設定服務");
        let mut available = status(UiLyricsState::Available);
        available.stop();
        assert_eq!(available.state, UiLyricsState::Stopped);
        assert!(available.label().contains("已停止傳送"));
        assert!(!available.label().contains("尚未定位或採用"));
        assert!(!available.label().contains("歌曲資料將送至此服務"));
    }
    #[test]
    fn local_lookup_labels_do_not_claim_cloud_processing() {
        let mut local = status(UiLyricsState::LookingUp);
        local.disclosure.as_mut().unwrap().transport = ContentTransport::Local;
        assert!(local.label().contains("本機處理"));
        assert!(!local.label().contains("送至"));
        local.stop();
        assert!(local.label().contains("已停止"));
    }
}
