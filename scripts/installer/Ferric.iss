; Ferric one-click installer (Inno Setup 6). Built by scripts\package-windows.ps1 after the release folder exists,
; or by hand:  ISCC /DFerricVersion=1.0.0-rc.3 scripts\installer\Ferric.iss
; Installs for the current user only (no administrator rights), like the script installer it replaces.

#ifndef FerricVersion
  #define FerricVersion "1.0.0"
#endif
#define SourceDir "..\..\release"

[Setup]
; Fixed id so a newer Setup upgrades the installed copy instead of installing beside it.
AppId={{1B459329-23B5-4DAF-A269-02D9AB91D98C}
AppName=Ferric Download Manager
AppVersion={#FerricVersion}
AppVerName=Ferric {#FerricVersion}
AppPublisher=Ferric
DefaultDirName={localappdata}\Programs\Ferric
DefaultGroupName=Ferric
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=Ferric-Setup-{#FerricVersion}
SetupIconFile=..\..\crates\ferric-qt\assets\ferric.ico
UninstallDisplayIcon={app}\ferric-qt.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; One click: no welcome, folder or "ready" pages. The user presses Install and then Finish.
DisableWelcomePage=yes
DisableDirPage=yes
DisableReadyPage=yes
AllowNoIcons=yes

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion; Excludes: "Install-Ferric.cmd,Install-Ferric.ps1,Uninstall-Ferric.ps1"

[Icons]
Name: "{autoprograms}\Ferric"; Filename: "{app}\ferric-qt.exe"; Comment: "Ferric Download Manager"
Name: "{autodesktop}\Ferric"; Filename: "{app}\ferric-qt.exe"; Comment: "Ferric Download Manager"

[Run]
; Connects the browsers to Ferric (native-messaging host). A failure here must not abort the install.
Filename: "{app}\ferric-host.exe"; Parameters: "--install"; Flags: runhidden; StatusMsg: "Connecting your browsers..."
Filename: "{app}\ferric-qt.exe"; Description: "Start Ferric"; Flags: nowait postinstall skipifsilent
Filename: "{win}\explorer.exe"; Parameters: """{app}\extension"""; Description: "Open the browser extension folder (to add it to Chrome, Edge or Brave)"; Flags: nowait postinstall skipifsilent unchecked

[UninstallRun]
Filename: "{app}\ferric-host.exe"; Parameters: "--uninstall"; Flags: runhidden; RunOnceId: "UnregisterBrowserHost"; Check: RegistrationIsOurs

[Messages]
FinishedLabel=Ferric is installed.%n%nTo capture downloads from your browser, add the Ferric extension to it. Tick "Open the browser extension folder" below for the files and steps.

[Code]
// A running copy would lock its own files, so stop it before installing or removing.
procedure StopFerric();
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM ferric-qt.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM ferricd.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM ferric-host.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Sleep(600);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopFerric();
  Result := '';
end;

// The browser registration is shared by every Ferric copy: remove it only when it points at this install.
function RegistrationIsOurs(): Boolean;
var
  Text: AnsiString;
  Mine: String;
begin
  Result := True;
  if LoadStringFromFile(ExpandConstant('{userappdata}\Ferric\com.ferric.host.json'), Text) then
  begin
    Mine := ExpandConstant('{app}\ferric-host.exe');
    StringChangeEx(Mine, '\', '\\', True);
    Result := Pos(Lowercase(Mine), Lowercase(String(Text))) > 0;
  end;
end;

function InitializeUninstall(): Boolean;
begin
  StopFerric();
  Result := True;
end;
