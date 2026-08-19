# Builds a per-app .msi from a dist folder using the WiX dotnet tool.
# The MSI installs the folder to Program Files\<Name> and adds a Start-menu shortcut.
param(
    [Parameter(Mandatory)] [string]$Name,       # display name, e.g. "CuePool"
    [Parameter(Mandatory)] [string]$Version,    # numeric x.y.z
    [Parameter(Mandatory)] [string]$SourceDir,  # folder whose files get installed
    [Parameter(Mandatory)] [string]$Exe,        # exe filename inside SourceDir
    [string]$Icon,                              # optional .ico for the shortcut
    [string]$FileExt,                           # optional extension to open with this app (e.g. "qproj")
    [Parameter(Mandatory)] [string]$Out         # output .msi path
)
$ErrorActionPreference = 'Stop'

if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
    # ponytail: pinned to v5 — WiX v6+ requires accepting the OSMF EULA (WIX7015)
    dotnet tool install --global wix --version 5.0.2 | Out-Null
}

# Deterministic UpgradeCode per app so a newer MSI replaces the older install.
$md5 = [System.Security.Cryptography.MD5]::Create()
$upgrade = [Guid]::new($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("rustjay-msi:$Name")))

$iconXml = ''
$shortcutIcon = ''
$iconFileComponent = ''
$progIdIcon = ''
if ($Icon -and (Test-Path $Icon)) {
    $iconXml = "<Icon Id=`"AppIcon`" SourceFile=`"$((Resolve-Path $Icon).Path)`" />"
    $shortcutIcon = ' Icon="AppIcon"'
    # For a non-advertised ProgId, Icon must reference an installed *file* holding
    # the icon (not the shortcut's Icon-table entry) — ship the .ico next to the exe.
    $iconFileComponent = "        <Component Id=`"AppIconComponent`"><File Id=`"AppIconFile`" Source=`"$((Resolve-Path $Icon).Path)`" /></Component>"
    $progIdIcon = ' Icon="AppIconFile"'
}

# Explorer double-click association: ProgId + open verb nested in the exe's
# component. "&quot;%1&quot;" is the WiX quoting idiom for the clicked file's path.
$fileAssoc = ''
if ($FileExt) {
    $fileAssoc = @"
<ProgId Id="$Name.$FileExt" Description="$Name Project File"$progIdIcon>
          <Extension Id="$FileExt">
            <Verb Id="open" Command="Open" TargetFile="AppExe" Argument="&quot;%1&quot;" />
          </Extension>
        </ProgId>
"@
}

$components = (Get-ChildItem $SourceDir -File | ForEach-Object {
    if ($FileExt -and $_.Name -eq $Exe) {
        "        <Component><File Id=`"AppExe`" Source=`"$($_.FullName)`" />$fileAssoc</Component>"
    } else {
        "        <Component><File Source=`"$($_.FullName)`" /></Component>"
    }
}) -join "`n"

$wxs = @"
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Name="$Name" Manufacturer="BlueJayLouche" Version="$Version"
           UpgradeCode="$upgrade" Scope="perMachine">
    <MajorUpgrade DowngradeErrorMessage="A newer version of $Name is already installed." />
    <MediaTemplate EmbedCab="yes" />
    $iconXml
    <StandardDirectory Id="ProgramFiles64Folder">
      <Directory Id="INSTALLFOLDER" Name="$Name">
$components
$iconFileComponent
      </Directory>
    </StandardDirectory>
    <StandardDirectory Id="ProgramMenuFolder">
      <Component Id="StartMenuShortcut">
        <Shortcut Id="AppShortcut" Name="$Name" Target="[INSTALLFOLDER]$Exe"$shortcutIcon />
        <RegistryValue Root="HKCU" Key="Software\BlueJayLouche\$Name" Name="installed"
                       Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </StandardDirectory>
  </Package>
</Wix>
"@

$wxsPath = Join-Path ([IO.Path]::GetTempPath()) "$Name.wxs"
Set-Content $wxsPath $wxs -Encoding utf8
wix build $wxsPath -arch x64 -o $Out
if ($LASTEXITCODE -ne 0) { throw "wix build failed" }
"MSI: $Out ($([math]::Round((Get-Item $Out).Length/1MB,1)) MB)"
