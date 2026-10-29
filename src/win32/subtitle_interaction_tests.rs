//! Native interaction regressions using only test-owned windows and synthetic text.
use super::{tests::*, *};
use crate::{
    ai::{
        provider::SpeechProvider,
        speech::{TranscriptEvent, TranscriptKind},
    },
    subtitle::timeline::TranscriptTimeline,
    ui::bridge::{UiPublisher, UiSource, channel},
};

const EM_GETSEL_TEST: u32 = 0x00b0;
const EM_LINEINDEX_TEST: u32 = 0x00bb;
const EM_LINEFROMCHAR_TEST: u32 = 0x00c9;

fn live_fixture() -> (Box<SubtitleShell>, UiPublisher, TranscriptTimeline) {
    let (mut publisher, inbox) = channel();
    let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
    *shell.inbox.borrow_mut() = Some(inbox);
    shell.create(false).unwrap();
    let timeline = fixture_timeline(9000, "合成譯文");
    publisher
        .begin(
            timeline.identity(),
            UiSource {
                pid: 42,
                provider: SpeechProvider::OpenAi,
            },
        )
        .unwrap();
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    (shell, publisher, timeline)
}

fn search(shell: &SubtitleShell, query: &str) {
    // SAFETY: synchronously updates a control owned by this fixture's UI thread.
    unsafe { SetWindowTextW(shell.search.get(), &windows::core::HSTRING::from(query)).unwrap() };
    assert!(!shell.failed.get());
}

#[test]
fn phase11_native_scroll_notification_without_user_input_keeps_live_following() {
    let (shell, mut publisher, mut timeline) = live_fixture();
    for sequence in 2..=90 {
        append(&mut timeline, sequence);
        publisher.publish(&timeline, true).unwrap();
        shell.poll_inbox().unwrap();
        // SAFETY: simulate a native EDIT notification from reflow, not user input.
        unsafe {
            SendMessageW(
                shell.main.get(),
                WM_COMMAND,
                Some(WPARAM(HISTORY | (EN_VSCROLL_VALUE << 16))),
                Some(LPARAM(shell.history.get().0 as isize)),
            );
        }
        assert!(
            shell.model.borrow().auto_follow,
            "native notification disabled follow at {sequence}"
        );
    }
    assert!(full_text(shell.history.get()).contains("synthetic row 090"));
    // SAFETY: only our fixture EDIT. This is a real scroll command, unlike EN_VSCROLL.
    unsafe {
        SendMessageW(
            shell.history.get(),
            WM_VSCROLL,
            Some(WPARAM(SB_LINEUP.0 as usize)),
            None,
        );
    }
    assert!(!shell.model.borrow().auto_follow);
    append(&mut timeline, 91);
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    assert!(!shell.model.borrow().auto_follow);
    shell.command(LIVE).unwrap();
    assert!(shell.model.borrow().auto_follow);
    assert_eq!(
        selection(&shell).1,
        full_text(shell.history.get()).encode_utf16().count() as u32
    );
    let mut scroll = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        ..Default::default()
    };
    unsafe {
        GetScrollInfo(shell.history.get(), SB_VERT, &mut scroll).unwrap();
    }
    assert_eq!(
        scroll.nPos,
        (scroll.nMax - scroll.nPage.saturating_sub(1) as i32).max(scroll.nMin)
    );
}

