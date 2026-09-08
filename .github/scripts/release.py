#!/usr/bin/env python3
"""CuePool product release invariants and the sole final publication operation.

Uses Python's standard library and the hosted runner's gh CLI. Never runs cargo
publish. Unit tests replace the GitHub boundary; policy tests run real release-plz.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib
import zipfile

REPOSITORY = "BlueJayLouche/cuePool"
ARTIFACTS = ("cuepool-macos-arm64.dmg", "cuepool-windows-x86_64.zip", "cuepool-windows-x86_64.msi")
MACOS_NOTE = ("**macOS:** the app is ad-hoc signed, not notarized. On first launch, "
              "right-click → Open, or approve it in System Settings → Privacy & Security.")
RELEASE_COMMIT = re.compile(r"^(feat|fix)(\([^)]*\))?!?:|^[a-z]+(\([^)]*\))?!:|^BREAKING[ -]CHANGE:", re.M)
TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\Z")


def run(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def git(*args):
    return run("git", *args)


def tag_name(tag):
    if not TAG.fullmatch(tag):
        raise ValueError(f"Expected a product tag vX.Y.Z, got {tag!r}")
    return tag


def product_version(root=Path(".")):
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    return manifest["workspace"]["package"]["version"]


def validate_workspace(root=Path(".")):
    version = product_version(root)
    tag_name("v" + version)
    packages = []
    for path in sorted((root / "crates").glob("*/Cargo.toml")):
        package = tomllib.loads(path.read_text())["package"]
        if package["version"] != {"workspace": True}:
            raise ValueError(f"{path} must inherit the product version")
        if package.get("publish") is False:
            raise ValueError(f"{path} is excluded from release analysis; disable publication in release-plz.toml")
        packages.append(package["name"])
    if not packages:
        raise ValueError("No workspace packages found")
    policy = tomllib.loads((root / "release-plz.toml").read_text())
    for name in ("publish", "git_release_enable", "git_tag_enable"):
        if policy["workspace"].get(name) is not False or any(p.get(name) is True for p in policy.get("package", [])):
            raise ValueError(f"Release preparation must disable {name}")
    lock = tomllib.loads((root / "Cargo.lock").read_text())["package"]
    for name in packages:
        entries = [p for p in lock if p["name"] == name and "source" not in p]
        if len(entries) != 1 or entries[0]["version"] != version:
            raise ValueError(f"Cargo.lock has an inconsistent product version for {name}")
    notes = (root / "CHANGELOG.md").read_text()
    if not re.search(r"^## \[" + re.escape(version) + r"\]", notes, re.M):
        raise ValueError(f"CHANGELOG.md has no {version} entry")
    return version


def release_notes(tag, root=Path(".")):
    version = tag_name(tag)[1:]
    notes = (root / "CHANGELOG.md").read_text()
    match = re.search(r"^## \[" + re.escape(version) + r"\][^\n]*\n(.*?)(?=^## \[|\Z)", notes, re.M | re.S)
    if not match or not match[1].strip():
        raise ValueError(f"No release notes for {tag}")
    return match[1].strip()


def output(values):
    for key, value in values.items():
        line = f"{key}={value}"
        print(line)
        if path := os.environ.get("GITHUB_OUTPUT"):
            with open(path, "a") as stream:
                stream.write(line + "\n")


def resolve_candidate():
    """Run after checkout of the event's exact source. No remote writes."""
    event = os.environ["GITHUB_EVENT_NAME"]
    ref = os.environ["GITHUB_REF"]
    version = validate_workspace()
    tag = ""
    active = True
    if event == "push" and ref == "refs/heads/main":
        # Detect outstanding candidates from repository state, not only the
        # version-changing push. A source fix after failed pre-tag verification
        # must be able to finish the same candidate at its corrected revision.
        candidate = "v" + version
        existing = api("git/ref/tags/" + candidate, missing_ok=True)
        active = existing is None or ref_commit(existing) == git("rev-parse", "HEAD")
        tag = candidate if active else ""
    elif event == "push" and ref.startswith("refs/tags/"):
        tag = tag_name(ref.removeprefix("refs/tags/"))
    elif event == "workflow_dispatch":
        tag = os.environ.get("RELEASE_TAG", "")
        if tag:
            tag_name(tag)
    else:
        raise ValueError("Unsupported release event")
    sha = git("rev-parse", "HEAD")
    if tag:
        if tag != "v" + version:
            raise ValueError("Tag and product version disagree")
        subprocess.run(["git", "merge-base", "--is-ancestor", sha, "origin/main"], check=True)
    published = bool(tag and (found := find_release(tag)) and not found["draft"])
    output({"active": str(active and not published).lower(), "tag": tag, "sha": sha,
            "published": str(published).lower()})


def release_eligible(root, tag):
    messages = run("git", "-C", str(root), "log", "--first-parent", "--format=%B%x00", f"{tag}..HEAD")
    return any(RELEASE_COMMIT.search(message.strip()) for message in messages.split("\x00"))


