# Builds the Windows installer of the Rust KeySwitch: cargo build --release -p ks-win, the staged folder
# (KeySwitch.exe and resources\), then Inno Setup. Needs Rust with the MSVC target and Inno Setup 6 (ISCC.exe).
param(
    [string]$OutputDirectory = "",
    # x64, arm64 or x86; empty means the architecture of this computer.
    [string]$Architecture = ""
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

function Invoke-NativeCommand {
    param([string]$Command, [string[]]$Arguments, [string]$FailureMessage)
    # Windows PowerShell 5.1 turns a native program's stderr into errors; cargo writes its progress there.
    $Previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        & $Command @Arguments
        $Code = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $Previous
    }
    if ($Code -ne 0) {
        throw "$FailureMessage (exit code $Code)"
    }
}

function Get-FileSha256 {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Missing file: $Path"
    }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-Frozen {
    # The same check build-windows.ps1 made of the language models: the size and the SHA-256 the training config names.
    param([string]$Path, $Policy, [string]$Label)
    $Item = Get-Item -LiteralPath $Path
    if ($Item.Length -ne [long]$Policy.bytes) {
        throw "$Label has $($Item.Length) bytes, expected $($Policy.bytes): $Path"
    }
    if ((Get-FileSha256 -Path $Path) -cne [string]$Policy.sha256) {
        throw "$Label differs from the frozen training source: $Path"
    }
}

function New-KeySwitchIcon {
    # The application icon (tools/create_windows_icon.py, which used Pillow): a blue rounded square with KS, as an ICO of
    # PNG images at the sizes of the Python's export.
    param([string]$Output)
    $Sizes = 16, 24, 32, 48, 64, 128, 256
    function New-RoundedPath([float]$Left, [float]$Top, [float]$Right, [float]$Bottom, [float]$Radius) {
        $Path = New-Object System.Drawing.Drawing2D.GraphicsPath
        $D = 2 * $Radius
        $Path.AddArc($Left, $Top, $D, $D, 180, 90)
        $Path.AddArc($Right - $D, $Top, $D, $D, 270, 90)
        $Path.AddArc($Right - $D, $Bottom - $D, $D, $D, 0, 90)
        $Path.AddArc($Left, $Bottom - $D, $D, $D, 90, 90)
        $Path.CloseFigure()
        return $Path
    }
    $Master = New-Object System.Drawing.Bitmap 256, 256, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $G = [System.Drawing.Graphics]::FromImage($Master)
    $G.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $G.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $Fill = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 35, 92, 190))
    $G.FillPath($Fill, (New-RoundedPath 8 8 248 248 56))
    $Pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(70, 255, 255, 255)), 5
    $G.DrawPath($Pen, (New-RoundedPath 31 31 225 225 40))
    $Font = New-Object System.Drawing.Font "Arial", 86, ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Pixel)
    $Format = New-Object System.Drawing.StringFormat
    $Format.Alignment = [System.Drawing.StringAlignment]::Center
    $Format.LineAlignment = [System.Drawing.StringAlignment]::Center
    $G.DrawString("KS", $Font, [System.Drawing.Brushes]::White, (New-Object System.Drawing.RectangleF 0, 0, 256, 256), $Format)
    $G.Dispose()
    $Images = @()
    foreach ($Size in $Sizes) {
        $Scaled = New-Object System.Drawing.Bitmap $Size, $Size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $S = [System.Drawing.Graphics]::FromImage($Scaled)
        $S.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $S.DrawImage($Master, 0, 0, $Size, $Size)
        $S.Dispose()
        $Stream = New-Object System.IO.MemoryStream
        $Scaled.Save($Stream, [System.Drawing.Imaging.ImageFormat]::Png)
        $Images += , $Stream.ToArray()
        $Scaled.Dispose()
    }
    $Master.Dispose()
    $File = New-Object System.IO.MemoryStream
    $Writer = New-Object System.IO.BinaryWriter $File
    $Writer.Write([uint16]0); $Writer.Write([uint16]1); $Writer.Write([uint16]$Sizes.Count)
    $Offset = 6 + 16 * $Sizes.Count
    for ($Index = 0; $Index -lt $Sizes.Count; $Index++) {
        $Size = $Sizes[$Index]
        $Dimension = if ($Size -ge 256) { 0 } else { $Size }
        $Writer.Write([byte]$Dimension); $Writer.Write([byte]$Dimension); $Writer.Write([byte]0); $Writer.Write([byte]0)
        $Writer.Write([uint16]1); $Writer.Write([uint16]32)
        $Writer.Write([uint32]$Images[$Index].Length); $Writer.Write([uint32]$Offset)
        $Offset += $Images[$Index].Length
    }
    foreach ($Image in $Images) { $Writer.Write($Image) }
    $Writer.Flush()
    [System.IO.File]::WriteAllBytes($Output, $File.ToArray())
}

