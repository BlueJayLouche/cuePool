# Packaging CuePool

The [release workflow](../.github/workflows/release.yml) builds macOS and
Windows packages with:

```sh
cargo build --release --locked -p cuepool --all-features
```

It produces a macOS Apple Silicon `.dmg` containing `CuePool.app`, plus a
Windows x86-64 portable `.zip` and `.msi` installer. Linux is built and tested
in CI but is not currently packaged by the release workflow.

The workflow verifies the candidate source and validates both platform packages
before publishing. A manual run without a release tag produces artifacts only.
See [releases](../docs/releases.md) for product versions, GitHub setup,
publication gates, and retries, and
[Contributing](../CONTRIBUTING.md#building-from-source) for build dependencies.

## macOS app bundle

Cargo produces a bare executable. Finder and the Dock take the app icon from
an `.app` bundle; launching the bare binary shows the generic `exec` icon.
winit window icons have no effect on macOS.

The release workflow assembles `dist/CuePool.app` using
[Info.plist.tmpl](../.github/packaging/Info.plist.tmpl) and `AppIcon.icns`.
It uses `dylibbundler` to copy the FFmpeg libraries into the bundle and rewrite
their load paths. Without that step, a local bundle still depends on the
build machine's Homebrew libraries. Install the bundler with:

```sh
brew install dylibbundler
```

After building the release binary, create a local `dist/CuePool.app` with:

```sh
./package-macos.sh
```

The script uses the same bundle template and icon as the release workflow.
Run it again after rebuilding the binary.

The app is ad-hoc signed, not notarized. If macOS blocks first launch, approve
it under **System Settings → Privacy & Security**.

## Windows packages

The workflow places `cuepool.exe` and the DLLs from `FFMPEG_DIR/bin` together
in the portable folder, so users do not need to configure a DLL search path.
The MSI adds a Start-menu shortcut and associates `.qproj` files with CuePool.

After building the release binary, create a local portable folder and ZIP with
the same `FFMPEG_DIR` used for the build:

```powershell
.\package-windows.ps1
```

The script writes `dist/cuepool` and `dist/cuepool-windows.zip`. It packages an
existing binary; it does not build CuePool.

## Icons

Icon slots picked up by the release workflow:

- `AppIcon.icns` — macOS bundle icon
- `icon.ico` — Windows Start-menu shortcut icon

Also here:

- `window-icon.png` — 64px, embedded in the binary and set as the winit window
  icon. Covers the Windows taskbar and Linux title bar, which the packaged
  `.ico` does not reach (the workflow builds a shortcut icon, not an embedded
  resource). macOS ignores window icons and reads `AppIcon.icns` instead.
- `cuepool-02-cue.svg`, `cuepool-02-cue-small.svg` — the mark itself, and the
  reduced form used at 24px and below once the counter stops resolving. These
  are the source of truth; the raster files above are generated from them.
