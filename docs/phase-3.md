# Phase 3 — Streaming STT

啟動：2026-10-08。使用者明確指示直接啟動，並指定 OpenAI（ChatGPT 系列 API）及 Google Gemini 可選。本階段進行中；Phase 2 的未驗項目仍見 [Phase 2](phase-2.md)。產品行為依 [SPEC](../SPEC.md) 第 16–28、77 節。

目前狀態（2026-10-09）：供應商 WSS、客戶端端點判斷、重採樣 worker、憑證設定、指定 PID 管線與同服務重連已實作。本機兩個供應商憑證項目均為 Missing；尚無真實五語辨識、準確度或雲端延遲驗收，因此未將 Phase 3 標為完整結案。以下第一／二批是歷史紀錄，第三批為目前實作。

使用者補充安排：有效金鑰測試留到結案，以本機互動提示手動輸入，再進行真實服務驗收；目前接續 Phase 4，不提前索取金鑰。--interactive-key 已加入 STT 命令，只保留當次 RAM 金鑰、不寫入安全憑證庫。既有 --set-stt-key 的永久安全憑證設定是可選操作，結案測試不需要先執行。

## 已實作的第一批範圍

src/ai/speech.rs 定義 SpeechRecognizer、SpeechCapabilities、SpeechSession 與工作 future。語言型別只有英、日、法、德、韓；Auto 要求供應商明確宣告可限制於這五語。每個串流帶 Session ID 與 generation，結果包含來源 sequence、utterance、revision、時間及 Partial／Final。未知時間與語言保留 None，不以接收順序假造來源時間。

音訊與結果都使用 Tokio 有界 channel。音訊上限 12 個 20 ms 區塊（240 ms），生產端 try_push 不等待網路或空位；讀取端再次檢查 250 ms 年齡、sequence、frame position 及 discontinuity。初始 discontinuity 可作為串流起點，後續中斷必須停止辨識。滿載／過期／缺口目前採明確錯誤與重新啟動策略，不把缺少音訊拼成連續語音。

結果上限 64 項、每項文字最多 8192 bytes。結果滿載會取消串流，避免默默遺失 Final。錯誤原因保留第一個；取消優先於待交付資料，清空已排隊結果。呼叫端持有並 poll SpeechTask，沒有 detached task；丟棄 session／task 會取消。實際供應商的 socket 與子工作仍需遵守這項契約，尚未由本批程式驗證。

TranscriptLedger 是辨識修訂的 RAM 暫存，不是 Phase 5 字幕介面。它按來源 sequence 排序，接受較新的 Partial 完整替換（可縮短文字），Final 不再被 Partial 覆寫；拒絕舊 Session／generation、過時修訂及已淘汰項目。最多保存 64 個 utterance，沒有檔案輸出或內容快取。

AppConfig 新增可省略的 speech_provider，值為 OpenAi、Gemini 或 null，預設未選擇；沒有金鑰欄位。啟動狀態更新為 phase 3，明確說明連線尚未實作；既有 capture 命令仍只擷取、不送雲端。

## 官方 API 查核與供應商設定

2026-10-08 查閱官方文件：OpenAI 的 [Realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription) 使用 gpt-live-transcribe、24 kHz PCM、delta／completed 事件；需要客戶端判斷 turn 結束並 commit，不能套用 server_vad。Gemini 的 [Live transcription](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe) 使用 gemini-3.5-transcribe-live、16 kHz PCM、interim／final 事件，單次連線有十分鐘限制。模型可用性與實際回應仍待帳號實測。

src/ai/provider.rs 僅產生供應商設定 JSON、模型名稱與採樣率計畫，不實作 SpeechRecognizer，不連線、不假裝辨識成功。Auto 的計畫只送出五語候選；這些語言提示不能證明服務端一定限制偵測，Auto 的正式能力宣告需等介接驗證。不同供應商的事件解析、時間對應與重連 generation 不得放入 WASAPI 或 UI。

## 真實服務驗收待辦

1. 使用各自有效 API 憑證，確認帳號可以使用官方文件指定的模型與 setup schema。
2. 驗證五語固定選取與受限 Auto 的真實 Partial／Final、語言回報、準確度與延遲。測量端點判斷對低音量語音、音樂背景及連續對話的影響。
3. 驗證 Gemini 自動活動偵測、連續音訊及 Partial／Final 段落修訂；Final 延後時音訊仍向前，不同 server utterance 不混用。未知擷取時間不參與精確來源定位，見 ADR 0091；真實中文與停止仍待人工驗收。
4. 驗證真實雲端網路中斷、速率限制、來源退出／停止／切換及九分鐘重建。離線 fixtures 與無效金鑰拒絕不能代替此項。

目前沒有翻譯或字幕 UI；這些屬後續 Phase。本次未送出私人音訊或真實金鑰。

## 第一批驗證

