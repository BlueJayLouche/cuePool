# Automation API

CuePool starts an HTTP API at `http://127.0.0.1:7133/v1` when the app launches.
It is read-only by default. The OpenAPI document is available at
`/v1/openapi.json`.

Read endpoints provide health, the loaded project, cues, active cues, the full
Help > Status snapshot, status history sampled once per second, and cursor-based logs.
Log reads accept an optional `limit` from 1 to 1000 for bounded paging.
`/v1/events` is an SSE stream of status samples, new logs, and command results.

Set `CUEPOOL_API_CONTROL_TOKEN` before launch to enable commands. Send the token
as `Authorization: Bearer <token>`. Without it, all reads remain available and
`POST /v1/commands` returns `403 control_disabled`.

```sh
curl http://127.0.0.1:7133/v1/health

curl -H "Authorization: Bearer $CUEPOOL_API_CONTROL_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"command":"go"}' \
  http://127.0.0.1:7133/v1/commands
```

Commands are `open_project`, `select_cue`, `go`, `stop`, `pause`, `resume`,
`preload`, `seek`, and `shutdown`. The API returns `202` with a pending command
ID. Poll `/v1/commands/{id}` or listen to `/v1/events` to learn whether the
command was applied or rejected.

Send an `Idempotency-Key` header when a command may be retried; reuse of
the same key returns the original command instead of executing it twice while
the result remains in CuePool's 256-command history. Once that result expires,
the old key is rejected rather than executed again.

`open_project` requires an absolute local `.qproj` path and rejects replacement
of a dirty project or a project with active cues.
`shutdown` waits for its final result before returning. It stops only the
CuePool process serving that API, and rejects projects with unsaved changes or
active playback.

`CUEPOOL_API_BIND` can change the loopback address or port, but CuePool rejects
non-loopback binds because the API is plain HTTP. For remote access, forward the
loopback listener through an authenticated TLS tunnel or reverse proxy; never
expose it directly to a network.

The [MCP server](../mcp/README.md) is a TypeScript STDIO sidecar for this API.
It exposes read tools by default and only advertises control tools when a
token is configured.

## Automation profiles

Set `CUEPOOL_AUTOMATION_PROFILE` to a lowercase name such as `smoke-a` when an
automation run must coexist with another CuePool process. Each named profile
gets its own single-instance lock, recent-file settings, and persistent log.
Use a different `CUEPOOL_API_BIND` port for each running profile.

The default profile is unchanged when the variable is unset or empty. Named
profile settings and logs live below the platform's normal CuePool directory at
`CuePool/automation/<name>/settings.json` and
`CuePool/automation/<name>/cuepool.log`.

## Unattended Windows smoke

Build the exact revision with its commit identity embedded, then run the
PowerShell smoke script from an interactive desktop session:

```powershell
$commit = (git rev-parse --short=7 HEAD).Trim()
$env:CUEPOOL_BUILD_ID = $commit
cargo build --release --locked -p cuepool --all-features
# A scheduled task does not inherit Cargo's DLL search path. Make the build
# directory runnable before launching it in an interactive desktop session.
Copy-Item "$env:FFMPEG_DIR\bin\*.dll" .\target\release\

.\scripts\unattended-smoke.ps1 `
  -Executable .\target\release\cuepool.exe `
  -Project C:\content\shows\smoke.qproj `
  -Profile smoke-a `
  -ApiPort 7141 `
  -ExpectedCommit $commit `
  -Artifact C:\content\smoke-results\cuepool-smoke.json `
  -CueQid 1
```

The script selects and plays the cue, checks that active playback blocks
shutdown, pauses, resumes, stops, captures health, GPU diagnostics, status
history and logs, then performs an acknowledged shutdown. It writes one compact
JSON artifact and never stores the generated control token. On failure it stops
only the process it launched.

MCP does not launch CuePool. The smoke artifact records application and GPU
state. Physical projector mapping still needs an attended or camera-backed
check.

See also: [command-line options](../guide/src/getting-started.md#command-line-options) ·
[Pixel feed](PIXEL_FEED.md) · [CuePool](../README.md)
