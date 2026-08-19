//! Route the macOS menu-bar / Dock quit through the in-app quit-confirm modal.
//!
//! winit installs a default macOS menu whose Quit item sends `terminate:`
//! (winit-0.30.13, `src/platform_impl/macos/menu.rs`). AppKit takes that
//! straight to `applicationWillTerminate:`, which winit turns into
//! `Event::LoopExiting` — it never produces a `WindowEvent::CloseRequested`, so
//! the dirty/active-cue check on the close path never ran. Cmd-Q, Apple menu ->
//! Quit and Dock -> Quit therefore all skipped the modal #218 added, and the
//! operator's unsaved edits went out with the process. The window's X button was
//! the only way out that asked.
//!
//! `applicationShouldTerminate:` runs before any of that and may veto, so it is
//! the hook. When there is work to lose it answers `NSTerminateCancel` and
//! leaves a note for the event loop to raise the modal; confirming there sets
//! `quit`, which the loop turns into the usual `hard_exit`. With nothing to lose
//! it answers `NSTerminateNow` and gets out of the way — which also keeps a
//! logout or restart moving instead of cancelling it under the user.
//!
//! # Why the hook touches no locks
//!
//! It runs on the main thread, and AppKit can dispatch it while that thread is
//! deep inside a nested run loop — `rfd`'s native file picker is one, and the
//! inspector holds the shared-state guard across it (`inspector::show`). Dock ->
//! Quit is not suppressed during an app-modal panel the way the menu bar is, so
//! a hook that locked shared state could be asked for an answer by the very
//! thread already holding that lock: `std::sync::Mutex` is not reentrant, and
//! that is a beachball mid-show. So the hook only touches atomics and leaves the
//! state write to `about_to_wait`, which owns the lock safely. The prompt lands
//! a tick later — or, if a modal panel is blocking the loop, as soon as it
//! closes, which is the first moment the operator could see it anyway.

// objc FFI: method-IMP transmutes (no clean named target type) and a selector
// type-encoding string that reads clearest as a raw nul-terminated literal.
#![allow(clippy::missing_transmute_annotations, clippy::manual_c_str_literals)]

use cuepool_gui::logging::PERSIST_TARGET;
use std::sync::atomic::{AtomicBool, Ordering};

/// `NSApplicationTerminateReply`. An `NSUInteger`, so `usize` on every Apple
/// target Rust builds for (`@encode` gives "Q").
const TERMINATE_CANCEL: usize = 0;
const TERMINATE_NOW: usize = 1;

/// Whether quitting right now would lose work, republished by the event loop
/// every tick. The authoritative answer needs `ShowEngine`, which lives in
/// `App` on the main thread and is not reachable from an AppKit callback, so
/// the loop leaves the answer here for the hook to read.
static NEEDS_CONFIRM: AtomicBool = AtomicBool::new(false);

/// Set when the hook has vetoed a quit: the event loop owes the operator the
/// confirm modal, in a window they can actually see it in.
static CONFIRM_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Publish whether a quit needs confirming. Called every event-loop tick.
pub fn publish_needs_confirm(needs_confirm: bool) {
    NEEDS_CONFIRM.store(needs_confirm, Ordering::Relaxed);
}

/// True once for each quit the hook vetoed. The caller owes the operator the
/// modal and a control window that is actually on screen.
pub fn take_confirm_request() -> bool {
    CONFIRM_REQUESTED.swap(false, Ordering::Relaxed)
}

/// The `applicationShouldTerminate:` decision, split out so it can be tested
/// without an `NSApplication`.
fn terminate_reply(needs_confirm: bool) -> usize {
    if !needs_confirm {
        return TERMINATE_NOW;
    }
    CONFIRM_REQUESTED.store(true, Ordering::Relaxed);
    log::info!(
        target: PERSIST_TARGET,
        "Quit requested from the macOS menu with work to lose; asking the operator"
    );
    TERMINATE_CANCEL
}

/// Install the hook. Must run after the event loop is built — winit registers
/// `WinitApplicationDelegate` in `EventLoop::new`, not in `run_app`, so calling
/// this earlier would find no class to graft onto.
///
/// # Safety Note
///
/// This uses `class_addMethod` + `std::mem::transmute` to graft a method onto
/// winit's delegate class at runtime, the same trick (and the same exposure) as
/// `crates/rustjay-engine/src/app/macos.rs`. If winit renames that class or
/// starts defining this selector itself the behaviour changes silently —
/// `class_addMethod` does not replace an existing method, it refuses — so both
/// outcomes are logged rather than assumed. winit 0.30.13 declares only
/// `applicationDidFinishLaunching:` and `applicationWillTerminate:` on the
/// class, so nothing here overrides winit's own behaviour. Re-check on any
/// winit upgrade.
pub fn install() {
    use objc::runtime::{Class, NO, Object, Sel, class_addMethod};
    use objc::{sel, sel_impl};
    use std::mem;
    use std::os::raw::c_char;

    extern "C" fn should_terminate(_self: &Object, _sel: Sel, _sender: *mut Object) -> usize {
        terminate_reply(NEEDS_CONFIRM.load(Ordering::Relaxed))
    }

    unsafe {
        let Some(delegate_class) = Class::get("WinitApplicationDelegate") else {
            log::warn!(
                target: PERSIST_TARGET,
                "WinitApplicationDelegate class not found — macOS quit hook not installed. \
                 Quitting from the menu bar or the Dock will skip the unsaved-changes prompt."
            );
            return;
        };

        let cls = delegate_class as *const _ as *mut Class;
        // NSUInteger return; (id self, SEL _cmd, id sender).
        let enc = "Q@:@\0".as_ptr() as *const c_char;
        let added = class_addMethod(
            cls,
            sel!(applicationShouldTerminate:),
            mem::transmute::<extern "C" fn(&Object, Sel, *mut Object) -> usize, _>(
                should_terminate,
            ),
            enc,
        );
        if added == NO {
            log::warn!(
                target: PERSIST_TARGET,
                "WinitApplicationDelegate already answers applicationShouldTerminate: — \
                 macOS quit hook not installed. Quitting from the menu bar or the Dock \
                 will skip the unsaved-changes prompt."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The statics are process-wide, so the reply tests share a lock rather than
    /// racing each other under the default multi-threaded test runner.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_clean_show_quits_without_a_prompt() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        CONFIRM_REQUESTED.store(false, Ordering::Relaxed);

        assert_eq!(terminate_reply(false), TERMINATE_NOW);
        assert!(
            !take_confirm_request(),
            "nothing to lose should not raise the dialog"
        );
    }

    /// The bug: this path used to reach `applicationWillTerminate:` and take the
    /// show with it.
    #[test]
    fn work_to_lose_cancels_the_quit_and_asks() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        CONFIRM_REQUESTED.store(false, Ordering::Relaxed);

        assert_eq!(terminate_reply(true), TERMINATE_CANCEL);
        assert!(
            take_confirm_request(),
            "the operator must be asked before the show is discarded"
        );
    }

    /// The event loop acts on the request once; a consumed request must not
    /// re-open the dialog on every later tick.
    #[test]
    fn a_vetoed_quit_is_claimed_exactly_once() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        CONFIRM_REQUESTED.store(false, Ordering::Relaxed);

        terminate_reply(true);
        assert!(take_confirm_request());
        assert!(!take_confirm_request(), "the request should be consumed");
    }
}
