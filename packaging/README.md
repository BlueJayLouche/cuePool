Icon slots for release packaging (picked up by `.github/workflows/release.yml`):

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
