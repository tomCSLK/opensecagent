; Installer of opensecagent for Windows (Inno Setup 6, https://jrsoftware.org/isinfo.php).
; Built by dev\build-installer.ps1, which builds the program first and passes the version from Cargo.toml.
;
; First stage (PLAN.md, Phase 3, step 3e): installs the program for all users and is signed, to try the code
; signing with Smart App Control. The Windows service (3c), the page for the enrollment code, removing the old
; script (3d) and a start menu entry for enrolling again follow with the next steps.

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

[Files]
Source: "{#Root}\target\release\opensecagent.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "{#Root}\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\README.md"; DestDir: "{app}"; Flags: ignoreversion
