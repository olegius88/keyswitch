#ifndef MyAppVersion
  #error MyAppVersion is required
#endif
#ifndef SourceDir
  #error SourceDir is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif
#ifndef SetupIcon
  #error SetupIcon is required
#endif
; x64 (the default) or arm64. The x64 installer also installs on Windows 11 on Arm,
; where the program runs under emulation; the arm64 one installs only there.
#ifndef Arch
  #define Arch "x64"
#endif
#if Arch == "arm64"
  #define ArchitecturesSupported "arm64"
#elif Arch == "x64"
  #define ArchitecturesSupported "x64compatible"
#else
  #error Arch must be x64 or arm64
#endif

[Setup]
AppId={{8E630D23-C19A-4B31-9D59-0F75F925BB95}
AppName=KeySwitch
AppVersion={#MyAppVersion}
AppPublisher=Oleg Shevchuk
AppPublisherURL=https://github.com/olegius88/keyswitch
AppSupportURL=https://github.com/olegius88/keyswitch/issues
AppUpdatesURL=https://github.com/olegius88/keyswitch/releases
DefaultDirName={localappdata}\Programs\KeySwitch
DefaultGroupName=KeySwitch
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir={#OutputDir}
OutputBaseFilename=KeySwitch-Setup-{#MyAppVersion}-{#Arch}
SetupIconFile={#SetupIcon}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed={#ArchitecturesSupported}
ArchitecturesInstallIn64BitMode={#ArchitecturesSupported}
UninstallDisplayIcon={app}\KeySwitch.exe
VersionInfoVersion={#MyAppVersion}.0
VersionInfoCompany=Oleg Shevchuk
VersionInfoDescription=KeySwitch installer
VersionInfoProductName=KeySwitch
VersionInfoProductVersion={#MyAppVersion}

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Создать ярлык на рабочем столе"; GroupDescription: "Дополнительные ярлыки:"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\KeySwitch"; Filename: "{app}\KeySwitch.exe"
Name: "{autodesktop}\KeySwitch"; Filename: "{app}\KeySwitch.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\KeySwitch.exe"; Description: "Запустить KeySwitch"; Flags: nowait postinstall skipifsilent
Filename: "{app}\KeySwitch.exe"; Parameters: "--hidden"; Flags: nowait skipifnotsilent; Check: IsKeySwitchAutoUpdate

[Code]
function IsKeySwitchAutoUpdate: Boolean;
begin
  Result := CompareText(
    ExpandConstant('{param:KEYSWITCHUPDATE|0}'),
    '1') = 0;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RegDeleteValue(
      HKEY_CURRENT_USER,
      'Software\Microsoft\Windows\CurrentVersion\Run',
      'KeySwitch');
end;
