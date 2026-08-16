use cuepool_core::LockExt;
use cuepool_gui::SharedStateHandle;
use cuepool_gui::logging::PERSIST_TARGET;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Autosave background thread: writes dirty show file to rotating backups every 60 s.
pub(crate) fn spawn_autosave_thread(state: SharedStateHandle, running: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut slot = 0usize;
        let mut elapsed = 0u64;
        while running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(1));
            if !running.load(Ordering::Relaxed) {
                break;
            }
            elapsed += 1;
            if elapsed < 60 {
                continue;
            }
            elapsed = 0;
            let (should_save, path, autosave_enabled) = {
                let state = state.lock_unpoisoned();
                (
                    state.dirty,
                    state.project_path.clone(),
                    state.show_file.show_settings.autosave_enabled,
                )
            };
            if !autosave_enabled || !should_save {
                continue;
            }
            let Some(_project_path) = path else { continue };

            let dir = dirs::data_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("CuePool");
            if let Err(e) = std::fs::create_dir_all(&dir) {
                log::warn!("Autosave: failed to create dir {:?}: {}", dir, e);
                continue;
            }

            slot = (slot % 5) + 1;
            let backup_path = dir.join(format!("autoback_{}.qproj", slot));
            let json = {
                let state = state.lock_unpoisoned();
                match serde_json::to_string_pretty(&state.show_file) {
                    Ok(j) => j,
                    Err(e) => {
                        log::warn!("Autosave: serialization failed: {}", e);
                        continue;
                    }
                }
            };
            if let Err(e) = std::fs::write(&backup_path, json) {
                log::warn!("Autosave: failed to write {:?}: {}", backup_path, e);
            } else {
                log::info!("Autosaved to {:?}", backup_path);
            }
        }
    });
}

/// Serialise the show for a recovery write, with the path to overwrite if the
/// project has one. Split out from [`emergency_save`] so the poison behaviour is
/// testable without touching the filesystem.
fn recovery_payload(state: &SharedStateHandle) -> Option<(String, Option<std::path::PathBuf>)> {
    // lock_unpoisoned: a panic is the reason this runs at all, so a poisoned
    // lock is the expected case, not a reason to skip. Bailing here threw the
    // operator's show away while the caller still went on to save settings.
    let state = state.lock_unpoisoned();
    match serde_json::to_string_pretty(&state.show_file) {
        Ok(json) => Some((json, state.project_path.clone())),
        Err(error) => {
            log::error!("Emergency save: serialization failed: {error}");
            None
        }
    }
}

/// Attempt an emergency save before the process exits.
pub(crate) fn emergency_save(state: &SharedStateHandle, reason: &str) {
    log::info!(target: PERSIST_TARGET, "Recovery save requested: {reason}");
    let Some((json, path)) = recovery_payload(state) else {
        return;
    };

    let dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("CuePool");
    let _ = std::fs::create_dir_all(&dir);

    // Prefer crash_recovery.qproj, but if a project_path exists, also save there
    let crash_path = dir.join("crash_recovery.qproj");
    if let Err(e) = std::fs::write(&crash_path, &json) {
        log::error!("Emergency save: failed to write {:?}: {}", crash_path, e);
    } else {
        log::info!(target: PERSIST_TARGET, "Recovery save written to {:?}", crash_path);
    }

    if let Some(project_path) = path {
        if let Err(e) = std::fs::write(&project_path, &json) {
            log::error!(
                "Emergency save: failed to overwrite {:?}: {}",
                project_path,
                e
            );
        } else {
            log::info!(target: PERSIST_TARGET, "Recovery save overwrote {:?}", project_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crash-recovery save is triggered *by* a panic, so it runs precisely
    /// when the state lock is most likely poisoned. Bailing out there lost the
    /// operator's cues while the settings write on the next line succeeded.
    #[test]
    fn a_poisoned_lock_still_yields_a_recovery_payload() {
        let state: SharedStateHandle =
            Arc::new(std::sync::Mutex::new(cuepool_gui::SharedState::default()));
        let project = std::path::PathBuf::from("/shows/gala.qproj");
        state.lock().unwrap().project_path = Some(project.clone());

        let poisoner = Arc::clone(&state);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the state lock");
        })
        .join();
        assert!(state.is_poisoned(), "the lock should be poisoned by now");

        let (json, path) =
            recovery_payload(&state).expect("a poisoned lock must not skip the recovery save");

        assert_eq!(path.as_deref(), Some(project.as_path()));
        assert!(
            json.contains("cues"),
            "the show file should have serialised, got: {}",
            &json[..json.len().min(80)]
        );
    }
}
