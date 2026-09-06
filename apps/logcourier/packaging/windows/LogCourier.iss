#ifndef MyAppVersion
  #error MyAppVersion is required
#endif
#ifndef SourceDir
  #error SourceDir is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif

[Setup]
AppId={{60EA65E9-FAEF-4FC9-A0D6-8F1DF258C8EC}
AppName=LogCourier
AppVersion={#MyAppVersion}
AppPublisher=Oleg Shevchuk
AppPublisherURL=https://github.com/olegius88/keyswitch
AppSupportURL=https://github.com/olegius88/keyswitch/issues
DefaultDirName={localappdata}\Programs\LogCourier
DefaultGroupName=LogCourier
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=LogCourier-Setup-{#MyAppVersion}-x64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
LicenseFile={#SourceDir}\LICENSE
UninstallDisplayIcon={app}\LogCourier.exe
VersionInfoVersion={#MyAppVersion}.0
VersionInfoDescription=LogCourier installer
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Создать ярлык на рабочем столе"; Flags: unchecked

; Explicit payload: never include a user profile or arbitrary files from dist.
[Files]
Source: "{#SourceDir}\LogCourier.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LogCourier-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\_internal\*"; DestDir: "{app}\_internal"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\docs\*.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceDir}\third-party-licenses\*"; DestDir: "{app}\third-party-licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\LogCourier"; Filename: "{app}\LogCourier.exe"; Parameters: "gui"; WorkingDir: "{app}"
Name: "{autodesktop}\LogCourier"; Filename: "{app}\LogCourier.exe"; Parameters: "gui"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\LogCourier.exe"; Parameters: "gui"; Description: "Запустить LogCourier"; Flags: nowait postinstall skipifsilent

[Code]
// Autostart is enabled only in the application UI. Do not remove another
// portable installation's Run entry, or any config/queue/Credential Locker data.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Command, Executable: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    Executable := ExpandConstant('{app}\LogCourier.exe');
    if RegQueryStringValue(HKEY_CURRENT_USER,
      'Software\Microsoft\Windows\CurrentVersion\Run', 'LogCourier', Command) then
    begin
      if (CompareText(Command, '"' + Executable + '" gui --minimized') = 0) or
         (CompareText(Command, Executable + ' gui --minimized') = 0) then
        RegDeleteValue(HKEY_CURRENT_USER,
          'Software\Microsoft\Windows\CurrentVersion\Run', 'LogCourier');
    end;
  end;
end;
