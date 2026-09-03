# CuePool development

CuePool is a standalone Cargo workspace. It was extracted from the CuePool example in
[rustjay-engine](https://github.com/BlueJayLouche/rustjay-engine); the only remaining tie is
`rustjay-lighting`, consumed from crates.io. Local edits to that crate in the engine tree do not
reach CuePool until they are published — bump the version in `Cargo.toml` to pick them up.

## Show behavior

- `crates/cuepool/src/engine.rs` is the source of truth for cue sequencing and lifecycle behavior. Put Go, Stop, pause, seek, delay, WithLast, AfterLast, loop, and EOF changes in `ShowEngine`.
- Keep `crates/cuepool/src/main.rs` as the winit adapter for windows, GPU presentation, device audio configuration, protocols, and lighting I/O. Do not add a second scheduler there.
- Keep engine time explicit. Commands, events, and ticks receive a monotonic `Duration`; `ShowEngine` must not read `Instant` or wall-clock time directly.
- Apply emitted `EngineAction`s in order before the next tick. Report asynchronous completion through `EngineEvent`, including the video instance and epoch so stale EOF events cannot finish a replacement stream.
- Add or change the shared engine behavior instead of writing a test-only cue interpreter.

## Projection auto-blend

Camera-driven output calibration lives in `crates/cuepool/src/autoblend.rs` (state machine, owned by `App`, OSC-driven) with support modules in `crates/cuepool-video/src/calibration/` (stream capture, AprilTag/color detection) and `crates/cuepool-video/src/homography.rs` (warp math). OSC surface: `/qplayer/projection/autoblend/{stream,markers,colors,white,apply,abort,run}`. Calibration writes into the live `ProjectionConfig` (`warp`, `edge_blend`, `gamma`, `black_uplift`); the per-tick output diff publishes it to the render threads — no window rebuild. `blend_enabled`/`uplift_enabled` bypass blends/black-lift when the projector handles them, keeping the calibrated values.

## Headless show tests

Use `cuepool_harness::HeadlessShowRunner` for full-show behavior that does not require a window, GPU, audio device, or external I/O:

1. Open a real current-format `.qproj` with `HeadlessShowRunner::open`.
2. Select a cue explicitly, then call `go`, `pause`, `resume`, `seek`, or `stop`.
3. Advance playback with `advance_blocks`. Each block renders through `NullSink`, advances `VirtualClock`, consumes due FFmpeg frames, and ticks `ShowEngine`.
4. Assert stable state with `snapshot`; use `take_trace` for ordered cue, frame, EOF, and side-effect events. `take_trace` drains the accumulated trace.

Sound and video use the production decoder paths. Network, text, image, PixelMap, lighting, and DMX actions are recorded without sending external I/O.

Generate test projects and media in a unique standard-library temporary directory, reference media with relative paths, and remove the directory after the test. Do not commit media fixtures or require the FFmpeg CLI. See `crates/cuepool-harness/tests/headless_show.rs` and its `support` module for the established pattern.

## Release notes

`RELEASE_NOTES_VERSION` in `crates/cuepool-gui/src/app/mod.rs` gates the "What's new" modal. It names the release whose copy the modal currently shows, as `major.minor`, and is compared against each operator's stored `last_seen_release_notes` so the modal appears once per generation.

A minor bump is not finished until the modal body is rewritten for that release and the constant is bumped to match. `release_notes_match_the_release` fails until both are done. Patch bumps leave both alone — a bug fix must not re-show the modal.

Do not derive the constant from `CARGO_PKG_VERSION`. That re-fires the modal on every patch bump and labels the previous release's copy with the new version, which is how the notes silently sat at 0.4 from 0.4.0 through 0.10.2.

## Platform gating

CuePool releases on macOS and Windows and is compiled and tested on Linux in CI, but any given author compiles only one. Rust does not typecheck `cfg`'d-out code, so platform divergence is invisible until another OS builds it — and "compiles everywhere, wrong on one platform" (e.g. a Windows-only audio driver presented on macOS) is invisible even then.

- Platform-specific user-facing options (drivers, hosts, APIs) must be gated to the platforms where they are meaningful. Make the platform decision once, in the crate that owns the type (e.g. `AUDIO_OUTPUT_DRIVER_OPTIONS` / `AudioOutputDriver::presented` in `cuepool-core`), and have every UI surface consume it — never hard-code a platform's option list in the GUI.
- Keep platform code behind module-level `cfg` gates (a whole `mod`, e.g. `d3d11_zero_copy.rs`) rather than scattered inline flags.
- If a change touches a `cfg(target_os)` path the author cannot compile locally, the PR must state which OS is untested.

## Verification

Run the focused checks first:

```sh
cargo test -p cuepool-harness --tests --locked
cargo test -p cuepool --locked
```

Before opening a PR, run:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Headless tests cover show logic and decoder timing. They do not prove presentation cadence, vsync behavior, audio-device routing, protocols, lighting hardware, or projector output. Changes at those boundaries still need an attended binary or rig smoke test.
