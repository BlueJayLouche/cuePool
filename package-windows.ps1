# Packages cuepool.exe + the FFmpeg DLLs from $env:FFMPEG_DIR\bin into a
# self-contained, shareable folder + zip, matching .github/workflows/release.yml.
# DLLs sit next to the exe so colleagues need no PATH setup. Re-run after a rebuild.
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot   # repo root
$exe  = Join-Path $root 'target\release\cuepool.exe'
$ffmpeg = $env:FFMPEG_DIR
$dist = Join-Path $root 'dist'
$out = Join-Path $dist 'cuepool'
$zip = Join-Path $dist 'cuepool-windows.zip'
$suffix = [guid]::NewGuid().ToString('N')
$staging = Join-Path $dist "cuepool.$suffix.partial"
$zipStaging = Join-Path $dist "cuepool-windows.$suffix.partial.zip"
$outBackup = Join-Path $dist "cuepool.$suffix.previous"
$zipBackup = Join-Path $dist "cuepool-windows.$suffix.previous.zip"

if (-not (Test-Path $exe)) { throw "Build first: cargo build --release --locked -p cuepool --all-features. Missing: $exe" }
if (-not $ffmpeg) { throw "Set FFMPEG_DIR to the FFmpeg 8.0 shared SDK used for the build (see .github/workflows/release.yml)." }
$ffmpegBin = Join-Path $ffmpeg 'bin'
$dlls = @(Get-ChildItem $ffmpegBin -Filter '*.dll' -File -ErrorAction Stop)
if ($dlls.Count -eq 0) { throw "No FFmpeg DLLs found in: $ffmpegBin" }

New-Item -ItemType Directory -Force $staging | Out-Null

try {
    Copy-Item $exe $staging
    Copy-Item $dlls.FullName $staging

    # VC++ runtime (app-local, redistributable) so machines without the redist still run.
    foreach ($d in 'vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll') {
        $src = Join-Path $env:SystemRoot "System32\$d"
        if (Test-Path $src) { Copy-Item $src $staging }
    }

    @"
cuepool (Windows)

Just run cuepool.exe. All required DLLs are in this folder.
If Windows blocks it ("Windows protected your PC"), click More info > Run anyway.
"@ | Out-File (Join-Path $staging 'README.txt') -Encoding utf8

    Compress-Archive -Path "$staging\*" -DestinationPath $zipStaging
} catch {
    Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $zipStaging -Force -ErrorAction SilentlyContinue
    throw
}

$outBackedUp = $false
$zipBackedUp = $false
$newOutPublished = $false
$newZipPublished = $false
try {
    if (Test-Path $out) {
        Move-Item $out $outBackup
        $outBackedUp = $true
    }
    if (Test-Path $zip) {
        Move-Item $zip $zipBackup
        $zipBackedUp = $true
    }
    Move-Item $staging $out
    $newOutPublished = $true
    Move-Item $zipStaging $zip
    $newZipPublished = $true
} catch {
    if ($newOutPublished -and (Test-Path $out)) {
        Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
    }
    if ($newZipPublished -and (Test-Path $zip)) {
        Remove-Item $zip -Force -ErrorAction SilentlyContinue
    }
    if ($outBackedUp -and (Test-Path $outBackup)) {
        Move-Item $outBackup $out
    }
    if ($zipBackedUp -and (Test-Path $zipBackup)) {
        Move-Item $zipBackup $zip
    }
    Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $zipStaging -Force -ErrorAction SilentlyContinue
    throw
}

Remove-Item $outBackup -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $zipBackup -Force -ErrorAction SilentlyContinue
"Packaged: $out"
"Zip:      $zip ($([math]::Round((Get-Item $zip).Length/1MB,1)) MB)"
