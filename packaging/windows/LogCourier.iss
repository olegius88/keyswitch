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

[InstallDelete]
; What the previous version kept beside its executable.
Type: filesandordirs; Name: "{app}\_internal"
Type: files; Name: "{app}\README.md"

[Files]
Source: "{#SourceDir}\LogCourier.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LogCourier-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\LogCourier"; Filename: "{app}\LogCourier.exe"
Name: "{autodesktop}\LogCourier"; Filename: "{app}\LogCourier.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\LogCourier.exe"; Description: "Запустить LogCourier"; Flags: nowait postinstall skipifsilent
