# ADR 0045 — 有界 RAM 音訊的 Prepared 辨識 HTTP adapters

日期：2026-10-12  
狀態：採用於 Phase 08 第十批；一般 GUI ASR、限五語 Auto 及真實服務驗收尚待接續。

## 問題

PreparedSession 已能透過 caller 交付的辨識服務建立同一媒體 pin 的 Ready，但還沒有正式 HTTP 辨識 adapter。不能把 Live 的 capture freshness 契約套到離線音訊，也不能以模型提示產生的時間文字代替原生字幕時間。使用者已指定金鑰留至結案，在固定遮蔽介面手動輸入；本批需要先完成可離線驗證的真實協定實作。

## 決策

新增 HttpPreparedRecognizer，caller 明確提供 OpenAI／Gemini 與 ApiKey。建構不讀憑證、不連線、不預設服務或使用 mock。固定 HTTPS endpoint、驗證 TLS、禁止重新導向及代理伺服器，金鑰只放 sensitive header，不放 URL、錯誤訊息或 log。client 重用連線池，不逐音訊塊建立 TLS client，也不跨供應商 fallback。

OpenAI 使用 audio/transcriptions 的 whisper-1、verbose_json 與 segment timestamp；來源語言使用明確 ISO code，有限的先前原文只作短提示。選擇這個模型是為了使用服務提供的時間欄位，而不是假稱其他 transcription 模型也有同樣能力。協定依據 [OpenAI transcription reference](https://developers.openai.com/api/reference/resources/audio/subresources/transcriptions/methods/create) 與 [OpenAI speech-to-text guide](https://developers.openai.com/api/docs/guides/speech-to-text)。

Gemini 使用 gemini-3.5-transcribe 的 Interactions unary API、VERBATIM 及 word timestamps。音訊透過 inline Base64 WAV 傳送，明確 store:false、background:false、stream:false；不使用 Files API、previous interaction ID、背景工作或保存結果。回應採 completed 的 model_output text／word_info 原生時間；UTF-8 byte 索引出現時必須與本文一致。協定依據 [Google transcription guide](https://ai.google.dev/gemini-api/docs/transcribe)、[Interactions schema](https://ai.google.dev/api/interactions-api) 與 [Interactions lifecycle](https://ai.google.dev/gemini-api/docs/interactions-overview)。

兩個 adapter 只接受明確指定的英、日、法、德、韓，支援本次 16／24 kHz 單聲道 PCM16 WAVE 包裝。constrained_auto_detection 設為 false：已查閱的 API 沒有建立可依賴的限五語 Auto 契約，Auto 在開檔或上傳前回報 UnsupportedLanguage。Gemini 的 language_codes 只是提示，來源語言沿用 caller 設定，不冒充服務偵測結果；不宣稱能偵測錯選語言或混語。Whisper 回傳的來源語言則必須與本次明確選擇一致。

音訊不存檔，將既有最多十一秒的 owned chunk 包裝為 RAM-only WAVE。WAVE、multipart／JSON 包裝及回應解析在既有有界 CPU lane，依 available_parallelism 最多兩項工作。OpenAI multipart boundary 不允許與資料碰撞；欄位與檔名固定，不加入來源路徑。Gemini 時間戳模式不使用 SMART／custom vocabulary；不得假稱它會處理完整前文或 ASR 詞彙表。Session glossary 仍由後續共用翻譯處理。

每個 client 最多兩個要求，依共用 CPU lane 的固定宣告容量調整；單核心用一個要求，避免同批兩個編碼工作必然爭用唯一 CPU 許可。宣告容量與 Semaphore 用同一個穩定值，不以暫時剩餘許可猜測核心數。額滿立即回報 Overloaded，不建立無限等待佇列。network 等待不持 CPU 許可。HTTP owner 與已開始的 wire worker 共用 request permit lease，取消或 Future Drop 不會在同步 worker 真正清理前提前釋放容量。CPU permit 同樣保留至 worker 退出，abort 不是同步清理證明。

每份要求的六十秒預算涵蓋排程、速率等待、編碼、最多兩次嘗試、完整回應與解析。只有網路／逾時／限速失敗可有限重試，間隔及 jitter 只依內部序號。Retry-After 不被截短成提早重送；有效較長冷卻保留在共用時鐘並回傳當次 typed error，非法或溢位值不重試。建立 Future 前配置連結到 caller 的內部子 scope，停止與切換由祖先傳遞；服務內部失敗、逾時及 Drop 只取消子 scope，不將 caller 的要求標成取消，避免把真正錯誤蓋成 Cancelled。取消後成功也拒收，不留下背景 HTTP 工作。

成功回應只接受 JSON／identity encoding，原始 body 限 512 KiB，Content-Length 及 chunked 累積都檢查容量。錯誤 body 不讀取或記錄。解析核對完成狀態、本文、語言、相對音訊起點的時間、順序、UTF-8、句數及全文容量，再交給原有 validate_for。Gemini annotations 必須完整涵蓋非空白本文，未標註標點也保守拒收；缺少正式輸出欄位不冒充無語音。只排除完整落在正文以外的 context；跨正文的句子或單字回報 BoundaryMismatch，不裁時間或補猜測時間。

## 結果與限制

正式協定實作可由 caller 交付現有 PreparedSession 接線，保留同一媒體 pin 的 Ready。loopback HTTP、合成 WAVE 與明確 mock 翻譯只驗證協定、所有權、時間及取消，不代表正式金鑰、雲端辨識品質或模型取樣率相容性已驗收。

一般 GUI 尚未建立 Prepared ASR client，固定金鑰欄位的實際用途不變。限五語 Auto、跨句切段品質、自動全文分析／詞彙建立、其他容器、內嵌字幕、URL 及播放定位待後續完成。沒有新 crate、unsafe、安裝、產品暫存媒體、內容快取、匯出或自動播放。Phase 09 尚未具備啟動條件，真正金鑰及雲端驗收留至結案。

實際指令與測試數見 [Phase 8 第十批驗證](../phase-8.md#第十批驗證)；本機合成測試不取代真實服務驗收。
