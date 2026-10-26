#ifndef AppVersion
  #error AppVersion must be supplied by the release workflow
#endif
#ifndef FileVersion
  #error FileVersion must be supplied by the release workflow
#endif

[Setup]
AppId={{4599F7A1-197F-4D3D-9DB1-5F1DB426B8B7}
AppName=Bytebeat Composer
AppVersion={#AppVersion}
VersionInfoVersion={#FileVersion}
AppPublisher=Bytebeat contributors
DefaultDirName={localappdata}\Programs\Bytebeat
DefaultGroupName=Bytebeat Composer
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=bytebeat-{#AppVersion}-windows-x86_64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\bytebeat.exe
CloseApplications=yes

[Files]
Source: "..\target\x86_64-pc-windows-msvc\release\bytebeat.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\DOCUMENTATION.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\VERTEXSHADERART-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Bytebeat Composer"; Filename: "{app}\bytebeat.exe"
Name: "{group}\Uninstall Bytebeat Composer"; Filename: "{uninstallexe}"
