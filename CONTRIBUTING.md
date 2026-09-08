# Contributing to CuePool

Bug reports, documentation improvements, and code contributions are welcome.
For bugs, include your operating system, CuePool version, steps to reproduce,
and relevant information from **Help → Status**. Remove private project details
from logs before sharing them.

## Building from source

CuePool is a standalone Cargo workspace; you do not need a rustjay-engine
checkout. Install Rust through [rustup](https://rustup.rs/), then clone the
repository:

```sh
git clone https://github.com/BlueJayLouche/cuePool.git
cd cuePool
```

The repository's [rust-toolchain.toml](rust-toolchain.toml) selects the Rust
version and installs rustfmt and Clippy through rustup. Install the native
dependencies for your platform before running Cargo. Use `--locked` when
building or testing so `Cargo.lock` stays authoritative.

### macOS

Install the Xcode Command Line Tools and the FFmpeg development libraries:

```sh
xcode-select --install
brew install ffmpeg pkg-config
```

### Linux

On Debian or Ubuntu, use the same packages as [CI](.github/workflows/ci.yml):

```sh
sudo apt-get update
sudo apt-get install -y \
  libasound2-dev libudev-dev pkg-config clang ninja-build \
  mesa-vulkan-drivers libvulkan1 \
  libavcodec-dev libavformat-dev libavutil-dev \
  libavfilter-dev libavdevice-dev libswscale-dev libswresample-dev
```

Other distributions need equivalent development packages. Running the app
requires a graphical desktop and audio output.

### Windows

Use the MSVC Rust toolchain with the Visual Studio C++ build tools and LLVM
for bindgen. Download and extract the shared FFmpeg SDK pinned in the
[release workflow](.github/workflows/release.yml), then set `FFMPEG_DIR` to its
root directory (containing `bin`, `include`, and `lib`):

```powershell
$env:FFMPEG_DIR = "C:\path\to\ffmpeg-sdk"
$env:PATH = "$env:FFMPEG_DIR\bin;$env:PATH"
```

Keep the SDK and runtime DLLs from the same FFmpeg build. The release workflow
pins FFmpeg 8.0 for the Windows D3D12VA zero-copy path's ABI requirements.

### Run

From the repository root:

```sh
cargo run --release --locked -p cuepool
```

To include Windows ASIO support, add `--features asio`. Release builds use
`--all-features`. See [command-line options](guide/src/getting-started.md#command-line-options)
for opening a project at startup and [packaging](packaging/README.md) for app
bundles and distributable builds.

## Checks before a pull request

Keep changes focused and include a regression test when changing show behavior.
Start with the affected crate's checks. For cue sequencing or playback changes:

```sh
cargo test -p cuepool-harness --tests --locked
cargo test -p cuepool --locked
```

Before opening a pull request, run the workspace checks:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

For documentation changes, check links and build the user guide with
[mdBook](https://rust-lang.github.io/mdBook/):

```sh
mdbook build guide
```

For MCP changes, run `npm ci` and `npm test` from `mcp/`.

Headless tests cover show logic and decoder timing. Presentation cadence,
audio-device routing, external protocols, lighting hardware, and projector
output still need an attended app or rig check. State which platforms you
tested and which remain untested.

Use a Conventional Commit PR title that describes the final change. See the
[release conventions](docs/releases.md#contributor-conventions) for change
types, product versions, and changelog guidance. Local builds identify their
source automatically; see [build identity](docs/build-identity.md) for overrides
and diagnostics, and [environment variables](docs/ENVIRONMENT.md) for other
build and runtime settings.

## Where to look

| Area | Location |
|---|---|
| Cue sequencing and lifecycle | `crates/cuepool/src/engine.rs` |
| App event loop, GPU presentation, devices, and external I/O | `crates/cuepool/src/main.rs` |
| Cue model, project files, and configuration | `crates/cuepool-core` |
| Audio decoding, DSP, and mixing | `crates/cuepool-audio` |
| Video decoding and projection rendering | `crates/cuepool-video` |
| User interface | `crates/cuepool-gui` |
| OSC, MIDI, and MSC | `crates/cuepool-protocols` |
| Headless show runner and integration tests | `crates/cuepool-harness` |
| User guide | `guide/src` |
| HTTP API and pixel-feed documentation | `docs/AUTOMATION.md`, `docs/PIXEL_FEED.md` |

Read [AGENTS.md](AGENTS.md) for engine, platform, and release conventions.
`rustjay-lighting` is consumed from crates.io; edits in a separate engine
checkout do not reach CuePool until the crate is published and its dependency
version is updated here.
