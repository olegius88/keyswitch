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
OutputBaseFilename=KeySwitch-Setup-{#MyAppVersion}-x64
SetupIconFile={#SetupIcon}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
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