def can_prepare():
    # A merged release PR advances Cargo's version before packaging creates its
    # tag. Do not prepare another release while that candidate is untagged.
    tag = "refs/tags/v" + product_version()
    result = subprocess.run(["git", "merge-base", "--is-ancestor", tag, "HEAD"], capture_output=True)
    ready = result.returncode == 0 and release_eligible(Path("."), tag)
    output({"ready": str(ready).lower()})
    if result.returncode:
        print("Release candidate awaits verification/tagging; release preparation is deferred.")
    elif not ready:
        print("No release-worthy commits; build identity changes only.")


def api(endpoint, method="GET", data=None, missing_ok=False):
    args = ["gh", "api", f"repos/{REPOSITORY}/{endpoint}", "--method", method]
    if data is not None:
        args += ["--input", "-"]
    result = subprocess.run(args, input=json.dumps(data) if data is not None else None,
                            capture_output=True, text=True)
    if result.returncode:
        if missing_ok and "HTTP 404" in result.stderr:
            return None
        raise RuntimeError(result.stderr.strip())
    return json.loads(result.stdout) if result.stdout.strip() else None


def ref_commit(ref):
    obj = ref["object"]
    while obj["type"] == "tag":
        obj = api("git/tags/" + obj["sha"])["object"]
    if obj["type"] != "commit":
        raise ValueError("Release ref does not point to a commit")
    return obj["sha"]


def ensure_tag(tag, sha):
    tag_name(tag)
    existing = api("git/ref/tags/" + tag, missing_ok=True)
    if existing:
        if ref_commit(existing) != sha:
            raise ValueError(f"Refusing to move existing tag {tag}")
        return
    api("git/refs", "POST", {"ref": "refs/tags/" + tag, "sha": sha})


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def validate_artifacts(directory, build_result, verification_result):
    if build_result != "success" or verification_result != "success":
        raise ValueError("Publication requires successful verification and BOTH platform builds")
    paths = {name: list(directory.rglob(name)) for name in ARTIFACTS}
    if any(len(found) != 1 or found[0].stat().st_size == 0 for found in paths.values()):
        raise ValueError("Missing, empty or duplicate platform artifact")
    paths = {name: found[0] for name, found in paths.items()}
    with zipfile.ZipFile(paths[ARTIFACTS[1]]) as archive:
        names = archive.namelist()
        if "cuepool.exe" not in names or not any(n.lower().endswith(".dll") for n in names):
            raise ValueError("Windows ZIP must contain cuepool.exe and runtime DLLs")
        if archive.testzip() is not None:
            raise ValueError("Corrupt Windows ZIP")
    with paths[ARTIFACTS[2]].open("rb") as stream:
        if stream.read(8) != bytes.fromhex("d0cf11e0a1b11ae1"):
            raise ValueError("Invalid MSI compound-file header")
    with paths[ARTIFACTS[0]].open("rb") as stream:
        stream.seek(-512, 2)
        if stream.read(4) != b"koly":
            raise ValueError("Invalid DMG trailer")
    return paths


def find_release(tag):
    release = api("releases/tags/" + tag, missing_ok=True)
    if release:
        return release
    # GitHub's tag lookup can omit drafts. Scan authenticated release listings
    # before creating anything so retries reuse the same draft.
    page = 1
    while True:
        releases = api(f"releases?per_page=100&page={page}")
        matches = [r for r in releases if r["tag_name"] == tag]
        if len(matches) > 1:
            raise ValueError("Multiple releases use the candidate tag")
        if matches:
            return matches[0]
        if len(releases) < 100:
            return None
        page += 1


def record_publication(tag, sha, release):
    if release["draft"] or not release.get("published_at") or release["tag_name"] != tag:
        raise ValueError("Cannot record an unpublished release")
    marker = "published/" + tag
    proof = {"tag": tag, "commit": sha, "url": release["html_url"], "published_at": release["published_at"]}
    existing = api("git/ref/tags/" + marker, missing_ok=True)
    if existing:
        if ref_commit(existing) != sha or existing["object"]["type"] != "tag":
            raise ValueError("Publication receipt points to different source")
        recorded = json.loads(api("git/tags/" + existing["object"]["sha"])["message"])
        if recorded != proof:
            raise ValueError("Publication receipt differs from the public release")
        return
    annotated = api("git/tags", "POST", {"tag": marker, "message": json.dumps(proof, sort_keys=True),
                                         "object": sha, "type": "commit"})
    api("git/refs", "POST", {"ref": "refs/tags/" + marker, "sha": annotated["sha"]})


def verify_remote_assets(tag, paths):
    with tempfile.TemporaryDirectory(prefix="cuepool-release-readback-") as tmp:
        for name, path in paths.items():
            run("gh", "release", "download", tag, "--repo", REPOSITORY,
                "--pattern", name, "--dir", tmp)
            if digest(Path(tmp) / name) != digest(path):
                raise ValueError(f"Remote artifact digest mismatch: {name}")


