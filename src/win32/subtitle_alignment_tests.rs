use super::*;
use crate::{
    session::aligned_playback::{AlignedPlaybackState, tests::attachment_fixture},
    ui::alignment::AlignmentCommand,
};
#[test]
fn phase10_hidden_native_view_does_not_claim_ui_presentation() {
    let (metrics, timeline) = crate::performance::live::tests::diagnostic_fixture();
    let (shell, state) = tests::controlled_mock();
    let (mut publisher, inbox) = crate::ui::bridge::channel();
    publisher
        .begin(
            metrics.identity(),
            crate::ui::bridge::UiSource {
                pid: 42,
                provider: crate::ai::provider::SpeechProvider::OpenAi,
            },
        )
        .unwrap();
    publisher.attach_diagnostics(Some(metrics.clone())).unwrap();
    publisher.publish(&timeline, true).unwrap();
    *shell.inbox.borrow_mut() = Some(inbox);
    assert!(shell.poll_inbox().unwrap());
    shell.refresh(false).unwrap();
    assert_eq!(
        metrics
            .distribution(crate::performance::live::Latency::TranslationToRenderSubmission)
            .samples,
        0
    );
    assert!(state.borrow().calls.is_empty());
}
#[test]
fn alignment_native_main_retains_ready_until_explicit_start_and_clears_its_source() {
    let (_directory, attachment, source) = attachment_fixture(true);
    let (shell, state) = tests::controlled_mock();
    *shell.prepared.borrow_mut() = Some(attachment);
    assert_eq!(state.borrow().prepared_starts, 0);
    {
        let panel = shell.panel.borrow();
        let panel = panel.as_ref().unwrap();
        // SAFETY: only hidden fixture-owned combo selections, no capture/network.
        unsafe {
            SendMessageW(panel.source, CB_SETCURSEL, Some(WPARAM(1)), None);
            SendMessageW(panel.provider, CB_SETCURSEL, Some(WPARAM(1)), None);
            SendMessageW(panel.language, CB_SETCURSEL, Some(WPARAM(1)), None);
        }
    }
    shell.start_from_controls().unwrap();
    assert_eq!(state.borrow().prepared_starts, 1);
    assert_eq!(state.borrow().calls.len(), 1);
    shell.command(CLEAR_PREPARED).unwrap();
    assert!(shell.prepared.borrow().is_none());
    assert!(
        crate::session::alignment_control::AlignmentController::from_source(
            &source,
            crate::ai::speech::StreamIdentity {
                session: crate::audio::capture::SessionId(88003),
                generation: 1
            }
        )
        .is_err()
    );
    assert_eq!(state.borrow().calls.len(), 1);
}
#[test]
fn alignment_native_menu_actions_use_the_bounded_mailbox_without_restarting_backend() {
    let (shell, state) = tests::controlled_mock();
    let (mut publisher, inbox) = crate::ui::bridge::channel();
    let identity = crate::ai::speech::StreamIdentity {
        session: crate::audio::capture::SessionId(88004),
        generation: 1,
    };
    publisher
        .begin(
            identity,
            crate::ui::bridge::UiSource {
                pid: 42,
                provider: crate::ai::provider::SpeechProvider::OpenAi,
            },
        )
        .unwrap();
    publisher
        .publish_alignment(identity, AlignedPlaybackState::Collecting(0))
        .unwrap();
    let receiver = publisher.alignment_commands();
    *shell.inbox.borrow_mut() = Some(inbox);
    shell.command(ALIGN_PAUSE).unwrap();
    assert!(!receiver.borrow().running);
    shell.command(ALIGN_RESET).unwrap();
    shell.command(ALIGN_RESUME).unwrap();
    assert_eq!(
        *receiver.borrow(),
        AlignmentCommand {
            running: true,
            reset: 1
        }
    );
    assert!(state.borrow().calls.is_empty());
}
