; Seamtype for Windows: the installer (Inno Setup 6).
;
;   powershell -ExecutionPolicy Bypass -File windows\installer\build.ps1
;
; builds the text service (x64 and x86) and this into windows\target\installer\Seamtype-Setup-<version>.exe.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Target
  #define Target "..\target"
#endif

[Setup]
AppId={{6C3F3C5E-2B9A-4E57-9D7B-5A3E1B0C8F21}
AppName=Seamtype
AppVersion={#Version}
AppVerName=Seamtype {#Version}
AppPublisher=HaloKim
AppPublisherURL=https://github.com/HaloKim/seamtype-rust
AppSupportURL=https://github.com/HaloKim/seamtype-rust/issues
DefaultDirName={autopf}\Seamtype
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#Target}\installer
OutputBaseFilename=Seamtype-Setup-{#Version}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
LicenseFile=..\..\LICENSE
UninstallDisplayName=Seamtype
; The text service is loaded by every app: closing them isn't asked for. A DLL in use is renamed
; (apps keep the copy they loaded) and the new one takes its place; apps pick it up when restarted.
CloseApplications=no
RestartIfNeededByRun=no

[Languages]
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
korean.DefaultTask=Seamtype을 기본 입력기로 지정 (재부팅해도 Seamtype으로 시작)
english.DefaultTask=Make Seamtype the default input method (Windows starts with it after a restart)
korean.Done=설치했습니다. Win+Space로 한국어 > Seamtype을 고르세요. 이미 실행 중인 앱은 다시 시작해야 새 버전을 씁니다.
english.Done=Installed. Press Win+Space and choose Korean > Seamtype. Apps already running use the new version once restarted.

[Tasks]
Name: "default"; Description: "{cm:DefaultTask}"

[Files]
Source: "{#Target}\release\seamtype.dll"; DestDir: "{app}"; Flags: ignoreversion regserver 64bit uninsrestartdelete
Source: "{#Target}\i686-pc-windows-msvc\release\seamtype.dll"; DestDir: "{app}\x86"; Flags: ignoreversion regserver 32bit uninsrestartdelete
Source: "keyboard.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\ThirdPartyNotices.txt"; DestDir: "{app}"; Flags: ignoreversion

[Run]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\keyboard.ps1"" -Add -Default"; Flags: runasoriginaluser runhidden waituntilterminated; Tasks: default
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\keyboard.ps1"" -Add"; Flags: runasoriginaluser runhidden waituntilterminated; Tasks: not default

[UninstallRun]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\keyboard.ps1"" -Remove"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveKeyboard"

[UninstallDelete]
Type: files; Name: "{app}\*.old"
Type: files; Name: "{app}\x86\*.old"

[Code]
{ A loaded DLL can't be replaced but can be renamed: apps keep the copy they loaded. }
procedure SetAside(const Path: String);
begin
  if FileExists(Path) and not DeleteFile(Path) then
    RenameFile(Path, Path + '.' + GetDateTimeString('yyyymmddhhnnss', #0, #0) + '.old');
end;

procedure DeleteOld(const Dir: String);
var
  Found: TFindRec;
begin
  if FindFirst(Dir + '\*.old', Found) then
  try
    repeat
      DeleteFile(Dir + '\' + Found.Name);
    until not FindNext(Found);
  finally
    FindClose(Found);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
  begin
    DeleteOld(ExpandConstant('{app}'));
    DeleteOld(ExpandConstant('{app}\x86'));
    SetAside(ExpandConstant('{app}\seamtype.dll'));
    SetAside(ExpandConstant('{app}\x86\seamtype.dll'));
  end;
  if CurStep = ssDone then
    MsgBox(CustomMessage('Done'), mbInformation, MB_OK);
end;