def repair_receipt(tag, sha):
    """Retry after publication without rebuilding or replacing public binaries."""
    tag_name(tag)
    release = find_release(tag)
    if not release or release["draft"]:
        raise ValueError("No public release to recover")
    if f"<!-- cuepool-source: {sha} -->" not in (release.get("body") or ""):
        raise ValueError("Public release has no matching CuePool source attestation")
    ref = api("git/ref/tags/" + tag)
    if ref_commit(ref) != sha:
        raise ValueError("Public release tag does not match the candidate")
    with tempfile.TemporaryDirectory(prefix="cuepool-public-release-") as tmp:
        root = Path(tmp)
        for name in (*ARTIFACTS, "SHA256SUMS"):
            run("gh", "release", "download", tag, "--repo", REPOSITORY, "--pattern", name, "--dir", tmp)
        expected = {}
        for line in (root / "SHA256SUMS").read_text().splitlines():
            checksum, name = line.split("  ", 1)
            if name in expected or name not in ARTIFACTS or not re.fullmatch(r"[0-9a-f]{64}", checksum):
                raise ValueError("Invalid published checksum manifest")
            expected[name] = checksum
        if set(expected) != set(ARTIFACTS):
            raise ValueError("Published checksum manifest is incomplete")
        for name, checksum in expected.items():
            if digest(root / name) != checksum:
                raise ValueError("Published artifact does not match its checksum")
    record_publication(tag, sha, release)


def should_promote_latest(tag):
    """Release runs are serialized; an older retry must not replace a newer Latest."""
    candidate = tuple(map(int, tag_name(tag)[1:].split(".")))
    page = 1
    while True:
        releases = api(f"releases?per_page=100&page={page}")
        for release in releases:
            name = release["tag_name"]
            if not release["draft"] and not release.get("prerelease") and TAG.fullmatch(name):
                if tuple(map(int, name[1:].split("."))) > candidate:
                    return False
        if len(releases) < 100:
            return True
        page += 1


def publish(tag, sha, directory, build_result, verification_result):
    tag_name(tag)
    existing = find_release(tag)
    if existing and not existing["draft"]:
        repair_receipt(tag, sha)
        return
    paths = validate_artifacts(directory, build_result, verification_result)
    ensure_tag(tag, sha)
    sums = directory / "SHA256SUMS"
    sums.write_text("".join(f"{digest(path)}  {name}\n" for name, path in sorted(paths.items())))
    paths[sums.name] = sums
    release = find_release(tag)
    if release and not release["draft"]:
        repair_receipt(tag, sha)
        return
    body = release_notes(tag) + "\n\n" + MACOS_NOTE + f"\n\n<!-- cuepool-source: {sha} -->\n"
    if release:
        claimed = re.findall(r"<!-- cuepool-source: (.*?) -->", release.get("body") or "")
        if claimed and claimed != [sha]:
            raise ValueError("Draft has a conflicting source attestation")
    with tempfile.TemporaryDirectory() as tmp:
        notes = Path(tmp) / "notes.md"
        notes.write_text(body)
        if release:
            run("gh", "release", "edit", tag, "--repo", REPOSITORY,
                "--title", "CuePool " + tag, "--notes-file", str(notes))
        else:
            run("gh", "release", "create", tag, "--repo", REPOSITORY, "--verify-tag",
                "--draft", "--title", "CuePool " + tag, "--notes-file", str(notes))
    release = find_release(tag)
    if not release or not release["draft"] or release.get("body") != body:
        raise ValueError("Draft source attestation/notes did not persist; refusing publication")
    run("gh", "release", "upload", tag, "--repo", REPOSITORY, "--clobber", *map(str, paths.values()))
    verify_remote_assets(tag, paths)
    latest = str(should_promote_latest(tag)).lower()
    run("gh", "release", "edit", tag, "--repo", REPOSITORY, "--draft=false", "--latest=" + latest)
    release = api("releases/tags/" + tag)
    record_publication(tag, sha, release)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["validate", "candidate", "can-prepare", "tag", "publish", "repair-receipt", "validate-artifacts"])
    parser.add_argument("--tag")
    parser.add_argument("--sha")
    parser.add_argument("--artifacts", type=Path)
    parser.add_argument("--build-result", default="")
    parser.add_argument("--verification-result", default="")
    args = parser.parse_args()
    if args.command == "validate":
        print(validate_workspace())
    elif args.command == "validate-artifacts":
        print("Validated packages:", ", ".join(validate_artifacts(args.artifacts, args.build_result, args.verification_result)))
    elif args.command == "candidate":
        resolve_candidate()
    elif args.command == "can-prepare":
        can_prepare()
    elif args.command == "repair-receipt":
        repair_receipt(args.tag, args.sha)
    elif args.command == "tag":
        ensure_tag(args.tag, args.sha)
    else:
        publish(args.tag, args.sha, args.artifacts, args.build_result, args.verification_result)


if __name__ == "__main__":
    main()