#[test]
fn phase11_dense_wrapped_history_keeps_the_last_line_visible_after_page_reflow() {
    let (shell, _state) = controlled_mock();
    shell.command(SETTINGS).unwrap();
    let (mut publisher, inbox) = channel();
    *shell.inbox.borrow_mut() = Some(inbox);
    let mut timeline = fixture_timeline(9001, "合成譯文");
    publisher
        .begin(
            timeline.identity(),
            UiSource {
                pid: 42,
                provider: SpeechProvider::OpenAi,
            },
        )
        .unwrap();
    // SAFETY: resize only the hidden fixture, using the real narrow main-page layout.
    unsafe {
        SetWindowPos(
            shell.main.get(),
            None,
            0,
            0,
            220,
            560,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .unwrap();
    }
    for sequence in 2..=70 {
        timeline
            .ingest(TranscriptEvent {
                identity: timeline.identity(),
                sequence,
                utterance: sequence,
                revision: 1,
                start_us: None,
                duration_us: None,
                language: None,
                kind: TranscriptKind::Final,
                text: "synthetic long original 🦀 ".repeat(80).into(),
            })
            .unwrap();
        let text = format!(
            "{} LAST-LINE-{sequence:03}",
            "合成字幕自動換行測試。".repeat(150)
        );
        timeline
            .apply(&crate::ai::translation::TranslationUpdate {
                identity: timeline.identity(),
                utterance: sequence,
                source_revision: 1,
                job_id: sequence,
                pass: crate::ai::translation::TranslationPass::Draft,
                revision: 1,
                stability: crate::subtitle::timeline::Stability::Final,
                stable_prefix_bytes: text.len(),
                text: text.into(),
                complete: true,
            })
            .unwrap();
        if [63, 64, 65, 70].contains(&sequence) {
            publisher.publish(&timeline, true).unwrap();
            shell.poll_inbox().unwrap();
        }
        assert!(shell.model.borrow().auto_follow);
    }
    assert!(full_text(shell.history.get()).encode_utf16().count() > 65_535);
    for command in [SETTINGS, SETTINGS, LIVE] {
        shell.command(command).unwrap();
        let mut scroll = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            ..Default::default()
        };
        let text = full_text(shell.history.get());
        let last =
            utf16_position(&text, "LAST-LINE-070") + "LAST-LINE-070".encode_utf16().count() - 1;
        // SAFETY: query actual EDIT visual lines and its 32-bit scroll range.
        unsafe {
            GetScrollInfo(shell.history.get(), SB_VERT, &mut scroll).unwrap();
            let line = SendMessageW(
                shell.history.get(),
                EM_LINEFROMCHAR_VALUE,
                Some(WPARAM(last)),
                None,
            )
            .0 as i32;
            let first = SendMessageW(
                shell.history.get(),
                EM_GETFIRSTVISIBLELINE_VALUE,
                None,
                None,
            )
            .0 as i32;
            assert!(
                line >= first && line < first + scroll.nPage as i32,
                "latest visual line {line} outside {first} + {}, follow={}",
                scroll.nPage,
                shell.model.borrow().auto_follow
            );
            assert_eq!(
                scroll.nPos,
                (scroll.nMax - scroll.nPage.saturating_sub(1) as i32).max(scroll.nMin)
            );
        }
    }
}
#[test]
fn phase11_native_scroll_end_and_returning_to_bottom_resume_follow() {
    let (shell, mut publisher, mut timeline) = live_fixture();
    for sequence in 2..=90 {
        append(&mut timeline, sequence);
    }
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    // SAFETY: only scalar messages to our fixture's owned native EDIT.
    unsafe {
        SendMessageW(
            shell.history.get(),
            WM_VSCROLL,
            Some(WPARAM(SB_ENDSCROLL.0 as usize)),
            None,
        );
        assert!(
            shell.model.borrow().auto_follow,
            "finishing a scroll at the live bottom must not freeze following"
        );
        SendMessageW(
            shell.history.get(),
            WM_VSCROLL,
            Some(WPARAM(SB_LINEUP.0 as usize)),
            None,
        );
        assert!(!shell.model.borrow().auto_follow);
        SendMessageW(
            shell.history.get(),
            WM_VSCROLL,
            Some(WPARAM(SB_BOTTOM.0 as usize)),
            None,
        );
        assert!(
            shell.model.borrow().auto_follow,
            "returning to the live bottom must resume following"
        );
        SendMessageW(shell.history.get(), WM_LBUTTONDOWN, None, None);
        SendMessageW(shell.history.get(), WM_LBUTTONUP, None, None);
        // A real selection or search must continue to protect manual reading.
        SendMessageW(
            shell.history.get(),
            EM_SETSEL_VALUE,
            Some(WPARAM(0)),
            Some(LPARAM(8)),
        );
        shell.model.borrow_mut().auto_follow = false;
        SendMessageW(
            shell.history.get(),
            WM_VSCROLL,
            Some(WPARAM(SB_BOTTOM.0 as usize)),
            None,
        );
        assert!(!shell.model.borrow().auto_follow);
    }
    search(&shell, "synthetic");
    unsafe {
        SendMessageW(
            shell.history.get(),
            WM_VSCROLL,
            Some(WPARAM(SB_BOTTOM.0 as usize)),
            None,
        );
    }
    assert!(!shell.model.borrow().auto_follow && shell.model.borrow().searching());
}
#[test]
fn phase11_manual_clear_button_preserves_work_and_filters_snapshot_refill_until_new_segment() {
    let (shell, mut publisher, mut timeline) = live_fixture();
    assert_eq!(control_text(shell.clear_button.get()), "清除字幕");
    // SAFETY: the real push-button belongs to this hidden synthetic fixture only.
    unsafe {
        SendMessageW(shell.clear_button.get(), BM_CLICK, None, None);
    }
    assert!(!publisher.cancellation().is_cancelled());
    assert_eq!(shell.model.borrow().identity(), Some(timeline.identity()));
    assert!(shell.model.borrow().overlay_text().is_empty());
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    assert_eq!(shell.model.borrow().rows().count(), 0);
    append(&mut timeline, 2);
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    assert_eq!(shell.model.borrow().rows().count(), 1);
    assert_eq!(shell.model.borrow().overlay_text(), "合成譯文 002");
    shell.command(STOP).unwrap();
    assert!(publisher.cancellation().is_cancelled());
    assert!(shell.model.borrow().overlay_text().contains("合成譯文 002"));
    shell.command(CLEAR_SUBTITLES).unwrap();
    assert!(shell.model.borrow().overlay_text().is_empty());
}

