# ADR 0008 — 共用字幕顯示層與原生視窗所有權

日期：2026-10-09。狀態：Phase 5 共用顯示層、有界通知與原生消費入口已採用；正式 live owner 接線見 [ADR 0009](0009-owned-live-window-backend.md)，視窗內來源及設定控制見 [ADR 0010](0010-interactive-source-controller.md)，字幕透明合成見 [ADR 0011](0011-subtitle-alpha-surface.md)。

SideChat 的搜尋與回看不能改變 BottomSubtitle 的即時內容，也不能因 Dual 模式啟動第二條辨識／翻譯管線。原生 UI callbacks 會同步重新進入，直接把捕捉／網路物件或大型鎖放進視窗程序會干擾低延遲與取消。

採 Presentation 作為無 Win32、無供應商的顯示模型。唯一 timeline 的有界快照包含 Session／generation／版本；只有明確的 Session owner 可以切換 identity。模式、搜尋、回看與原文偏好只改 renderer 狀態，底部字幕總是取同一快照的最新項目。

原生 HWND、控制項、字型與選單只由主執行緒管理。穩定 Box 指標供 callback 借用，WM_NCDESTROY 清除指標，Drop 銷毀 HWND 後才釋放 Box 與字型；所有 RefCell 借用在 Win32 呼叫前結束，避免同步重新進入造成借用衝突。後續 backend 應透過有界訊息送快照，不跨執行緒傳 HWND 資料指標、COM 物件或 audio callback，也不讓 capture 等待 UI。

第二批採用一格 watch 通知。Lifecycle 與 snapshot 是同一份 UiFrame，略過中間 frame 不會漏掉 Session 切換；Arc 共用快照，UI 複製 Arc 後釋放 watch 借用才繪製。一般更新 33 ms 節流、完成結果明確強制發布；失敗／停止／重連清除文字。UI 關閉發出取消，實際 backend owner 必須自行取消並 join，避免把通知端的取消誤認為工作已回收。

--ui 必須是真正待命視窗，不顯示假的辨識內容。此批不把空視窗當成完整即時字幕功能；來源控制、有界後端通知與顯示延遲仍需後續實作與驗證。
