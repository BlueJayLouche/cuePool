# Preparing and publishing CuePool

CuePool has one product version in `[workspace.package]`, inherited by every
internal crate, one `CHANGELOG.md`, and product tags named `vX.Y.Z`. Release-plz
0.3.162 prepares changes entirely from Git; registry publication is disabled in
its configuration. We run its `update` and `release-pr` commands, never
`cargo publish` or `release-plz release`.

Always invoke it through `scripts/prepare-release.py`. That wrapper supplies an
ancestral version-tag checkout using the supported `--registry-manifest-path`
option. Despite that option name, the baseline is local Git source, never a
registry download. It fails if the baseline cannot be constructed or its version
does not match the tag. The tool's built-in `git_only = true` path runs
`cargo package` on old tags; CuePool's versionless private path dependencies and
fork-only wgpu features cannot be packaged as registry crates. The local-baseline
path avoids changing those historical manifests or compiling old application code.
The wrapper supplies original-manifest copies and the checked-in binary lockfile
in its temporary baseline only. It exposes legacy `publish=false` members in the
analysis manifests while retaining byte-for-byte historical `.orig` copies, so
the old private harness is compared at the tag rather than treated as a new crate.

Cargo manifests remain visible to release-plz's workspace analysis (including the
harness); the workflow's `publish = false` is the publication guard. Do not run raw
release-plz without the wrapper or introduce a registry/publishing fallback.

## Contributor conventions

Squash-merge PRs using a Conventional Commit title that describes the final change:

- `fix: ...` for application fixes: patch increment.
- `fix(deps): ...` for application dependency updates: patch increment.
- `feat: ...` for application features: minor increment, including during `0.x`.
- `feat!: ...` or another type with `!`, with a `BREAKING CHANGE:` explanation,
  for incompatible changes. Release-plz highlights these for review. In `0.x`,
  breaking changes advance the minor; do not describe that as a compatible patch.
- `ci: ...`, `docs: ...`, `build: ...` and `chore(deps): ...` for build tooling,
  GitHub Actions and documentation. These alone do not request a product release.

Review application dependency PRs so their squash title uses `fix(deps)`, not the
maintenance-only `chore(deps)` convention used by GitHub Actions updates. Do not
label application behavior changes as maintenance to suppress a release.

Write concise operator descriptions under `[Unreleased]` in `CHANGELOG.md` when
PR titles alone would not explain the impact. Generated release notes include
changes from all internal crates in the product changelog. Release preparation
only updates workspace lockfile versions (`dependencies_update = false`); it
must not refresh unrelated dependency versions. For root Cargo.toml/Cargo.lock
changes, release-plz generates a dependency summary rather than keeping the PR
title; add the operator impact to `[Unreleased]`. The exact generated dependency
messages are explicitly allowed by the release filter, and both root-only cases
are tested against the pinned tool.

## Release PR

The Release preparation workflow runs after pushes to main and completed main
Release runs, and can be retried with workflow_dispatch. The completion trigger
rechecks current main so changes merged while a candidate was awaiting its tag
are not forgotten. It checks out current main with complete history and
maintains a reviewable release-plz PR updating Cargo.toml, Cargo.lock and the
product changelog. CI checks that every crate still inherits the product version
and that its lockfile entry matches. An untagged version already merged into main
is an outstanding release candidate: preparation defers until it has passed
verification and been tagged, avoiding recursive version bumps. The wrapper skips
maintenance-only commit history before invoking release-plz, including documentation
inside crate directories; the pinned tool's release filter alone can otherwise
change only the workspace version for those commits. Before creating a PR, the
wrapper rehearses the update in an isolated clone and verifies exactly the three
product files change, all internal versions match, and external dependencies stay
locked.

Review the proposed increment and notes. For a minor release, rewrite the
operator-facing welcome copy in `crates/cuepool-gui/src/app/mod.rs` and set
`RELEASE_NOTES_VERSION` to that major.minor. The existing
`release_notes_match_the_release` test remains required. A patch release leaves
the constant and welcome copy alone, so it does not reopen the modal.
Add the welcome copy and matching constant to the release PR before merging.
Release-plz may replace that PR after human edits when more changes arrive;
reapply and recheck the copy in the final proposal. Do not advance the constant
on main while its product version still names the previous minor release.
Never bypass the failing notes gate.

## One publication owner

