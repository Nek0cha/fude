; fude の Windows のインストーラー（Inno Setup 6。10b の設計書 §5.5）。
; scripts/build-installer.ps1 から、/DAppVersion=<版> /DFudeExe=<fude.exe のパス>（署名するときは /DSign と /Sfudesign=...）を付けて
; ISCC で作る。ユーザーごとに %LOCALAPPDATA%\Programs\fude へ入れ、管理者の権限は求めない。

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef FudeExe
  #define FudeExe "..\target\release\fude.exe"
#endif

[Setup]
; AppId は以後変えない（変えると上書きのインストールにならない）。
AppId={{AFDD4770-E5CF-4D3A-B07E-62AA9A48DD44}
AppName=fude
AppVersion={#AppVersion}
AppVerName=fude {#AppVersion}
AppPublisher=Nek0cha
AppPublisherURL=https://github.com/Nek0cha/fude
AppSupportURL=https://github.com/Nek0cha/fude/issues
DefaultDirName={localappdata}\Programs\fude
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=fude-{#AppVersion}-setup
SetupIconFile=..\assets\icon\fude.ico
UninstallDisplayIcon={app}\fude.exe
UninstallDisplayName=fude
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ShowLanguageDialog=no
ChangesEnvironment=yes
; インストールと上書きのとき、動いている fude を閉じてよいかを聞く。
CloseApplications=yes
RestartApplications=no
#ifdef Sign
; scripts/sign.ps1 で、アンインストーラーとインストーラーを署名する。
SignTool=fudesign
SignedUninstaller=yes
#endif

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
japanese.CloseFude=fude が起動しています。fude を閉じてから「再試行」を押してください。
english.CloseFude=fude is running. Close fude, then click Retry.
japanese.CleaningUp=Claude Code の hook と WSL の fude を片付けています…
english.CleaningUp=Removing the Claude Code hook and fude in WSL...

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#FudeExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\NOTICE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\fude"; Filename: "{app}\fude.exe"; AppUserModelID: "Nek0cha.fude"
Name: "{userdesktop}\fude"; Filename: "{app}\fude.exe"; AppUserModelID: "Nek0cha.fude"; Tasks: desktopicon

[Run]
Filename: "{app}\fude.exe"; Description: "{cm:LaunchProgram,fude}"; Flags: postinstall nowait skipifsilent

[Code]
const
  EnvKey = 'Environment';

{ PATH の 1 項目を比べるための形（末尾の \ を除き、小文字にする）。 }
function NormPath(S: String): String;
begin
  S := Trim(S);
  while (Length(S) > 0) and (S[Length(S)] = '\') do
    S := Copy(S, 1, Length(S) - 1);
  Result := Lowercase(S);
end;

{ ユーザーの PATH に Dir が（大文字小文字と末尾の \ を無視して）あるか。 }
function PathHas(Path, Dir: String): Boolean;
var
  Rest, Item: String;
  P: Integer;
begin
  Result := False;
  Rest := Path;
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    if P = 0 then
    begin
      Item := Rest;
      Rest := '';
    end else begin
      Item := Copy(Rest, 1, P - 1);
      Rest := Copy(Rest, P + 1, Length(Rest));
    end;
    if NormPath(Item) = NormPath(Dir) then
    begin
      Result := True;
      Exit;
    end;
  end;
end;

{ Path から Dir と同じ項目だけを取り除く。ほかの項目と並びはそのまま。 }
function PathWithout(Path, Dir: String): String;
var
  Rest, Item: String;
  P: Integer;
begin
  Result := '';
  Rest := Path;
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    if P = 0 then
    begin
      Item := Rest;
      Rest := '';
    end else begin
      Item := Copy(Rest, 1, P - 1);
      Rest := Copy(Rest, P + 1, Length(Rest));
    end;
    if (Item <> '') and (NormPath(Item) <> NormPath(Dir)) then
    begin
      if Result <> '' then
        Result := Result + ';';
      Result := Result + Item;
    end;
  end;
end;

procedure AddAppToPath();
var
  Path, Dir: String;
begin
  Dir := ExpandConstant('{app}');
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Path) then
    Path := '';
  if PathHas(Path, Dir) then
    Exit;
  if (Path <> '') and (Path[Length(Path)] <> ';') then
    Path := Path + ';';
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Path + Dir);
end;

procedure RemoveAppFromPath();
var
  Path, Dir: String;
begin
  Dir := ExpandConstant('{app}');
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Path) then
    Exit;
  if PathHas(Path, Dir) then
    RegWriteExpandStringValue(HKCU, EnvKey, 'Path', PathWithout(Path, Dir));
end;

// インストール先の fude.exe が動いているか（WMI で、実行ファイルのパスを比べる）。調べられなければ False。
function FudeRunning(): Boolean;
var
  Locator, Service, Procs, Proc: Variant;
  I: Integer;
  Exe, Found: String;
begin
  Result := False;
  Exe := Lowercase(ExpandConstant('{app}\fude.exe'));
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Service := Locator.ConnectServer('.', 'root\CIMV2');
    Procs := Service.ExecQuery('SELECT ExecutablePath FROM Win32_Process WHERE Name = ''fude.exe''');
    for I := 0 to Procs.Count - 1 do
    begin
      Proc := Procs.ItemIndex(I);
      if not VarIsNull(Proc.ExecutablePath) then
      begin
        Found := Proc.ExecutablePath;
        if Lowercase(Found) = Exe then
        begin
          Result := True;
          Exit;
        end;
      end;
    end;
  except
    Result := False;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    AddAppToPath();
end;

function InitializeUninstall(): Boolean;
begin
  Result := True;
  while FudeRunning() do
  begin
    if SuppressibleMsgBox(CustomMessage('CloseFude'), mbError, MB_RETRYCANCEL, IDCANCEL) = IDCANCEL then
    begin
      Result := False;
      Exit;
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Code: Integer;
begin
  if CurUninstallStep = usUninstall then
  begin
    { ファイルを消す前に、hook と WSL の fude を片付ける（fude uninstall-cleanup。終了コードは常に 0）。 }
    UninstallProgressForm.StatusLabel.Caption := CustomMessage('CleaningUp');
    Exec(ExpandConstant('{app}\fude.exe'), 'uninstall-cleanup', '', SW_HIDE, ewWaitUntilTerminated, Code);
  end;
  if CurUninstallStep = usPostUninstall then
    RemoveAppFromPath();
end;
