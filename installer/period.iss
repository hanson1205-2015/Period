#define MyAppName "Period"
#define MyAppVersion "1.0.4"
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
Source: "..\dist\period-core.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\stdlib\*"; DestDir: "{app}\stdlib"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\dist\tcc\*"; DestDir: "{app}\tcc"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\assets\period.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\vscode-extension\period-language-{#MyAppVersion}.vsix"; DestDir: "{tmp}"; Flags: ignoreversion
Source: "..\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\examples\*"; DestDir: "{app}\examples"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Period REPL"; Filename: "{cmd}"; Parameters: "/k ""{app}\period.exe"""; IconFilename: "{app}\period.ico"

[Run]
Filename: "{cmd}"; Parameters: "/c ""{app}\period.exe"" --version"; Description: "Verify installation"; Flags: nowait runhidden
Filename: "{cmd}"; Parameters: "/c code --uninstall-extension ""period.period-language"""; Description: "Remove old VS Code extension"; Flags: runhidden
Filename: "{cmd}"; Parameters: "/c code --install-extension ""{tmp}\period-language-{#MyAppVersion}.vsix"""; Description: "Install VS Code extension"; Flags: runhidden

[Registry]
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath('{app}')
Root: HKLM; Subkey: "Software\Classes\.period"; ValueType: string; ValueName: ""; ValueData: "PeriodFile"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\PeriodFile"; ValueType: string; ValueName: ""; ValueData: "Period Source File"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\PeriodFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\period.ico"
Root: HKLM; Subkey: "Software\Classes\PeriodFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\period.exe"" ""%1"""
Root: HKLM; Subkey: "Software\Classes\.periodi"; ValueType: string; ValueName: ""; ValueData: "PeriodInterfaceFile"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\PeriodInterfaceFile"; ValueType: string; ValueName: ""; ValueData: "Period Interface File"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\PeriodInterfaceFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\period.ico"
Root: HKLM; Subkey: "Software\Classes\PeriodInterfaceFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\period.exe"" ""%1"""

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
