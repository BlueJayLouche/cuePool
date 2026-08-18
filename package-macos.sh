#!/usr/bin/env bash
# Wraps the cuepool release binary in dist/CuePool.app, the only form macOS will
# take an icon from. A bare Mach-O always draws the generic green "exec" tile, and
# packaging/window-icon.png cannot help — winit window icons are a no-op on macOS,
# where Finder and the Dock read Contents/Resources/AppIcon.icns instead.
# Re-run after a rebuild. Windows counterpart: package-windows.ps1.
#
# The Info.plist template and icon slot are the ones release-apps.yml ships, so a
# local bundle matches the released one. Without dylibbundler the .app still loads
# FFmpeg from Homebrew, which is fine here but will not run on another Mac.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)   # repo root
bin="$root/examples/cuepool/target/release/cuepool"
app="$root/dist/CuePool.app"
exe="$app/Contents/MacOS/cuepool"

if [ ! -x "$bin" ]; then
    echo "Build first: cargo build --release --manifest-path examples/cuepool/Cargo.toml -p cuepool" >&2
    echo "Missing: $bin" >&2
    exit 1
fi

version=$(cargo pkgid --manifest-path "$root/examples/cuepool/Cargo.toml" -p cuepool)
version=${version##*#}

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$exe"
cp "$root/examples/cuepool/packaging/AppIcon.icns" "$app/Contents/Resources/"
sed -e 's/__NAME__/CuePool/g' \
    -e 's/__BIN__/cuepool/g' \
    -e "s/__VERSION__/$version/g" \
    "$root/.github/packaging/Info.plist.tmpl" > "$app/Contents/Info.plist"

if command -v dylibbundler > /dev/null; then
    dylibbundler -cd -od -b -x "$exe" -d "$app/Contents/libs/" -p @executable_path/../libs/
    # dylibbundler rewrites every existing rpath to its -p value, leaving
    # duplicates that macOS 26+ dyld refuses to load. Strip them all; nothing in
    # this bundle loads through an rpath afterwards.
    otool -l "$exe" | awk '/LC_RPATH/{getline; getline; print $2}' | while read -r rp; do
        install_name_tool -delete_rpath "$rp" "$exe" 2> /dev/null || true
    done
    portability="FFmpeg is bundled, so the .app runs on any Mac."
else
    portability="FFmpeg still loads from Homebrew, so this .app runs here only. For a shareable one: brew install dylibbundler, then re-run."
fi

# install_name_tool invalidates the linker's ad-hoc signature, and arm64 will not
# launch an unsigned binary. Harmless when dylibbundler was skipped.
codesign --force --deep -s - "$app" 2> /dev/null
touch "$app"   # nudge Finder into re-reading the icon

echo "Packaged: $app ($version)"
echo "$portability"
