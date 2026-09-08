#[path = "../build_support/identity.rs"]
mod identity;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "cuepool-identity-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn init(&self) {
        run(&self.0, "git", &["init", "-b", "main"]);
        run(&self.0, "git", &["config", "user.name", "Identity test"]);
        run(
            &self.0,
            "git",
            &["config", "user.email", "identity@example.invalid"],
        );
        fs::write(self.0.join(".gitignore"), "target/\n").unwrap();
    }
    fn commit(&self, subject: &str) -> String {
        run(&self.0, "git", &["add", "."]);
        run(&self.0, "git", &["commit", "--allow-empty", "-m", subject]);
        run(&self.0, "git", &["rev-parse", "HEAD"]).trim().into()
    }
    fn identity(&self) -> identity::Metadata {
        identity::collect(&self.0, "0.12.3", None)
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(root: &Path, program: &str, args: &[&str]) -> String {
    let o = Command::new(program)
        .args(args)
        .current_dir(root)
        .env_remove("CUEPOOL_BUILD_ID")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

#[test]
fn tag_dirty_archive_and_publication_are_distinct() {
    let repo = Repo::new();
    let archive = repo.identity();
    assert!(archive.commit.is_none());
    assert!(archive.display.contains("identity unavailable"));
    let explicit = identity::collect(&repo.0, "0.12.3", Some("packager-123"));
    assert_eq!(explicit.build_id.as_deref(), Some("packager-123"));
    assert!(explicit.display.contains("Source identity unavailable"));
    repo.init();
    fs::write(repo.0.join("source.rs"), "old").unwrap();
    let first = repo.commit("fix: original source");
    run(&repo.0, "git", &["tag", "v0.12.3"]);
    let clean = repo.identity();
    assert_eq!(clean.commit.as_deref(), Some(first.as_str()));
    assert_eq!(clean.dirty, Some(false));
    assert_eq!(clean.commits_since_tag, Some(0));
    assert_eq!(clean.publication, "unverified");
    fs::write(repo.0.join("source.rs"), "changed").unwrap();
    let dirty = repo.identity();
    assert_eq!(dirty.dirty, Some(true));
    assert!(dirty.source_fingerprint.is_some());
    fs::write(repo.0.join("source.rs"), "changed again").unwrap();
    assert_ne!(dirty.source_fingerprint, repo.identity().source_fingerprint);
    run(&repo.0, "git", &["add", "source.rs"]);
    assert_eq!(repo.identity().dirty, Some(true));
    let second = repo.commit("fix: changed source");
    assert_eq!(repo.identity().commits_since_tag, Some(1));
    assert_eq!(
        repo.identity().changes_baseline,
        "v0.12.3 (version tag; publication unverified)"
    );
    let proof = format!(
        r#"{{"tag":"v0.12.3","commit":"{first}","url":"https://github.com/BlueJayLouche/cuePool/releases/tag/v0.12.3","published_at":"2026-09-08T00:00:00Z"}}"#
    );
    run(
        &repo.0,
        "git",
        &["tag", "-a", "published/v0.12.3", &first, "-m", &proof],
    );
    assert_eq!(repo.identity().publication, "published");
    run(&repo.0, "git", &["tag", "-d", "published/v0.12.3"]);
    run(
        &repo.0,
        "git",
        &["tag", "-a", "published/v0.12.3", &second, "-m", &proof],
    );
    assert_eq!(repo.identity().publication, "unverified");
    let generated = repo.identity().rust_source();
    assert!(generated.contains("BuildIdentity"));
    // No timestamp or unstable process state in identity generation.
    assert_eq!(repo.identity(), repo.identity());
}

#[test]
fn changelog_excludes_baseline_and_future_versions() {
    let notes = "# Changelog\n## [Unreleased]\n- pending\n## [0.13.0]\n- future\n## [0.12.3]\n- current\n## [0.12.2]\n- baseline\n";
    let selected = identity::changelog_excerpt(notes, "0.12.3", Some("v0.12.2"));
    assert_eq!(identity::changelog_excerpt(notes, "0.12.3", None), selected);
    assert!(selected.contains("pending") && selected.contains("current"));
    assert!(!selected.contains("future") && !selected.contains("baseline"));
}

#[test]
fn history_classifies_paths_and_uses_previous_baseline_at_exact_tag() {
    let repo = Repo::new();
    repo.init();
    fs::create_dir(repo.0.join("crates")).unwrap();
    fs::write(repo.0.join("crates/app.rs"), "a").unwrap();
    repo.commit("feat: application");
    run(&repo.0, "git", &["tag", "v0.12.2"]);
    fs::create_dir(repo.0.join(".github")).unwrap();
    fs::write(repo.0.join(".github/ci.yml"), "ci").unwrap();
    repo.commit("chore(deps): update checkout");
    run(&repo.0, "git", &["tag", "v0.12.3"]);
    let metadata = repo.identity();
    assert!(metadata.changes_baseline.starts_with("v0.12.2"));
    assert!(
        metadata
            .changes
            .contains("Build maintenance and documentation")
    );
    assert!(metadata.changes.contains("update checkout"));
    assert!(!metadata.changes.contains("feat: application"));
}

#[test]
fn detached_worktree_shallow_clone_and_nested_archive() {
    let repo = Repo::new();
    repo.init();
    repo.commit("fix: first");
    run(&repo.0, "git", &["tag", "v0.12.3"]);
    let head = repo.commit("fix: second");
    let worktree = Repo::new();
    run(
        &repo.0,
        "git",
        &[
            "worktree",
            "add",
            "--detach",
            worktree.0.to_str().unwrap(),
            &head,
        ],
    );
    assert_eq!(worktree.identity().commit.as_deref(), Some(head.as_str()));
    assert_eq!(worktree.identity().dirty, Some(false));
    let shallow = Repo::new();
    run(
        &shallow.0,
        "git",
        &[
            "clone",
            "--depth",
            "1",
            &format!("file://{}", repo.0.display()),
            ".",
        ],
    );
    assert_eq!(shallow.identity().commit.as_deref(), Some(head.as_str()));
    assert!(shallow.identity().shallow);
    assert!(shallow.identity().version_tag.is_none());
    let nested = repo.0.join("archive");
    fs::create_dir(&nested).unwrap();
    assert!(identity::collect(&nested, "0.12.3", None).commit.is_none());
}

#[test]
fn real_cargo_build_refreshes_without_cleaning_or_rust_changes() {
    let repo = Repo::new();
    repo.init();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crate_dir = repo.0.join("crates/probe");
    fs::create_dir_all(crate_dir.join("src")).unwrap();
    fs::create_dir(crate_dir.join("build_support")).unwrap();
    fs::write(
        repo.0.join("Cargo.toml"),
        "[workspace]\nmembers=[\"crates/probe\"]\nresolver=\"3\"\n",
    )
    .unwrap();
    fs::write(crate_dir.join("Cargo.toml"), "[package]\nname=\"identity-probe\"\nversion=\"0.12.3\"\nedition=\"2024\"\n[build-dependencies]\nserde_json=\"1\"\n").unwrap();
    fs::copy(source.join("build.rs"), crate_dir.join("build.rs")).unwrap();
    fs::copy(
        source.join("build_support/identity.rs"),
        crate_dir.join("build_support/identity.rs"),
    )
    .unwrap();
    let model = fs::read_to_string(source.join("src/build_identity.rs"))
        .unwrap()
        .replace("Debug, serde::Serialize", "Debug");
    fs::write(
        crate_dir.join("src/main.rs"),
        format!("{model}\nfn main() {{ println!(\"{{:?}}\", BUILD); }}"),
    )
    .unwrap();
    let cargo = || {
        let output = Command::new(env!("CARGO"))
            .args(["run", "--quiet", "--offline", "-p", "identity-probe"])
            .current_dir(&repo.0)
            .env("CARGO_TARGET_DIR", repo.0.join("target"))
            .env_remove("CUEPOOL_BUILD_ID")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    cargo(); // generate Cargo.lock before committing the fixture
    let first = repo.commit("fix: initial");
    run(&repo.0, "git", &["tag", "v0.12.3"]);
    let clean = cargo();
    assert!(clean.contains(&first));
    assert!(clean.contains("dirty: Some(false)"));
    assert_eq!(clean, cargo());
    fs::write(repo.0.join("new-source.txt"), "untracked").unwrap();
    let dirty = cargo();
    assert!(dirty.contains("dirty: Some(true)"));
    assert_ne!(clean, dirty);
    fs::write(repo.0.join("new-source.txt"), "different").unwrap();
    assert_ne!(dirty, cargo());
    let second = repo.commit("fix: source update");
    let moved = cargo();
    assert!(moved.contains(&second));
    assert!(moved.contains("commits_since_tag: Some(1)"));
    run(&repo.0, "git", &["checkout", "--detach", &first]);
    assert_eq!(clean, cargo());
    fs::remove_dir_all(repo.0.join(".git")).unwrap();
    let archive = cargo();
    assert!(archive.contains("commit: None"));
    assert!(archive.contains("Source identity unavailable"));
}
