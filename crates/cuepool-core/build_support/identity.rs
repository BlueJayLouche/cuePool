use std::path::Path;
use std::process::{Command, Stdio};
use std::{fs, io::Write};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub version: String,
    pub build_id: Option<String>,
    pub commit: Option<String>,
    pub dirty: Option<bool>,
    pub source_fingerprint: Option<String>,
    pub version_tag: Option<String>,
    pub commits_since_tag: Option<u64>,
    pub shallow: bool,
    pub publication: String,
    pub display: String,
    pub changes_baseline: String,
    pub changes: String,
}

fn git_bytes(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .args(["--no-optional-locks", "-C"])
        .arg(root)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    String::from_utf8(git_bytes(root, args)?)
        .ok()
        .map(|s| s.trim().to_owned())
}

fn version(tag: &str) -> Option<(u64, u64, u64)> {
    let numbers: Vec<_> = tag.trim_start_matches('v').split('.').collect();
    if numbers.len() != 3 {
        return None;
    }
    Some((
        numbers[0].parse().ok()?,
        numbers[1].parse().ok()?,
        numbers[2].parse().ok()?,
    ))
}

fn tags(root: &Path) -> Vec<String> {
    git(
        root,
        &[
            "tag",
            "--merged",
            "HEAD",
            "--sort=-version:refname",
            "--list",
            "v[0-9]*",
        ],
    )
    .unwrap_or_default()
    .lines()
    .filter(|t| version(t).is_some())
    .map(str::to_owned)
    .collect()
}

fn distance(root: &Path, from: &str) -> Option<u64> {
    git(root, &["rev-list", "--count", &format!("{from}..HEAD")])?
        .parse()
        .ok()
}

fn published(root: &Path, tag: &str) -> bool {
    let marker = format!("refs/tags/published/{tag}");
    let Some(commit) = git(root, &["rev-parse", &format!("{tag}^{{commit}}")]) else {
        return false;
    };
    if git(root, &["rev-parse", &format!("{marker}^{{commit}}")]).as_deref() != Some(&commit) {
        return false;
    }
    let entry: serde_json::Value = git(root, &["for-each-ref", "--format=%(contents)", &marker])
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    entry["tag"].as_str() == Some(tag)
        && entry["commit"].as_str() == Some(&commit)
        && entry["published_at"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
        && entry["url"].as_str().is_some_and(|s| {
            s.starts_with("https://github.com/BlueJayLouche/cuePool/releases/tag/")
        })
}

fn fingerprint(root: &Path, status: &[u8]) -> Option<String> {
    // Git's own object hash covers paths, staged state, binary diffs, and the
    // contents of untracked files. This never writes objects or alters the index.
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut input = child.stdin.take()?;
    input.write_all(status).ok()?;
    let mut diff = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--no-ext-diff", "--binary", "HEAD", "--"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    std::io::copy(&mut diff.stdout.take()?, &mut input).ok()?;
    if !diff.wait().ok()?.success() {
        return None;
    }
    for path in git_bytes(root, &["ls-files", "--others", "--exclude-standard", "-z"])?
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
    {
        let path_str = std::str::from_utf8(path).ok()?;
        input.write_all(path).ok()?;
        input.write_all(&[0]).ok()?;
        let absolute = root.join(path_str);
        if absolute.is_symlink() {
            input
                .write_all(fs::read_link(absolute).ok()?.to_string_lossy().as_bytes())
                .ok()?;
        } else {
            std::io::copy(&mut fs::File::open(absolute).ok()?, &mut input).ok()?;
        }
        input.write_all(&[0]).ok()?;
    }
    drop(input);
    let output = child.wait_with_output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Select only maintained entries newer than the comparison baseline. Do not
/// include older release copy, or later sections in an archive with a stale version.
pub fn changelog_excerpt(text: &str, current: &str, baseline: Option<&str>) -> String {
    let current = version(current);
    let baseline = baseline.and_then(version);
    let mut selected = false;
    let mut result = String::new();
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## [").and_then(|s| s.split(']').next()) {
            selected = heading == "Unreleased"
                || version(heading).is_some_and(|v| {
                    current.is_some_and(|c| match baseline {
                        Some(b) => v <= c && v > b,
                        None => v == c,
                    })
                });
        }
        if selected {
            result.push_str(line);
            result.push('\n');
        }
    }
    result.trim().to_owned()
}

