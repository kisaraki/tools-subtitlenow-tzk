# ADR 0099：原生譯文快照更新與回到底部跟隨

日期：2026-10-25。狀態：採用，Phase 11 人工回報後修正；新版實機結果仍待驗收。

## 問題與重現

使用者回報 Gemini 3.5 Live Translate 沒有可見字幕，狀態仍為正在翻譯、正確新模型及未啟用獨立內容服務；並回報大量字幕時捲軸持續縮小但最後行不可見。沒有讀取正式憑證或字幕本文。

新增 wire → LiveTranslation 的本機測試，連續輸入空白、合成中文及後續文字快照，不提供 generationComplete。原版第二個 partial 被拒絕，測試失敗。Native wire 每次輸出是完整替換快照，原實作卻把後續快照當尚未完成的普通重譯 delta，觸發 TranscriptTimeline 為防止短文字覆蓋可讀長譯文的保護，直到服務定稿才更新。第一個快照若為空白或很短，就可能長時間看不到有效中文。本機已重現此缺陷，但不宣稱已取得這次正式服務的原始回應或排除其他原因。

新增原生 EDIT 測試：已在底部時送 SB_ENDSCROLL，原版自動跟隨變 false，重現錯誤。另以超過 65,535 UTF-16 單位、真實控制列、窄視窗與 64 列淘汰測試最新有意義文字行及設定頁切換後的底部範圍；首次無控制列的大文字測試通過，不能單憑大量文字長度認定根因。

## 決定

原生譯文使用既有完整快照套用機制，complete 代表本次快照已完整，讓後續 partial 可原子取代前一快照；不建立翻譯工作。這不代表語句 Final：TranscriptKind／Stability 仍只由真正服務完成標記決定，partial 的穩定前綴仍為零。正常 ASR → HTTP 翻譯的修訂保護不變，所有 Session／generation／sequence／revision、新鮮度、容量、取消與清除防回填仍維持。原文、語言偵測與捕捉時間依舊不補造，沒有改模型、設定 schema 或目標語言。

原生滑鼠／捲軸／導覽鍵操作先暫停跟隨，呼叫下一層 EDIT 程序一次，待實際操作完成後才檢查位置。沒有搜尋、選取且已到底部時恢復跟隨；仍在回看、搜尋或選取時不搶回畫面。程式產生的刷新／捲動受原可巢狀 RefreshFlag 保護，不當作人工操作。沒有 blanket 忽略所有輸入，也不定時強制拉走人工回看。

自動捲動移到 layout／reflow 之後，設定完整 UTF-16 結尾並用 EM_GETLINECOUNT／EM_LINESCROLL 的 visual line 數到底。這不依賴輸入焦點或 WM_VSCROLL 的 16 位元 thumb position。原 UTF-16 選取／歷史錨點與有界 RAM 歷史不變。回呼不保留 RefCell 借用跨 DefSubclassProc，NCDESTROY 不在委派後存取 reference，所有程式碼仍受 panic 防護，下一層始終只呼叫一次。

## 驗證與界線

先保留兩個修正前可重現失敗，再驗證連續 partial 不等 Final、真正 Final／取消／手動清除／舊 Session 拒收，以及原生 scroll-end、回到底部恢復、選取與搜尋保護、大文字換行及最後行可見。完整命令與結果見 [Phase 11 診斷](../phase-11-gemini-diagnostics.md)。

僅修改譯文快照交接與原生顯示，未修改 WASAPI、時鐘、ExactCueVerifier、來源仲裁、API 金鑰、TLS、模型能力或有界佇列；沒有新依賴、遙測或內容落盤。工具不啟動正式擷取／雲端，使用者以固定遮蔽欄位與所選自有來源驗收。

完整驗證另遇既有原生 MP4 測試的五秒啟動誤判，造成 186 項共用鎖 poisoned 連帶失敗。單案 backtrace 指向等待解碼後的注入 Network 錯誤，而非停止。實測該 13 秒合成影片的啟動約 5.7～6.3 秒，取消約 1.8～3.6 毫秒；測試 helper 的原生啟動改用同檔既有三十秒 codec fixture 預算，停止及中文字幕旁路仍維持五秒。保留全部釋放、服務呼叫與來源檔不變斷言，增加只含合成階段耗時的測試診斷；不改產品期限，不忽略案例，不宣稱產品媒體效能問題已修復。失敗及第二輪驗證均記入診斷。

查閱 [Gemini Live API](https://ai.google.dev/api/live) 的文字與完成事件定義，以及 Microsoft [Edit controls](https://learn.microsoft.com/en-us/windows/win32/controls/about-edit-controls)；不以文件代替正式服務或目視結果。
