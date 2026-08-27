param(
    [string]$OutputDirectory = "",
    [string]$ModelDirectory = "",
    [string]$ModelLicense = ""
)

$ErrorActionPreference = "Stop"

function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Command,
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,
        [Parameter(Mandatory = $true)]
        [string]$FailureMessage
    )

    # Windows PowerShell 5.1 exposes a native program's stderr as the PowerShell
    # error stream. Tools such as Nuitka write normal progress messages there,
    # so ErrorActionPreference=Stop would abort an otherwise successful build.
    $PreviousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        & $Command @Arguments
        $NativeExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $PreviousErrorActionPreference
    }

    if ($NativeExitCode -ne 0) {
        throw "$FailureMessage (exit code $NativeExitCode)"
    }
}

$ProjectDirectory = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $ProjectDirectory "dist"
}
if (-not $ModelDirectory) {
    $ModelDirectory = Join-Path $ProjectDirectory "build\windows-models\models"
}
if (-not $ModelLicense) {
    $ModelLicense = Join-Path $ProjectDirectory "build\windows-models\COPYRIGHT.onboard-data"
}

$PyProject = Get-Content (Join-Path $ProjectDirectory "pyproject.toml") -Raw
$Module = Get-Content (Join-Path $ProjectDirectory "src\keyswitch\__init__.py") -Raw
$VersionMatch = [regex]::Match($PyProject, '(?m)^version = "([^"]+)"\r?$')
$ModuleMatch = [regex]::Match($Module, '(?m)^__version__ = "([^"]+)"\r?$')
if (-not $VersionMatch.Success -or -not $ModuleMatch.Success) {
    throw "Cannot read KeySwitch version"
}
$Version = $VersionMatch.Groups[1].Value
if ($Version -ne $ModuleMatch.Groups[1].Value) {
    throw "Version mismatch between pyproject.toml and keyswitch.__init__"
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Windows packaging requires a three-part numeric version"
}

foreach ($Locale in @("en_US", "ru_RU")) {
    $Model = Join-Path $ModelDirectory "$Locale.lm"
    if (-not (Test-Path $Model -PathType Leaf)) {
        throw "Required language model is missing: $Model"
    }
}
if (-not (Test-Path $ModelLicense -PathType Leaf)) {
    throw "Required language-model license is missing: $ModelLicense"
}

$BuildDirectory = Join-Path $ProjectDirectory "build\windows"
$NativeOutput = Join-Path $BuildDirectory "native"
$Icon = Join-Path $BuildDirectory "keyswitch.ico"
$EntryPoint = Join-Path $ProjectDirectory "packaging\keyswitch_windows_entry.py"
$NativeDistribution = Join-Path $NativeOutput "keyswitch_windows_entry.dist"
$Executable = Join-Path $NativeDistribution "KeySwitch.exe"

if (Test-Path $BuildDirectory) {
    Remove-Item -Recurse -Force $BuildDirectory
}
New-Item -ItemType Directory -Force $NativeOutput, $OutputDirectory | Out-Null

Invoke-NativeCommand `
    -Command "python" `
    -Arguments @((Join-Path $ProjectDirectory "tools\create_windows_icon.py"), $Icon) `
    -FailureMessage "Windows icon generation failed"

$env:PYTHONPATH = (Join-Path $ProjectDirectory "src")
$NuitkaArguments = @(
    "-m", "nuitka",
    "--mode=standalone",
    "--lto=no",
    "--msvc=latest",
    "--assume-yes-for-downloads",
    "--enable-plugin=tk-inter",
    "--windows-console-mode=disable",
    "--windows-icon-from-ico=$Icon",
    "--company-name=Oleg Shevchuk",
    "--product-name=KeySwitch",
    "--file-description=Automatic EN/RU keyboard layout correction",
    "--file-version=$Version.0",
    "--product-version=$Version.0",
    "--copyright=GNU GPL-3.0-or-later",
    "--output-dir=$NativeOutput",
    "--output-filename=KeySwitch.exe",
    "--include-module=keyswitch.windows_ui",
    "--include-module=keyswitch.windows_native",
    "--include-module=keyswitch.windows_instance_native",
    "--include-module=keyswitch.windows_registry",
    "--include-module=keyswitch.windows_tray_native",
    "--include-module=pystray._base",
    "--include-module=pystray._util",
    "--include-module=pystray._util.win32",
    "--include-module=pystray._win32",
    "--include-package=PIL",
    "--include-package-data=keyswitch",
    "--include-data-files=$(Join-Path $ModelDirectory 'en_US.lm')=keyswitch/resources/models/en_US.lm",
    "--include-data-files=$(Join-Path $ModelDirectory 'ru_RU.lm')=keyswitch/resources/models/ru_RU.lm",
    "--nofollow-import-to=keyswitch.app",
    "--nofollow-import-to=keyswitch.ui",
    "--nofollow-import-to=keyswitch.tray",
    "--nofollow-import-to=keyswitch.x11_backend",
    "--nofollow-import-to=pystray._appindicator",
    "--nofollow-import-to=pystray._darwin",
    "--nofollow-import-to=pystray._dummy",
    "--nofollow-import-to=pystray._gtk",
    "--nofollow-import-to=pystray._xorg",
    "--no-progressbar",
    "--report=$(Join-Path $NativeOutput 'compilation-report.xml')",
    $EntryPoint
)
Invoke-NativeCommand `
    -Command "python" `
    -Arguments $NuitkaArguments `
    -FailureMessage "Nuitka Windows build failed"