fn maintenance(path: &str) -> bool {
    path.starts_with(".github/")
        || path.starts_with("docs/")
        || path.starts_with("guide/")
        || matches!(
            path,
            "AGENTS.md" | "README.md" | "CHANGELOG.md" | "release-plz.toml"
        )
}

fn history(root: &Path, baseline: Option<&str>) -> String {
    let range = baseline.map_or_else(|| "HEAD".to_owned(), |t| format!("{t}..HEAD"));
    let Some(log) = git(
        root,
        &[
            "log",
            "--first-parent",
            "--format=%H%x09%s",
            "-61",
            &range,
            "--",
        ],
    ) else {
        return "Commit history unavailable.\n".into();
    };
    let mut app = Vec::new();
    let mut upkeep = Vec::new();
    let mut other = Vec::new();
    for line in log.lines().take(60) {
        let Some((sha, subject)) = line.split_once('\t') else {
            continue;
        };
        // Comparing with the first parent also covers ordinary merge commits.
        let files = git(
            root,
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--name-only",
                "-r",
                "-m",
                sha,
            ],
        );
        let bucket = match files.as_deref() {
            Some(paths) if !paths.is_empty() && paths.lines().all(maintenance) => &mut upkeep,
            Some(paths)
                if paths.lines().any(|p| {
                    p.starts_with("crates/")
                        || p.starts_with("mcp/")
                        || matches!(p, "Cargo.toml" | "Cargo.lock")
                }) =>
            {
                &mut app
            }
            _ => &mut other,
        };
        bucket.push(format!("- {subject} ({})", &sha[..7]));
    }
    let mut out = String::new();
    for (name, entries) in [
        ("Application history", app),
        ("Build maintenance and documentation", upkeep),
        ("Other history", other),
    ] {
        if !entries.is_empty() {
            out.push_str(&format!("\n### {name}\n{}\n", entries.join("\n")));
        }
    }
    if log.lines().count() > 60 {
        out.push_str("\nShowing the latest 60 commits; older changes are omitted.\n");
    }
    if out.is_empty() {
        out.push_str("No commits in this comparison. This does not describe uncommitted edits.\n");
    }
    out
}

