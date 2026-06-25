#define MyAppName "Period"
#define MyAppVersion "1.0.2"
#define MyAppPublisher "Period Language"
#define MyAppURL "https://exploremaths.github.io/Period/"

[Setup]
AppId={{PERIOD-LANG-1234-5678-90AB-CDEF12345678}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\Period
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=Period-Setup
SetupIconFile=..\assets\period.ico
Compression=lzma
SolidCompression=yes
WizardStyle=modern
ChangesEnvironment=yes
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "..\dist\period.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\period.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\vscode-extension\period-language.vsix"; DestDir: "{tmp}"; Flags: ignoreversion
Source: "..\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\examples\*"; DestDir: "{app}\examples"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Period REPL"; Filename: "{cmd}"; Parameters: "/k ""{app}\period.exe"""; IconFilename: "{app}\period.ico"

[Run]
Filename: "{cmd}"; Parameters: "/c ""{app}\period.exe"" --version"; Description: "Verify installation"; Flags: nowait runhidden
Filename: "{cmd}"; Parameters: "/c code --install-extension ""{tmp}\period-language.vsix"""; Description: "Install VS Code extension"; Flags: nowait runhidden

[Registry]
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath('{app}')
Root: HKLM; Subkey: "Software\Classes\.period"; ValueType: string; ValueName: ""; ValueData: "PeriodFile"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\PeriodFile"; ValueType: string; ValueName: ""; ValueData: "Period Source File"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\PeriodFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\period.ico"
Root: HKLM; Subkey: "Software\Classes\PeriodFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\period.exe"" ""%1"""

[Code]
function NeedsAddPath(Param: string): boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', OrigPath) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Param + ';', ';' + OrigPath + ';') = 0;
end;