Only `.github/workflows/release.yml` publishes GitHub releases. Each main push
checks for an outstanding untagged product version; a pushed product tag also
starts the workflow. A source fix after failed verification can therefore recover
an untagged candidate without another version bump. Once tagged, that source is
immutable: subsequent fixes go through a new release PR. It validates the
version/changelog and runs the complete reusable CI suite at the exact candidate
SHA. After verification it creates the product tag, refusing to move an existing
tag. It then builds and packages that same SHA on macOS and Windows.

A read-only prerequisite gate runs even on artifacts-only rehearsals and failed
builds. The final publication job requires that gate to pass, including successful
verification and both platform jobs. It also requires exactly one nonempty DMG, portable ZIP and MSI, validates
their basic container structure, and checks the ZIP contains the executable and
runtime DLLs. macOS packaging verifies the DMG and application signature.

The publication script creates or reuses a **draft**, writes the exact changelog
notes and source attestation (rejecting conflicting source claims), uploads the packages and
SHA256SUMS, downloads each uploaded file and verifies its SHA-256, then makes the
draft public. A failed build, missing package, upload failure or readback mismatch
leaves publication blocked. Nothing is published by release preparation itself.

After confirmed publication the same workflow creates an annotated
`published/vX.Y.Z` receipt with the exact commit, release URL and publication time.
It never triggers another product release. These tags are publication metadata;
the product version and product tag remain unchanged. Local builds use receipts
offline as described in [build identity](build-identity.md).

A failed run can be rerun, or manually dispatched with the existing product tag
created by this automation. Older tags that predate these scripts require a new
release candidate; they do not satisfy this publication contract.
An existing tag is never moved; a draft is reused. A previously public release is
never overwritten: recovery downloads its packages and checksum manifest, checks
its source attestation, and repairs only a missing publication receipt. Therefore
retrying after publication does not depend on reproducing byte-identical binaries.
The workflow serializes release attempts with `queue: max` (up to 100 waiting
runs), so a later push does not replace a queued candidate. Interrupted runs can
leave a private draft or an unreceipted public release, both recoverable by retry.
An older retry is never marked Latest when a newer product version is public.

Manual dispatch **without** `release_tag` builds artifacts only. It creates no
product tag, release or receipt. Use this on an implementation branch to exercise
both packagers without publishing.

## GitHub setup and token behavior

Install a GitHub App on **BlueJayLouche/cuePool** with repository **Contents: read
and write**, **Pull requests: read and write**, and the mandatory Metadata read
permission. Store its numeric ID in repository variable `RELEASE_APP_ID` and its
private key in Actions secret `RELEASE_APP_PRIVATE_KEY`. No crates.io token is
needed. The preparation workflow fails with an explicit setup message if these
are absent. Neither key contents nor tokens belong in this repository.

The App token creates release PRs so their CI starts normally. We deliberately do
not fall back silently to GITHUB_TOKEN: its generated PR events may require manual
workflow approval, and its pushed tags do not start ordinary push workflows.

Publication uses the job-scoped GITHUB_TOKEN with Contents write. The verification
and packaging jobs continue in the **same release workflow** after it creates a
tag, so they do not depend on a second tag-triggered run. Publication receipts
use a different tag prefix and cannot trigger packaging. Repository tag rules must
allow this workflow to create `v*` and `published/v*` refs; it requires no direct
write or branch-protection bypass on main. Keep the release PR's CI checks required.

## Verification

Run the commands in AGENTS.md, `npm ci && npm test` in `mcp`, and:

```
python3 .github/scripts/release.py validate
python3 -m unittest discover -s .github/scripts -p 'test_*.py'
python3 scripts/test-release-policy.py
```

The policy test requires release-plz 0.3.162 on PATH (or `RELEASE_PLZ` pointing to
it). It exercises the production wrapper and real `release-plz update` in temporary
workspaces with the same crate names and inherited version: fix, feature, harness
feature, crate/root-manifest/lockfile dependencies, docs inside/outside crates,
CI-only and breaking changes. It checks every lockfile version, the single changelog and an
unrelated older locked dependency. It never creates a remote PR or publishes.

A passing Linux suite does not prove Windows or macOS packaging. If the Windows
AprilTag build still lacks pthread.h, its build job must fail and publication must
remain blocked. Resolve that platform prerequisite through its own reviewed change;
do not remove Windows from the release gate.
