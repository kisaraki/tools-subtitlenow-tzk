# 安裝、更新與發佈

作者：kisaraki。程式版本唯一來源：`Cargo.toml`。

## 安裝與使用

從 [GitHub Releases](https://github.com/kisaraki/tools-subtitlenow-tzk/releases/latest) 下載 `setup.exe`。Windows 10／11 x64，擷取支援依 Windows 實際能力偵測。執行安裝精靈，預設放在目前使用者的 `%LOCALAPPDATA%\Programs\SubtitleNow`，不需系統管理員權限。開始功能表啟動 `subtitlenow-desktop.exe`，不會自動開始擷取。`subtitlenow.exe` 保留命令列使用方式。

同一產品先前透過此安裝程式安裝的版本會被辨識，先解除安裝才安裝新版；解除安裝失敗停止，較新版本不能降級。執行中的程式必須先關閉。既有無登錄的 portable 版本保留原位置，不擅自刪除。設定位於 `%LOCALAPPDATA%\SubtitleNow\config.json`，金鑰在 Windows 認證管理員；解除安裝或升級均不刪除。

目前沒有 Authenticode 簽章，Windows 可能提示未識別發行者。可使用 `Get-FileHash .\setup.exe -Algorithm SHA256`，與同個 Release 的 `SHA256SUMS.txt` 核對。

## 線上更新

開啟「關於」，按「檢查更新」。有新正式版時按「下載並安裝」並確認關閉程式的提示。下載與 SHA-256 驗證成功才啟動安裝；程式正常停止後端、關閉，安裝精靈接手。沒有新版本、連線失敗或檔案不符均明示，不假裝成功。下載途中關閉關於視窗可取消。

只傳送固定 GitHub 版本要求與程式 User-Agent，不帶音訊、字幕、來源名稱或 API 金鑰。不定時自動檢查，不傳至 AI 服務。未執行的下載會刪除；已交給安裝程式的暫存封包在下次桌面啟動／檢查時，滿一天後清理，避免刪除正在執行的安裝來源。

## 建置

```powershell
winget install --id JRSoftware.InnoSetup --exact
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked
./scripts/build-installer.ps1
```

輸出：`dist/setup.exe`、`dist/SHA256SUMS.txt`，不加入 Git。安裝精靈繁中語言檔來自 [Inno Setup 官方原始碼](https://github.com/jrsoftware/issrc/blob/main/Files/Languages/ChineseTraditional.isl)，保留翻譯作者註記。

## GitHub 發佈

1. 更新 Cargo 版本與鎖定檔、網站版本，完成上述檢查與安裝生命週期驗證。
2. 提交並推送 `main`。Pages 工作流程只發佈 `site/`。
3. 推送相同版本的 `vX.Y.Z` tag；Windows setup workflow 產生 draft Release 與封包。不要覆蓋已公開封包。
4. 核對版本、安裝／升級／解除安裝與 SHA-256，再公開 Release。正式版須同時包含 `setup.exe` 與 `SHA256SUMS.txt`；GitHub API 的資產 digest 是程式內更新的驗證依據。

真實雲端辨識品質與既有應用程式相容性不因封包發佈而擴大宣稱；歷史驗收範圍見 Phase 11 結案紀錄。
