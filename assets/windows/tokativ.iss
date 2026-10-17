; Compile with /DAppVersion, /DVersionInfo, /DArtifactVersion, /DTarget,
; /DBinary and /DInstallerOutput. One AppId keeps upgrades in the same installation.
#ifndef AppVersion
  #error AppVersion is required
#endif
#if Target == "aarch64-pc-windows-msvc"
  #define Architecture "arm64"
#else
  #define Architecture "x64compatible"
#endif

[Setup]
AppId={{AA37B293-CAFA-4BD7-A735-33B5AC08ECEF}
AppName=Tokativ
AppVersion={#AppVersion}
VersionInfoVersion={#VersionInfo}
AppPublisher=nolight132
AppPublisherURL=https://github.com/nolight132/tokativ
AppSupportURL=https://github.com/nolight132/tokativ/issues
AppUpdatesURL=https://github.com/nolight132/tokativ/releases
DefaultDirName={localappdata}\Programs\Tokativ
DefaultGroupName=Tokativ
DisableProgramGroupPage=yes
UsePreviousAppDir=yes
PrivilegesRequired=lowest
ArchitecturesAllowed={#Architecture}
ArchitecturesInstallIn64BitMode={#Architecture}
MinVersion=10.0
UninstallDisplayIcon={app}\tokativ.exe
LicenseFile=..\..\COPYING
OutputDir={#InstallerOutput}
OutputBaseFilename=tokativ-{#ArtifactVersion}-{#Target}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#Binary}"; DestDir: "{app}"; DestName: "tokativ.exe"; Flags: ignoreversion
Source: "..\..\COPYING"; DestDir: "{app}\licenses"; DestName: "LICENSE"; Flags: ignoreversion
Source: "..\fonts\LICENSE.txt"; DestDir: "{app}\licenses"; DestName: "LICENSE.Inter"; Flags: ignoreversion
Source: "..\silero\LICENSE"; DestDir: "{app}\licenses"; DestName: "LICENSE.silero-vad"; Flags: ignoreversion
Source: "..\icons\LICENSE"; DestDir: "{app}\licenses"; DestName: "LICENSE.lucide"; Flags: ignoreversion
Source: "..\emoji\LICENSE"; DestDir: "{app}\licenses"; DestName: "LICENSE.Unicode"; Flags: ignoreversion

[Icons]
Name: "{group}\Tokativ"; Filename: "{app}\tokativ.exe"
Name: "{autodesktop}\Tokativ"; Filename: "{app}\tokativ.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\tokativ.exe"; Description: "Launch Tokativ"; Flags: nowait postinstall skipifsilent; Check: not Updating
Filename: "{app}\tokativ.exe"; Flags: nowait; Check: Updating

[UninstallRun]
Filename: "{app}\tokativ.exe"; Parameters: "quit"; Flags: runhidden; RunOnceId: "StopTokativ"

[Code]
function Updating: Boolean;
begin
  Result := ExpandConstant('{param:UPDATE|0}') = '1';
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  // The embedded CLI also stops a daemon started from an older portable download.
  ExtractTemporaryFile('tokativ.exe');
  Exec(ExpandConstant('{tmp}\tokativ.exe'), 'quit', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and RegValueExists(HKCU,
    'Software\Microsoft\Windows\CurrentVersion\Run', 'Tokativ') then
    RegWriteStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run',
      'Tokativ', '"' + ExpandConstant('{app}\tokativ.exe') + '" daemon');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Tokativ');
end;