$ProjectDirectory = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $ProjectDirectory "dist"
}
if (-not $Architecture) {
    $Architecture = switch ($env:PROCESSOR_ARCHITECTURE) { "ARM64" { "arm64" } "x86" { "x86" } default { "x64" } }
}
$Triple = switch ($Architecture) {
    "arm64" { "aarch64-pc-windows-msvc" }
    "x86" { "i686-pc-windows-msvc" }
    "x64" { "x86_64-pc-windows-msvc" }
    default { throw "Architecture must be x64, arm64 or x86" }
}

$WorkspaceManifest = Get-Content -LiteralPath (Join-Path $ProjectDirectory "Cargo.toml") -Raw
if ($WorkspaceManifest -notmatch '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"') {
    throw "The workspace version is not in Cargo.toml"
}
$Version = $Matches[1]

# The language models the engine reads, from the frozen training sources and checked against the training config.
$FrozenDirectory = Join-Path $ProjectDirectory "model\intent_v1\sources"
$Config = Get-Content -LiteralPath (Join-Path $ProjectDirectory "model\intent_v1\config.json") -Raw -Encoding UTF8 | ConvertFrom-Json
$EnglishPolicy = $Config.sources.languages.en_US
$RussianPolicy = $Config.sources.languages.ru_RU
$LicensePolicy = $Config.sources.license_evidence
if ($EnglishPolicy.path -cne "model/intent_v1/sources/en_US.lm" -or $RussianPolicy.path -cne "model/intent_v1/sources/ru_RU.lm") {
    throw "The frozen language-model paths differ from the packaging contract"
}
Assert-Frozen -Path (Join-Path $FrozenDirectory "en_US.lm") -Policy $EnglishPolicy -Label "English language model"
Assert-Frozen -Path (Join-Path $FrozenDirectory "ru_RU.lm") -Policy $RussianPolicy -Label "Russian language model"
Assert-Frozen -Path (Join-Path $FrozenDirectory "COPYRIGHT.onboard-data") -Policy $LicensePolicy -Label "language-model license"

# Static CRT: the executable needs no Visual C++ redistributable on the target.
$env:RUSTFLAGS = "-C target-feature=+crt-static"
Push-Location $ProjectDirectory
try {
    Invoke-NativeCommand -Command "cargo" -Arguments @("build", "--release", "-p", "ks-win", "--target", $Triple) -FailureMessage "cargo build failed"
}
finally {
    Pop-Location
}
$TargetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $ProjectDirectory "target" }
$Executable = Join-Path $TargetRoot "$Triple\release\KeySwitch.exe"
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
    throw "cargo did not produce $Executable"
}

# Stage: KeySwitch.exe and the resources the program reads (the svg icons, style.css and the Linux and macOS files are not).
$Stage = Join-Path $ProjectDirectory "dist\KeySwitch"
if (Test-Path -LiteralPath $Stage) {
    Remove-Item -Recurse -Force -LiteralPath $Stage
}
$ResourceStage = Join-Path $Stage "resources"
$ModelStage = Join-Path $ResourceStage "models"
New-Item -ItemType Directory -Force $ModelStage, (Join-Path $ResourceStage "licenses") | Out-Null
Copy-Item -LiteralPath $Executable -Destination (Join-Path $Stage "KeySwitch.exe")
$Source = Join-Path $ProjectDirectory "resources"
foreach ($Name in "identifiers.json", "protected_tokens.txt", "lexicon-supplement-ru_RU.json", "typo-frequency-en_US.bin", "typo-frequency-ru_RU.bin") {
    Copy-Item -LiteralPath (Join-Path $Source $Name) -Destination $ResourceStage
}
foreach ($Name in "layout_intent_v1.ksm", "context_policy_v1.json", "context-term-frequency.json", "context-alnum-frequency.json",
                  "context-short-frequency.json", "prefix_policy_v1.json", "boundary-v2.json", "ortho_v1.json") {
    Copy-Item -LiteralPath (Join-Path $Source "models\$Name") -Destination $ModelStage
}
Copy-Item -LiteralPath (Join-Path $FrozenDirectory "en_US.lm") -Destination $ModelStage
Copy-Item -LiteralPath (Join-Path $FrozenDirectory "ru_RU.lm") -Destination $ModelStage
Copy-Item -LiteralPath (Join-Path $FrozenDirectory "COPYRIGHT.onboard-data") -Destination (Join-Path $ResourceStage "licenses\COPYRIGHT.onboard-data.txt")
Copy-Item -LiteralPath (Join-Path $ProjectDirectory "LICENSE") -Destination (Join-Path $ResourceStage "licenses\LICENSE.KeySwitch.txt")
# What was copied is what was checked.
Assert-Frozen -Path (Join-Path $ModelStage "en_US.lm") -Policy $EnglishPolicy -Label "staged English language model"
Assert-Frozen -Path (Join-Path $ModelStage "ru_RU.lm") -Policy $RussianPolicy -Label "staged Russian language model"
$Icon = Join-Path $ResourceStage "keyswitch.ico"
New-KeySwitchIcon -Output $Icon

