# ADR 0033：所選程序的可靠視窗標題

日期：2026-10-11

狀態：採用；正式歌曲／電影目錄與使用者服務設定仍待完成。

來源清單的視窗標題只供顯示，現有探索會選第一個可見標題，也會快取程序資訊。不能解析已截短的 UI label、選最大的視窗或跟隨前景視窗，來替電影查詢提供來源證據。電影線索必須來自實際擷取所選 PID，並核對該程序的建立身分。

新增明確的 windows_configuration。Live 以仍存活的 pinned ProcessTarget 之 PID 與建立時間綁定 factory；這一步只建立設定，不讀視窗或連網。每次擷取／重連，factory 才建立新的 Session metadata reader。綁定失敗只讓內容分支回報有型別的錯誤，保留目的服務提示，STT／翻譯仍獨立。原有明確 metadata 配置仍可使用，普通 --ui 維持 None。

Native reader 擁有可停止的工作執行緒及 query／synchronize-only 程序 handle。工作執行緒開啟所選程序、核對建立時間，並在掃描前後以零等待確認存活；保留 handle，避免只憑重用的 PID 誤認另一個程序。每 250 毫秒進行一次有界列舉，process-wide 最多一個工作，以 available_parallelism 作為上限輸入。不在音訊 callback、UI callback 或 metadata 的同步 RAM 查詢中呼叫 Win32。

只接受直接屬於該 PID、可見、非最小化、非工具視窗的唯一非空標題。多視窗即使標題相同也拒收；超過列舉上限、不完整列舉、標題改變競態、無法查詢樣式、截斷、無效 UTF-16、控制字元或超過 1024 bytes 都拒收。callback 只使用固定大小陣列及 scalar，不配置字串、不留下指標，也不跨 FFI 展開 panic；字串轉換在 callback 外完成。沒有程序父子推論、前景／大小排名、hooks 或對其他程序送 WM_GETTEXT。

Microsoft 說明 GetWindowTextW 與 GetWindowTextLengthW 對同程序視窗會同步傳送訊息；因此拒絕本程式 PID，避免 UI 等待 worker join 時死結。外部程序的讀取用 caption 機制。GetWindowTextLengthW 可能因 ANSI／Unicode 換算高估，這裡保守要求讀前／複製／讀後長度一致；可能拒絕部分有效標題，但不接受截短或猜測的文字。[GetWindowTextW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowtextw)、[GetWindowTextLengthW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowtextlengthw)、[EnumWindows](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumwindows)。

SourceMetadataBuffer 只保留一筆最多 1024 bytes 的標題及原始掃描開始時間。它核對 Session、capture stamp、sequence 與觀測時間水位；資料最多保留兩秒的有效性，UI 接收或查詢時間不能重設期限。相同標題的新掃描只更新未來 lease，已交付 lease 的原期限不變。變更、消失、模糊、錯誤及 Drop 取消舊 token；過期清除。錯誤保留型別，只有合法後續掃描能復原，舊回呼不能清除新資料。

MovieIdentifier 同時核對 metadata token、ASR token、使用者取消，以及 metadata／對白／實際擷取原期限的最小值；服務回覆後再次核對。無可靠 metadata 不送查詢，片名之外的 filename、year 與完整 runtime 保持 None。Native worker 的暫時掃描失敗仍繼續觀察；程序退出、啟動錯誤或 panic 退回可靠錯誤。Reader Drop 同步撤回標題、喚醒並 join，釋放 handle 及 worker permit。

這是來源 metadata 的實作，不是電影服務或時間同步驗收。沒有直接所屬視窗的音訊程序、同程序多視窗、受保護或無法查詢的來源仍使用 Live AI，並顯示內容分支的錯誤。瀏覽器音訊子程序不會被偷偷改綁瀏覽器父程序或分頁。正式目錄及字幕／歌詞 metadata source 仍需配置；沒有新增音訊／文字檔案、持久快取、provider 金鑰或跨服務 fallback。
