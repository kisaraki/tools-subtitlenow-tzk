use super::*;
use crate::session::{
    aligned_playback::tests::attachment_fixture,
    alignment_control::{AlignmentControlState, AlignmentController},
};
use crate::{ai::speech::StreamIdentity, audio::capture::SessionId};
fn capture() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(88002),
        generation: 1,
    }
}
#[test]
fn alignment_native_ready_handoff_survives_dialog_close_and_is_revoked_by_final_owner_drop() {
    for paired in [false, true] {
        let (directory, attachment, source) = attachment_fixture(paired);
        let owner = Owner::new();
        let dialog = Dialog::new(
            Box::new(crate::app::prepared::PreparedController::from_attachment(
                attachment,
            )),
            Box::new(MockNotification(Rc::new(RefCell::new(
                NoticeState::default(),
            )))),
        );
        dialog.create(owner.0, owner.1, false).unwrap();
        dialog.poll().unwrap();
        set_edit(dialog.openai_key.get(), "SYNTHETIC_OPENAI").unwrap();
        set_edit(dialog.gemini_key.get(), "SYNTHETIC_GEMINI").unwrap();
        assert!(unsafe { IsWindowEnabled(dialog.keep_ready.get()) }.as_bool());
        click(&dialog, KEEP_READY);
        assert!(dialog.hwnd.get().0.is_null());
        let attachment = dialog.attachment.borrow_mut().take().unwrap();
        assert_eq!(attachment.cue_count(), 16);
        drop(dialog);
        let control = AlignmentController::from_source(&source, capture()).unwrap();
        assert_eq!(control.state(), AlignmentControlState::Paused);
        let path = directory.path().join("synthetic-opaque.media");
        #[cfg(windows)]
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        drop(attachment);
        assert!(AlignmentController::from_source(&source, capture()).is_err());
        #[cfg(windows)]
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    }
}
#[test]
fn alignment_native_normal_close_does_not_retain_ready_without_the_explicit_handoff() {
    let (_directory, attachment, source) = attachment_fixture(false);
    let owner = Owner::new();
    let dialog = Dialog::new(
        Box::new(crate::app::prepared::PreparedController::from_attachment(
            attachment,
        )),
        Box::new(MockNotification(Rc::new(RefCell::new(
            NoticeState::default(),
        )))),
    );
    dialog.create(owner.0, owner.1, false).unwrap();
    dialog.poll().unwrap();
    click(&dialog, CLOSE);
    assert!(dialog.attachment.borrow().is_none());
    assert!(AlignmentController::from_source(&source, capture()).is_err());
}
#[test]
fn alignment_native_reference_choices_are_explicit_passive_and_cleared_on_close() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    fill(&dialog, 1, 0);
    assert!(dialog.read_input().unwrap().0.reference.is_none());
    set_edit(dialog.reference_path.get(), r"C:\Synthetic\reference.vtt").unwrap();
    assert!(dialog.read_input().is_err());
    select(dialog.reference_format.get(), 2);
    select(dialog.reference_language.get(), 2);
    set_edit(dialog.reference_edition.get(), "synthetic-same-edition").unwrap();
    let request = dialog.read_input().unwrap().0;
    assert!(
        request.reference.as_ref().unwrap().input.language
            == TextLanguage::Source(SpeechLanguage::Japanese)
    );
    assert_eq!(state.borrow().starts, 0);
    assert!(text(dialog.destination.get()).contains("不使用雲端"));
    click(&dialog, CLOSE);
    assert_eq!(state.borrow().starts, 0);
}
#[test]
fn alignment_native_optional_reference_fields_lock_while_working_and_clear_with_the_form() {
    let owner = Owner::new();
    let (dialog, state, _) = owner.dialog();
    fill(&dialog, 1, 0);
    set_edit(dialog.reference_path.get(), r"C:\Synthetic\reference.srt").unwrap();
    select(dialog.reference_format.get(), 1);
    select(dialog.reference_language.get(), 1);
    set_edit(dialog.reference_edition.get(), "synthetic-edition").unwrap();
    click(&dialog, PREPARE);
    for control in [
        dialog.reference_path.get(),
        dialog.reference_format.get(),
        dialog.reference_language.get(),
        dialog.reference_edition.get(),
    ] {
        assert!(!unsafe { IsWindowEnabled(control) }.as_bool());
    }
    state.borrow_mut().state = PreparedUiState::Cancelled;
    dialog.poll().unwrap();
    click(&dialog, CLEAR);
    assert!(
        text(dialog.reference_path.get()).is_empty()
            && text(dialog.reference_edition.get()).is_empty()
    );
    assert_eq!(selected(dialog.reference_format.get()), 0);
    assert_eq!(selected(dialog.reference_language.get()), 0);
}
