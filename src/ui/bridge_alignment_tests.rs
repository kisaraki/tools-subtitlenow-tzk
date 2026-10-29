use super::*;
fn identity(id: u64) -> StreamIdentity {
    StreamIdentity {
        session: crate::audio::capture::SessionId(id),
        generation: 1,
    }
}
fn source() -> UiSource {
    UiSource {
        pid: 42,
        provider: SpeechProvider::OpenAi,
    }
}
use crate::{
    session::aligned_playback::AlignedPlaybackState,
    ui::alignment::{AlignmentAction, AlignmentCommand},
};
#[tokio::test]
async fn alignment_bridge_disclosure_uses_the_existing_visible_gate() {
    let (mut publisher, mut inbox) = channel();
    assert_eq!(
        publisher.prepare_alignment(),
        Err(UiBridgeError::InvalidState)
    );
    publisher.prepare_source(source()).unwrap();
    let previous = inbox.poll().unwrap();
    inbox.acknowledge_visible(previous.version);
    publisher.prepare_alignment().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), publisher.wait_until_visible())
            .await
            .is_err()
    );
    let frame = inbox.poll().unwrap();
    assert!(frame.status_text().contains("僅在本機定位"));
    assert!(frame.identity.is_none());
    assert!(inbox.alignment_action(AlignmentAction::Resume).is_err());
    inbox.acknowledge_visible(frame.version);
    publisher.wait_until_visible().await.unwrap();
}
#[test]
fn alignment_bridge_commands_are_bounded_and_reset_survives_pause_resume_coalescing() {
    let (mut publisher, inbox) = channel();
    publisher.begin(identity(100), source()).unwrap();
    assert!(inbox.alignment_action(AlignmentAction::Reset).is_err());
    publisher
        .publish_alignment(identity(100), AlignedPlaybackState::Collecting(0))
        .unwrap();
    let receiver = publisher.alignment_commands();
    for _ in 0..100 {
        inbox.alignment_action(AlignmentAction::Reset).unwrap();
        inbox.alignment_action(AlignmentAction::Pause).unwrap();
        inbox.alignment_action(AlignmentAction::Resume).unwrap();
    }
    assert_eq!(
        *receiver.borrow(),
        AlignmentCommand {
            running: true,
            reset: 100
        }
    );
    publisher.finish(UiStatus::Stopped).unwrap();
    assert!(inbox.alignment_action(AlignmentAction::Resume).is_err());
}
#[test]
fn alignment_bridge_old_session_status_cannot_replace_current_metadata() {
    let (mut publisher, mut inbox) = channel();
    publisher.begin(identity(100), source()).unwrap();
    publisher
        .publish_alignment(identity(100), AlignedPlaybackState::Collecting(1))
        .unwrap();
    publisher.begin(identity(101), source()).unwrap();
    assert!(
        !publisher
            .publish_alignment(identity(100), AlignedPlaybackState::Paused)
            .unwrap()
    );
    assert!(inbox.poll().unwrap().alignment.is_none());
    publisher
        .publish_alignment(identity(101), AlignedPlaybackState::Collecting(2))
        .unwrap();
    drop(publisher);
    let frame = inbox.poll().unwrap();
    assert!(frame.identity.is_none() && frame.alignment.is_none());
}
#[test]
fn alignment_bridge_reset_counter_does_not_wrap_or_fake_success() {
    let mut command = AlignmentCommand {
        running: false,
        reset: u64::MAX,
    };
    assert!(!command.apply(AlignmentAction::Reset));
    assert_eq!(command.reset, u64::MAX);
    assert!(command.apply(AlignmentAction::Resume));
    assert!(command.running);
}
