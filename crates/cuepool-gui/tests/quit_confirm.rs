//! Quit-confirm modal behaviour (#173): every way out of the dialog, and the
//! modal-layer collision that left it visible but unclickable.

use cuepool_gui::{CuePoolApp, SharedStateHandle, preview};
use egui_kittest::{Harness, kittest::Queryable};

fn harness_for(mut app: CuePoolApp) -> (Harness<'static>, SharedStateHandle) {
    let state = app.state().clone();
    let mut harness = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_pixels_per_point(1.0)
        .with_theme(egui::Theme::Dark)
        .build_ui(move |ui| app.update(ui));
    harness.ctx.set_theme(egui::Theme::Dark);
    harness.input_mut().time = Some(harness.ctx.input(|i| i.time));
    harness.step();
    (harness, state)
}

/// The demo show, past the launch card, with a close request pending.
fn pending_close() -> (Harness<'static>, SharedStateHandle) {
    let (mut harness, state) = harness_for(preview::demo_app());
    {
        let mut s = state.lock().unwrap();
        s.last_seen_release_notes = Some(cuepool_gui::RELEASE_NOTES_VERSION.into());
        s.dirty = true;
        s.pending_close_confirm = true;
    }
    harness.input_mut().time = Some(60.0);
    settle(&mut harness);
    (harness, state)
}

fn settle(harness: &mut Harness<'static>) {
    for _ in 0..4 {
        harness.step();
    }
}

fn modal_visible(harness: &Harness<'static>) -> bool {
    harness.query_by_label("Quit CuePool?").is_some()
}

#[test]
fn discard_quits_without_saving() {
    let (mut harness, state) = pending_close();
    assert!(modal_visible(&harness));

    harness.get_by_label("Discard & Quit").click();
    settle(&mut harness);

    let s = state.lock().unwrap();
    assert!(s.quit, "Discard & Quit should set the quit flag");
    assert!(!s.pending_close_confirm);
    assert!(s.dirty, "discard must not have saved");
}

/// Nothing to save, but cues still running: the dialog is about losing the
/// show, not just the file, so it has to come up here too. The macOS Quit hook
/// reaches this state far more often than the window's close button ever did —
/// Cmd-Q mid-show is one keystroke.
#[test]
fn a_running_show_is_confirmed_even_with_nothing_to_save() {
    let (mut harness, state) = harness_for(preview::demo_app());
    {
        let mut s = state.lock().unwrap();
        s.last_seen_release_notes = Some(cuepool_gui::RELEASE_NOTES_VERSION.into());
        s.dirty = false;
        s.pending_close_confirm = true;
    }
    harness.input_mut().time = Some(60.0);
    settle(&mut harness);
    assert!(modal_visible(&harness), "a running show must still ask");

    harness.get_by_label("Discard & Quit").click();
    settle(&mut harness);

    let s = state.lock().unwrap();
    assert!(s.quit, "Discard & Quit should set the quit flag");
    assert!(!s.pending_close_confirm);
}

#[test]
fn cancel_clears_the_close_request() {
    let (mut harness, state) = pending_close();

    harness.get_by_label("Cancel").click();
    settle(&mut harness);

    let s = state.lock().unwrap();
    assert!(!s.pending_close_confirm, "Cancel should clear the request");
    assert!(!s.quit);
    drop(s);
    assert!(!modal_visible(&harness), "the modal should be gone");
}

/// Escape used to do nothing at all: `ModalResponse` was dropped, so the
/// dialog stayed up with the close request still pending.
#[test]
fn escape_cancels_the_close_request() {
    let (mut harness, state) = pending_close();

    harness.key_press(egui::Key::Escape);
    settle(&mut harness);

    let s = state.lock().unwrap();
    assert!(!s.pending_close_confirm, "Escape should cancel the close");
    assert!(!s.quit);
    drop(s);
    assert!(!modal_visible(&harness));
}

#[test]
fn save_and_quit_writes_the_project_then_quits() {
    let dir = std::env::temp_dir().join(format!("cuepool-quit-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("show.qproj");

    let (mut harness, state) = pending_close();
    state.lock().unwrap().project_path = Some(path.clone());
    settle(&mut harness);

    harness.get_by_label("Save & Quit").click();
    settle(&mut harness);

    let s = state.lock().unwrap();
    assert!(s.quit, "Save & Quit should set the quit flag");
    assert!(!s.dirty, "the project should have been saved");
    drop(s);
    assert!(path.is_file(), "the project file should exist");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A failed save must not quit: losing the show to an unwritable path is
/// worse than staying open.
#[test]
fn save_and_quit_holds_the_dialog_when_the_save_fails() {
    let (mut harness, state) = pending_close();
    state.lock().unwrap().project_path = Some(std::path::PathBuf::from(
        "//?/nonexistent-volume/show.qproj",
    ));
    settle(&mut harness);

    harness.get_by_label("Save & Quit").click();
    settle(&mut harness);

    let s = state.lock().unwrap();
    assert!(!s.quit, "a failed save must not quit");
    assert!(s.pending_close_confirm, "the dialog should stay up");
    assert!(
        s.operator_alert.is_some(),
        "the operator should be told why"
    );
}

/// egui gives the modal layer to the most recently created modal area and
/// blocks input below it, so a modal opening after this one used to leave a
/// visible dialog whose buttons did nothing and which Escape could not close.
#[test]
fn a_later_modal_cannot_wedge_the_dialog() {
    let (mut harness, state) = pending_close();
    assert!(modal_visible(&harness));

    state.lock().unwrap().show_about_window = true;
    settle(&mut harness);
    assert!(modal_visible(&harness), "the dialog should stay up");

    harness.get_by_label("Discard & Quit").click();
    settle(&mut harness);
    assert!(
        state.lock().unwrap().quit,
        "the dialog must stay clickable under a later modal"
    );
}

/// Same collision, checked through Escape rather than the buttons.
#[test]
fn escape_still_works_under_a_later_modal() {
    let (mut harness, state) = pending_close();
    state.lock().unwrap().show_about_window = true;
    settle(&mut harness);

    harness.key_press(egui::Key::Escape);
    settle(&mut harness);
    assert!(
        !state.lock().unwrap().pending_close_confirm,
        "Escape must reach the quit dialog, not the modal above it"
    );
}

/// The other ordering: a modal already open when the close request arrives.
#[test]
fn a_close_request_during_another_modal_still_resolves() {
    let (mut harness, state) = harness_for(preview::demo_app());
    {
        let mut s = state.lock().unwrap();
        s.last_seen_release_notes = Some(cuepool_gui::RELEASE_NOTES_VERSION.into());
        s.show_about_window = true;
    }
    harness.input_mut().time = Some(60.0);
    settle(&mut harness);

    state.lock().unwrap().pending_close_confirm = true;
    settle(&mut harness);

    harness.get_by_label("Discard & Quit").click();
    settle(&mut harness);
    assert!(state.lock().unwrap().quit);
}
