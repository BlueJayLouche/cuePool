#!/usr/bin/env python3
"""Run real release-plz update on disposable miniature CuePool workspaces.

No GitHub token, release-pr, tag publication or cargo publish is used. The
inherited product version and package names match the real workspace.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
NAMES = sorted(tomllib.loads(p.read_text())["package"]["name"] for p in (ROOT / "crates").glob("*/Cargo.toml"))


def run(root, *args):
    result = subprocess.run(args, cwd=root, capture_output=True, text=True,
                            env={k: v for k, v in os.environ.items() if k not in ("GIT_TOKEN", "GITHUB_TOKEN", "GH_TOKEN")})
    if result.returncode:
        raise AssertionError(f"{args}:\n{result.stdout}\n{result.stderr}")
    return result.stdout


def exercise(subject, filename, content, expected, expected_group=None):
    with tempfile.TemporaryDirectory(prefix="cuepool-release-policy-") as tmp:
        root = Path(tmp)
        run(root, "git", "init", "-b", "main")
        run(root, "git", "config", "user.name", "Release policy test")
        run(root, "git", "config", "user.email", "release@example.invalid")
        (root / "Cargo.toml").write_text('[workspace]\nmembers=["crates/*"]\nresolver="3"\n[workspace.package]\nversion = "0.12.3"\nedition="2024"\nrepository="https://github.com/BlueJayLouche/cuePool"\n')
        (root / ".gitignore").write_text("target/\n")
        for name in NAMES:
            crate = root / "crates" / name
            (crate / "src").mkdir(parents=True)
            (crate / "src/lib.rs").write_text("pub fn sample() {}\n")
            if name == "cuepool":
                (crate / "src/main.rs").write_text("fn main() {}\n")
            (crate / "Cargo.toml").write_text(f'[package]\nname="{name}"\nversion.workspace=true\nedition.workspace=true\nrepository.workspace=true\n')
        # A deliberately older, still-compatible dependency proves release-plz
        # does not opportunistically upgrade unrelated registry dependencies.
        manifest = root / "crates/cuepool/Cargo.toml"
        if filename == "Cargo.toml":
            workspace = root / "Cargo.toml"
            workspace.write_text(workspace.read_text() + '\n[workspace.dependencies]\nitoa="=1.0.15"\n')
            manifest.write_text(manifest.read_text() + '\n[dependencies]\nitoa.workspace=true\n')
        else:
            manifest.write_text(manifest.read_text() + '\n[dependencies]\nitoa="1"\n')
        shutil.copy(ROOT / "release-plz.toml", root / "release-plz.toml")
        (root / "CHANGELOG.md").write_text("# Changelog\n\n## [Unreleased]\n\n## [0.12.3]\n\n- Initial version.\n")
        run(root, "cargo", "generate-lockfile")
        run(root, "cargo", "update", "-p", "itoa", "--precise", "1.0.15")
        run(root, "git", "add", ".")
        legacy = root / "crates/cuepool-harness/Cargo.toml"
        legacy.write_text(legacy.read_text().replace("[package]", "[package]\npublish=false"))
        run(root, "git", "add", ".")
        run(root, "git", "commit", "-m", "feat: historical baseline sentinel")
        run(root, "git", "tag", "v0.12.3")
        legacy.write_text(legacy.read_text().replace("publish=false\n", ""))
        path = root / filename
        path.parent.mkdir(parents=True, exist_ok=True)
        if filename == "Cargo.toml":
            path.write_text(path.read_text().replace('itoa="=1.0.15"', 'itoa="=1.0.16"'))
        elif filename == "Cargo.lock":
            pass  # The dependency-only update below supplies the actual change.
        elif content is None:
            # Dependency-only change in an internal crate, without source edits.
            path.write_text(path.read_text() + '\n[dependencies]\ncuepool-core={path="../cuepool-core"}\n')
        else:
            path.write_text(content)
        run(root, "cargo", "generate-lockfile")
        run(root, "cargo", "update", "-p", "itoa", "--precise", "1.0.16" if filename in ("Cargo.lock", "Cargo.toml") else "1.0.15")
        run(root, "git", "add", ".")
        if expected_group:
            changelog = root / "CHANGELOG.md"
            changelog.write_text(changelog.read_text().replace("## [Unreleased]", "## [Unreleased]\n\n- Operator impact: " + subject.splitlines()[0].split(": ", 1)[1]))
            run(root, "git", "add", "CHANGELOG.md")
        run(root, "git", "commit", "-m", subject)
        before = tomllib.loads((root / "Cargo.lock").read_text())["package"]
        run(root, "python3", str(ROOT / "scripts/prepare-release.py"), "update")
        version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        assert version == expected, (subject, version, expected)
        after = tomllib.loads((root / "Cargo.lock").read_text())["package"]
        assert {p["version"] for p in after if p["name"] in NAMES} == {expected}, {p["name"]: p["version"] for p in after if p["name"] in NAMES}
        assert [p for p in before if "source" in p] == [p for p in after if "source" in p], "Unrelated dependency changed"
        assert list(root.rglob("CHANGELOG.md")) == [root / "CHANGELOG.md"]
        notes = (root / "CHANGELOG.md").read_text()
        assert "historical baseline sentinel" not in notes, "Pre-tag history leaked into release notes"
        if expected_group:
            assert expected_group in notes, notes
            assert subject.splitlines()[0].split(": ", 1)[1] in notes, notes
        else:
            assert run(root, "git", "status", "--porcelain") == "", run(root, "git", "diff")
        print(f"PASS {subject}: {expected}; coherent workspace, one changelog, external dependencies unchanged")


if __name__ == "__main__":
    exercise("fix: repair show playback", "crates/cuepool/src/lib.rs", "pub fn fixed() {}\n", "0.12.4", "Application fixes")
    exercise("feat: expose a new cue control", "crates/cuepool-gui/src/lib.rs", "pub fn feature() {}\n", "0.13.0", "Application features")
    exercise("fix(deps): update application support", "crates/cuepool-video/Cargo.toml", None, "0.12.4", "Application dependencies")
    exercise("fix(deps): update runtime lockfile", "Cargo.lock", "", "0.12.4", "Application dependencies")
    exercise("fix(deps): update workspace runtime dependency", "Cargo.toml", "", "0.12.4", "Application dependencies")
    exercise("ci: maintain build automation", ".github/ci.yml", "name: example\n", "0.12.3")
    exercise("docs: clarify operator guide", "guide/intro.md", "Guide\n", "0.12.3")
    exercise("docs: clarify crate usage", "crates/cuepool/README.md", "Crate guide\n", "0.12.3")
    exercise("feat: extend headless show behavior", "crates/cuepool-harness/src/lib.rs", "pub fn feature() {}\n", "0.13.0", "Application features")
    exercise("refactor: revise cue model\n\nBREAKING CHANGE: cue data requires conversion", "crates/cuepool-core/src/lib.rs", "pub fn changed() {}\n", "0.13.0", "breaking")
    exercise("feat!: change the show contract", "crates/cuepool-core/src/lib.rs", "pub fn breaking() {}\n", "0.13.0", "breaking")
