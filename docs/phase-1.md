# Phase 1 — Windows 音訊來源探索

本文記錄 Phase 1 完成時的範圍與驗收，依 `SPEC.md` 第 9、75 節提供真實 Windows Core Audio 探索。後續來源選取與擷取的目前狀態見 [Phase 2](phase-2.md)；Phase 3／4 的 AI、Phase 5 的視窗介面尚未加入。

## 操作

```powershell
cargo run --locked -- --list-sources
cargo run --locked -- --watch-sources
```

單次列舉完成後結束；持續探索按 Ctrl+C 停止。沒有音訊裝置／工作階段時顯示空列表。裝置或音訊服務錯誤會顯示明確錯誤；持續探索在下一輪重試，失敗快照不帶上一輪的來源。部分裝置／工作階段失效時仍提供可用來源並標示資料不完整。

## 資料與邊界

- 探索所有啟用中的 render endpoint，不探索麥克風；不呼叫擷取 API。
- 身分使用 endpoint ID + session instance ID。同一 PID 在不同裝置或多個工作階段上不會被合併，HWND 不是來源身分。
- `GetProcessId` 保留 `AUDCLNT_S_NO_SINGLE_PROCESS`，跨程序工作階段的 PID 僅代表初始建立者。系統音效有獨立標記。
- 程式名稱優先使用工作階段顯示名稱，再取程序執行檔名稱。受保護／已結束的程序可能無法查詢。
- 視窗標題只來自相同 PID 的可見頂層視窗，最多 512 個 UTF-16 單位。多視窗取第一個符合者；不以視窗推定音訊身分。
- 圖示優先使用工作階段的圖示位置，再以執行檔圖示補足。取得後轉為 32×32、預乘 alpha 的 BGRA 像素；不將 HICON／GDI handle 交給顯示端。UNC、網路磁碟、URL 與相對圖示路徑不會直接交給圖示擷取 API。
- `Active` 表示串流正在執行，訊號提示以峰值 > 0.001（約 -60 dBFS）判斷。此提示不是語音偵測；硬體、靜音或端點音量仍可能影響可聽見的聲音。無法取得峰值／音量／靜音時保留未知值。

## 執行緒、上限與釋放

單一專用執行緒初始化 MTA，持有並釋放 COM、程序、GDI 和圖示資源。scanner 本身不可跨執行緒移動；`AudioSessionScanner` 不要求 `Send`，以 worker 內的 factory 建立。顯示端只取得自有 Rust 資料。

`IAudioSessionManager2` 建立通知經 `AgileReference` 傳送新工作階段，使用 256 格有界通道；滿載或無法建立參照時標示通知遺失，後續列舉協助重新核對。通知不做圖示讀取、畫面更新或日誌輸出。250 ms 的輪詢間隔用於裝置核對、活動與音量更新；每輪耗時另計，不宣稱固定刷新率。

最多追蹤 32 個播放裝置、256 個工作階段，每輪最多檢查 1,024 個頂層視窗，超過時標示上限。原生字串讀取有 32,768 UTF-16 單位上限。過期工作階段不重新加入集合。

快照用 Tokio watch 通道，只保留最新一筆；慢速終端機可能跳過中間狀態，但不會累積舊快照。每筆含探索 run ID、遞增序號與單調時鐘的經過時間，重新啟動探索有新 run ID。這些身分與未來 Live／Prepared Session ID 分開。

停止會設取消旗標、喚醒等待中的 worker 並 join；掃描迴圈在裝置與工作階段邊界檢查取消。原生 API 呼叫本身不能強制中斷，停止需等待當下呼叫返回。COM 通知先取消註冊，所有 COM 介面與待處理參照釋放後才 `CoUninitialize`。worker 的 Drop 也會停止並回收執行緒。

所有探索資料、圖示、程序與視窗資訊都留在 RAM。來源清單只在使用者明確要求的 stdout 輸出，結構化日誌不列出來源名稱、視窗標題、圖示路徑或工作階段 ID。沒有音訊內容、雲端請求、內容快取或新增設定檔欄位。

## 驗證

一般 `cargo test` 不需要音訊服務或 API 金鑰，包含替代 scanner 測試：慢速接收、長輪詢等待中停止、Drop 回收、重新啟動身分、初始化失敗、掃描失敗／復原與 worker panic。Windows 圖示測試從系統 Explorer 資源取得圖示。

另外的本機實機測試以兩個獨立子程序建立靜音 WASAPI render stream，驗證初始列舉、後續新增、PID、活動／閒置、零峰值與程序退出後移除。採獨立程序是因為同程序的 WASAPI 控制參照可能延長測試 render client 的存活時間，且外部程序更符合產品使用方式。

```powershell
cargo test --locked native_session_appears_updates_and_disappears -- --ignored --nocapture
```

2026-10-05：本機 `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`、`cargo build` 通過。一般測試共 34 項通過、1 項實機測試預設略過；另外明確執行該實機生命週期測試並通過。命令列單次列舉觀察到 2 個播放裝置、3 個工作階段，未標示部分資料錯誤。持續探索也在終端機實測更新及 Ctrl+C，確認 worker join 後輸出停止訊息，程式以結束碼 0 離開。此結果只描述當次本機環境。

仍需人工驗證的項目：

| 項目 | 狀態 |
| --- | --- |
| VLC、PotPlayer、Chrome、Edge、Spotify、Media Player、至少一款遊戲 | 尚未逐項驗證 |
| 拔除／切換播放裝置、音訊服務重啟 | 尚未實機驗證；掃描錯誤／復原以替代 scanner 測試 |
| 過量原生建立通知、32 裝置／256 工作階段極限 | 有界設計與可見上限狀態；尚未實機壓力測試 |
| 長時間 COM／HANDLE／記憶體成長 | 尚未長時間量測 |

## 已知限制

瀏覽器音訊 helper 程序可能沒有直接所屬視窗，標題會顯示未知；不推定父程序視窗、個別分頁或擷取範圍。受保護程序、UWP 圖示、間接資源名稱與某些圖示路徑可能無法解析。未知資料不影響其他來源列舉。

峰值為 Core Audio 回報的採樣值，輪詢可能錯過短暫聲音。專屬模式或未支援計量的來源不保證能取得音量。來源在兩個快照間迅速出現又消失時，慢速顯示端可能看不到該短暫來源。

相關 API 行為依 [Microsoft 工作階段列舉文件](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nn-audiopolicy-iaudiosessionenumerator)、[程序識別文件](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-getprocessid) 與 [工作階段狀態文件](https://learn.microsoft.com/en-us/windows/win32/api/audiosessiontypes/ne-audiosessiontypes-audiosessionstate)。
