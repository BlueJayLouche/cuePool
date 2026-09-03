use cuepool_core::LockExt;
use cuepool_gui::SharedStateHandle;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct AppSettings {
    pub(crate) recent_files: Vec<std::path::PathBuf>,
    pub(crate) last_seen_release_notes: Option<String>,
    /// Master output gain in dB. Per machine: a room trim belongs to the box
    /// driving the room, not to a show file that may arrive from another one.
    pub(crate) master_volume_db: f32,
}

pub(crate) const AUTOMATION_PROFILE_ENV: &str = "CUEPOOL_AUTOMATION_PROFILE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppProfile {
    Default,
    Automation(String),
}

impl AppProfile {
    pub(crate) fn from_env() -> Result<Self, String> {
        match std::env::var(AUTOMATION_PROFILE_ENV) {
            Ok(value) if value.is_empty() => Ok(Self::Default),
            Ok(value) if valid_profile_name(&value) => Ok(Self::Automation(value)),
            Ok(_) => Err(format!(
                "{AUTOMATION_PROFILE_ENV} must contain 1 to 64 lowercase letters, digits, or hyphens, start with a letter or digit, and not be a reserved Windows device name"
            )),
            Err(std::env::VarError::NotPresent) => Ok(Self::Default),
            Err(std::env::VarError::NotUnicode(_)) => {
                Err(format!("{AUTOMATION_PROFILE_ENV} must be valid Unicode"))
            }
        }
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::Automation(name) => name,
        }
    }

    pub(crate) fn lock_name(&self) -> String {
        let name = match self {
            Self::Default => "CuePool".to_string(),
            Self::Automation(name) => format!("CuePool-automation-{name}"),
        };
        #[cfg(unix)]
        return std::env::temp_dir()
            .join(format!("{name}.lock"))
            .to_string_lossy()
            .into_owned();
        #[cfg(not(unix))]
        return name;
    }

    pub(crate) fn settings_path(&self) -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|root| self.path_in(root, "settings.json"))
    }

    pub(crate) fn persistent_log_path(&self) -> std::path::PathBuf {
        self.path_in(
            dirs::data_dir().unwrap_or_else(std::env::temp_dir),
            "cuepool.log",
        )
    }

    fn path_in(&self, root: std::path::PathBuf, filename: &str) -> std::path::PathBuf {
        let path = root.join("CuePool");
        match self {
            Self::Default => path.join(filename),
            Self::Automation(name) => path.join("automation").join(name).join(filename),
        }
    }
}

fn valid_profile_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !matches!(name, "con" | "prn" | "aux" | "nul")
        && !(name.len() == 4
            && matches!(&name[..3], "com" | "lpt")
            && matches!(name.as_bytes()[3], b'1'..=b'9'))
}

