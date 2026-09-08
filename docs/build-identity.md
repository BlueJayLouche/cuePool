# Identifying a CuePool build

Help → About, startup logs, Status diagnostics and `/v1/health` use one source
identity. Ordinary `cargo build` needs no environment setup. It records the
workspace version, short and full Git commit, clean/modified state, nearest
reachable `vX.Y.Z` tag and distance from it. Modified builds include a fingerprint
of their diff and untracked files; this identifies local edits, but does not save
a copy of them. Keep the checkout when diagnosing such a build.

Cargo checks Git every time it builds. This intentionally adds a small build-script
cost: watching HEAD or the Git index alone misses new files, tag changes and some
worktree operations. The generated data contains no timestamp and is only rewritten
when its content changes. Source identity is stable for unchanged source. It does
not promise byte-identical binaries across toolchains, platforms or build flags.

`CUEPOOL_BUILD_ID` remains an optional packaging identifier. It is displayed in
addition to detected source data and remains the value of the health API's existing
`commit` field. Without an override, that field is the seven-character source SHA.
The API additionally provides `build_identity`, `source_commit` (full SHA),
`source_dirty`, `version_tag`, `commits_since_tag` and `tag_publication`.
The existing `dirty` API field still describes the open show project, not source.

Detached HEADs and Git worktrees work without special setup. Shallow clones show
their commit but explicitly warn that tag/history information can be incomplete.
Fetch full history and tags for complete comparisons. Archives without Git metadata
report source identity unavailable; an explicit build identifier does not prove
that the archive matches a release.

## Offline changes

Help → Changes embeds maintained `CHANGELOG.md` entries and up to 60 first-parent
commit descriptions at compile time. It states the comparison baseline, prefers a
reachable confirmed published release, and otherwise labels its baseline as an
unverified version tag. At an exact tag it compares against an earlier baseline.
Local edits and incomplete history are explicitly disclosed.

Commit history is grouped using the changed paths: application code/manifests,
build maintenance/documentation, and other changes. Commit descriptions are
attribution, not a guarantee of behavior. Empty history is never presented as proof
that no application behavior changed. Maintained notes should explain operator
impact concisely; the history retains PR numbers for further investigation.

Confirmed publications are recorded by the publication workflow in annotated
`published/vX.Y.Z` Git tags. These are receipts, not additional product versions.
Their JSON annotation records the product tag, peeled commit, public URL and
publication time. The product tag remains `vX.Y.Z`. Both must point to the same
commit before CuePool describes the version tag as published. Fetch tags to obtain
receipts; without a receipt publication is unverified, not assumed failed.

The repository had no published releases when this was introduced. A tag from a
failed packaging run is not a release. Publication is a snapshot at build time;
a candidate built before publication cannot claim a future outcome. Rebuilding
that commit after fetching its publication receipt can report it as published.
