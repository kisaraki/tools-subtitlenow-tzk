# ADR 0098：Live 模型、原生譯文與主頁來源操作

日期：2026-10-25。狀態：採用，Phase 11 使用者指定功能；正式服務驗收待人工。

## 原因

使用者要求支援 gpt-live-transcribe 與 Gemini 3.5 Live Translate，並回報最新原文待譯段被隱藏、來源與語系應在主頁、系統狀態不應重複在設定頁。既有 Gemini Transcribe Live 已有人工正例，不能在未告知下改成不同模型。

## 模型與能力

供應商仍決定 WinCred 儲存位置、TLS 端點及既有 Prepared 行為；獨立 LiveSpeechModel 明確列出三個固定模型，資料格式留在 provider 模組。未設定模型沿用供應商預設，無跨供應商或跨模型自動 fallback。固定雙列金鑰及留空讀取本機流程保持。

GPT 使用既有 STT → 翻譯流程，Fast／Standard／High 的辨識 delay 分別為 minimal／low／medium，原翻譯品質與期限不變。官方不提供 detected-language predictions，因此明選五語，拒絕 Auto，不將使用者選項冒充偵測結果。

Gemini 新模型固定為 gemini-3.5-live-translate-preview，使用 AUDIO 回應與 translationConfig 的 zh-Hant，inputAudioTranscription／outputAudioTranscription 位於 setup 頂層（依 WebSocket API reference；不照抄指南中不同的 generationConfig 巢狀範例）。不傳 system instruction、文字提示、tools；只傳原 16 kHz PCM。輸出音訊只在既有有界框架中略過，不解碼、播放或保存，不新增音效裝置。

inputTranscription 不保證與其他訊息排序，也沒有與 outputTranscription 共用的對應 ID。不得以到達順序、轉錄 offset 或本機時間猜測原文／譯文／擷取位置。本批只顯示中文，標示「僅譯文」，原文、來源語言與 capture timestamp 留空；仍使用同一 TranscriptTimeline。支援明選五語；有 input languageCode 時核對，缺少不虛構。只支援 Fast／Standard，實際沿用 60／160 ms 合併策略，High 明確拒絕，因無可核對原文執行原精修。Auto、Prepared 定位或獨立內容識別設定組合明確拒絕，不靜默停用功能或改用服務。

## 生命週期與界線

NativeCaption 是有型別的譯文輸入，不是假 ASR：Session／generation／sequence／revision／transport freshness 俱全，來源欄位與時間皆不補造。最多 64 筆的具名輸出通道與 8192 bytes 當前段沿用容量界線，送入原 coordinator／timeline 的譯文更新，不建立 HTTP 翻譯工作或虛構 Translator。臺灣在地化轉換器於擷取前暖機，擷取 callback 不接觸翻譯。

服務真正 generationComplete／turnComplete 或明確 finished 才 Final；中斷回報具型別錯誤並沿用有限復原，不把中斷當完成。transport receipt 只作三秒排隊新鮮度，不作 media clock。取消、飽和根因、Session 切換、先清除段落不回填及下一段可見沿用原政策。原文從不送給 resolver／ExactCueVerifier；LivePlaybackClock 仍只接實際 capture 觀測。

無音訊診斷明確使用所選 Gemini 模型，metadata 名稱須精確核對；不把 GET 成功當執行權限，連線初始化不啟動擷取。HTTP／WSS 固定 HTTPS／WSS 與原憑證驗證，沒有正式端點 override。

## UI 與偏好

來源／語系／品質與開始／停止／清除在主頁可操作，在設定頁亦保留目前來源脈絡。系統狀態與錯誤只在字幕主頁；開始及診斷先返回主頁，來源／模型披露經真正 UI 顯示確認後才允許雲端啟動。設定仍原位展開／返回還原。字幕與媒體進階入口留在獨立頁，服務／模型及固定金鑰留在服務頁。SideChat 不再壓住最新無中文原文，BottomSubtitle 仍保留最近可讀中文且不錯配原文。自動跟隨、人工回看與有界 RAM 歷史不變。

偏好 schema 2 加可選模型，必須與供應商一致；v1 只作 RAM 遷移，留空保持原模型，明確儲存才原子寫入。無金鑰、來源 PID 或內容。沒有新依賴、音訊落盤、內容快取、遙測或第二份時間軸。

## 來源與驗證

2026-10-25 查閱官方 [OpenAI Realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription)、[Gemini Live Translate](https://ai.google.dev/gemini-api/docs/live-api/live-translate)、[模型](https://ai.google.dev/gemini-api/docs/models/gemini-3.5-live-translate-preview) 與 [WebSocket reference](https://ai.google.dev/api/live)。文件描述不等於此帳戶權限或實測延遲。

本機驗證涵蓋模型精確設定、實際 delay／合併策略、有界純譯文通道、真實 loopback WebSocket 握手與取消、臺灣在地化、無 ASR／額外翻譯工作、清除／舊 Session 隔離、設定檔遷移及原生頁面可見性。實際命令、結果與限制見 Phase 11 診斷；正式金鑰與所選程序測試仍由使用者固定遮蔽 UI 手動啟動，不宣稱 Phase 11 結案或 SLA。