明確測試用 MockSpeechRecognizer 只存在 cfg(test)，不被正式程式使用。測試涵蓋 Partial／Final、預先取消、pending provider 取消、供應商失敗、有界滿載、慢消費端、過期／不連續 PCM、非法樣本、舊 Session／generation、亂序完成、Partial 回退、Final 保護、五語限制與供應商偏好往返。沒有雲端要求或私人音訊測試。

驗收命令：

    cargo fmt --check
    cargo clippy --locked --all-targets --all-features -- -D warnings
    cargo test --locked
    cargo build --locked

2026-10-09（臺灣時間）上述四項命令均通過；一般測試 84 項通過（61 library、12 設定、4 探索、7 啟動），其中新增 12 項串流核心測試與 1 項供應商設定測試。既有十二項硬體／手動／長時間測試預設略過，不算本批已驗。Debug binary 實際執行與 --help 均確認顯示 Phase 3 尚未提供實際辨識的限制。

第一批沒有改動原生 capture／helper／COM／unsafe 路徑，也沒有新增套件、API 要求、音訊輸出或憑證檔案。架構理由見 [ADR 0005](adr/0005-streaming-speech-boundary.md)。

## 第二批實作與驗證

src/audio/resample.rs 加入 48 kHz → 24／16 kHz PCM16 串流重採樣，使用 127 taps Hamming FIR，保持跨區塊濾波與抽樣相位。63 個輸入 frame 的群延遲為 1.3125 ms；已知來源時間會扣除延遲，起始負值及未知時間仍為 None。每區塊不配置 heap、不做 I/O；應在有界預處理 worker 呼叫，尚未接入正式管線。音訊缺口會使該實例停止接受後續音訊，要求重新建立。五項測試涵蓋不同切塊的逐樣本一致性、通帶與抗混疊、靜音／長度、Session／缺口及時間。

src/ai/transcript_adapter.rs 將 OpenAI delta 累積成 Partial，completed 校正成 Final；Gemini interim 完整替換可回退，inputTranscription 形成 Final。關聯表最多 64 項、文字最多 8192 bytes、JSON 訊息最多 64 KiB；未來 transport 也必須在接收緩衝前限制大小。來源時間、sequence 與語言由已提交音訊的明確 context 提供，不能從回應抵達時間推算。Gemini 必須由呼叫端提供已確認的 turn 關聯；本批沒有證明實際服務如何對應 turn。五項測試涵蓋亂序完成、Final 保護、Partial 回退、未知／過大／損壞資料、關聯容量與安全錯誤。

src/ai/credentials.rs 與 src/win32/credentials.rs 加入只讀 Windows Generic Credential 邊界。僅讀取 SubtitleNow/OpenAI/ApiKey 或 SubtitleNow/Gemini/ApiKey，不列舉其他憑證。格式為不含結尾 NUL 的 UTF-16LE 可列印 ASCII 金鑰；上限 2560 bytes。ApiKey 不可序列化或複製，Debug 遮蔽內容；新增 zeroize 1.9.1 以在丟棄時清除本程式持有的金鑰及 WinCred 回傳 blob，RAII 釋放原生配置。這是本程式持有記憶體的清理，不代表未來 TLS／HTTP 套件中的副本都已清除。格式與管理方式依 [CredReadW](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credreadw) 及 [CREDENTIALW](https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw)。

三項憑證測試使用人工測試字串，驗證格式、控制字元拒絕、大小與遮蔽。本批沒有讀取真實 Windows 金鑰、寫入憑證或送出音訊，也尚未提供憑證設定 UI／命令。WinCred 原生路徑只有編譯驗證，不能視為實際認證管理員驗收。新增 unsafe 僅限該原生邊界，附指標長度、所有權與生命週期說明；既有 capture／helper／COM 未改動。

2026-10-09 第二批最終驗收：cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked 均通過。一般測試共 97 項通過（74 library、12 設定、4 探索、7 啟動），新增 13 項。既有十二項硬體／手動／長時間測試略過，未在本批重跑。Clippy 首次指出常數大小 chunks_exact 可改為 as_chunks；修正後重跑通過，沒有停用警告。

## 第三批：實際管線接入

