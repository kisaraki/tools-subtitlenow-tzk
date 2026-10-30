# ADR 0111 — 關於、明確線上更新與 Windows 安裝程式

日期：2026-10-30。依使用者要求採用。

## 原因

既有程式無安裝／更新入口，且 Gemini 無音訊診斷已完成其歷史用途。發佈時需要可重現封包與固定產品身分，同時維持原本的擷取、隱私及後端隔離。

## 決定

刪除診斷按鈕、控制器命令、專用 UI 狀態、模型查詢與初始化模組及僅為其存在的測試。保留一般安全錯誤資訊與效能計數，避免改變辨識／翻譯的可靠性。

版本來自 Cargo；作者為 kisaraki。「關於」不會自動連線。單一可取消工作在獨立執行緒的 Tokio runtime 執行網路 I/O，UI 以計時器收取一個有界完成結果。關閉視窗取消並加入工作。檢查版本只接受正式 `vMAJOR.MINOR.PATCH`，不降級、不採用預覽版本。安裝檔只能是指定倉庫該版的 `setup.exe`，限制 128 MiB，版本資料限制 256 KiB；檢查上限 20 秒、下載 180 秒，重新導向只容許 HTTPS GitHub 發佈資產主機。檔案寫入只在更新執行緒；取消時先關閉檔案再清理。

使用 [GitHub Release API](https://docs.github.com/en/rest/releases/releases#get-the-latest-release) 的 SHA-256 digest 驗證內容與長度；缺少 digest 拒絕安裝。這信任指定 GitHub 倉庫與 HTTPS，並非 Windows Authenticode 簽章。尚無簽章憑證，不宣稱「已簽署」。下載成功後使用直接程序啟動，無 shell、遠端引數或任意執行檔路徑。

原生 GUI launcher 共用相同 app/UI 與 capture-helper 路徑；CLI 保持原功能。Setup 使用固定 AppId 與目前使用者範圍，依登錄解除安裝資訊辨識舊版。[PrepareToInstall](https://jrsoftware.org/ishelp/topic_scriptevents.htm) 先拒絕降級／執行中／無效路徑，再等待舊版解除安裝成功與登錄移除，才安裝新版。獨立 AppId 的安裝測試不接觸正式安裝。已安裝版之後可自動辨識；無登錄的舊 portable 複本不會被掃描或刪除。

成功啟動安裝後，GUI 走正常關閉與後端取消流程。安裝程式來源檔須跨越程式退出，因此以 OS 暫存獨有目錄暫留；下次桌面啟動／更新檢查只清除超過一天、無連結且僅含 setup.exe 的自有目錄。未啟動的下載立即清理。不保存媒體資料。

GitHub Pages 使用 `site/` 靜態檔與官方 Pages Actions。版本標籤的工作流程執行檢查並產生 draft Release，維護者完成封包驗證後再公開。既有 Release 不覆蓋，以維持下載與 digest 一致。

## 限制

網路中斷、API 限流、沒有正式 Release、磁碟／驗證／啟動錯誤會顯示明確訊息。安裝精靈取消不代表已更新；解除安裝後的安裝失敗需重新執行已下載的 setup.exe。此版不提供自動回復舊程式、背景定時更新或跨 Windows 使用者／機器範圍遷移。從所有使用者安裝範圍遷移須先由管理者解除安裝。網站不使用追蹤或分析服務。
