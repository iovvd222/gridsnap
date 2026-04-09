; GridSnap InnoSetup Script
; ビルド手順:
;   1. cargo build --release
;   2. iscc installer/gridsnap.iss
;   → Output/ に GridSnapSetup.exe が生成される

#define MyAppName      "GridSnap"
#define MyAppVersion   "0.1.0"
#define MyAppPublisher "Fujino Kosei"
#define MyAppExeName   "gridsnap.exe"
#define MyAppURL       "https://github.com/your-repo/gridsnap"

[Setup]
AppId={{B7E2F9A1-3C4D-4E5F-8A6B-7C8D9E0F1A2B}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
OutputDir=..\Output
OutputBaseFilename=GridSnapSetup
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=lowest
; PrivilegesRequired=lowest → HKCU にインストール。管理者権限不要。
; Program Files ではなく %LOCALAPPDATA%\Programs\GridSnap に入る。
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppExeName}
SetupIconFile=..\assets\gridsnap.ico
; ↑ アイコンがなければこの行を削除またはコメントアウト

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "startup"; Description: "Windows 起動時に自動実行する"; GroupDescription: "その他:"

[Files]
; リリースビルドの exe
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
; サンプル設定ファイル（既存なら上書きしない）
Source: "..\gridsnap.toml"; DestDir: "{app}"; Flags: onlyifdoesntexist uninsneveruninstall

[Icons]
; スタートメニュー
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{#MyAppName} を削除"; Filename: "{uninstallexe}"
; デスクトップ（タスクで選択時のみ）
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; スタートアップ登録（タスクで選択時のみ）
; startup.rs と同じキーに書くため、どちらか一方が有効になる
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; \
    ValueType: string; ValueName: "GridSnap"; ValueData: """{app}\{#MyAppExeName}"""; \
    Tasks: startup; Flags: uninsdeletevalue

[Run]
; インストール完了後に起動するオプション
Filename: "{app}\{#MyAppExeName}"; Description: "{#MyAppName} を起動"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; アンインストール前にプロセスを終了させる
Filename: "taskkill.exe"; Parameters: "/F /IM {#MyAppExeName}"; Flags: runhidden; RunOnceId: "KillGridSnap"

[UninstallDelete]
; 実行時に生成されるファイル（ログ等）があれば掃除
Type: filesandordirs; Name: "{app}\logs"

[Code]
// インストール前に既に起動中なら終了を促す
function InitializeSetup(): Boolean;
var
  ResultCode: Integer;
begin
  Result := True;
  // taskkill で静かに終了させる（失敗しても続行）
  Exec('taskkill.exe', '/F /IM {#MyAppExeName}', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;