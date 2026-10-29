# 原生字幕視窗外觀更新

本次為 Phase 11 結案後的 UI 美化，沿用既有 Win32 視窗、控制項與單一字幕時間軸，沒有開啟新 Phase 或變更辨識／翻譯流程。

## 外觀

主視窗底部新增品牌字樣「KOSMOS TOOLKIT 探真拓知酷」。與內容左緣對齊，採系統字型與現有灰藍配色；窄版自動換行，過矮視窗優先保留字幕空間並暫隱品牌列。品牌為獨立原生 STATIC，不混入字幕、狀態或搜尋資料。

- 淺灰藍背景、白色字幕區與淡藍狀態區，讓字幕、診斷及操作有明確層次。
- 「開始翻譯」與「儲存服務與偏好」使用藍色主要按鈕；次要操作為白底圓角按鈕。保留按下、停用與鍵盤焦點的辨識方式。
- 字幕與狀態框增加內距，搜尋及遮蔽金鑰欄增加左右留白；移除主頁唯讀框的舊式凹陷外框。
- 保留 Windows 系統訊息字型、既有字級、DPI 換算、窄版排列與設定頁原位置展開。沒有更改使用者的視窗偏好或字幕浮層外觀。

## 實作邊界

`src/win32/subtitle_theme.rs` 集中管理 UI 色彩、brush 與原生按鈕繪製。Theme 及其 GDI 物件只存在 UI 執行緒，主視窗與子視窗銷毀後才釋放；繪製前後儲存／還原 DC。只讀取固定按鈕標籤，不讀取金鑰或字幕本文。

開啟 Windows 高對比模式時使用系統配色，系統查詢失敗時亦採系統色。收到系統色彩／主題設定通知時重建樣式並重繪；外觀更新失敗不停止翻譯。只增加既有 windows crate 的 Accessibility API feature，沒有安裝新元件或新增外部相依套件。

控制項仍是原生 BUTTON／EDIT／COMBOBOX，保留名稱、命令 ID、鍵盤及輔助使用行為。來源、語言、品質、主操作、狀態／字幕分框、手動清除與歷史跟隨政策保持。未讀取正式憑證或啟動音訊／雲端測試。

## 外觀初版驗證

沿用原生視窗配置、DPI、捲動、設定切換、狀態視窗及隔離 GDI／USER 生命週期測試；資源循環另外涵蓋主題重建，以及所有操作按鈕的正常／按下／停用／焦點繪製，並檢查 DC 色彩與 brush 選取還原。

| 檢查 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過，零警告 |
| `cargo test --locked --offline --lib win32::subtitle_window -- --test-threads=1` | 62 通過、零失敗、1 個 child-only fixture 在父測試內另外執行；90.97 秒 |
| `cargo build --locked --offline` | 通過，55.09 秒 |
| `cargo build --locked --offline --release` | 通過，2 分 33 秒 |

第一次測試編譯發現 `EM_SETMARGINS` 匯入遺漏，已補正後重新執行上述原生測試通過。早期 check 的 GDI 回傳值警告也已修正，最後 Clippy 零警告。本次未重跑全套媒體／服務測試，不把 Phase 11 既有 2166 項結果當作本批重新執行。

視窗畫面擷取工具兩次逾時；可讀取原生輔助使用樹，但不能據此宣稱改版後實際像素外觀、高對比或多螢幕已由目視驗收。保留這項限制，不使用假的畫面預覽代替。

## 外觀初版交付

標準執行檔 `target/release/subtitlenow.exe` 已更新，9,961,984 bytes，SHA256：`54F99B975FAB20553B5D95E3D9BF2E9C2EA1EC4063FB51F80B8FA1D9A114603C`。已開啟新版 PID 7996／MainWindowHandle 10423418／Responding true；原生輔助使用樹確認來源／語言／品質、主操作、搜尋、字幕及獨立狀態控制項名稱仍存在，處於「待命，尚未選擇來源，未啟動擷取或雲端處理」。沒有讀取正式金鑰或自動開始翻譯。

原版保留於 `target/ui-polish-backup-87306229c12443d7a4abeea3f23fcdd6/subtitlenow.before.exe`，SHA256 `E3C6C5CD0A267980DC6B898742986CE91AFB384C41A7DE78678A39C20C157462`。需還原時先關閉程式，再用該備份取代標準執行檔；偏好與 Windows 認證項目不需重設。這是執行檔回復路徑，未備份任何音訊、字幕或金鑰。

## 品牌字樣更新（2026-10-29）

新增「KOSMOS TOOLKIT 探真拓知酷」品牌列。品牌配置的原生視窗回歸為 62 成功、零失敗、1 個 child-only fixture 由父測試另外執行，83.12 秒；最後補上可見性切換的無啟用防護後，精確焦點／最小化案例另 1 成功、0.60 秒，集合重疊不累加。格式、零警告 Clippy（25.11 秒）、Debug（43.76 秒）及 Release（1 分 22 秒）通過；未重跑無關的雲端／媒體全套測試。

本版 `target/release/subtitlenow.exe` 為 9,995,264 bytes，SHA256 `4E4AB9B9C696571356B11825A84DBEC7EF744F12381CE4E32778EAC5B7D7EE5F`。已開啟 PID 26420／MainWindowHandle 8457168／Responding true，原生輔助使用樹確認可見文字「KOSMOS TOOLKIT 探真拓知酷」（ID 206），仍為待命，未開始擷取或雲端處理。這確認品牌控制項及文字，並非像素外觀目視驗收。

前一版外觀執行檔備份於 `target/ui-brand-backup-4f5d8d1a5fab4143bbc9e60d8d1e365d/subtitlenow.before.exe`，SHA256 `54F99B975FAB20553B5D95E3D9BF2E9C2EA1EC4063FB51F80B8FA1D9A114603C`。品牌為 UI 固定文字，未增加內容保存或更改翻譯流程。
