; Inno-Setup-Skript. Aufruf: iscc /DAppVersion=0.2.0 packaging\windows\setup.iss
; Erwartet target\release\restic-backup-app.exe und optional packaging\windows\restic.exe (wird mitinstalliert).
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6B1F7C2E-3D44-4C0B-9A57-0A1B2C3D4E5F}
AppName=restic Backup
AppVersion={#AppVersion}
AppPublisher=Cyberhuter
DefaultDirName={autopf}\restic-backup-app
DefaultGroupName=restic Backup
UninstallDisplayIcon={app}\restic-backup-app.exe
SetupIconFile=icon.ico
OutputDir=..\..\target\installer
OutputBaseFilename=restic-backup-app-setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequiredOverridesAllowed=dialog
WizardStyle=modern

[Languages]
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[Tasks]
Name: "desktopicon"; Description: "Desktop-Verknüpfung erstellen"; Flags: unchecked

[Files]
Source: "..\..\target\release\restic-backup-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "restic.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\restic Backup"; Filename: "{app}\restic-backup-app.exe"
Name: "{group}\restic Backup deinstallieren"; Filename: "{uninstallexe}"
Name: "{autodesktop}\restic Backup"; Filename: "{app}\restic-backup-app.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\restic-backup-app.exe"; Description: "restic Backup starten"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "schtasks"; Parameters: "/delete /f /tn restic-backup-app"; Flags: runhidden; RunOnceId: "RemoveSchedule"