pub fn collect(root: &Path, package_version: &str, override_id: Option<&str>) -> Metadata {
    let mut m = Metadata {
        version: package_version.into(),
        publication: "unverified".into(),
        ..Metadata::default()
    };
    let override_id = override_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    // An extracted archive inside some other repository is not that repository.
    let is_root = git(root, &["rev-parse", "--show-toplevel"])
        .and_then(|p| fs::canonicalize(p).ok())
        == fs::canonicalize(root).ok();
    if is_root {
        m.commit = git(root, &["rev-parse", "--verify", "HEAD"]);
    }
    let changelog = fs::read_to_string(root.join("CHANGELOG.md")).unwrap_or_default();
    if let Some(commit) = m.commit.clone() {
        m.build_id = override_id.clone().or_else(|| Some(commit[..7].into()));
        let status = git_bytes(
            root,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        );
        m.dirty = status.as_ref().map(|s| !s.is_empty());
        if m.dirty == Some(true) {
            m.source_fingerprint = status.as_ref().and_then(|s| fingerprint(root, s));
        }
        m.shallow = git(root, &["rev-parse", "--is-shallow-repository"]).as_deref() == Some("true");
        let candidates = tags(root);
        let nearest = candidates
            .iter()
            .filter_map(|tag| distance(root, tag).map(|d| (tag, d)))
            .min_by_key(|(_, d)| *d);
        if let Some((tag, d)) = nearest {
            m.version_tag = Some(tag.clone());
            m.commits_since_tag = Some(d);
            if published(root, tag) {
                m.publication = "published".into();
            }
        }
        // Prefer a confirmed published baseline, otherwise be explicit that this
        // comparison starts at a version tag whose publication is unverified.
        // At an exact tag compare against the preceding version, not itself.
        let baseline = candidates
            .iter()
            .filter(|tag| distance(root, tag).is_some_and(|n| n > 0))
            .filter(|tag| published(root, tag))
            .min_by_key(|tag| distance(root, tag).unwrap_or(u64::MAX))
            .or_else(|| {
                candidates
                    .iter()
                    .filter(|tag| distance(root, tag).is_some_and(|n| n > 0))
                    .min_by_key(|tag| distance(root, tag).unwrap_or(u64::MAX))
            });
        m.changes_baseline = baseline.map_or_else(
            || "No earlier version baseline available; visible history only".into(),
            |tag| {
                format!(
                    "{tag} ({})",
                    if published(root, tag) {
                        "published release"
                    } else {
                        "version tag; publication unverified"
                    }
                )
            },
        );
        let excerpt = changelog_excerpt(&changelog, package_version, baseline.map(String::as_str));
        m.changes = if excerpt.is_empty() {
            String::new()
        } else {
            format!("{excerpt}\n\n")
        };
        m.changes
            .push_str(&history(root, baseline.map(String::as_str)));
        if m.dirty == Some(true) {
            m.changes.push_str(
                "\nLocally modified: commit descriptions do not cover uncommitted edits.\n",
            );
        }
        if m.shallow {
            m.changes.push_str(
                "\nShallow checkout: tags, comparison baselines and history may be incomplete.\n",
            );
        }
        let position = match (&m.version_tag, m.commits_since_tag) {
            (Some(tag), Some(0)) if tag == &format!("v{package_version}") => {
                format!("Version tag {tag}; {}", m.publication)
            }
            (Some(tag), Some(n)) => format!(
                "Development: {n} commits beyond {tag}; tag {}",
                m.publication
            ),
            _ => "Development: version baseline unavailable".into(),
        };
        let state = match m.dirty {
            Some(false) => "clean".into(),
            Some(true) => format!(
                "locally modified {}",
                m.source_fingerprint
                    .as_deref()
                    .map(|s| &s[..12])
                    .unwrap_or("(fingerprint unavailable)")
            ),
            None => "checkout state unavailable".into(),
        };
        m.display = format!("{package_version} · {} · {state}\n{position}", &commit[..7]);
        if let Some(id) = override_id {
            m.display.push_str(&format!(" · Build {id}"));
        }
        if m.shallow {
            m.display.push_str(" · shallow history");
        }
    } else {
        m.build_id = override_id;
        m.display = format!(
            "{package_version} · {} · Source identity unavailable",
            m.build_id.as_ref().map_or_else(
                || "Build identity unavailable".into(),
                |s| format!("Build {s}")
            )
        );
        m.changes_baseline = "Source history unavailable; maintained notes for this version".into();
        m.changes = changelog_excerpt(&changelog, package_version, None);
        if m.changes.is_empty() {
            m.changes = "Change information unavailable in this source archive.".into();
        }
    }
    m
}

impl Metadata {
    pub fn rust_source(&self) -> String {
        let option = |s: &Option<String>| {
            s.as_ref()
                .map_or_else(|| "None".into(), |s| format!("Some({s:?})"))
        };
        format!(
            "pub const BUILD: BuildIdentity = BuildIdentity {{\nversion: {:?},\nbuild_id: {},\ncommit: {},\ndirty: {:?},\nsource_fingerprint: {},\nversion_tag: {},\ncommits_since_tag: {:?},\nshallow: {},\npublication: {:?},\ndisplay: {:?},\nchanges_baseline: {:?},\nchanges: {:?},\n}};\n",
            self.version,
            option(&self.build_id),
            option(&self.commit),
            self.dirty,
            option(&self.source_fingerprint),
            option(&self.version_tag),
            self.commits_since_tag,
            self.shallow,
            self.publication,
            self.display,
            self.changes_baseline,
            self.changes
        )
    }
}