if (-not (Test-Path $Executable -PathType Leaf)) {
    throw "Nuitka did not produce $Executable"
}

$UnexpectedPython = Get-ChildItem $NativeDistribution -Recurse -File | Where-Object {
    $_.Extension -in @(".py", ".pyc", ".pyo")
}
if ($UnexpectedPython) {
    throw "Native distribution contains Python source or bytecode"
}

$ProductVersion = (Get-Item $Executable).VersionInfo.ProductVersion
if ($ProductVersion -ne "$Version.0") {
    throw "Unexpected executable product version: $ProductVersion"
}

$LicenseDirectory = Join-Path $NativeDistribution "licenses"
New-Item -ItemType Directory -Force $LicenseDirectory | Out-Null
Invoke-NativeCommand `
    -Command "python" `
    -Arguments @(
        (Join-Path $ProjectDirectory "tools\collect_python_licenses.py"),
        $LicenseDirectory,
        "Nuitka",
        "pystray",
        "Pillow",
        "six"
    ) `
    -FailureMessage "Third-party license collection failed"
Copy-Item (Join-Path $ProjectDirectory "LICENSE") (Join-Path $LicenseDirectory "LICENSE.KeySwitch.txt")
Copy-Item (Join-Path $ProjectDirectory "README.en.md") (Join-Path $NativeDistribution "README.en.md")
Copy-Item (Join-Path $ProjectDirectory "README.md") (Join-Path $NativeDistribution "README.md")
Copy-Item $ModelLicense (Join-Path $LicenseDirectory "COPYRIGHT.onboard-data.txt")

$ZipPath = Join-Path $OutputDirectory "KeySwitch-$Version-windows-x64.zip"
if (Test-Path $ZipPath) {
    Remove-Item -Force $ZipPath
}
Compress-Archive -Path (Join-Path $NativeDistribution "*") -DestinationPath $ZipPath -CompressionLevel Optimal

$IsccCommand = Get-Command ISCC.exe -ErrorAction SilentlyContinue
$IsccPath = if ($IsccCommand) { $IsccCommand.Source } else { "" }
if (-not $IsccPath) {
    $DefaultIscc = Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"
    if (Test-Path $DefaultIscc -PathType Leaf) {
        $IsccPath = $DefaultIscc
    }
}
if (-not $IsccPath) {
    throw "Inno Setup 6 compiler (ISCC.exe) is required"
}

$InstallerScript = Join-Path $ProjectDirectory "packaging\windows\KeySwitch.iss"
Invoke-NativeCommand `
    -Command $IsccPath `
    -Arguments @(
        "/DMyAppVersion=$Version",
        "/DSourceDir=$NativeDistribution",
        "/DOutputDir=$OutputDirectory",
        "/DSetupIcon=$Icon",
        $InstallerScript
    ) `
    -FailureMessage "Inno Setup build failed"

$Installer = Join-Path $OutputDirectory "KeySwitch-Setup-$Version-x64.exe"
if (-not (Test-Path $Installer -PathType Leaf)) {
    throw "Inno Setup did not produce $Installer"
}

Write-Host "Built $ZipPath"
Write-Host "Built $Installer"
