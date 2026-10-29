# ADR 0076：一般準備視窗的明確 HTTPS 字幕模式

日期：2026-10-18。狀態：Phase 08 第四十三批採用。

## 決策與原因

前兩批已提供有界 HTTPS 字幕讀取及明確 Prepared Task 到 RAM Ready。本批將其接入一般準備視窗，使用兩個明確的來源／格式選項：HTTPS SRT 與 HTTPS WebVTT。原本本機 SRT、WebVTT、內嵌軌選項的索引與語意保留，不依副檔名、網址內容或模型猜測格式與語言。

新選項使用既有字幕欄輸入完整 HTTPS 網址，必須非空且通過原有嚴格 URL／容量驗證。空欄、HTTP、相對位置、本機路徑、登入資訊、fragment、空白／控制字元或超量輸入都明確失敗；不 trim、修復、降級或改做 ASR。舊本機／內嵌入口仍拒收字幕網址，避免既有呼叫因傳入文字而增加網路目的地。

## 明確入口與共用生命週期

`PreparedInputDraft::into_https_subtitle_request` 只建立與驗證資料，不做 I/O。`PreparedBackend::start_with_https_subtitles` 是新的明確 opt-in 入口；尚未支援的 backend 預設回 UnsupportedUrl，不能委派至可能做音訊辨識的舊 start。

PreparedController 的兩個入口共用原本單一 worker、current-thread runtime、容量一的 completion 通道、Session、Stop／reap／Ready 狀態與服務工廠。新入口必須有 HTTPS 候選，metadata 規劃及歧義先處理，遠端媒體帶任一內嵌選取仍拒收；舊 start 對任何遠端字幕候選仍維持拒收。背景工作明確呼叫上一批的 `run_with_https_subtitles`，不新增辨識器、來源仲裁或第二份時間軸。

中文三類在本機轉為臺灣用語，不讀 password edits、安全憑證或建立服務。英、日、法、德、韓來源字幕只建立所選服務的分析／翻譯器，沿用同一全文摘要／詞彙、三品質策略與最後本地化；音訊 Auto 不影響明確字幕。只有本次選定服務取用一次金鑰，錯誤工廠不能改建辨識服務或跨服務 fallback。

## 可見用途與原生狀態

按準備前，固定用途區顯示會連線至使用者指定 HTTPS 字幕位置讀取全文；媒體為本機時保留唯讀來源，遠端則只讀既有 PCM16 WAVE 標頭。中文處理不連 AI；五語明示所選 OpenAI 或 Gemini 端點及原文、全文摘要、專名／詞彙、翻譯前後文用途。音訊與私有媒體／字幕網址不送至 AI；內容僅存本次 RAM。

用途區只使用固定說明與所選 AI 端點，不回顯私有網址、主機、signed query、媒體路徑、金鑰或服務本文。既有固定雙列遮蔽金鑰、取用後兩欄與 undo 清空、狀態／通知的一般文字保持。選擇 HTTPS 模式即視為明確字幕，即使網址空白仍不啟用音訊語言或 ASR 用途。

HTTPS 字幕模式停用 MP4 字幕軌探索，直接命令也有防護，避免探索完成後自動切換成內嵌模式並覆寫手動網址。原有本機模式仍可探索；遠端媒體原有探索限制不變。切換格式不自動清除手動字幕。Busy／Stopping／Ready 鎖定來源，停止等待既有工作真正退出後才可重啟；清除／關閉移除網址及 undo，通知仍只送一次，不自動播放。

## 驗證與限制

使用被動草稿測試、隱藏 Win32 控制項、原生文字量測、合成媒體與 literal loopback HTTP 及明確服務 mock，核對模式邊界、實際 Task 路由、服務／金鑰隔離、來源／原文時間、全文品質策略、typed error、取消、Ready 與舊 Session 隔離。正式字幕仍只接受 HTTPS，HTTP loopback 例外只在測試程式內。

沒有新增相依套件、內容落盤、永久 cache、temp、export、遙測或音訊上傳分支；保留原有媒體／字幕／CPU／並行／期限上限與持有者到實際清理的規則。遠端媒體仍只支援既有直接 PCM16 WAVE 子集合，不提供登入、重新導向、一般網站或遠端內嵌軌。

Ready 表示本次文件完整準備，不提供媒體配對、內容識別、播放定位或自動播放證據。真實使用者媒體、語意／雲端品質、真人多螢幕／DPI 及播放驗收仍待完成；有效 OpenAI／Gemini 金鑰維持結案固定遮蔽介面手動輸入。本批完整工程驗證已通過，已具備開始 Phase 09 對齊核心的工程條件；這不代表 Phase 08 全面產品驗收已結案，本批不自行啟動下一階段。實際檢查結果見 [Phase 8](../phase-8.md)。