# The staged program must run and see its models (the same check the Python build made of its executable).
$Probe = Join-Path $ProjectDirectory "dist\post-build-diagnostics.json"
$env:KEYSWITCH_CONFIG_DIR = Join-Path $ProjectDirectory "dist\post-build-config"
$env:KEYSWITCH_DATA_DIR = Join-Path $ProjectDirectory "dist\post-build-data"
Remove-Item Env:KEYSWITCH_INTENT_MODEL_PATH -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $env:KEYSWITCH_CONFIG_DIR, $env:KEYSWITCH_DATA_DIR | Out-Null
$Process = Start-Process -FilePath (Join-Path $Stage "KeySwitch.exe") -ArgumentList "--diagnose" -RedirectStandardOutput $Probe -PassThru
if (-not $Process.WaitForExit(30000)) {
    $Process.Kill()
    throw "Post-build diagnostics timed out after 30 seconds"
}
if ($Process.ExitCode -notin @(0, 1)) {
    throw "Post-build diagnostics failed with exit code $($Process.ExitCode)"
}
$Diagnostics = Get-Content -LiteralPath $Probe -Raw -Encoding UTF8 | ConvertFrom-Json
if (-not $Diagnostics.intent_model.available) { throw "The staged program cannot load its intent model: $($Diagnostics.intent_model.error)" }
if (-not $Diagnostics.context_model.available) { throw "The staged program cannot load its contextual model: $($Diagnostics.context_model.status)" }
if (-not $Diagnostics.prefix_model.available) { throw "The staged program cannot load its prefix model" }
if (-not $Diagnostics.context_field_access.available) { throw "The staged program cannot start UI Automation" }
Remove-Item Env:KEYSWITCH_CONFIG_DIR, Env:KEYSWITCH_DATA_DIR

$IsccCommand = Get-Command ISCC.exe -ErrorAction SilentlyContinue
$IsccPath = if ($IsccCommand) { $IsccCommand.Source } else { "" }
if (-not $IsccPath) {
    $Default = Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"
    if (Test-Path -LiteralPath $Default -PathType Leaf) { $IsccPath = $Default }
}
if (-not $IsccPath) {
    throw "Inno Setup 6 compiler (ISCC.exe) is required"
}
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
Invoke-NativeCommand -Command $IsccPath -Arguments @(
    "/DMyAppVersion=$Version", "/DArch=$Architecture", "/DSourceDir=$Stage", "/DOutputDir=$OutputDirectory",
    "/DSetupIcon=$Icon", (Join-Path $PSScriptRoot "windows\KeySwitch.iss")
) -FailureMessage "Inno Setup build failed"
$Installer = Join-Path $OutputDirectory "KeySwitch-Setup-$Version-$Architecture.exe"
if (-not (Test-Path -LiteralPath $Installer -PathType Leaf)) {
    throw "Inno Setup did not produce $Installer"
}

function Format-Size([long]$Bytes) { return "{0:N1} MB" -f ($Bytes / 1MB) }
$ResourceBytes = (Get-ChildItem -LiteralPath $ResourceStage -Recurse -File | Measure-Object Length -Sum).Sum
Write-Host ("KeySwitch.exe: {0}" -f (Format-Size (Get-Item (Join-Path $Stage "KeySwitch.exe")).Length))
Write-Host ("resources:     {0}" -f (Format-Size $ResourceBytes))
Write-Host ("Installer:     {0}  {1}" -f (Format-Size (Get-Item $Installer).Length), $Installer)