#[test]
fn phase11_reconnection_preserves_selected_history_and_appends_new_session_with_distinct_ids() {
    let (shell, mut publisher, mut timeline) = live_fixture();
    for sequence in 2..=20 {
        append(&mut timeline, sequence);
    }
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    let before = full_text(shell.history.get());
    scroll_to(&shell, utf16_position(&before, "synthetic row 010"));
    fn selected(shell: &SubtitleShell) -> String {
        let text: Vec<_> = full_text(shell.history.get()).encode_utf16().collect();
        let (start, end) = selection(shell);
        String::from_utf16_lossy(&text[start as usize..end as usize])
    }
    let expected = selected(&shell);
    publisher.clear(UiStatus::Reconnecting).unwrap();
    shell.poll_inbox().unwrap();
    assert_eq!(selected(&shell), expected);
    assert!(full_text(shell.history.get()).contains("先前字幕"));
    let next = fixture_timeline(9901, "新連線合成字幕");
    publisher
        .begin(
            next.identity(),
            UiSource {
                pid: 43,
                provider: SpeechProvider::Gemini,
            },
        )
        .unwrap();
    publisher.publish(&next, true).unwrap();
    shell.poll_inbox().unwrap();
    assert_eq!(selected(&shell), expected);
    assert!(!shell.model.borrow().auto_follow);
    assert_eq!(shell.model.borrow().rows().count(), 21);
    assert!(full_text(shell.history.get()).contains("新連線合成字幕"));
    shell.command(CLEAR_SUBTITLES).unwrap();
    assert!(!publisher.publish(&timeline, true).unwrap());
    shell.poll_inbox().unwrap();
    assert_eq!(shell.model.borrow().rows().count(), 0);
}

#[test]
fn stop_preserves_search_and_history_until_user_clears() {
    let (shell, _publisher, _) = live_fixture();
    search(&shell, "合成譯文");
    assert!(shell.model.borrow().searching());
    shell.command(STOP).unwrap();
    assert!(shell.model.borrow().searching());
    assert!(!shell.model.borrow().auto_follow);
    assert_eq!(control_text(shell.search.get()), "合成譯文");
    assert!(control_text(shell.history.get()).contains("合成譯文"));
    shell.command(CLEAR_SUBTITLES).unwrap();
    assert!(!shell.model.borrow().searching());
    assert!(control_text(shell.search.get()).is_empty());
    assert!(!control_text(shell.history.get()).contains("合成譯文"));
}

#[test]
fn restart_preserves_search_before_session_identity_is_assigned() {
    let (shell, _state) = controlled_mock();
    search(&shell, "待命搜尋");
    {
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        // SAFETY: own mock selectors; no credentials or real backend are used.
        unsafe {
            SendMessageW(panel.source, CB_SETCURSEL, Some(WPARAM(1)), None);
            SendMessageW(panel.provider, CB_SETCURSEL, Some(WPARAM(1)), None);
        }
    }
    shell.start_from_controls().unwrap();
    assert!(shell.model.borrow().identity().is_none());
    assert_eq!(control_text(shell.search.get()), "待命搜尋");
    shell.poll_inbox().unwrap(); // source prepared, identity is still None
    assert!(shell.model.borrow().searching());
    assert!(!shell.model.borrow().auto_follow);
    shell.command(CLEAR_SUBTITLES).unwrap();
    assert!(control_text(shell.search.get()).is_empty());
    assert!(!shell.failed.get());
}

