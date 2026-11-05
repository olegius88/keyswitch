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
Copy-Item -LiteralPath (Join-Path $ProjectDirectory "packaging\third-party-licenses\windows.txt") -Destination (Join-Path $ResourceStage "licenses\third-party-rust.txt")
# What was copied is what was checked.
Assert-Frozen -Path (Join-Path $ModelStage "en_US.lm") -Policy $EnglishPolicy -Label "staged English language model"
Assert-Frozen -Path (Join-Path $ModelStage "ru_RU.lm") -Policy $RussianPolicy -Label "staged Russian language model"
$Icon = Join-Path $ResourceStage "keyswitch.ico"
# The icon is drawn once from resources/keyswitch.svg and kept in the repository; the executable embeds the same file.
Copy-Item -LiteralPath (Join-Path $ProjectDirectory "packaging\windows\keyswitch.ico") -Destination $Icon

# The staged program must run and see its models (the same check the Python build made of its executable).
$Probe = Join-Path $ProjectDirectory "dist\post-build-diagnostics.json"
$env:KEYSWITCH_CONFIG_DIR = Join-Path $ProjectDirectory "dist\post-build-config"
$env:KEYSWITCH_DATA_DIR = Join-Path $ProjectDirectory "dist\post-build-data"
Remove-Item Env:KEYSWITCH_INTENT_MODEL_PATH -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $env:KEYSWITCH_CONFIG_DIR, $env:KEYSWITCH_DATA_DIR | Out-Null
$Process = Start-Process -FilePath (Join-Path $Stage "KeySwitch.exe") -ArgumentList "--diagnose" -RedirectStandardOutput $Probe -PassThru
# Without a handle taken while it runs, Windows PowerShell 5.1 reports no exit code for a -PassThru process.
$null = $Process.Handle
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
# Inno Setup resolves a relative output folder against the script, not the current one.
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
Invoke-NativeCommand -Command $IsccPath -Arguments @(
    "/DMyAppVersion=$Version", "/DArch=$Architecture", "/DSourceDir=$Stage", "/DOutputDir=$OutputDirectory",
    "/DSetupIcon=$Icon", (Join-Path $PSScriptRoot "windows\KeySwitch.iss")
) -FailureMessage "Inno Setup build failed"
$Installer = Join-Path $OutputDirectory "KeySwitch-Setup-$Version-$Architecture.exe"
if (-not (Test-Path -LiteralPath $Installer -PathType Leaf)) {
    throw "Inno Setup did not produce $Installer"
}

# The portable archive: the staged folder as it is, for a computer where nothing is installed.
$Archive = Join-Path $OutputDirectory "KeySwitch-$Version-windows-$Architecture.zip"
if (Test-Path -LiteralPath $Archive) {
    Remove-Item -Force -LiteralPath $Archive
}
Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $Archive -CompressionLevel Optimal

function Format-Size([long]$Bytes) { return "{0:N1} MB" -f ($Bytes / 1MB) }
$ResourceBytes = (Get-ChildItem -LiteralPath $ResourceStage -Recurse -File | Measure-Object Length -Sum).Sum
Write-Host ("KeySwitch.exe: {0}" -f (Format-Size (Get-Item (Join-Path $Stage "KeySwitch.exe")).Length))
Write-Host ("resources:     {0}" -f (Format-Size $ResourceBytes))
Write-Host ("Installer:     {0}  {1}" -f (Format-Size (Get-Item $Installer).Length), $Installer)
Write-Host ("Archive:       {0}  {1}" -f (Format-Size (Get-Item $Archive).Length), $Archive)
