#!/usr/bin/env python3
"""Prepare CuePool releases using release-plz and an explicit local Git baseline.

No registry baseline or publication command is used. The pinned tool's built-in
Git-only reconstruction runs cargo package on historical tags, which rejects our
private path dependencies and fork-only wgpu features. Its supported local
baseline option preserves those manifests and does not compile historical code.
"""
import argparse
import contextlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / ".github/scripts"))
from release import validate_workspace, product_version, tag_name, release_eligible

REPOSITORY = "https://github.com/BlueJayLouche/cuePool"
TOOL = os.environ.get("RELEASE_PLZ", "release-plz")


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()



@contextlib.contextmanager
def baseline(root, tag):
    tag_name(tag)
    subprocess.run(["git", "-C", str(root), "merge-base", "--is-ancestor", tag, "HEAD"], check=True)
    with tempfile.TemporaryDirectory(prefix="cuepool-release-baseline-") as tmp:
        path = Path(tmp)
        subprocess.run(["git", "clone", "--quiet", "--shared", str(root), str(path)], check=True)
        subprocess.run(["git", "-C", str(path), "checkout", "--quiet", "--detach", tag], check=True)
        if "v" + product_version(path) != tag:
            raise ValueError("Git baseline tag and workspace version disagree")
        # release-plz's comparison expects the original manifest beside the
        # analysis manifest. Cargo metadata resolves workspace inheritance from
        # this complete checkout. Legacy tags mark the harness publish=false:
        # expose it to analysis so its pre-tag history is not treated as new.
        # Preserve the byte-for-byte historical original for source comparison.
        for manifest in (path / "crates").glob("*/Cargo.toml"):
            shutil.copyfile(manifest, manifest.with_name("Cargo.toml.orig"))
            manifest.write_text(manifest.read_text().replace("publish = false\n", "").replace("publish=false\n", ""))
        # Its lockfile comparator expects the binary package's lock beside the
        # manifest. This is the exact checked-in workspace lock, not a re-resolve.
        shutil.copyfile(path / "Cargo.lock", path / "crates/cuepool/Cargo.lock")
        yield path


def external_dependencies(root):
    return [p for p in tomllib.loads((root / "Cargo.lock").read_text())["package"] if "source" in p]


def update(root, previous):
    before = external_dependencies(root)
    env = {k: v for k, v in os.environ.items() if k not in ("GIT_TOKEN", "GH_TOKEN", "GITHUB_TOKEN")}
    subprocess.run([TOOL, "update", "--config", "release-plz.toml", "--registry-manifest-path",
                    str(previous / "Cargo.toml"), "--repo-url", REPOSITORY], cwd=root, env=env, check=True)
    validate_workspace(root)
    if external_dependencies(root) != before:
        raise ValueError("Release preparation changed unrelated external dependencies")
    changed = set(git(root, "diff", "--name-only").splitlines())
    if changed != {"Cargo.toml", "Cargo.lock", "CHANGELOG.md"}:
        raise ValueError(f"Expected exactly product version, lockfile and changelog updates; got {changed}")
    notes = (root / "CHANGELOG.md").read_text()
    if notes.count("## [Unreleased]") != 1:
        raise ValueError("Release preparation duplicated the changelog baseline")


def prepare(root, command):
    root = root.resolve()
    if git(root, "status", "--porcelain"):
        raise ValueError("Release preparation requires a clean checkout")
    version = validate_workspace(root)
    tag = "v" + version
    with baseline(root, tag) as previous:
        if not release_eligible(root, tag):
            print(f"No release-worthy commits since {tag}; build identity changes only.")
            return
        if command == "update":
            update(root, previous)
        else:
            # Verify the proposal before a remote write. Native release-plz then
            # maintains its existing PR using the same source, config and baseline.
            with tempfile.TemporaryDirectory(prefix="cuepool-release-proposal-") as tmp:
                proposal = Path(tmp)
                subprocess.run(["git", "clone", "--quiet", "--shared", str(root), str(proposal)], check=True)
                update(proposal, previous)
            subprocess.run([TOOL, "release-pr", "--config", "release-plz.toml", "--registry-manifest-path",
                            str(previous / "Cargo.toml"), "--repo-url", REPOSITORY], cwd=root, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["update", "release-pr"])
    args = parser.parse_args()
    prepare(Path.cwd(), args.command)
