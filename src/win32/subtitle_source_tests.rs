//! Subtitle status regression with only test-owned hidden windows and synthetic text.
use super::{tests::*, *};
use crate::{
    ai::provider::SpeechProvider,
    resolver::contracts::ResolverError,
    ui::{
        bridge::{UiSource, channel},
        content::{ContentDisclosure, ContentTransport},
        subtitles::{UiSubtitleState, UiSubtitleStatus},
    },
};

#[test]
fn subtitle_status_is_native_visible_without_adopting_candidates_or_replacing_live_text() {
    let (mut publisher, inbox) = channel();
    let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
    *shell.inbox.borrow_mut() = Some(inbox);
    shell.create(false).unwrap();
    let timeline = fixture_timeline(9300, "合成字幕狀態測試譯文");
    let source = UiSource {
        pid: 42,
        provider: SpeechProvider::OpenAi,
    };
    publisher.prepare_source(source).unwrap();
    shell.poll_inbox().unwrap();
    assert!(!control_text(shell.status.get()).contains("字幕查詢"));
    let disclosure = ContentDisclosure {
        service: "明確測試字幕服務",
        transport: ContentTransport::Remote,
    };
    publisher.prepare_subtitles(Some(disclosure)).unwrap();
    shell.poll_inbox().unwrap();
    let history = control_text(shell.history.get());
    assert!(control_text(shell.status.get()).contains("字幕查詢：明確測試字幕服務"));
    assert!(!history.contains("字幕查詢："));
    assert!(control_text(shell.status.get()).contains("僅讀取已設定的字幕位置"));
    assert!(shell.model.borrow().identity().is_none());
    publisher.begin(timeline.identity(), source).unwrap();
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    let snapshot = shell
        .frame
        .borrow()
        .as_ref()
        .unwrap()
        .snapshot
        .as_ref()
        .unwrap()
        .clone();
    for state in [
        UiSubtitleState::Waiting,
        UiSubtitleState::LookingUp,
        UiSubtitleState::Available,
        UiSubtitleState::Verifying,
        UiSubtitleState::Localizing,
        UiSubtitleState::Preparing,
        UiSubtitleState::Unavailable,
        UiSubtitleState::Failed(ResolverError::Timeout),
    ] {
        publisher
            .publish_subtitles(
                timeline.identity(),
                UiSubtitleStatus {
                    disclosure: Some(disclosure),
                    state,
                },
            )
            .unwrap();
        shell.poll_inbox().unwrap();
        assert!(Arc::ptr_eq(
            shell
                .frame
                .borrow()
                .as_ref()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap(),
            &snapshot
        ));
        assert_eq!(shell.model.borrow().overlay_text(), "合成字幕狀態測試譯文");
        let history = control_text(shell.history.get());
        assert!(history.contains("合成字幕狀態測試譯文"));
        assert!(control_text(shell.status.get()).contains("字幕仍由即時翻譯提供"));
        assert!(control_text(shell.status.get()).contains("字幕查詢：明確測試字幕服務"));
        assert!(!history.contains("字幕查詢："));
        if state == UiSubtitleState::Localizing {
            assert!(control_text(shell.status.get()).contains("本機處理臺灣用語"));
        }
        assert!(!shell.failed.get());
    }
    shell.command(STOP).unwrap();
    assert_eq!(
        shell.frame.borrow().as_ref().unwrap().subtitles.state,
        UiSubtitleState::Stopped
    );
    let history = control_text(shell.history.get());
    assert!(control_text(shell.status.get()).contains("已停止讀取"));
    assert!(history.contains("合成字幕狀態測試譯文"));
    shell.command(CLEAR_SUBTITLES).unwrap();
    assert!(shell.model.borrow().overlay_text().is_empty());
}
