; Installer of opensecagent for Windows (Inno Setup 6, https://jrsoftware.org/isinfo.php).
; Built by dev\build-installer.ps1, which builds the programs first and passes the version from Cargo.toml.
;
; Installs the programs for all users, sets up the Windows service (opensecagent.exe service-install) and then
; opens the enrollment dialog (opensecagent-enroll.exe), unless the computer is enrolled already; closing it means
; enrolling later with "Enroll with opensecmon" in the start menu. An update stops the service first and keeps the
; enrollment. Uninstalling removes the service, the enrollment and the agent's folder.
;
; Without questions, e.g. for a software distribution (the code is used once and expires):
;     opensecagent-<version>-x64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /CODE=osa1_...
; The code goes to the program through a file in the temporary folder of Setup, not on its command line.

#ifndef AppVersion
  #error AppVersion is missing; build the installer with dev\build-installer.ps1
#endif

#define Root "..\.."

[Setup]
; identifies the program for updates and uninstalling; never change it
AppId={{6A0F3C52-8E4B-4D1A-9C37-2B5E91D7F4A8}
AppName=opensecagent
AppVersion={#AppVersion}
AppVerName=opensecagent {#AppVersion}
AppPublisher=Thomas Cieslik
AppPublisherURL=https://github.com/tomCSLK/opensecagent
AppSupportURL=https://github.com/tomCSLK/opensecagent
VersionInfoVersion={#AppVersion}
VersionInfoDescription=opensecagent Setup
VersionInfoCopyright=Copyright (C) 2026 Thomas Cieslik, AGPL-3.0-or-later
; the agent reads the software of all users and runs as a service: always for all users
PrivilegesRequired=admin
DefaultDirName={autopf}\opensecagent
DisableProgramGroupPage=yes
DisableDirPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=yes
OutputDir={#Root}\target\installer
; the name opensecmon expects (docs/agent-api.md there, "Updates of opensecagent")
OutputBaseFilename=opensecagent-{#AppVersion}-x64-setup
SetupIconFile=opensecagent.ico
UninstallDisplayIcon={app}\opensecagent.exe
UninstallDisplayName=opensecagent
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
#ifdef Sign
; installer, uninstaller and the temporary copies Setup starts are signed with the command dev\build-installer.ps1
; passes as /Sopensecagent=...
SignTool=opensecagent
SignedUninstaller=yes
#endif

[Languages]
Name: "de"; MessagesFile: "compiler:Languages\German.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
de.EnrollName=Bei opensecmon anmelden
en.EnrollName=Enroll with opensecmon
de.EnrollComment=Diesen Rechner mit einem Anmeldecode aus opensecmon anmelden
en.EnrollComment=Enroll this computer with an enrollment code from opensecmon
de.SettingUpService=Der Dienst opensecagent wird eingerichtet ...
en.SettingUpService=Setting up the service opensecagent ...
de.ServiceFailed=Der Dienst opensecagent konnte nicht eingerichtet werden:%n%n%1
en.ServiceFailed=The service opensecagent could not be set up:%n%n%1
de.Enrolling=Anmeldung bei opensecmon ...
en.Enrolling=Enrolling with opensecmon ...
de.EnrollFailed=Die Anmeldung mit dem Code aus /CODE hat nicht geklappt:%n%n%1%n%nMelden Sie den Rechner später mit „Bei opensecmon anmelden“ im Startmenü an.
en.EnrollFailed=Enrolling with the code from /CODE did not work:%n%n%1%n%nEnroll the computer later with "Enroll with opensecmon" in the start menu.

[Files]
Source: "{#Root}\target\release\opensecagent.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\target\release\opensecagent-enroll.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "{#Root}\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{cm:EnrollName}"; Filename: "{app}\opensecagent-enroll.exe"; Comment: "{cm:EnrollComment}"

[UninstallRun]
; stops and removes the service before its program is deleted
Filename: "{app}\opensecagent.exe"; Parameters: "service-uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveService"

[UninstallDelete]
; the enrollment (token), the state and the log
Type: filesandordirs; Name: "{commonappdata}\opensecagent"

[Code]
const
  ServiceName = 'opensecagent';

// An update: stops the service, so its program can be replaced (net stop waits until it has stopped). Without the
// service, net stop only fails.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\net.exe'), 'stop ' + ServiceName, '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;

function Lines(const Text: TArrayOfString): String;
var
  I: Integer;
begin
  Result := '';
  for I := 0 to GetArrayLength(Text) - 1 do
    Result := Result + Text[I] + #13#10;
  Result := Trim(Result);
end;

// Runs opensecagent.exe without a window; its output on standard error, or an empty string when it worked.
function Agent(const Params: String): String;
var
  ResultCode: Integer;
  Output: TExecOutput;
begin
  Log('opensecagent.exe ' + Params);
  if not ExecAndCaptureOutput(ExpandConstant('{app}\opensecagent.exe'), Params, '', SW_HIDE,
      ewWaitUntilTerminated, ResultCode, Output) then
    Result := SysErrorMessage(ResultCode)
  else if ResultCode <> 0 then begin
    Result := Lines(Output.StdErr);
    if Result = '' then
      Result := Lines(Output.StdOut);
    if Result = '' then
      Result := 'exit code ' + IntToStr(ResultCode);
  end else
    Result := '';
  if Result <> '' then
    Log('failed: ' + Result);
end;

function IsEnrolled: Boolean;
begin
  Result := FileExists(ExpandConstant('{commonappdata}\opensecagent\enrollment.json'));
end;

// The enrollment code from /CODE=..., through a file in the temporary folder of Setup that the program deletes.
procedure EnrollWithCode(const Code: String);
var
  CodeFile, Error: String;
begin
  WizardForm.StatusLabel.Caption := CustomMessage('Enrolling');
  CodeFile := ExpandConstant('{tmp}\enrollment-code.txt');
  if not SaveStringToFile(CodeFile, Code, False) then
    Error := 'the code could not be handed to the program'
  else
    Error := Agent('enroll --code-file "' + CodeFile + '"');
  DeleteFile(CodeFile);
  if Error <> '' then
    SuppressibleMsgBox(FmtMessage(CustomMessage('EnrollFailed'), [Error]), mbError, MB_OK, IDOK);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Code, Error: String;
  ResultCode: Integer;
begin
  if CurStep <> ssPostInstall then
    Exit;
  // also after an update: the path or the settings of the service may have changed; starts it again
  WizardForm.StatusLabel.Caption := CustomMessage('SettingUpService');
  Error := Agent('service-install');
  if Error <> '' then
    SuppressibleMsgBox(FmtMessage(CustomMessage('ServiceFailed'), [Error]), mbError, MB_OK, IDOK);
  Code := Trim(ExpandConstant('{param:CODE|}'));
  if Code <> '' then
    EnrollWithCode(Code)
  else if not WizardSilent and not IsEnrolled then
    // the dialog of the program; Setup has administrator rights already, so there is no second question of UAC
    Exec(ExpandConstant('{app}\opensecagent-enroll.exe'), '', '', SW_SHOWNORMAL, ewWaitUntilTerminated, ResultCode);
end;