/// Read settings from an explicit path. Split from [`load_settings`] so the
/// corrupt-file behaviour is testable without the real config directory.
///
/// A missing file is the ordinary first run and stays quiet. Anything else is
/// reported, because falling back to defaults here means the next
/// [`save_settings_at`] writes those defaults over whatever is on disk.
fn load_settings_at(path: &std::path::Path) -> AppSettings {
    match std::fs::read_to_string(path) {
        Ok(data) => match serde_json::from_str(&data) {
            Ok(settings) => return settings,
            Err(error) => log::error!(
                "{} is unreadable as settings ({error}); continuing with empty settings, which will replace the file on exit",
                path.display()
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::warn!(
            "Could not read {} ({error}); continuing with empty settings",
            path.display()
        ),
    }
    AppSettings::default()
}

pub(crate) fn load_settings(profile: &AppProfile) -> AppSettings {
    profile
        .settings_path()
        .map(|path| load_settings_at(&path))
        .unwrap_or_default()
}

/// Write settings to an explicit path, via a sibling temp file and a rename.
///
/// A plain write is not atomic: losing power or being killed partway through
/// leaves a truncated file, which [`load_settings_at`] can only treat as empty,
/// silently costing the user their recent files. Renaming over the target means
/// a reader sees either the old file or the new one.
fn save_settings_at(path: &std::path::Path, settings: &AppSettings) {
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        log::warn!(
            "Could not create {} ({error}); settings not saved",
            parent.display()
        );
        return;
    }

    let data = match serde_json::to_string_pretty(settings) {
        Ok(data) => data,
        Err(error) => {
            log::error!("Could not serialise settings ({error}); settings not saved");
            return;
        }
    };

    // Same directory as the target, so the rename stays on one filesystem. The
    // pid keeps concurrent automation profiles from sharing a temp file.
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    if let Err(error) = std::fs::write(&temp, &data) {
        log::warn!(
            "Could not write {} ({error}); settings not saved",
            temp.display()
        );
        return;
    }
    if let Err(error) = std::fs::rename(&temp, path) {
        log::warn!(
            "Could not replace {} ({error}); settings not saved",
            path.display()
        );
        let _ = std::fs::remove_file(&temp);
    }
}

pub(crate) fn save_settings(profile: &AppProfile, settings: &AppSettings) {
    match profile.settings_path() {
        Some(path) => save_settings_at(&path, settings),
        None => log::warn!("No config directory available; settings not saved"),
    }
}

/// Snapshot the persistable settings out of shared state. Split out from
/// [`save_settings_from_state`] so the poison behaviour is testable without
/// touching the filesystem, and so the guard drops before the write.
fn settings_from_state(state: &SharedStateHandle) -> AppSettings {
    // lock_unpoisoned: a poisoned state lock must not fall back to defaults
    // here — save_settings would overwrite the user's settings.json with them,
    // erasing recent_files and re-showing the release notes. Recovered state
    // may be partial (see LockExt), but a half-updated list beats an empty one.
    let state = state.lock_unpoisoned();
    AppSettings {
        recent_files: state.recent_files.clone(),
        last_seen_release_notes: state.last_seen_release_notes.clone(),
        master_volume_db: state.master_volume_db,
    }
}

pub(crate) fn save_settings_from_state(profile: &AppProfile, state: &SharedStateHandle) {
    save_settings(profile, &settings_from_state(state));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this guards against wrote `AppSettings::default()` over the
    /// user's settings.json whenever any thread had panicked while holding the
    /// state lock, silently losing their recent files and re-showing the
    /// release notes.
    #[test]
    fn a_poisoned_lock_keeps_the_real_settings() {
        let state: SharedStateHandle =
            std::sync::Arc::new(std::sync::Mutex::new(cuepool_gui::SharedState::default()));
        {
            let mut guard = state.lock().unwrap();
            guard.recent_files = vec![std::path::PathBuf::from("/shows/gala.qproj")];
            guard.last_seen_release_notes = Some("9.9.9".into());
            guard.master_volume_db = -6.0;
        }

        // Poison it the way a real panic does: unwind while holding the guard.
        let poisoner = std::sync::Arc::clone(&state);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the state lock");
        })
        .join();
        assert!(state.is_poisoned(), "the lock should be poisoned by now");

        let settings = settings_from_state(&state);

        assert_eq!(
            settings.recent_files,
            vec![std::path::PathBuf::from("/shows/gala.qproj")],
            "a poisoned lock must not blank recent_files"
        );
        assert_eq!(settings.last_seen_release_notes.as_deref(), Some("9.9.9"));
        assert_eq!(settings.master_volume_db, -6.0);
    }

    /// A scratch directory that removes itself, so these tests never touch the
    /// real config directory.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("cuepool-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch dir");
            Self(path)
        }

        fn join(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn populated() -> AppSettings {
        AppSettings {
            recent_files: vec![std::path::PathBuf::from("/shows/gala.qproj")],
            last_seen_release_notes: Some("9.9.9".into()),
            master_volume_db: -6.0,
        }
    }

    #[test]
    fn settings_survive_a_save_and_load() {
        let dir = TempDir::new("roundtrip");
        let path = dir.join("settings.json");

        save_settings_at(&path, &populated());
        let loaded = load_settings_at(&path);

        assert_eq!(loaded.recent_files, populated().recent_files);
        assert_eq!(loaded.last_seen_release_notes.as_deref(), Some("9.9.9"));
    }

    static CAPTURED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    struct CaptureLogger;

    impl log::Log for CaptureLogger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            CAPTURED
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(record.args().to_string());
        }

        fn flush(&self) {}
    }

    /// Lines logged so far that mention `needle`. Every test here uses a unique
    /// scratch directory name, so filtering on it isolates each from the others
    /// running in parallel.
    fn logged_about(needle: &str) -> Vec<String> {
        let _ = log::set_logger(&CaptureLogger);
        log::set_max_level(log::LevelFilter::Trace);
        CAPTURED
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|line| line.contains(needle))
            .cloned()
            .collect()
    }

    /// The destructive loop this guards: a truncated file loads as defaults, and
    /// the next save writes those defaults over it. Once the file is broken the
    /// data is gone either way, so the thing that must not happen is silence.
    #[test]
    fn a_corrupt_settings_file_is_reported() {
        let dir = TempDir::new("corrupt");
        let path = dir.join("settings.json");
        logged_about("install the logger");
        save_settings_at(&path, &populated());

        // Truncate it the way an interrupted non-atomic write would.
        let full = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, &full[..full.len() / 2]).unwrap();
        assert!(
            serde_json::from_str::<AppSettings>(&std::fs::read_to_string(&path).unwrap()).is_err(),
            "the truncated file should not parse, or this test proves nothing"
        );

        let loaded = load_settings_at(&path);

        assert!(loaded.recent_files.is_empty());
        let complaints = logged_about("cuepool-corrupt");
        assert!(
            !complaints.is_empty(),
            "a corrupt settings file must not load silently"
        );
    }

    /// The mirror of the above. A first run has no file and must stay quiet, or
    /// the report above becomes noise that everyone learns to skip past.
    #[test]
    fn a_missing_settings_file_stays_quiet() {
        let dir = TempDir::new("missing");
        logged_about("install the logger");

        let loaded = load_settings_at(&dir.join("settings.json"));

        assert!(loaded.recent_files.is_empty());
        let complaints = logged_about("cuepool-missing");
        assert!(
            complaints.is_empty(),
            "a first run should log nothing, got: {complaints:?}"
        );
    }

    /// Atomicity itself is `std::fs::rename`'s contract rather than something to
    /// re-test here. What is worth pinning is that the temp file never outlives
    /// the save, since a leaked one would accumulate in the config directory.
    #[test]
    fn saving_leaves_no_temp_file_behind() {
        let dir = TempDir::new("atomic");
        let path = dir.join("settings.json");
        save_settings_at(&path, &AppSettings::default());

        save_settings_at(&path, &populated());

        let leftovers: Vec<_> = std::fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "settings.json")
            .collect();
        assert!(
            leftovers.is_empty(),
            "stray files left behind: {leftovers:?}"
        );
        assert_eq!(
            load_settings_at(&path).recent_files,
            populated().recent_files
        );
    }

    #[test]
    fn old_settings_leave_release_notes_unseen() {
        let settings: AppSettings = serde_json::from_str(r#"{"recent_files": []}"#).unwrap();

        assert_eq!(settings.last_seen_release_notes, None);
    }

    #[test]
    fn default_profile_keeps_existing_paths() {
        let root = std::path::PathBuf::from("root");
        let profile = AppProfile::Default;

        assert_eq!(profile.name(), "default");
        assert_eq!(
            profile.path_in(root.clone(), "settings.json"),
            root.join("CuePool").join("settings.json")
        );
        assert_eq!(
            profile.path_in(root, "cuepool.log"),
            std::path::PathBuf::from("root")
                .join("CuePool")
                .join("cuepool.log")
        );
    }

    #[test]
    fn automation_profiles_are_validated_and_isolated() {
        assert!(valid_profile_name("smoke-a"));
        for invalid in [
            "",
            "UPPER",
            "has_space",
            "../escape",
            "-leading",
            "con",
            "com1",
            "lpt9",
        ] {
            assert!(!valid_profile_name(invalid), "{invalid}");
        }

        let root = std::path::PathBuf::from("root");
        let first = AppProfile::Automation("smoke-a".into());
        let second = AppProfile::Automation("smoke-b".into());
        assert_ne!(
            first.path_in(root.clone(), "settings.json"),
            second.path_in(root.clone(), "settings.json")
        );
        assert_ne!(first.lock_name(), second.lock_name());
        assert_eq!(
            first.path_in(root, "cuepool.log"),
            std::path::PathBuf::from("root")
                .join("CuePool")
                .join("automation")
                .join("smoke-a")
                .join("cuepool.log")
        );
    }
}