#[test]
fn terminal_frames_preserve_search_even_when_identity_remains_none() {
    let (shell, mut publisher, _) = live_fixture();
    publisher.clear(UiStatus::Reconnecting).unwrap();
    shell.poll_inbox().unwrap();
    search(&shell, "重連搜尋");
    assert!(shell.model.borrow().identity().is_none());
    publisher.clear(UiStatus::Error).unwrap();
    shell.poll_inbox().unwrap();
    assert!(shell.model.borrow().searching());
    assert_eq!(control_text(shell.search.get()), "重連搜尋");
    shell.command(CLEAR_SUBTITLES).unwrap();
    assert!(control_text(shell.search.get()).is_empty());
}

#[test]
fn content_status_is_native_visible_without_replacing_or_clearing_the_shared_timeline() {
    use crate::{
        resolver::{
            candidates::SongState,
            contracts::{ContentType, ResolverError},
        },
        ui::content::{ContentDisclosure, ContentTransport, UiContentState, UiContentStatus},
    };
    let (mut publisher, inbox) = channel();
    let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
    *shell.inbox.borrow_mut() = Some(inbox);
    shell.create(false).unwrap();
    let timeline = fixture_timeline(9100, "合成內容狀態測試譯文");
    publisher
        .prepare_source(UiSource {
            pid: 42,
            provider: SpeechProvider::OpenAi,
        })
        .unwrap();
    let disclosure = ContentDisclosure {
        service: "明確測試服務",
        transport: ContentTransport::Remote,
    };
    publisher.prepare_content(Some(disclosure)).unwrap();
    shell.poll_inbox().unwrap();
    assert!(control_text(shell.status.get()).contains("內容資料將送至此服務"));
    publisher
        .begin(
            timeline.identity(),
            UiSource {
                pid: 42,
                provider: SpeechProvider::OpenAi,
            },
        )
        .unwrap();
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
        UiContentState::Identifying,
        UiContentState::Ready(ContentType::Music, SongState::Confirmed),
        UiContentState::Failed(ResolverError::Timeout),
    ] {
        publisher
            .publish_content(
                timeline.identity(),
                UiContentStatus {
                    disclosure: Some(disclosure),
                    movie_dialogue: false,
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
        assert_eq!(shell.model.borrow().overlay_text(), "合成內容狀態測試譯文");
        assert!(control_text(shell.status.get()).contains("內容辨識：明確測試服務"));
        assert!(!shell.failed.get());
    }
    assert!(control_text(shell.status.get()).contains("內容辨識服務逾時"));
    shell.command(STOP).unwrap();
    assert_eq!(
        shell.frame.borrow().as_ref().unwrap().content.state,
        UiContentState::Stopped
    );
    assert!(control_text(shell.history.get()).contains("合成內容狀態測試譯文"));
    assert!(shell.model.borrow().overlay_text().contains("先前字幕"));
    shell.command(CLEAR_SUBTITLES).unwrap();
    assert!(shell.model.borrow().overlay_text().is_empty());
}

#[test]
fn lyrics_status_is_native_visible_without_adopting_candidates_or_replacing_live_text() {
    use crate::{
        resolver::contracts::ResolverError,
        ui::{
            content::{ContentDisclosure, ContentTransport},
            lyrics::{UiLyricsState, UiLyricsStatus},
        },
    };
    let (mut publisher, inbox) = channel();
    let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
    *shell.inbox.borrow_mut() = Some(inbox);
    shell.create(false).unwrap();
    let timeline = fixture_timeline(9200, "合成歌詞狀態測試譯文");
    let source = UiSource {
        pid: 42,
        provider: SpeechProvider::OpenAi,
    };
    publisher.prepare_source(source).unwrap();
    shell.poll_inbox().unwrap();
    assert!(!control_text(shell.status.get()).contains("歌詞查詢"));
    let disclosure = ContentDisclosure {
        service: "明確測試歌詞服務",
        transport: ContentTransport::Remote,
    };
    publisher.prepare_lyrics(Some(disclosure)).unwrap();
    shell.poll_inbox().unwrap();
    assert!(control_text(shell.status.get()).contains("歌詞查詢：明確測試歌詞服務"));
    assert!(control_text(shell.status.get()).contains("歌曲資料將送至此服務"));
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
        UiLyricsState::LookingUp,
        UiLyricsState::Available,
        UiLyricsState::Unavailable,
        UiLyricsState::Failed(ResolverError::Timeout),
        UiLyricsState::Waiting,
    ] {
        publisher
            .publish_lyrics(
                timeline.identity(),
                UiLyricsStatus {
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
        assert_eq!(shell.model.borrow().overlay_text(), "合成歌詞狀態測試譯文");
        let history = control_text(shell.history.get());
        assert!(history.contains("合成歌詞狀態測試譯文"));
        assert!(control_text(shell.status.get()).contains("字幕仍由即時翻譯提供"));
        assert!(control_text(shell.status.get()).contains("歌詞查詢：明確測試歌詞服務"));
        if state == UiLyricsState::Available {
            assert!(control_text(shell.status.get()).contains("已取得歌詞候選，尚未定位或採用"));
        }
        assert!(!shell.failed.get());
    }
    shell.command(STOP).unwrap();
    assert_eq!(
        shell.frame.borrow().as_ref().unwrap().lyrics.state,
        UiLyricsState::Stopped
    );
    let history = control_text(shell.history.get());
    assert!(control_text(shell.status.get()).contains("已停止傳送"));
    assert!(history.contains("合成歌詞狀態測試譯文"));
    shell.command(CLEAR_SUBTITLES).unwrap();
    assert!(shell.model.borrow().overlay_text().is_empty());
}
fn append(timeline: &mut TranscriptTimeline, sequence: u64) {
    timeline
        .ingest(TranscriptEvent {
            identity: timeline.identity(),
            sequence,
            utterance: sequence,
            revision: 1,
            start_us: Some(sequence * 1_000_000),
            duration_us: Some(100_000),
            language: None,
            kind: TranscriptKind::Final,
            text: format!("synthetic row {sequence:03} 🦀 測試").into(),
        })
        .unwrap();
    translated(timeline, sequence);
}
fn translated(timeline: &mut TranscriptTimeline, sequence: u64) {
    let text = format!("合成譯文 {sequence:03}");
    timeline
        .apply(&crate::ai::translation::TranslationUpdate {
            identity: timeline.identity(),
            utterance: sequence,
            source_revision: 1,
            job_id: sequence,
            pass: crate::ai::translation::TranslationPass::Draft,
            revision: 1,
            stable_prefix_bytes: text.len(),
            text: text.into(),
            stability: crate::subtitle::timeline::Stability::Final,
            complete: true,
        })
        .unwrap();
}
#[test]
fn phase11_native_untranslated_tail_retains_caption_and_manual_history_keeps_original() {
    let (shell, mut publisher, mut timeline) = live_fixture();
    timeline
        .ingest(TranscriptEvent {
            identity: timeline.identity(),
            utterance: 2,
            sequence: 2,
            revision: 1,
            start_us: None,
            duration_us: None,
            language: None,
            kind: TranscriptKind::Final,
            text: "pending native synthetic source".into(),
        })
        .unwrap();
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    assert!(shell.model.borrow().auto_follow);
    assert!(full_text(shell.history.get()).contains("合成譯文"));
    assert!(full_text(shell.history.get()).contains("pending native synthetic source"));
    assert_eq!(shell.model.borrow().overlay_text(), "合成譯文");
    assert_eq!(shell.model.borrow().identity(), Some(timeline.identity()));
    shell.command(FOLLOW).unwrap();
    assert!(!shell.model.borrow().auto_follow);
    assert!(full_text(shell.history.get()).contains("pending native synthetic source"));
    shell.command(LIVE).unwrap();
    assert!(full_text(shell.history.get()).contains("pending native synthetic source"));
    translated(&mut timeline, 2);
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    assert!(full_text(shell.history.get()).contains("pending native synthetic source"));
    assert_eq!(shell.model.borrow().overlay_text(), "合成譯文 002");
    publisher.clear(UiStatus::Stopped).unwrap();
    shell.poll_inbox().unwrap();
    assert!(full_text(shell.history.get()).contains("合成譯文"));
    shell.command(CLEAR_SUBTITLES).unwrap();
    assert!(shell.model.borrow().overlay_text().is_empty());
}
fn utf16_position(text: &str, needle: &str) -> usize {
    text[..text.find(needle).unwrap()].encode_utf16().count()
}
fn selection(shell: &SubtitleShell) -> (u32, u32) {
    let mut start = 0u32;
    let mut end = 0u32;
    // SAFETY: pointers to DWORDs are valid for this synchronous, same-thread message.
    unsafe {
        SendMessageW(
            shell.history.get(),
            EM_GETSEL_TEST,
            Some(WPARAM((&mut start as *mut u32) as usize)),
            Some(LPARAM((&mut end as *mut u32) as isize)),
        );
    }
    (start, end)
}
fn scroll_to(shell: &SubtitleShell, position: usize) {
    shell.model.borrow_mut().auto_follow = false;
    // SAFETY: own EDIT; line indices and UTF-16 offsets are scalar values.
    unsafe {
        let line = SendMessageW(
            shell.history.get(),
            EM_LINEFROMCHAR_TEST,
            Some(WPARAM(position)),
            None,
        )
        .0;
        let previous = SendMessageW(
            shell.history.get(),
            EM_GETFIRSTVISIBLELINE_VALUE,
            None,
            None,
        )
        .0;
        SendMessageW(
            shell.history.get(),
            EM_LINESCROLL_VALUE,
            None,
            Some(LPARAM(line - previous)),
        );
        SendMessageW(
            shell.history.get(),
            EM_SETSEL_VALUE,
            Some(WPARAM(position)),
            Some(LPARAM(position as isize + 9)),
        );
    }
}
fn top_character(shell: &SubtitleShell) -> usize {
    // SAFETY: own EDIT queried synchronously; test text is long enough to scroll.
    unsafe {
        let line = SendMessageW(
            shell.history.get(),
            EM_GETFIRSTVISIBLELINE_VALUE,
            None,
            None,
        )
        .0;
        SendMessageW(
            shell.history.get(),
            EM_LINEINDEX_TEST,
            Some(WPARAM(line as usize)),
            None,
        )
        .0 as usize
    }
}

#[test]
fn tail_updates_preserve_manual_scrollback_and_selection() {
    let (shell, mut publisher, mut timeline) = live_fixture();
    for sequence in 2..=55 {
        append(&mut timeline, sequence);
    }
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    let text = full_text(shell.history.get());
    let end = text.encode_utf16().count() as u32;
    assert_eq!(selection(&shell), (end, end));
    let position = utf16_position(&text, "synthetic row 040");
    scroll_to(&shell, position);
    let top = top_character(&shell);
    let selected = selection(&shell);
    shell.refresh(false).unwrap(); // unchanged content must not clear selection
    assert_eq!(selection(&shell), selected);
    assert_eq!(top_character(&shell), top);
    append(&mut timeline, 56);
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    assert_eq!(selection(&shell), selected);
    assert_eq!(top_character(&shell), top);
    assert!(!shell.model.borrow().auto_follow);
}
fn full_text(control: HWND) -> String {
    // SAFETY: private bounded fixture; capacity includes its terminator.
    unsafe {
        let mut text = vec![0u16; GetWindowTextLengthW(control) as usize + 1];
        let len = GetWindowTextW(control, &mut text) as usize;
        String::from_utf16_lossy(&text[..len])
    }
}

#[test]
fn phase11_status_details_resize_both_axes_and_mirror_only_status() {
    let (shell, mut publisher, timeline) = live_fixture();
    shell.command(STATUS_LABEL).unwrap();
    let details = shell.status_window.get();
    assert!(!details.0.is_null());
    assert!(full_text(shell.history.get()).contains("合成譯文"));
    assert_eq!(
        full_text(shell.status_window_edit.get()),
        *shell.status_text.borrow()
    );
    assert!(!full_text(shell.status_window_edit.get()).contains("合成譯文"));
    let original_history = full_text(shell.history.get());
    // SAFETY: only test-owned hidden HWNDs; SetWindowPos synchronously sends WM_SIZE.
    unsafe {
        assert_ne!(
            GetWindowLongW(details, GWL_STYLE) as u32 & WS_THICKFRAME.0,
            0
        );
        for (width, height) in [(320, 220), (740, 500)] {
            SetWindowPos(
                details,
                None,
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            let mut outer = RECT::default();
            let mut client = RECT::default();
            let mut edit = RECT::default();
            GetWindowRect(details, &mut outer).unwrap();
            GetClientRect(details, &mut client).unwrap();
            GetClientRect(shell.status_window_edit.get(), &mut edit).unwrap();
            assert_eq!(
                (outer.right - outer.left, outer.bottom - outer.top),
                (width, height)
            );
            assert!(edit.right > 0 && edit.bottom > 0);
            assert!(edit.right < client.right && edit.bottom < client.bottom);
        }
    }
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    *shell.notice.borrow_mut() = Some("合成狀態錯誤\r\n".repeat(80));
    shell.refresh(false).unwrap();
    assert_eq!(
        full_text(shell.status_window_edit.get()),
        *shell.status_text.borrow()
    );
    assert!(full_text(shell.status_window_edit.get()).contains("合成狀態錯誤"));
    assert_eq!(full_text(shell.history.get()), original_history);
    assert!(!publisher.cancellation().is_cancelled());
    assert!(!shell.failed.get());
}

#[test]
fn phase11_status_close_keeps_live_session_and_reopens_same_size_then_drop_cleans_window() {
    let (shell, publisher, _) = live_fixture();
    shell.command(STATUS_DETAILS).unwrap();
    let details = shell.status_window.get();
    let edit = shell.status_window_edit.get();
    // SAFETY: fixture window is owned by this UI thread; no OS-wide input.
    unsafe {
        SetWindowPos(
            details,
            None,
            0,
            0,
            510,
            360,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .unwrap();
        SendMessageW(details, WM_CLOSE, None, None);
        assert!(!IsWindowVisible(details).as_bool());
        assert!(IsWindow(Some(details)).as_bool());
    }
    assert!(!publisher.cancellation().is_cancelled());
    shell.command(STATUS_LABEL).unwrap();
    assert_eq!(shell.status_window.get(), details);
    // SAFETY: same retained fixture window.
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(details, &mut rect).unwrap();
        assert_eq!((rect.right - rect.left, rect.bottom - rect.top), (510, 360));
    }
    drop(shell);
    // SAFETY: IsWindow accepts stale handles to check destruction.
    unsafe {
        assert!(!IsWindow(Some(details)).as_bool());
        assert!(!IsWindow(Some(edit)).as_bool());
    }
}

#[test]
fn phase11_status_details_cannot_bypass_a_disabled_modal_owner() {
    let (shell, state) = controlled_mock();
    // SAFETY: disable only the fixture owner, simulating its existing modal dialog.
    unsafe {
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(shell.main.get(), false);
    }
    shell.command(STATUS_DETAILS).unwrap();
    assert!(shell.status_window.get().0.is_null());
    assert!(state.borrow().calls.is_empty());
    // SAFETY: restore fixture owner before normal RAII cleanup.
    unsafe {
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(shell.main.get(), true);
    }
}

#[test]
fn history_retirement_keeps_the_same_retained_row_in_view() {
    let (shell, mut publisher, mut timeline) = live_fixture();
    for sequence in 2..=64 {
        append(&mut timeline, sequence);
    }
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    let previous = full_text(shell.history.get());
    scroll_to(&shell, utf16_position(&previous, "synthetic row 040"));
    append(&mut timeline, 65); // the 64-item timeline retires its oldest row
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    let current = full_text(shell.history.get());
    let position = utf16_position(&current, "synthetic row 040");
    assert_eq!(top_character(&shell), position);
    assert_eq!(selection(&shell), (position as u32, position as u32 + 9));
    shell.command(LIVE).unwrap();
    assert!(shell.model.borrow().auto_follow);
    let end = current.encode_utf16().count() as u32;
    assert_eq!(selection(&shell), (end, end));
    assert!(
        top_character(&shell) > position,
        "top={} row={position}",
        top_character(&shell)
    );
}

#[test]
fn retired_anchor_clamps_without_discarding_other_history_on_new_session() {
    let (shell, mut publisher, mut timeline) = live_fixture();
    for sequence in 2..=64 {
        append(&mut timeline, sequence);
    }
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    let old = full_text(shell.history.get());
    scroll_to(&shell, utf16_position(&old, "test source"));
    append(&mut timeline, 65);
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    let current = full_text(shell.history.get());
    assert_eq!(
        top_character(&shell),
        utf16_position(&current, "[00:02.000")
    );
    assert_eq!(selection(&shell), (0, 0));
    let next = fixture_timeline(9001, "新工作階段合成譯文");
    publisher
        .begin(
            next.identity(),
            UiSource {
                pid: 43,
                provider: SpeechProvider::OpenAi,
            },
        )
        .unwrap();
    publisher.publish(&next, true).unwrap();
    shell.poll_inbox().unwrap();
    assert!(full_text(shell.history.get()).contains("synthetic row"));
    assert!(!full_text(shell.history.get()).contains("synthetic row 002"));
    assert_eq!(selection(&shell), (0, 0));
    assert!(!shell.model.borrow().auto_follow);
    assert_eq!(shell.model.borrow().identity(), Some(next.identity()));
}

#[test]
fn large_utf16_selection_survives_tail_and_status_header_changes() {
    let (shell, mut publisher, mut timeline) = live_fixture();
    for sequence in 2..=20 {
        timeline
            .ingest(TranscriptEvent {
                identity: timeline.identity(),
                sequence,
                utterance: sequence,
                revision: 1,
                start_us: Some(sequence * 1_000_000),
                duration_us: None,
                language: None,
                kind: TranscriptKind::Final,
                text: format!("synthetic long {sequence:03} {} 🦀", "a".repeat(4000)).into(),
            })
            .unwrap();
        translated(&mut timeline, sequence);
    }
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    let previous = full_text(shell.history.get());
    let position = utf16_position(&previous, "synthetic long 020");
    assert!(position > u16::MAX as usize);
    scroll_to(&shell, position);
    let top = top_character(&shell);
    let selected = selection(&shell);
    append(&mut timeline, 21);
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    assert_eq!(selection(&shell), selected);
    assert_eq!(top_character(&shell), top);
    *shell.notice.borrow_mut() = Some("合成狀態提示".into());
    shell.refresh(false).unwrap();
    let current = full_text(shell.history.get());
    let shifted = utf16_position(&current, "synthetic long 020");
    assert_eq!(selection(&shell), (shifted as u32, shifted as u32 + 9));
    assert_eq!(top_character(&shell), shifted);
    assert!(!shell.model.borrow().auto_follow);
}

#[test]
fn refresh_preserves_thread_activation_and_minimized_owner_hides_overlay() {
    let (shell, mut publisher, mut timeline) = live_fixture();
    for sequence in 2..=64 {
        append(&mut timeline, sequence);
    }
    publisher.publish(&timeline, true).unwrap();
    shell.poll_inbox().unwrap();
    let text = full_text(shell.history.get());
    scroll_to(&shell, utf16_position(&text, "synthetic row 040"));
    // SAFETY: show/minimize only this fixture, always without activation. No input,
    // foreground changes, external application messages, or screenshot capture.
    unsafe {
        SetWindowPos(
            shell.main.get(),
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .unwrap();
        let active = windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow();
        shell.refresh(true).unwrap();
        assert_eq!(
            windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow(),
            active
        );
        assert!(IsWindowVisible(shell.overlay.get()).as_bool());
        let mut before = RECT::default();
        GetClientRect(shell.history.get(), &mut before).unwrap();
        let _ = ShowWindow(shell.main.get(), SW_SHOWMINNOACTIVE);
        append(&mut timeline, 65);
        publisher.publish(&timeline, true).unwrap();
        shell.poll_inbox().unwrap();
        shell.refresh(true).unwrap();
        assert!(IsIconic(shell.main.get()).as_bool());
        assert!(!IsWindowVisible(shell.overlay.get()).as_bool());
        let mut minimized = RECT::default();
        GetClientRect(shell.history.get(), &mut minimized).unwrap();
        assert_eq!(minimized, before);
        let _ = ShowWindow(shell.main.get(), SW_SHOWNOACTIVATE);
        assert!(!IsIconic(shell.main.get()).as_bool());
        assert!(IsWindowVisible(shell.overlay.get()).as_bool());
    }
    assert!(!shell.failed.get());
    let text = full_text(shell.history.get());
    assert_eq!(
        top_character(&shell),
        utf16_position(&text, "synthetic row 040")
    );
    assert_eq!(shell.model.borrow().identity(), Some(timeline.identity()));
}

#[test]
fn minimized_source_frame_never_opens_startup_gate_before_restore() {
    let (mut publisher, inbox) = channel();
    publisher
        .prepare_source(UiSource {
            pid: 42,
            provider: SpeechProvider::OpenAi,
        })
        .unwrap();
    let shell = SubtitleShell::allocate(DisplayMode::Dual, false);
    *shell.inbox.borrow_mut() = Some(inbox);
    shell.create(false).unwrap();
    // SAFETY: private mock view is minimized/restored without activation or input.
    unsafe {
        let _ = ShowWindow(shell.main.get(), SW_SHOWMINNOACTIVE);
    }
    shell.visible.set(true);
    shell.poll_inbox().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(5), publisher.wait_until_visible()).await
            })
            .is_err()
    );
    // SAFETY: restore only this test's window. The next poll acknowledges visibility.
    unsafe {
        let _ = ShowWindow(shell.main.get(), SW_SHOWNOACTIVATE);
    }
    shell.poll_inbox().unwrap();
    runtime
        .block_on(async {
            tokio::time::timeout(Duration::from_millis(100), publisher.wait_until_visible()).await
        })
        .unwrap()
        .unwrap();
    assert!(!shell.failed.get());
}
