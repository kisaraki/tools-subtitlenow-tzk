#ifndef AppVersion
  #error AppVersion must come from Cargo.toml
#endif
#ifndef ProductId
  #define ProductId "SubtitleNow.kisaraki"
#endif
#ifndef ProductName
  #define ProductName "SubtitleNow"
#endif
#ifndef SourceDir
  #define SourceDir ".."
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif
#ifndef BinaryDir
  #define BinaryDir SourceDir + "\target\x86_64-pc-windows-msvc\release"
#endif

[Setup]
AppId={#ProductId}
AppName={#ProductName}
AppVersion={#AppVersion}
AppPublisher=kisaraki
AppPublisherURL=https://kisaraki.github.io/tools-subtitlenow-tzk/
AppSupportURL=https://github.com/kisaraki/tools-subtitlenow-tzk/issues
AppUpdatesURL=https://github.com/kisaraki/tools-subtitlenow-tzk/releases/latest
DefaultDirName={localappdata}\Programs\{#ProductName}
DefaultGroupName={#ProductName}
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\subtitlenow-desktop.exe
VersionInfoVersion={#AppVersion}
CloseApplications=yes
RestartApplications=no
DisableProgramGroupPage=yes
SetupMutex={#ProductId}.Setup
AppMutex=Local\SubtitleNow.Running

[Languages]
Name: "zhTW"; MessagesFile: "ChineseTraditional.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "建立桌面捷徑"; Flags: unchecked

[Files]
Source: "{#BinaryDir}\subtitlenow-desktop.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinaryDir}\subtitlenow.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceDir}\target\package-licenses\*"; DestDir: "{app}\licenses\packages"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#ProductName}"; Filename: "{app}\subtitlenow-desktop.exe"
Name: "{autodesktop}\{#ProductName}"; Filename: "{app}\subtitlenow-desktop.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\subtitlenow-desktop.exe"; Description: "開啟 SubtitleNow"; Flags: nowait postinstall skipifsilent

[Code]
const
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#ProductId}_is1';
var
  RemovedPrevious: Boolean;

function Part(var Version: String): Integer;
var P: Integer; S: String;
begin
  P := Pos('.', Version);
  if P = 0 then begin S := Version; Version := ''; end
  else begin S := Copy(Version, 1, P - 1); Delete(Version, 1, P); end;
  Result := StrToIntDef(S, 0);
end;

function IsNewer(OldVersion, NewVersion: String): Boolean;
var I, A, B: Integer;
begin
  Result := False;
  for I := 1 to 4 do begin
    A := Part(OldVersion); B := Part(NewVersion);
    if A <> B then begin Result := A > B; Exit; end;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var Command, InstallDir, OldVersion: String; Code: Integer;
begin
  Result := '';
  if RemovedPrevious then Exit;
  if CheckForMutexes('Local\SubtitleNow.Running') then begin
    Result := '請先關閉 SubtitleNow，再按「重試」繼續安裝。'; Exit;
  end;
  if RegKeyExists(HKLM64, UninstallKey) or RegKeyExists(HKLM32, UninstallKey) then begin
    Result := '偵測到所有使用者的舊版，請先從 Windows「已安裝的應用程式」解除安裝。'; Exit;
  end;
  if not RegKeyExists(HKCU, UninstallKey) then Exit;
  if not RegQueryStringValue(HKCU, UninstallKey, 'DisplayVersion', OldVersion) then begin
    Result := '無法讀取舊版版本，已停止安裝。'; Exit;
  end;
  if IsNewer(OldVersion, '{#AppVersion}') then begin
    Result := '已安裝較新版本，不能使用此安裝程式降級。'; Exit;
  end;
  if not RegQueryStringValue(HKCU, UninstallKey, 'UninstallString', Command) or
     not RegQueryStringValue(HKCU, UninstallKey, 'InstallLocation', InstallDir) then begin
    Result := '找不到舊版解除安裝資訊，已停止安裝。'; Exit;
  end;
  Command := RemoveQuotes(Command);
  if (CompareText(RemoveBackslash(ExtractFileDir(Command)), RemoveBackslash(InstallDir)) <> 0) or
     (Pos('unins', Lowercase(ExtractFileName(Command))) <> 1) or
     (CompareText(ExtractFileExt(Command), '.exe') <> 0) or not FileExists(Command) then begin
    Result := '舊版解除安裝程式無效，已停止安裝。'; Exit;
  end;
  Log('Removing registered previous version ' + OldVersion);
  if not Exec(Command, '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART', '', SW_HIDE, ewWaitUntilTerminated, Code) then begin
    Result := '無法啟動舊版解除安裝程式，已停止安裝。'; Exit;
  end;
  if (Code <> 0) or RegKeyExists(HKCU, UninstallKey) then begin
    Result := '舊版尚未成功移除，已停止安裝；請重新啟動 Windows 後再試。'; Exit;
  end;
  RemovedPrevious := True;
  Log('Previous version removed successfully; installing new version.');
end;