src/ai/streaming.rs 實作 SpeechRecognizer。OpenAI 使用 [官方 cookbook 的 transcription WebSocket](https://developers.openai.com/cookbook/examples/speech_transcription_methods) 與 [目前串流模型的設定／事件](https://developers.openai.com/api/docs/guides/realtime-transcription)；Gemini 使用 [Live WebSocket API](https://ai.google.dev/api/live) 與 [Live transcribe 設定](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe)。僅允許固定 WSS 位址，以 rustls、ring 與系統根憑證驗證 TLS，沒有關閉憑證驗證或自訂任意 endpoint 的正式選項。模型實際可用性仍受帳號權限影響。

握手／setup 期限 20 秒，完成後才啟動擷取；每次寫入期限 100 ms。frame／message 上限 64 KiB、寫入緩衝上限 128 KiB。持續連線，不為每個音訊區塊建立 TLS。Ping／Pong 分開處理，不送入辨識 JSON adapter；心跳每 10 秒，30 秒未收到任何回覆即停止。已提交段落等 Final 的期限為 8 秒。九分鐘期限與 goAway 會結束舊工作，讓組合層重建。

src/ai/preprocess_worker.rs 是每個辨識工作的一個專用 CPU worker，不在 async executor 跑 FIR，也不每區塊 spawn。輸入、輸出及送出暫存均有界；任何區塊來源年齡超過 250 ms 就停止。OpenAI 使用 RMS 0.003、靜音 400 ms／連續 4 秒的能量分段並手動 commit。2026-10-24 Gemini 改用服務端自動活動偵測，持續送出 PCM（含靜音），不使用本機 activityStart／activityEnd。

OpenAI 以同一 socket 上有序的 committed acknowledgement 對應已提交的音訊 context，再以 item_id 接受亂序 Final。Gemini 連續送出新鮮音訊，不因前段 Final 延後而停止送出；服務 utterance 順序與修訂只供文字排序，不當成擷取序號，無可靠時間時 start／duration 保持 None。相同 TranscriptTimeline 仍能顯示並翻譯，精確定位維持拒收未知時間；沒有用本機回覆時間偽造來源關係，詳見 ADR 0091。原有界資源、音訊年齡、取消與共用有限復原保持，真實連續辨識仍待人工。

Auto 送出五語提示後仍檢查服務端回報。語言未知的 Partial 只保留最新修訂；取得五語內的回報才釋放。Final 缺少回報或含不支援語言時拒絕輸出；多個受支援語言不假造單一偵測值。固定語言遇到不相符回報會停止；沒回報的語言仍保持 None。

src/app/live.rs 接入 --stt-pid、供應商／語言選取、來源可見性、Ctrl+C、CaptureSession 與單一辨識工作。先保留指定程序 handle，重新連線仍確認原程序存活。失敗時取消、停止／join 擷取、丟棄辨識 future 與 worker，然後才連回同一供應商；每次用新的 Session ID 與 ledger，舊內容不重播。網路、速率限制、音訊滿載／缺口及太快的服務期限事件最多三次有界退避（500／1000／2000 ms 加 0–250 ms jitter），穩定 30 秒後重設。正常九分鐘輪替也清空未完成段落，不跨服務 fallback。

--set-stt-key 透過隱藏終端機輸入及 [CredWriteW](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew) 寫入本程式的固定 Generic Credential，重設會替換該供應商金鑰。--check-stt-key 不連線。新增原生測試使用唯一 SubtitleNow/Test 名稱與人工字串，驗證 Missing → 寫入 → 讀取 → 替換 → 刪除 → Missing，清理也在 panic 時執行；不覆寫正式憑證。實際 STT 拒絕 stdout 重導向，不提供內容存檔／匯出。操作見 README。

新增相依套件：base64、futures-util、tokio-tungstenite、rustls 與 rpassword，版本固定於 Cargo.lock。第一次外部 WSS 測試發現 rustls 未選定 crypto provider，已明確啟用 ring 後重跑；未關閉 TLS。人工無效金鑰的外部測試通過：OpenAI 回報 Credential，Gemini 以關閉碼 1007 回報 InvalidData。這只確認安全連線與拒絕／錯誤分類，沒有送出音訊，不能證明可用帳號的模型 setup 或辨識能力。

2026-10-09 第三批最終驗收：cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked、cargo build --locked --release 均通過。一般測試 113 項通過（89 library、12 設定、4 探索、8 啟動），新增 16 項。13 項預設略過，包含既有十二項硬體／手動／長時間測試與一項外部 WSS 測試；WSS 測試另以 --ignored 明確執行並通過。沒有把未重跑的硬體矩陣算成本批通過。

本機 WebSocket fixtures 覆蓋兩個供應商 × 五語選項的合成 PCM 串流，檢查 PCM 採樣率／長度、端點、Partial／Final、來源 ID／sequence／duration 與語言回報。這些文字是 cfg(test) 人工資料，完全不代表五語 ASR 準確度。另驗證取消、供應商失敗原因不被 worker 清理覆寫、慢 transport 100 ms 逾時、超大 WebSocket message 拒絕、錯誤語言欄位、未知語言 Partial 暫存、亂序 Final 與 Gemini 單一未完成 turn。

實際執行 --check-stt-key openai 與 gemini 都回報 Missing，未取得任何真實金鑰。--help 已檢查新的操作說明。架構理由見 [ADR 0006](adr/0006-selected-provider-streaming.md)，剩餘服務驗收步驟見 [測試操作](phase-3-test-procedure.md)。本次未提供翻譯、字幕視窗或持久內容快取，沒有啟動未授權的後續 Phase。
