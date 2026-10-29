//! Bounded status only. No candidate titles, lyrics, fingerprints or provider payloads.
use crate::resolver::{
    candidates::SongState,
    contracts::{ContentType, ResolverError},
};

pub const MAX_CONTENT_SERVICE_BYTES: usize = 96;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentTransport {
    Local,
    Remote,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ContentDisclosure {
    pub service: &'static str,
    pub transport: ContentTransport,
}
impl ContentDisclosure {
    pub fn validate(self) -> Result<(), ResolverError> {
        if self.service.trim().is_empty()
            || self.service.len() > MAX_CONTENT_SERVICE_BYTES
            || self.service.chars().any(char::is_control)
        {
            Err(ResolverError::InvalidData)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiContentState {
    #[default]
    Disabled,
    Collecting,
    Identifying,
    Ready(ContentType, SongState),
    Failed(ResolverError),
    Stopped,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct UiContentStatus {
    pub disclosure: Option<ContentDisclosure>,
    pub movie_dialogue: bool,
    pub state: UiContentState,
}
impl UiContentStatus {
    pub fn stop(&mut self) {
        self.state = if self.disclosure.is_some() {
            UiContentState::Stopped
        } else {
            UiContentState::Disabled
        };
    }
    pub fn label(self) -> String {
        let Some(disclosure) = self.disclosure else {
            return "歌曲／影片辨識：未啟用獨立服務；不影響即時語音辨識與翻譯".into();
        };
        let state = match self.state {
            UiContentState::Disabled => "尚未啟動".into(),
            UiContentState::Collecting => "正在收集音訊".into(),
            UiContentState::Identifying => "正在辨識內容".into(),
            UiContentState::Ready(kind, song) => match (kind, song) {
                (ContentType::Music, SongState::Confirmed) => "已確認歌曲，字幕仍使用即時翻譯",
                (ContentType::Music, SongState::Transition) => "歌曲切換中，字幕仍使用即時翻譯",
                (ContentType::Music, _) => "音樂候選，字幕仍使用即時翻譯",
                (ContentType::Speech, _) => "語音",
                (ContentType::MovieOrShow, _) => "電影／影集候選，字幕仍使用即時翻譯",
                (ContentType::Mixed, _) => "混合內容，字幕仍使用即時翻譯",
                (ContentType::Unknown, _) => "尚無可靠內容結果",
            }
            .into(),
            UiContentState::Failed(error) => format!("{error} 即時辨識與翻譯繼續運作。"),
            UiContentState::Stopped => "已停止".into(),
        };
        format!(
            "內容辨識：{}（{}）· {state}",
            disclosure.service,
            match (disclosure.transport, self.movie_dialogue, self.state) {
                (ContentTransport::Local, true, UiContentState::Stopped) => {
                    "本機處理，已停止使用辨識文字與媒體線索"
                }
                (ContentTransport::Local, true, _) => {
                    "本機處理，使用目前已定稿語音辨識文字與已設定媒體線索"
                }
                (ContentTransport::Remote, true, UiContentState::Stopped) => {
                    "雲端服務，已停止傳送辨識文字與媒體線索"
                }
                (ContentTransport::Remote, true, _) => {
                    "目前已定稿語音辨識文字與已設定媒體線索將送至此服務"
                }
                (ContentTransport::Local, false, _) => "本機處理",
                (ContentTransport::Remote, false, _) => match self.state {
                    UiContentState::Stopped => "雲端服務，已停止傳送",
                    _ => "內容資料將送至此服務",
                },
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(transport: ContentTransport) -> UiContentStatus {
        UiContentStatus {
            disclosure: Some(ContentDisclosure {
                service: "明確電影測試服務",
                transport,
            }),
            movie_dialogue: true,
            state: UiContentState::Disabled,
        }
    }

    #[test]
    fn movie_hint_labels_describe_current_final_dialogue_and_configured_hints() {
        let local = status(ContentTransport::Local).label();
        assert!(local.contains("本機處理，使用目前已定稿語音辨識文字與已設定媒體線索"));
        assert!(!local.contains("送至"));
        let remote = status(ContentTransport::Remote).label();
        assert!(remote.contains("目前已定稿語音辨識文字與已設定媒體線索將送至此服務"));
        for label in [local, remote] {
            assert!(!label.contains("傳送音訊"));
            assert!(!label.contains("上傳音訊"));
        }
    }

    #[test]
    fn movie_hint_stop_labels_preserve_the_destination_without_promising_transmission() {
        for transport in [ContentTransport::Local, ContentTransport::Remote] {
            let mut status = status(transport);
            status.stop();
            let label = status.label();
            assert!(label.contains("明確電影測試服務"));
            assert!(label.contains("已停止"));
            assert!(label.contains("辨識文字與媒體線索"));
            assert!(!label.contains("將送至"));
            assert!(!label.contains("使用目前已定稿"));
        }
    }

    #[test]
    fn movie_hints_are_disabled_by_default_and_do_not_change_ordinary_disclosure() {
        let empty = UiContentStatus::default();
        assert!(!empty.movie_dialogue);
        assert_eq!(
            empty.label(),
            "歌曲／影片辨識：未啟用獨立服務；不影響即時語音辨識與翻譯"
        );
        let ordinary = UiContentStatus {
            movie_dialogue: false,
            ..status(ContentTransport::Remote)
        };
        assert!(ordinary.label().contains("內容資料將送至此服務"));
        assert!(!ordinary.label().contains("已定稿"));
    }
}
