# Environment variables

The `QPLAYER_*` names are the legacy prefix from before the CuePool rename and
are retained for compatibility.

| Variable | Read | Purpose | Default |
|---|---|---|---|
| `CUEPOOL_API_BIND` | Launch | Bind address for the read-only/control HTTP API; non-loopback addresses are rejected. | `127.0.0.1:7133` |
| `CUEPOOL_API_CONTROL_TOKEN` | Launch | Non-empty bearer token that enables API commands. | Unset; commands are disabled. |
| `CUEPOOL_PIXELS_BIND` | Launch | Bind address for the unauthenticated pixel-feed WebSocket listener. | Unset; the listener is disabled. |
| `CUEPOOL_PIXELS_ORIGINS` | Launch | Comma-separated browser `Origin` allowlist for the pixel feed; `null` permits `file://` pages. | Unset; any origin is accepted. |
| `CUEPOOL_AUTOMATION_PROFILE` | Launch | Lowercase profile name for isolated locks, settings, and logs. | Unset or empty; use the default profile. |
| `CUEPOOL_BUILD_ID` | Build | Embeds a build identifier in diagnostics and API status. | Unset or empty; derive identity from Git, or report identity unavailable. |
| `CUEPOOL_LX_FIXTURES` | Example launch | Fixture directory used by the `gen_lx_test` example. | The workspace's `testFiles` directory. |
| `QPLAYER_ZEROCOPY` | Launch | Enables the Windows D3D12VA zero-copy path when set to `1` and no CLI override is supplied. | Disabled. |
| `QPLAYER_PRESENT_MODE` | Launch | Requests `fifo`, `fifo_relaxed`, `mailbox`, or `immediate` for every output. | `fifo`; unsupported requests also fall back to `fifo`. |
| `QPLAYER_FPS_DEBUG` | Launch | Enables once-per-second frame-pacing diagnostics when present. | Unset; diagnostics are disabled. |
| `QPLAYER_NO_HWACCEL` | Launch | Forces software video decode and disables GPU-native HAP when set to `1`. | Hardware acceleration may be used. |
| `RUST_LOG` | Launch | Configures stderr and in-app log filtering with `env_logger` syntax. | Error-level stderr filtering; CuePool still persists warnings and field events. |
| `FFMPEG_DIR` | Build/package | Points Windows builds and `package-windows.ps1` at the FFmpeg shared SDK. | Unset; ignored by the custom build step off Windows MSVC. |
| `CARGO_CFG_TARGET_OS` | Build (Cargo-provided) | Lets the video build script detect whether the target is Windows. | Set by Cargo. |
| `CARGO_CFG_TARGET_ENV` | Build (Cargo-provided) | Lets the video build script detect whether the target uses MSVC. | Set by Cargo. |

See also: [Automation API](AUTOMATION.md) · [Pixel feed](PIXEL_FEED.md) ·
[Build identity](build-identity.md) · [CuePool](../README.md)
