# Builds the LogCourier installer: cargo build --release -p logcourier, the two executables and Inno Setup.
param([string]$OutputDirectory = "")
$ErrorActionPreference = "Stop"
$Project = Split-Path -Parent $PSScriptRoot
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $Project "dist" }
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$Version = ((Select-String -LiteralPath (Join-Path $Project "crates\logcourier\Cargo.toml") -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value)
$env:RUSTFLAGS = "-C target-feature=+crt-static"
$Previous = $ErrorActionPreference
$ErrorActionPreference = "Continue"
& cargo build --release -p logcourier --target x86_64-pc-windows-msvc --manifest-path (Join-Path $Project "Cargo.toml")
$Code = $LASTEXITCODE
$ErrorActionPreference = $Previous
if ($Code -ne 0) { throw "cargo build failed with exit code $Code" }
$Target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Project "target" }
$Built = Join-Path $Target "x86_64-pc-windows-msvc\release"
$Stage = Join-Path $Project "dist\LogCourier"
Remove-Item -Recurse -Force -LiteralPath $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $Stage | Out-Null
Copy-Item (Join-Path $Built "logcourier.exe") (Join-Path $Stage "LogCourier.exe")
Copy-Item (Join-Path $Built "logcourier-cli.exe") (Join-Path $Stage "LogCourier-cli.exe")
Copy-Item (Join-Path $Project "LICENSE") (Join-Path $Stage "LICENSE")
$Iscc = Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"
& $Iscc "/DMyAppVersion=$Version" "/DSourceDir=$Stage" "/DOutputDir=$OutputDirectory" (Join-Path $Project "packaging\windows\LogCourier.iss") | Select-Object -Last 2
if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }
"LogCourier.exe: {0:N1} MB; installer: {1:N1} MB" -f ((Get-Item (Join-Path $Stage "LogCourier.exe")).Length / 1MB), ((Get-Item (Join-Path $OutputDirectory "LogCourier-Setup-$Version-x64.exe")).Length / 1MB)
