# Universal AI Subtitle for Windows
## SPEC.md — 完整專案規格書

版本：Draft 1.4  
主要平台：Windows 10 / Windows 11  
主要開發語言：Rust  
目標架構：Win32 / Windows Core Audio / 非同步雲端 AI / 多執行緒處理

---

## 1. 專案定位

本專案開發一套 Windows 通用型 AI 即時翻譯字幕軟體。

核心目標：

1. 偵測目前 Windows 中正在發出聲音的應用程式。
2. 讓使用者指定要處理的音訊來源。
3. 從指定應用程式擷取音訊，而不是預設錄製整台電腦混音。
4. 判斷音訊內容類型。
5. 優先尋找是否存在可直接使用的字幕、歌詞或翻譯文字。
6. 若無適合內容，執行串流語音辨識與 AI 即時翻譯。
7. 將翻譯結果顯示成浮動字幕或側邊翻譯紀錄。
8. 提供影音檔案或網址的「預先翻譯模式」。
9. 不提供音訊保存功能。
10. 不建立永久字幕、歌詞、翻譯、指紋或影音內容 Cache。

本專案定位為：

```text
Windows Universal Audio Translator
```

而不是：

```text
YouTube Translator
Browser Extension
Video Downloader
Media Player
Audio Recorder
Subtitle Archive
```

---

## 2. 核心設計原則

優先順序：

1. 正確性
2. 音訊擷取穩定性
3. 低延遲
4. 系統穩定性
5. 使用者隱私
6. Windows 軟體通用性
7. 翻譯一致性
8. UI 易用性
9. 額外內容辨識功能

---

## 3. 平台與技術

- Windows 10 / Windows 11
- 第一版優先 x86_64
- Rust
- Windows Core Audio / WASAPI
- 優先使用 `windows` crate
- 網路與非同步工作可使用 Tokio
- CPU-heavy 工作使用有界 Worker Pool
- 不依賴 Chrome / Edge / Firefox Extension
- 不使用自製 Windows Audio Driver 作為第一版必要條件

第一版的主要開發與擷取驗收基準為 Windows 10 22H2、組建 19045、x86_64；Windows 11 列入延伸相容性驗證。此基準是產品目標，不代表所有累積更新、裝置或應用程式均已通過驗收。

---

## 4. 支援語言

來源語言固定支援：

- English
- 日本語
- Français
- Deutsch
- 한국어

```rust
pub enum SourceLanguage {
    Auto,
    English,
    Japanese,
    French,
    German,
    Korean,
}
```

`Auto` 僅能在上述五種語言間判斷。

Gemini Transcribe Live 的 Auto 使用服務原生自動偵測（空 `languageCodes`），本機再核對五語；非空清單是提示，不代表服務硬性限制。接受五語的一般地區／script／variant 標籤，只在同一 utterance 沿用服務實際回報的語言，定稿後退休。標記存在但不合法／支援外時拒收；缺少標記時改用有界本機文字語言模型備援，不能假冒為服務標記、默選英文或沿用前段。見 [ADR 0101](docs/adr/0101-gemini-auto-language-metadata-and-status-details.md) 及後續 [ADR 0104](docs/adr/0104-local-text-language-fallback.md)。

Auto 持續未確認語言的 partial 另有八秒總等待上限，從第一個待核對 partial 起算，後續未確認 partial 或 flush 不得續期；同段服務標記或符合本機模型門檻的文字估計才能解除等待，期限後拒收遲到結果。真正 final 若本機仍不確定，明示略過並繼續下一段，不交付該段文字；未收到真正 final 時不得自行定稿。尚未收到任何轉錄時不據此判錯。主頁每秒顯示純數值進度、語言依據、本機採用及未確認略過數；錯誤保留已停止的最後取樣，新連線清除。無音訊、本文或播放證據，不改字幕／擷取流程。見 [ADR 0102](docs/adr/0102-bounded-auto-language-wait-and-numeric-progress.md) 與 [ADR 0104](docs/adr/0104-local-text-language-fallback.md)。

待確認語言持續兩秒時，限次送 audioStreamEnd 要求服務真正定稿；等待語言期間已因輸入閒置送過收尾則不重送。此要求不暫停後續音訊、不延長八秒期限，也不代替語言／Final；文字經語言核對才可交付，段落仍須服務真正定稿才退休。純數值另列「語言收尾要求」成功送出次數。明選語言與其他模型保持既有路徑。見 [ADR 0103](docs/adr/0103-auto-language-finalization-request.md)。

本機備援使用 whatlang 0.18.0 的全部內建模型後篩選五語，至少十二個字母字元並通過函式庫可靠性門檻，不確定不得硬選。每個缺標記修訂重新判定，不回填未判定舊 partial；工作綁定 Session／generation、訊息序號、精確本文、種類及四十 ms 期限，使用原有全程序 CPU lane，不新增雲端要求／內容保存。UI 明示「本機文字估計」，使用者可手選語言；統計信心不代表音訊證據或準確率保證。見 [ADR 0104](docs/adr/0104-local-text-language-fallback.md)。

---

## 5. 翻譯目標

固定為：

```text
臺灣繁體中文
zh-TW
```

不提供其他翻譯目標。

所有中文輸出必須採用臺灣常用語法、標點與詞彙。

---

## 6. 簡體中文字幕處理

若網路或影音來源取得簡體中文字幕：

```text
Simplified Chinese
→ 繁體字轉換
→ 臺灣用語本地化
→ zh-TW
```

不得只做字形轉換。

例如：

```text
视频   → 影片
软件   → 軟體
网络   → 網路
默认   → 預設
文件夹 → 資料夾
鼠标   → 滑鼠
出租车 → 計程車
打印机 → 印表機
硬盘   → 硬碟
服务器 → 伺服器
```

---

## 7. 翻譯品質

```rust
pub enum TranslationQuality {
    Fast,
    Standard,
    High,
}
```

預設：

```text
Standard
```

### Fast
- 最低延遲優先
- 較短上下文
- 較早顯示 Tentative 字幕
- 最少二次校訂

### Standard
- 預設模式
- 延遲與品質平衡
- 使用適量前文
- Stable / Tentative 翻譯
- 基本術語一致化

### High
- 較長上下文
- 強化語境理解
- 專有名詞一致性
- 必要時二次校訂
- Prepared Mode 可做全文一致性 Pass

---

## 8. 主要工作模式

### Live Mode
掃描 Windows Audio Session，讓使用者選擇正在發聲的程式並即時翻譯。

### Prepared Mode
使用者提供本機影音檔或 HTTP(S) URL，先完成字幕／ASR／翻譯，再提醒使用者開始播放。

---

## 9. Windows 音訊來源

優先使用 Windows Core Audio：

- `IAudioSessionManager2`
- `IAudioSessionEnumerator`
- `IAudioSessionControl2`

UI 應盡可能顯示：

- 應用程式圖示
- 應用程式名稱
- 視窗標題
- 是否正在發聲
- 即時音量 Meter

內部核心以 Audio Session / Process 為主，不把 HWND 當成真正音訊來源。

---

## 10. Process Loopback

優先採用 Windows Application / Process Loopback Audio Capture。

目標：

```text
Selected Process
      ↓
Application Audio Only
      ↓
Translator
```

來源失效時，不可偷偷切成 System Mix。

Process Loopback 的可用性須由所選來源的實際 API 啟動、初始化與擷取結果判定，不可僅因組建低於 20348 就拒絕 Windows 10 19045。版本資訊保留作為診斷；啟動成功、收到音訊與來源隔離須分別驗證。無法擷取時應說明失敗階段，不可自動改抓整體混音、麥克風或其他來源。

Phase 2 的實作順序、證據與驗收設計見 [Windows 10 19045 擷取設計](docs/windows-10-capture-plan.md)。

Windows 10 19045 的原生擷取與正規化在目前工作階段專用的背景程序執行。背景程序不自行選取來源；父程序固定使用者指定的 PID、持有程序 handle，並以有界本機 pipe 接收音訊。最新 Core Audio 列舉與獨立程序音訊來源確認，須在工作階段背景程序完成；確認成功前不得建立原生擷取 client 或發布 Running。父程序在擷取流程中不得反覆建立 Core Audio 來源確認物件。停止、來源切換或父程序退出時須回收背景程序及其原生資源，並保留 Session ID、順序、時間戳與逾期策略。不得把程序隔離改成整體混音或讓背景程序永久保存內容。設計理由見 [ADR 0004](docs/adr/0004-session-owned-capture-process.md)。

---

## 11. 瀏覽器

Chrome、Edge、Firefox 視為一般 Windows 音訊來源。

不保證可穩定拆分至個別分頁。

不加入 Browser Extension。

---

## 12. 多執行緒與非同步

本專案必須採用：

```text
Multi-threaded
+
Async
+
Pipeline Parallelism
```

即時流程不得串列成：

```text
Capture
→ STT
→ Translation
→ UI
→ 下一段音訊
```

應允許：

```text
Segment A → Translation
Segment B → STT
Segment C → Preprocess
Segment D → Capture
```

同時執行。

---

## 13. 執行緒模型

建議：

- UI / Win32 message loop：主執行緒
- WASAPI Capture：專用執行緒
- Audio Preprocess：有界 Worker
- CPU-heavy Resolver：有界 Worker Pool
- STT / Translation / Remote Lookup：Async Runtime
- Subtitle Stabilizer / Timeline：輕量 Worker

不得一個 Audio Chunk 開一個 OS Thread。

---

## 14. Audio Capture 絕對限制

Audio Capture Thread 不得等待：

- Network
- AI
- Translation
- UI
- Disk I/O
- Content Resolver
- 長時間 Mutex

理想工作：

```text
Receive PCM
→ Timestamp
→ Push to bounded buffer
→ Return
```

---

## 15. Bounded Queue / Backpressure

所有高頻管線必須使用：

- Bounded Channel
- Ring Buffer
- Semaphore

禁止 Unbounded Audio Queue。

當後端追不上時：

- 優先保留較新內容
- 取消 Stale Work
- 必要時丟棄過時 Partial Work
- 不得讓字幕延遲無限增加

---

## 16. Audio Segment

```rust
pub struct AudioSegment {
    pub session_id: SessionId,
    pub sequence: u64,
    pub start_us: i64,
    pub duration_us: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: AudioBuffer,
}
```

所有非同步結果必須保留 Session / Sequence / Timestamp。

2026-10-24 Windows Process Loopback 的原生裝置位置可能持續為零；此情況以實際 PCM frame 數及原生 QPC 間隔共同驗證擷取相對位置，不能把零值誤判為每包音訊缺口。原生錯誤旗標、未知或倒退時間、真實 QPC 缺口、佇列／序號缺口仍須保留；不得用本機收到時間、無條件 frame 累加或補靜音偽造連續。此座標只屬擷取觀測，不代表媒體播放位置。見 [ADR 0090](docs/adr/0090-process-loopback-zero-position-clock.md)。

---

## 17. Session Identity

每次 Live / Prepared 工作建立唯一：

```rust
pub struct SessionId(pub u64);
```

必要時再加入 Generation。

舊 Session 的晚到結果不得污染新 Session。

---

## 18. Cancellation

以下情況必須可取消：

- 切換來源
- 停止翻譯
- 關閉程式
- Prepared Mode 取消
- Provider 重建
- 既有字幕取代 Live AI

不得留下 Orphan Task。

---

## 19. Streaming STT

優先使用 Streaming STT。

gpt-live-transcribe 接近既有四秒活動區段上限時，優先使用已觀察到的短停頓收尾：累計達 3.6 秒且連續低能量達 100 ms，可在當前包之後 commit；一般較早的停頓仍採 400 ms，無合適停頓仍於四秒硬上限切段，不延長等待或改動真正定稿對應。此為獨立前處理 policy，Gemini 維持原策略；不把能量停頓冒稱詞界或保證辨識完整度。見 [ADR 0109](docs/adr/0109-prefer-observed-pause-before-openai-turn-limit.md)。

OpenAI 活動起點前，保留同 Live Session 尚未傳送的 inactive PCM 尾端，最多八十毫秒與十二包；以原始 received 計齡，超過既有音訊年齡扣除傳送預算即淘汰。啟動時合併單一 append，已送內容不重播；來源上下文採最舊實際包，未知 start 保持未知，實際 duration 包含前綴 frames。服務回覆處理後讓出排程供 inline owner 消費，事件佇列仍有界且真正超量明確失敗。診斷區分輸入包、送出音訊訊息及合併前綴包；不推測遺失字詞。見 [ADR 0108](docs/adr/0108-openai-bounded-prefix-and-consumer-fairness.md)。

OpenAI gpt-live-transcribe 可在音訊提交確認前回覆 delta。未知 item 回覆只能在確有活躍或待提交音訊時，先以有界 Session RAM 暫存：沿用事件佇列上限六十四項、每項追加暫定合併及真正 Final 本文各最多 8 KiB，首份回覆八秒期限不得續期。真正 input_audio_buffer.committed 的相同 item_id 與已送音訊上下文核對後才交付，不猜測 item 順序、不阻擋後續 PCM、不虛構時間或定稿。OpenAI 格式錯誤加安全階段及固定代碼，其他 payload 不顯示；明選 Transcribe 也顯示純數值進度。見 [ADR 0107](docs/adr/0107-openai-early-transcript-correlation.md)。

Gemini 串流回覆可由 Text 或 Binary WebSocket 訊框承載 JSON；兩者都必須通過相同的大小、UTF-8／JSON、初始化及事件驗證。Binary 訊框不能直接視為音訊。沿用有界訊息、同一辨識／字幕管線、取消與 Session 隔離，無效或超量資料明確失敗。

Gemini Transcribe Live／Auto 另採既有本機能量端點要求服務收尾：四百毫秒低能量或四秒音訊區段後，在對應 PCM 之後送 audioStreamEnd，不必先收到轉錄。純靜音不反覆要求，與既有語言等待收尾去重；後續新鮮 PCM 仍持續送出。這不建立 utterance／定稿、語言或媒體時間證據，也不是實際延遲保證。明選語言及其他模型的收尾策略維持原樣，翻譯三秒總年齡不放寬。見 [ADR 0105](docs/adr/0105-auto-client-audio-endpoints.md)。

Gemini 的 Live 辨識前處理（含 Transcribe 的 Auto／明選及 Live Translate）對有效低音量 PCM 採有界區塊增益：RMS 低於 0.0002 或達 0.03 保持原樣，其他區塊以上限十六倍、目標 RMS 0.03 及放大後 peak 0.9 餘裕取最小增益，不衰減原音。先驗證再放大，之後重採樣與能量端點；不改原始擷取、Windows 音量、resolver／時鐘、Prepared 或 OpenAI Original 路徑。診斷明示套用封包數，沒有語音或降噪保證，實際錄音仍須測試。見 [ADR 0106](docs/adr/0106-bounded-live-recognition-input-gain.md)。

流程：

```text
Continuous Audio
      ↓
Partial STT
      ↓
Partial Translation
      ↓
Tentative Subtitle

Final STT
      ↓
Final Translation
      ↓
Stable Subtitle
```

---

## 20. Stable / Tentative

字幕內部至少區分：

```text
Tentative
Stable
Final
```

並支援：

```text
Stable Prefix
+
Mutable Tail
```

避免整句頻繁閃爍。

---

## 21. Translation Rollback

Live 的單次 Incomplete 翻譯與 Timeout／Stale 一樣，依完整目前工作身分退休並明示未完成定稿，允許後續新段落繼續；既有 Tentative 可保留但不當成 Final 或穩定脈絡。連續第三次未完成仍停止，真正完成並核對採用後才重設；一般 TranslationJob／Prepared 仍原子失敗。Incomplete 不計為音訊／翻譯年齡過期，已知 content_filter 為 Refused，憑證、格式、服務拒絕及原期限／上限不放寬。狀態警告與字幕歷史獨立，見 [ADR 0110](docs/adr/0110-isolate-incomplete-live-translation-requests.md)。

日文、德文、韓文及長英文句可能在句尾改變前文語意。

允許修改尚未 Commit 的尾端。

已 Stable 的內容盡量避免重寫。

2026-10-25 Live 的相容暫定原文若持續更新，不得反覆取消所有已啟動的翻譯。相同工作階段／段落／來源 metadata 且前綴相容時，每段以有界槽合併最新原文，先讓目前要求串流顯示中文；完成套用後再譯最新版本。Final、回退或來源變更仍立即失效舊要求，所有來源修訂／job 核對維持。合併槽沿用接收時的原翻譯期限，不延長等待，不用舊結果冒充新結果；停止或新 Session 不得重送舊槽。見 [ADR 0092](docs/adr/0092-live-partial-translation-coalescing.md)。

---

## 22. Translation Context

Standard / High 模式可使用：

- Previous stable sentences
- Current sentence
- Content metadata
- Speaker context（若未來可得）
- Session glossary

不得每句完全孤立翻譯。

---

## 23. Session Glossary

允許在 RAM 內建立暫態詞彙表：

- 人名
- 地名
- 專有名詞
- 組織名稱
- 作品術語

只存在目前 Session。

不建立永久使用者翻譯資料庫。

---

## 24. AI Provider 抽象

不得把核心綁死單一 AI Vendor。

```rust
pub trait SpeechRecognizer: Send + Sync {
    // provider-independent API
}

pub trait Translator: Send + Sync {
    // provider-independent API
}

pub trait ContentIdentifier: Send + Sync {
    // provider-independent API
}
```

Provider-specific request / response 留在 Provider 模組。

---

## 25. Provider Capability

應明確描述：

- streaming_stt
- partial_transcript
- timestamps
- word_timestamps
- language_detection
- cancellation
- streaming_translation

核心依 Capability 決定策略。

---

## 26. API Concurrency

不得無限制平行呼叫雲端服務。

使用：

- Semaphore
- Concurrency Limit
- Provider Rate Limit

考量：

- 速率限制
- 成本
- 網路
- 排序
- 延遲

---

## 27. Retry

區分：

### Retryable
- temporary network failure
- connection reset
- HTTP 429
- 部分 5xx

### Non-retryable
- invalid credential
- permission denied
- unsupported media
- invalid request

使用有界 Exponential Backoff + Jitter。

---

## 28. Provider Failover

不得在使用者不知情下，將私人音訊由一個 AI Provider 自動改送另一個 Provider。

第一版自動復原只限同一 Provider 重連。

2026-10-25 Live 的單次翻譯 Timeout／Stale 不立即終止整個辨識連線：維持原三秒總年齡，依完整工作身分退休該要求及拒收未套用輸出，不補送過時文字。待譯及合併槽過期亦捨棄，記錄修訂水位避免舊段重新取得期限；後續新修訂／新段可繼續。狀態區明示捨棄次數及後續恢復，已顯示 Tentative 不冒充完成。連續第三次期限失敗仍交回原共用有限復原；只有有效 complete 套用才重設連續計數，其他錯誤及 Prepared 契約保持。見 [ADR 0096](docs/adr/0096-live-translation-deadline-isolation.md)。

---

## 29. Content Resolver

```rust
pub enum ContentType {
    Unknown,
    Speech,
    Music,
    MovieOrShow,
    Mixed,
}
```

Resolver 必須與 Live STT 平行工作。

不可先等待歌曲辨識才開始 STT。

---

## 30. Subtitle Source

```rust
pub enum SubtitleSource {
    LiveAi,
    ExistingSubtitle,
    ExistingLyrics,
    PreparedTranslation,
    Hybrid,
}
```

所有來源必須先進入 SubtitleSourceArbiter。

---

## 31. Subtitle Source Arbitration

依：

- Confidence
- Alignment
- Freshness
- Availability
- Translation quality

選擇 Active Source。

不得讓 Resolver 各自直接操作 UI。

---

## 32. 字幕來源優先順序

一般偏好：

1. Prepared verified text
2. Verified native zh-TW
3. Other Traditional Chinese → Taiwan localization
4. Simplified Chinese → zh-TW localization
5. Source-language subtitle → zh-TW translation
6. Live AI

但低品質或時間軸不符的既有字幕不得強制取代 Live AI。

---

## 33. Hybrid Mode

可處理：

- Existing subtitle 缺句
- 歌曲中主持人講話
- 字幕局部錯位
- 未收錄片段
- Transition

用 Live AI 補洞。

最終仍輸出單一 TranscriptTimeline。

---

## 34. 歌曲辨識

使用 Sliding Window Audio Fingerprint。

概念：

- Window：約 8–15 秒
- Interval：約 2–5 秒
- 實際值需 Benchmark

使用：

- candidate voting
- confidence threshold
- hysteresis

避免組曲切換時反覆跳動。

---

## 35. 組曲 / DJ / Crossfade

允許：

```text
Song A
→ Transition / Mixed
→ Song B
```

而不是硬辨識。

Transition 可回退 Live AI。

---

## 36. 電影 / 影集識別

不假設有完整全球影音 Fingerprint Database。

多線索：

- Window Title
- Filename
- Process
- ASR Dialogue
- Language
- Duration
- Subtitle metadata
- Remote metadata

只有在 Confidence 與 Timeline Alignment 足夠時，才自動採用既有字幕。

---

## 37. 字幕定位

使用：

```text
Current ASR
→ Fuzzy Match
→ Existing Subtitle
→ Current Timestamp
```

不得假定播放從 00:00 開始。

---

## 38. Drift Correction

需能處理：

- 不同發行版本
- 額外片頭
- Blu-ray / DVD 差異
- Streaming Cut
- Director's Cut
- 廣告插入
- 小幅時間漂移

可週期性重新校時。

---

## 39. Prepared Mode

接受：

```text
Local Media File
HTTP(S) URL
```

用途是「預先準備字幕後播放」，不是字幕保存工具。

---

## 40. Prepared Mode 優先順序

1. 內嵌 / 現成 zh-TW
2. 其他繁中 → 臺灣本地化
3. 簡中 → zh-TW
4. 原文字幕 → AI 翻譯
5. 無字幕 → ASR → AI 翻譯

不要有可靠字幕仍做完整 ASR。

---

## 41. Prepared Mode 流程

```text
Media
↓
Metadata
↓
Subtitle Discovery
↓
Existing Subtitle?
├─ Yes → Normalize / Localize / Translate
└─ No  → Audio Extraction → ASR
                          ↓
                    Full Transcript
                          ↓
                   Context Analysis
                          ↓
                       Glossary
                          ↓
                    Translation
                          ↓
                  Consistency Pass
                          ↓
                  Timeline Validation
                          ↓
                        Ready
```

---

## 42. Prepared Mode 多執行緒

可以更積極利用 CPU / 雲端並行：

```text
Segment A → Worker 1
Segment B → Worker 2
Segment C → Worker 3
Segment D → Worker 4
```

但每段要保留：

- 前文
- Overlap Context
- Global Glossary
- 後文提示

最後必須做 Merge / Consistency / Timeline Validation。

---

## 43. Prepared Mode 完成後

完成後：

- Windows Notification
- 顯示 Ready
- 由使用者手動開始播放
- 不自動播放

播放開始後重新透過音訊 / ASR 對齊目前位置。

---

## 44. Prepared Mode Seek

需支援：

- Pause
- Seek forward
- Seek backward
- Chapter jump
- Restart

不得只用 Wall Clock 推進字幕。

---

## 45. URL 限制

第一版只接受：

```text
http://
https://
```

不得實作：

- DRM bypass
- Paywall bypass
- Credential theft
- Access-control bypass

若無法預處理：

```text
Prepared Mode 不可用
仍可改用 Live Mode
```

---

## 46. 不保存音訊

硬性規格：

- 無 Save Audio
- 無錄音模式
- 無 Export WAV
- 無 Export PCM
- 無永久 Audio Cache
- 無 Audio History

Audio Buffer 必須：

```text
Bounded
In-memory
Short-lived
Session-only
```

---

## 47. 不建立永久內容 Cache

禁止永久保存：

- Audio
- ASR Transcript
- Translation
- Lyrics
- Subtitle
- Fingerprint
- Content identification
- Alignment result
- Remote metadata

不得因效能理由自行重新加入 Disk Cache。

---

## 48. 網路字幕 / 歌詞

只能：

```text
Current Session
```

使用。

可以在 RAM 暫存以避免同一 Session 重抓。

程式退出後全部失效。

---

## 49. Temporary File

僅在媒體工具技術上必須時使用 OS Temp。

要求：

1. Unique app-owned directory
2. 完成即刪除
3. Cancel 即刪除
4. Error 即刪除
5. 下次啟動可安全清理本程式遺留 Temp
6. 不得視為 Cache

---

## 50. 不提供永久字幕匯出

第一版不提供：

- Export SRT
- Export VTT
- Save translated subtitle archive

Prepared Translation 僅供本次 Session 播放。

---

## 51. 顯示模式

```rust
pub enum DisplayMode {
    SideChat,
    BottomSubtitle,
    Dual,
}
```

三種全部實作。

預設：

```rust
DisplayMode::SideChat
```

---

## 52. SideChat

主視窗底部顯示「KOSMOS TOOLKIT 探真拓知酷」，使用系統訊息字型並允許窄版換行；視窗過矮時優先保留字幕閱讀空間，暫隱品牌列。品牌文字獨立於字幕與狀態，不進入搜尋或處理管線。

原生主視窗採淺灰藍背景、白色字幕區與淡藍狀態區；主要操作使用藍色圓角按鈕，次要操作白底，保留停用與鍵盤焦點提示。文字框增加內距，維持系統訊息字型、既有字級與 DPI 配置。高對比模式使用 Windows 系統色，系統主題變更只更新 UI，不重啟處理、不更動來源、服務或字幕紀錄。設定頁原位展開、主頁操作與狀態分框保持。見 [原生 UI 外觀更新](docs/ui-refresh.md)。

預設先定位右側後保持自由移動，可改為靠左／靠右。初始外框高度為所在螢幕實體解析度高度的一半，寬度為該高度的三分之一；受可用工作區限制，不再額外乘 DPI。側邊視窗文字使用該螢幕 DPI 的 Windows 系統訊息字體與尺寸。

窄字幕視窗預設為字幕主頁，固定顯示來源、來源語言、品質、開始翻譯、停止與清除字幕。來源／語系選擇在設定頁亦保留，繁忙時維持鎖定。選單分為「字幕與媒體設定／回到字幕主頁」及獨立「服務與金鑰設定／回到字幕主頁」；服務頁可明確儲存所選模型、來源語言及品質，金鑰仍由各列按鈕儲存至 Windows 認證管理員，不放入一般偏好檔。設定於目前位置展開、必要時限制於工作區，保留原字幕外框，切回時還原；不重啟處理或改換來源。340 DIP 以上主操作三顆同列，更窄時開始全列、停止與清除同列。「系統狀態與錯誤」只在字幕主頁顯示，設定頁不重複；按開始或無音訊診斷先回主頁，來源與模型必須實際顯示後才通過雲端啟動閘門。底部字幕的既有獨立外觀選項保持。見 [ADR 0097](docs/adr/0097-main-actions-service-settings-and-history-follow.md) 與後續 [ADR 0098](docs/adr/0098-live-model-profiles-native-captions-and-main-source-controls.md)。

支援：

- 原文
- 臺灣繁中翻譯
- 時間戳
- 本次應用程式開啟期間的有界 RAM 字幕紀錄
- Auto Follow
- 手動回看
- 搜尋
- 回到即時字幕

Tentative 字幕只更新最底部目前處理區，不為每個 Token 新增一個泡泡。

當同段原文修訂觸發整段重譯，已有可讀中文時保留舊暫定譯文，等下一版翻譯完整輸出再原子替換，避免串流開頭短字串蓋掉長段中文。第一版無中文時仍即時顯示 partial；暫定內容不冒充穩定或定稿，修訂／Session／job 與 UTF-8 驗證不放寬。見 [ADR 0093](docs/adr/0093-readable-draft-replacement-and-compact-side-defaults.md)。

主視窗將「系統狀態與錯誤」與字幕顯示於獨立原生框架。狀態、來源與診斷不插入字幕本文，不參與字幕搜尋或改動字幕捲動錨點。依 2026-10-25 使用者指示，停止、錯誤、換來源、重連或重新開始只取消／清理後端工作，不再自動清空已交付顯示的字幕。先前連線的字幕標示「先前字幕」，不當作目前辨識結果或定位證據；後續新段落往上捲動。只有使用者按「清除字幕」才清空顯示紀錄，關閉應用程式釋放 RAM。見 [ADR 0095](docs/adr/0095-user-owned-bounded-subtitle-display-history.md)。

來源暫停播放或不再交付音訊，不等同使用者停止工作階段。正常暫停時繼續處理已收到的原文及翻譯，不因音訊閒置清空字幕。Gemini 在真正輸入／前處理／傳送佇列已排空、資料中斷超過一秒時，用一次 `audioStreamEnd` 請服務排出最後結果；恢復音訊可使用原連線，仍受原有 Final 等待、心跳與連線期限限制。不得補造靜音、Final 或擷取／播放時間；後端取消與資源清理保持，顯示紀錄依使用者手動清除。音訊收尾見 [ADR 0094](docs/adr/0094-gemini-idle-input-drain-and-caption-retention.md)，後續顯示保留以 ADR 0095 為準。

新尾段原文尚無中文時，BottomSubtitle 延用顯示紀錄中最近有中文的段落，中文＋原文必須屬同一段；非目前處理 Session 的內容前置「先前字幕」。依 2026-10-25 使用者後續回報最後一段不可見，SideChat 顯示最新原文待譯段，並正常跟隨底部，不再隱藏尚無中文的尾段。已有中文仍保留於各自段落，不與新原文錯配；手動回看、64 筆有界紀錄及使用者清除政策保持。

Live 模型可明確選擇 `gpt-live-transcribe`、既有 `gemini-3.5-transcribe-live` 或 `gemini-3.5-live-translate-preview`。舊偏好仍使用原供應商預設模型，不自動改用新模型。GPT 使用既有有界 STT → 翻譯流程，Fast／Standard／High 分別設定真實辨識 delay minimal／low／medium，翻譯策略保持。該模型未提供語言預測，因此須明選原支援五語之一，Auto 明確拒絕。

模型能力須在主頁啟動前清楚呈現。只有本程式目前支援五語自動偵測的 Gemini Transcribe Live 提供語言第一項「自動偵測」；Live Translate 與 GPT 的同一索引顯示「請手選語言」。切換模型保留既有語言索引，不改成英文、不改模型、不清金鑰，也不自動寫回偏好。尚未選定必要語言或選到其他不支援品質時，停用開始並將原因置於狀態最前方；直接發出開始／診斷命令亦須在讀取欄位金鑰或建立後端前核對。固定儲存／檢查本機仍可獨立操作。此為目前應用程式整合的能力界線，不宣稱 Live Translate 服務本身沒有語言偵測能力。見 [ADR 0100](docs/adr/0100-live-option-capability-preflight.md)。

Gemini Live Translate 為明確「中文串流／僅譯文」模式：使用原 16 kHz PCM 串流、官方 AUDIO 輸出模式及繁中目標 `zh-Hant`，只取文字 sidecar 並進行臺灣用語在地化，合成語音不解碼、不播放、不保存。原文與譯文沒有可靠共用段落 ID，故不虛構原文配對、來源語言偵測、擷取時間或播放定位。譯文經具名有界通道進入同一 TranscriptTimeline，保留 Session／generation／sequence／revision、取消、手動清除及歷史容量。僅真正服務完成標記能定稿；中斷不得冒充 Final。Fast／Standard 實際採用不同合併間隔，High 因無可核對原文精修而明確不可用；Auto、已準備字幕定位及獨立內容服務組合亦明確拒絕，不自動改模型。需選原支援五語之一；服務提供來源語言代碼時須核對，缺少代碼不補造。此模式不作歌曲／影片或既有字幕定位證據。模型權限、費用與真實速度須以使用者固定遮蔽欄位及所選自有測試音訊驗收。見 [ADR 0098](docs/adr/0098-live-model-profiles-native-captions-and-main-source-controls.md)。

---

2026-10-26 主頁狀態標題「狀態／錯誤（展開）」與選單可開啟單一可調整大小的狀態視窗，寬、高可獨立拖曳，關閉只隱藏且重開保留本次尺寸。主頁原預覽保留，設定頁不重複；視窗僅鏡映狀態，不加入字幕或變更處理／捲動。主程式關閉釋放 HWND／字型，不保存狀態本文。見 [ADR 0101](docs/adr/0101-gemini-auto-language-metadata-and-status-details.md)。

## 53. SideChat History

只存在 RAM。

有固定上限。

達上限後丟棄最舊項目。

顯示歷史總上限沿用 64 筆（原文／譯文沿用原容量限制），超過時逐筆淘汰最舊項目，不整批清空。歷史列以完整 Session／generation、sequence、utterance 區分，不讓跨連線重複段落編號互相覆寫；晚到的舊 Session 結果仍拒收。歷史只供顯示／搜尋，不供 ASR、翻譯上下文或來源仲裁重用。

手動回看時，持續更新、重連或淘汰其他項目應保留目前仍存在的字幕位置與選取；已淘汰項目移至最早仍保留項目。搜尋／手動捲動不因停止或重連重設。「清除字幕」按鈕在字幕視窗固定可見，選單亦可操作；清除會重設搜尋、選取與捲動，不停止處理，已清除段落不得因重複快照或同段修訂重新補回，後續新段落照常顯示。回到即時字幕須實際捲至最新內容。

原生 EDIT 重排／程式捲動通知不得自行停用自動跟隨；僅由該控制項的實際滑鼠、捲軸或導覽鍵操作進入手動回看。持續更新跨過 64 列淘汰後仍須跟隨最新內容，恢復跟隨需抵達原生捲動範圍底部。

2026-10-25 後續修正：捲動結束不等於離開底部。原生控制項完成實際操作後，若位於 32 位元捲動範圍底部、沒有文字選取且未搜尋，恢復自動跟隨；離開底部、文字選取及搜尋仍保護人工回看。跟隨動作在版面／換行完成後，以完整 UTF-16 結尾與實際 visual line count 捲到底，不依賴焦點或 16 位元捲軸位置。大量文字、64 列淘汰及設定頁返回後須保持最後有意義文字行可見。見 [ADR 0099](docs/adr/0099-native-caption-snapshots-and-bottom-follow.md)。

Live Translate 每次輸出是完整文字快照，不是一般翻譯要求尚未完成的 delta。後續暫定快照須可替換前一快照，不能被一般 ASR 整段重譯的可讀性保護擋到真正 Final 才顯示。快照完整不等同段落定稿，穩定性仍為 Tentative，只有真正服務完成標記能 Final；既有 ASR 重譯保護、三秒新鮮度、來源／Session／版本與容量限制保持。見 ADR 0099。

不得寫入磁碟延伸歷史。

---

## 54. Bottom Subtitle

支援：

- Always-on-top
- 拖曳
- 鎖定
- Click-through
- 字型
- 字級
- 描邊
- 陰影
- 背景透明度
- 最大行數
- Monitor 選擇
- 原文 + 中文
- 中文 Only

預設以 zh-TW 為主。

---

## 55. Dual Mode

同時顯示 SideChat + BottomSubtitle。

兩者共用：

```text
Single TranscriptTimeline
```

不得重複執行 STT / Translation / Resolver。

---

## 56. 多螢幕與 DPI

UI 必須 Per-monitor DPI aware。

至少測：

- 100%
- 125%
- 150%
- 175%
- 200%

支援：

- Primary monitor
- Specific monitor
- Follow source window

---

## 57. Fullscreen

盡可能支援：

- Windowed
- Borderless fullscreen

Exclusive fullscreen 不保證支援。

不得為此加入侵入式遊戲 Hook。

---

## 58. 狀態機

建議：

```rust
pub enum AppState {
    Idle,
    SourceSelected,
    StartingLive,
    LiveRunning,
    PreparingMedia,
    PreparedReady,
    PreparedPlaying,
    Stopping,
    Recovering,
    Error,
}
```

避免大量互相矛盾 Boolean。

---

## 59. 狀態顯示

UI 應能顯示：

- 待命
- 正在聆聽
- 正在辨識
- 正在翻譯
- 使用現成字幕
- 預先翻譯中
- 重新連線
- 已停止
- 錯誤

---

## 60. 隱私

必須：

- 清楚顯示目前擷取來源
- 提供停止功能
- 不偷偷擷取未選來源
- 不永久保存原始音訊
- 不永久保存字幕 / 翻譯 / 歌詞 Cache
- 預設不保存完整 Transcript Log
- Cloud AI 使用時應讓使用者知道

---

## 61. API Key

不得 hard-code。

優先：

- Windows Credential Manager
- DPAPI
- 其他 Windows 安全儲存

一般 config 只保存 Credential Identifier。

2026-10-09 使用者要求固定的金鑰輸入介面。主視窗須固定提供 OpenAI（ChatGPT）與 Gemini 的獨立遮蔽 API 金鑰欄位，不能將切換服務當成共用欄位的金鑰轉移。輸入預設只供本次啟動；使用者明確按下對應「儲存」才寫入 Windows 認證管理員，不寫一般設定或日誌，也不自動連線。開始只取用所選服務的金鑰，清空輸入及復原紀錄；留空時沿用該服務的安全憑證。有效金鑰與真實服務驗收仍留至結案手動操作。

2026-10-23 依使用者對儲存狀態的回報，各服務列提供獨立「檢查本機」按鈕；只在明確按下時讀取該服務的安全憑證以確認可讀性，結果只回傳狀態，不回填金鑰、不修改未儲存輸入、不啟動擷取或雲端連線。主視窗須核對同一服務的寫入後讀回才顯示儲存成功；寫入完成但讀回無法核對，應顯示「尚未確認」，不能誤稱「未儲存」或成功。儲存後欄位清空是保護措施，留空仍可使用該服務的已存金鑰。本機檢查不能代表服務端權限驗證成功；實際雲端結果由連線狀態呈現。

---

2026-10-24 針對 Gemini 初始化失敗，主視窗提供明確「Gemini 診斷（連線、不傳音訊）」；只有使用者按下才向 Google 查詢固定模型資訊並嘗試一次既有串流初始化。不需音訊來源、不建立擷取、不傳音訊／字幕、不自動開始翻譯、不切模型或服務。只取用 Gemini 欄位並清空該欄與 undo；留空沿用該服務安全憑證，保留另一服務未儲存輸入。沿用目前來源語言；兩階段結果分開，模型資訊可讀不代表串流權限或辨識／翻譯驗收通過。查詢最多八秒、回應最多十六 KiB，整次共用三十秒期限；單一既有 worker、不可重入、可停止取消、不重試、不存結果。TLS 驗證保持，不跟隨重新導向，錯誤只顯示安全階段及數字代碼。

## 62. Telemetry

第一版預設：

```text
No telemetry
```

不得自動上傳：

- Audio
- Transcript
- URL
- Source app name
- Subtitle content

---

## 63. Logging

支援：

- ERROR
- WARN
- INFO
- DEBUG
- TRACE

Release 預設 INFO。

禁止 Log：

- API Key
- Authorization Header
- Raw PCM
- Signed URL credential
- 完整私人 Transcript（預設）

---

## 64. Error Handling

至少處理：

- Audio Source Closed
- Audio Device Changed
- Network Lost
- AI Timeout
- Rate Limit
- Invalid Credential
- Malformed Response
- Unsupported Codec
- Subtitle Mismatch
- Remote Service Failure

Runtime 可恢復情況不得大量 `unwrap()` / `expect()`。

2026-10-24 音訊過期或不連續的錯誤須保留安全的固定階段與原因，區分送入／取出辨識佇列、重採樣、準備傳送／傳送佇列及等待 Gemini 上一段辨識完成。可指出期限、區塊序號、取樣位置或擷取端不連續旗標；不得附帶 PCM、辨識本文、金鑰或來源私有資訊。診斷不放寬原音訊期限、取消、Session 隔離或共用有限重試規則；只標示實際觸發的判定，不以本機模擬情境宣稱真實錯誤根因。

2026-10-24 取出端的彙總不連續旗標不得掩蓋已觀測的序號或取樣位置缺口。錯誤時可附有界數值摘要：前後序號、前包 frame 數、相對預期結尾的位置差、相鄰 QPC 間隔及旗標；未知資料須明示，不顯示絕對 QPC、音訊或辨識本文。摘要僅供診斷，不改變音訊拒收、復原預算或擷取／媒體定位界線。

2026-10-24 Gemini 轉錄以 `inputTranscription` 作為最終辨識結果；已送本段 activityEnd 後，不得再因缺少對話代理的 `turnComplete` 而永久阻擋後段音訊。仍不得在前段最終轉錄未到時混用段落身分，也不延長音訊期限。佇列已滿須明示具名邊界與容量，維持原根因、有限復原、Session 與容量。錯誤／重連須明示實際狀態，不用「等待字幕」掩蓋中斷；依使用者後續決定保留的顯示紀錄標示為先前字幕，不接受晚到結果為新工作成果。實際重現與驗證見 [Phase 11 診斷紀錄](docs/phase-11-gemini-diagnostics.md)。

2026-10-24 後續人工回報仍為傳送佇列滿，Gemini 改採連續音訊／服務自動斷句，取代前述本機 activityEnd 後等待 Final 的屏障。服務回覆段落識別僅供文字排序；未取得精確擷取關聯時，起點／長度保持未知，不把回覆時間或遞增段落號當作擷取／媒體定位。未知時間仍可進行即時原文與 zh-TW 翻譯，依賴完整時間的精確定位不得採用。250 ms 音訊期限、所有容量、取消、共用有限復原及原生時鐘保持。設計見 [ADR 0091](docs/adr/0091-gemini-continuous-transcription.md)。

---

## 65. unsafe 規範

Win32 / COM / WASAPI 可能需要 `unsafe`。

要求：

- Small
- Encapsulated
- Documented
- Reviewed

重要 `unsafe` block 前：

```rust
// SAFETY:
```

說明：

- Pointer validity
- Buffer bounds
- Ownership
- Lifetime
- COM lifetime
- Thread requirements

---

## 66. COM / Handle

- 正確初始化 COM apartment
- 不非法跨執行緒傳遞 COM 介面
- 優先 RAII
- HANDLE / event / audio client 必須有清楚 ownership
- 重複切換來源不得造成 Handle leak

---

## 67. 設定可永久保存

一般 UI / Preference 可以保存：

- 視窗位置
- 字型
- 字級
- 透明度
- SideChat 寬度
- Display Mode
- Translation Quality
- Source Language preference
- Monitor preference
- Provider selection

設定 schema 2 可儲存可選的 Live 模型，與供應商必須一致。schema 1 僅在記憶體遷移至 2，模型留空沿用舊供應商預設；明確「儲存服務與偏好」才寫回。不保存來源 PID、API 金鑰或媒體內容；未知欄位、未知模型、供應商不一致仍拒絕，保留 16 KiB 上限與原子寫入。

但不得包含：

- Transcript
- Subtitle text
- Lyrics
- Translation output
- Fingerprint
- Audio

---

## 68. 效能監控

Debug Mode 可觀察：

- Audio queue depth
- STT queue depth
- Translation queue depth
- Capture latency
- STT first partial
- STT final
- Translation first partial
- Translation final
- UI latency
- Dropped frames
- Dropped stale segments
- Active workers
- API concurrency
- Reconnect count

---

## 69. 延遲目標

工程目標，不是 SLA。

Fast / Standard：

```text
First useful subtitle < 1 second
```

理想：

```text
300–700 ms
```

UI 更新：

```text
Translation ready → visible < 100 ms
```

---

## 70. 測試

Unit：

- Language mapping
- zh-CN → zh-TW
- Stable Prefix
- Rollback
- Subtitle segmentation
- Timeline ordering
- Fuzzy matching
- Config
- Backpressure
- Session generation filtering
- Source arbitration

Integration：

- Mock Audio
- Mock STT
- Mock Translator
- Mock Resolver
- Reconnect
- Cancellation
- Source switch
- Out-of-order response

Manual：

- VLC
- PotPlayer
- Chrome
- Edge
- Spotify
- Windows Media Player / Media Player
- 至少一款遊戲

---

## 71. Display Test Matrix

測試：

- SideChat
- BottomSubtitle
- Dual

DPI：

- 100%
- 125%
- 150%
- 200%

Monitor：

- 單螢幕
- 雙螢幕
- 不同 DPI 雙螢幕

---

## 72. Long-running Test

檢查：

- Memory growth
- Thread growth
- Handle leak
- COM leak
- Queue growth
- Network reconnect
- Timeline trimming
- Temp cleanup

---

## 73. 建議模組

```text
src/
├── main.rs
├── app/
│   ├── state.rs
│   └── config.rs
├── audio/
│   ├── sessions.rs
│   ├── capture.rs
│   ├── process.rs
│   ├── ring_buffer.rs
│   ├── resample.rs
│   └── vad.rs
├── media/
│   ├── source.rs
│   ├── local.rs
│   ├── remote.rs
│   └── metadata.rs
├── resolver/
│   ├── content.rs
│   ├── music.rs
│   ├── movie.rs
│   └── fingerprint.rs
├── ai/
│   ├── provider.rs
│   ├── stt.rs
│   ├── translate.rs
│   ├── stream.rs
│   └── glossary.rs
├── subtitle/
│   ├── timeline.rs
│   ├── segment.rs
│   ├── stabilize.rs
│   ├── align.rs
│   └── formats.rs
├── session/
│   ├── state.rs
│   ├── transcript_store.rs
│   └── subtitle_store.rs
├── ui/
│   ├── main_window.rs
│   ├── source_picker.rs
│   ├── side_chat.rs
│   ├── bottom.rs
│   └── settings.rs
└── win32/
    ├── audio.rs
    ├── process.rs
    ├── window.rs
    └── notification.rs
```

不得新增永久 `cache/` 子系統。

---

## 74. Phase 0 — Skeleton

完成：

- Cargo project/workspace
- 模組
- logging
- config
- typed errors
- test framework
- CI

驗收：

```bash
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

---

## 75. Phase 1 — Audio Session Discovery

完成：

- 列出音訊 Session
- Process/PID
- Window title
- Icon
- Audio activity
- Volume meter
- 動態出現 / 消失

---

## 76. Phase 2 — Process Capture

完成：

- 指定來源
- Process Loopback
- PCM
- Ring Buffer
- Audio normalization
- Disconnect handling
- Multi-thread pipeline skeleton

---

## 77. Phase 3 — Streaming STT

2026-10-08 已依使用者指示啟動；首批供應商選項為 OpenAI 與 Google Gemini。共用介面、實作與實際雲端驗收的進度分開記錄於 [Phase 3](docs/phase-3.md)，不以 mock 通過代表實際辨識完成。

完成：

- Partial
- Final
- EN / JA / FR / DE / KO
- Cancellation
- Session isolation

---

## 78. Phase 4 — Translation

2026-10-09 已接續啟動，共用核心與正式供應商接線分開記錄於 [Phase 4](docs/phase-4.md)。依使用者安排，有效 API 金鑰在結案測試時才以本機互動提示手動輸入；不以缺少金鑰阻擋本階段的離線實作與驗證。

完成：

- 五語 → zh-TW
- Fast / Standard / High
- Stable / Tentative
- Rollback
- Session glossary

---

## 79. Phase 5 — UI

2026-10-09 依使用者「繼續製作」接續啟動。共用顯示層、來源圖示／活動、左右停靠、視窗來源／服務／語言／品質控制、字幕外觀、主螢幕／指定螢幕／跟隨來源所在螢幕與重新啟動進度見 [Phase 5](docs/phase-5.md)。DPI 實機／視覺、應用程式矩陣與真實服務驗收仍待完成；不以模型測試代表即時字幕品質已驗收。

完成：

- SideChat
- BottomSubtitle
- Dual

預設：

```text
SideChat
```

Display 切換不得中斷後端。

---

## 80. Phase 6 — Content Resolver

2026-10-09 使用者明確批准啟動。核心、本機指紋、有界音訊分流／缺口重設、LRCLIB 歌詞 adapter、電影 metadata 交叉核對、已確認歌曲到有界文字查詢的生命週期、正式 Live 的可選內容分支與 GUI 處理狀態、同步取消／重設清除與未讀結果過期、工程初值與待接線服務見 [Phase 6](docs/phase-6.md)；Phase 5 尚待的實機／雲端驗收另列，不能視為已通過。此階段不自動啟動未設定的內容服務，也不以 mock 或候選 metadata 宣稱已識別實際歌曲／影片。

完成：

- Speech / Music / Mixed / Unknown
- Fingerprint
- Song Candidate
- Existing lyrics abstraction
- Movie metadata resolver
- Existing subtitle abstraction

Resolver 不得阻塞 Live STT。

---

## 81. Phase 7 — Existing Subtitle Integration

2026-10-10 依使用者要求銜接 Phase 7；先修復 Phase 6 中斷的歌詞查詢接線與測試，再加入原文／時間資訊保留、中文臺灣用語本地化及有界 CPU 工作。第二批提供五語來源字幕翻譯與品質／前文策略；第三批提供依明確定位證據的仲裁；第四批接入 LiveTranslation、單一時間軸與共用 UI 快照；第五批串接 Live 的查詢文字、候選準備及仲裁提交，加入跨階段取消與確認信心上限；第六批提供由最終 ASR 與完整來源 cue 計算的保守唯一文字／時間驗證；第七批接入有界 Live／GUI 候選排程，使用實際擷取游標採用，補齊最新替代、取消、重複抑制與狀態提示；第八批加入中文本文與 provider 聲明同版本來源參考軌的原子出版物，以來源 ASR 唯一完整命中及中文 cue 精確時間對應產生證據，沿用本機本地化、共用期限及出版物隔離；第九批提供明確配置的 HTTPS SRT／WebVTT 純文字 adapter，同一份有界要求原子交付主軌與可選參考軌，保留原始時間、期限與取消；2026-10-11 第十批加入可靠電影候選的字幕管理、Session metadata 核對及 Live／GUI 服務提示，字幕與歌詞共用一個候選 task 和原有時間軸。第十一批接入明確啟用的當前 Final ASR 電影線索分流，以完整原生時間落於實際捕捉視窗、Session metadata 全 stamp 核對及提示取消保護查詢，啟動前呈現對白／媒體線索用途。第十二批提供明確配置的所選程序唯一可靠標題來源，核對建立身分、觀測原期限及取消，每次 Session 使用獨立有界 native 工作，不推論父程序或其他視窗。第十三批提供一筆歌曲／電影的 Session 文字 metadata 管理、完整身分與發布水位核對、歌詞跨 Session 拒收、版本更新重新綁定及父到子取消；舊資料撤回會取消進行中查詢與已完成出版物，相同資料不重抓。第十四批提供一般 GUI 的本次已知影片來源字幕設定，正式 HTTPS document 由本機對白核對與字幕 provider 共用，完整來源 cue 的唯一一致率產生候選，仍經可靠程序標題及精確標點／時間定位才採用；原始語音／對白不送字幕網站。使用者暫緩外部歌曲辨識服務；正式全球電影目錄與中文可靠配對資料仍待完成。進度與未完成項目見 [Phase 7](docs/phase-7.md)。中文保守配對證據核心已實作，正式來源服務／真實配對資料配置與媒體驗收仍待完成，進階 fuzzy／seek／drift 留於 Phase 9；Phase 6 正式服務／實機驗收保留待辦，金鑰維持結案手動輸入。第十四批驗證已通過，已知字幕取得、核對及共用顯示入口可支援後續 Prepared Mode，具備啟動 Phase 8 的條件；正式外部目錄與實機／雲端驗收仍列待辦，本批不直接啟動 Phase 8。

完成：

- zh-TW
- Other Traditional Chinese → Taiwan localization
- Simplified Chinese → zh-TW
- Source subtitle → translation
- Session-only remote data

---

## 82. Phase 8 — Prepared Mode

2026-10-11 使用者明確批准啟動。首批提供本機媒體 handle 與明確外掛 SRT／WebVTT 字幕的有界讀取、Prepared 獨立解析 scope、繁中／簡中臺灣本地化與 RAM Ready owner，沿用 Live 的原有容量／期限並共用唯一 Session allocator。第二批提供五語來源字幕全文翻譯，限制並行要求、保留前後文及目前 Session 詞彙表、還原順序；Standard／High 完成全文草稿後另做鄰近草稿一致性校訂。Prepared 逐要求使用獨立三十秒上限，等待服務時釋放 CPU 許可；取消或失敗不提交部分 Ready。2026-10-12 第三批新增一般 GUI 的本次準備視窗、固定遮蔽金鑰與服務目的地、單一可回收工作執行緒、有界結果通道、Ready／取消／清除及 Windows Shell 完成通知；主視窗與準備視窗的開始操作互斥，關閉釋放本次文件，不自動播放。

2026-10-12 第四批以目前 Windows 19045 為基準，新增本機 RIFF/WAVE 的 Media Foundation library 解碼：從唯讀 pinned handle 及自訂 IStream 建立來源，協商 PCM16 並保留原生取樣率、聲道及媒體時間，單一 native sample 限一 MiB、交付 block 限六十四 KiB。解碼與字幕讀取／本地化共用依可用核心數限制、最多兩項工作的 CPU lane；拉取與消費同步、有界且可合作取消，已開始工作持有許可至真正退出。每個 block 核對 Session／序號／格式／媒體時間；不使用 Live capture stamp 或重設時間規避期限。音訊只在本次 RAM，不保存、上傳或播放。解碼尚未接入 GUI 或 Prepared ASR，現有外掛字幕 GUI 不會觸發解碼，Ready 仍只代表文件準備完成，尚非媒體配對或播放定位證據。MP3／AAC／影片容器、內嵌字幕、URL 實際讀取、ASR、自動全文專名／glossary 建立及播放仍待接續。進度見 [Phase 8](docs/phase-8.md) 與 [ADR 0039](docs/adr/0039-bounded-local-media-decoding.md)，Phase 9 尚未具備啟動條件；金鑰仍留至結案固定介面手動輸入。

2026-10-12 第五批新增獨立的 PreparedAudioProcessor／prepare_local_audio library：本機 RIFF/WAVE 原生 PCM16 等權混合一至八聲道，明確轉為 16 kHz 或 24 kHz 單聲道，每次交付最多二十毫秒及 EOF 尾段。257 taps／256 phases 的 centered Blackman windowed-sinc FIR 使用固定 526,336 bytes 係數表與 2,056 bytes history、128 native frames lookahead；同率 bypass 不配置 filter。每個連續區段依原生起點及累積 frame 計時，核對一微秒取整差並拒絕累積漂移，gap 先完成前區段再重設 filter／clock，不補合成靜音。首尾 extension 只供 FIR 計算，總數為 floor(native_frames * output_rate / input_rate)，尾端少於一個 target frame，不超出原區段；不足一個 output frame 回報明確錯誤。私有 EOF-aware consumer 與 native 解碼、flush、銷毀共用同一 blocking worker／CPU 許可，無音訊佇列；過載、取消、舊身分或失敗不交付部分摘要。未新增 crate、unsafe、安裝或持久化音訊。此摘要仍不是字幕 Ready 或播放定位證據，沒有 GUI ASR、服務連線、音訊上傳或播放，可靠外掛字幕仍不額外解碼；其他影音格式、URL 與全文 ASR 合併／脈絡分析仍待完成。進度及測試另記於 [Phase 8](docs/phase-8.md) 與 [ADR 0040](docs/adr/0040-prepared-audio-clock-and-bounded-preprocessing.md)，Phase 9 尚未具備啟動條件，金鑰仍留至結案固定遮蔽介面手動輸入。

2026-10-12 第六批新增 provider-neutral 離線 ASR 契約、owned PCM 分段、順序合併及 caller-owned 有界 async runner。正文十秒、左右各最多五百毫秒真實 overlap，單份最多十一秒；分段 ready 最多兩份且未取走時拒收新 input，每個辨識 batch 最多四份，服務須明確宣告五語／Auto、16／24 kHz、取消及原生定稿時間能力，並行要求限一至四個，每個要求使用獨立六十秒離線期限。保留原生媒體時間與全身分，連續 run 的正文只由一份 chunk 擁有；跨正文的句子回報 BoundaryMismatch，不以無 word timing 的裁句猜測合併。倒序結果按序提交，Auto 非空結果跨語言拒收；全文限 8192 cues／四 MiB，前文限同 run 已提交的八筆／六十四 KiB。完成後只交付獨立 PreparedTranscript，不製造 ExistingSubtitle provenance、Ready 或播放定位證據，取消／失敗／Future Drop 清除 RAM。沒有新增 crate、unsafe、落盤、真正服務或 GUI 音訊接線；舊 borrowed decode／preprocess callback 仍只能做短有界 CPU 工作，不得保留 PCM 或等待網路，後續需另建私有、依需求拉取的 native actor。GUI 無字幕仍回報 AsrUnavailable，正式雲端及檔案 ASR 尚未驗收，可靠外掛字幕不額外解碼。進度與決策見 [Phase 8](docs/phase-8.md) 與 [ADR 0041](docs/adr/0041-bounded-prepared-recognition-and-transcript-provenance.md)；Phase 9 尚未具備啟動條件，真實金鑰仍於結案固定遮蔽介面手動輸入。

第六批的合併亦核對 EOF 正文覆蓋：切換 run 前及整份 finish 時，前／最後 chunk 的 audio_end_us 必須等於 body_end_us，最後真實音訊已有正文；尚未交付的右 context 不得視為完整全文，不符即 typed error，不交付部分 transcript。PCM／前文 owner 使用 boxed slices，回應及 batch 移除多餘 reserved capacity；超過四份的 batch 在建立 Future 前即釋放，不等 poll 才套用容量上限。

2026-10-12 第七批接上本機 WAVE 到 caller 明確提供的 PreparedSpeechRecognizer：私有 pull actor 固定在同一 native worker 建立、使用及銷毀 COM／MF 與唯讀檔案 pin；owned DSP 直接移交有界分段，原公開借用 callback 契約不變。Native-owner 與 CPU 許可各依可用核心數最多兩份，舊正式解碼入口亦共用 owner 池；等待辨識釋放 CPU、停止新拉取並可由取消喚醒，owner 持到真正退出。命令容量一、回覆最多兩份、辨識 batch 最多四份；只有完整 EOF／FIR 尾段／正文閉合及實際 worker Join 後才完成原生端，最後有序辨識成功交付獨立 PreparedTranscript。開始工作後 Drop 不提前釋放資源，panic 保持 CPU 至 core 銷毀；沒有新增 crate、unsafe、落盤、真正服務或 GUI 接線。此 library 不產生 Ready、不持媒體播放 owner、不假冒既有字幕，可靠外掛字幕不額外解碼；正式 ASR adapter、全文脈絡／詞彙／翻譯與 Ready／GUI、其他容器、URL、播放定位仍待完成。進度見 [Phase 8](docs/phase-8.md) 與 [ADR 0042](docs/adr/0042-demand-driven-native-prepared-asr.md)，Phase 9 尚未具備啟動條件；有效金鑰維持結案固定遮蔽介面手動輸入。

2026-10-12 第八批提供 translate_recognized_transcript library，將 caller 交付的 owned PreparedTranscript 與既有來源字幕接入同一借用 view／全文翻譯引擎。辨識來源保持獨立，不建假 ExistingText／ExistingSubtitle、content ID、來源證據或整份原文複本；兩種輸入各自嚴格驗證後共用有界 batch、原生時間與順序還原。Fast／Standard／High 使用一／三／八句前後文，Fast 做一輪 Draft，Standard／High 在全部 Draft 完成後進行全文 Review；保留同 Session／generation glossary、獨立三十秒要求期限及 client 共用的並行／速率限制。等待服務不持 CPU 許可；最後在原有有界 CPU lane 本地化臺灣用語，核對身分、原始時間、句數、每句 8192 bytes 與全文四 MiB，已開始 worker 保留許可至真正退出。完整成功才交付 RAM-only PreparedTranslatedTranscript，持有原 ASR、boxed zh-TW 譯文及實際品質；取消／失敗／未 poll 或執行中 Future Drop 不交付部分本文。結果無 Debug／Clone／Serialize、不持媒體 pin、不代表 Ready 或播放定位，沒有新增 crate、unsafe、安裝、真正雲端、內容寫檔、快取或匯出。正式 ASR adapter、自動全文脈絡／glossary 分析、同一媒體 handle 的 Ready 組合及 GUI ASR 仍待接續；可靠外掛字幕仍不額外解碼，一般 GUI 無字幕仍回報 AsrUnavailable。進度與決策見 [Phase 8](docs/phase-8.md) 與 [ADR 0043](docs/adr/0043-recognized-transcript-translation-provenance.md)，Phase 9 尚未具備啟動條件，有效金鑰維持結案固定遮蔽介面手動輸入。

2026-10-12 第九批將同一媒體 pin 的 ASR fallback 接入 PreparedSession library。新增 caller 明確交付 recognizer／translator 的 run_with_recognizer；先判定外掛字幕優先序，有字幕仍走原流程且不額外辨識，無字幕先核對本次 glossary 身分。Native 只開啟一次媒體，保留原 OpenedMedia，以 File::try_clone 將同一檔案物件的 handle 給 IStream；EOF 在同一執行緒銷毀 decoder／MF／COM 並等待實際 join，再移交 pin。Native-owner 許可此時釋放，原 pin 在最後 ASR、全文翻譯及 Ready 期間持續禁止 writer／刪除，網路等待不持 CPU 許可。Ready body 明確區分既有字幕與辨識來源，保留完整原文、五語、真正時間及翻譯品質，不虛構 publication、字幕配對或播放定位證據。下層使用連結的子取消 scope，失敗保留 typed error；完整成功且目前 Session apply 的身分／sequence 核對通過才交付 Ready，停止／新工作／Drop 撤回本文與 pin。原獨立辨識／翻譯 library 仍不持媒體或代表 Ready。本批沒有正式 ASR adapter、GUI 音訊上傳、播放或自動專名分析，未新增 crate、unsafe、安裝、內容寫檔／快取／匯出。完整進度與限制見 [Phase 8](docs/phase-8.md) 及 [ADR 0044](docs/adr/0044-pinned-recognized-prepared-ready.md)，Phase 09 尚未具備啟動條件，有效金鑰維持結案固定介面手動輸入。

2026-10-12 第十批新增 caller 明確選擇的 OpenAI／Gemini Prepared 辨識 HTTP adapters。短期單聲道 PCM 在 RAM 包裝 WAVE，OpenAI 使用 whisper-1 verbose_json segment 原生時間，Gemini 使用 gemini-3.5-transcribe 的 Interactions VERBATIM word_info、inline audio 與 store／background／stream:false。只接受明確指定的五語，限五語 Auto 尚未建立可靠契約，於開檔或上傳前拒收；Gemini 語言欄位保留 caller 配置，不冒充已偵測語言。編碼／解析共用有界 CPU lane，HTTP client 重用連線池、固定 HTTPS／敏感 header／禁止重新導向，最多兩個要求且依穩定 CPU 容量調整，額滿明確 Overloaded。六十秒涵蓋全部步驟與最多兩次嘗試，尊重 Retry-After，不把較長冷卻縮短成提早重送；原始 JSON 限 512 KiB，錯誤 body 不讀取或記錄。內部子取消範圍與 worker request lease 保留真正錯誤、停止及實際清理語意。只接受可核對的原生時間／本文，完整 context 可排除，跨正文及未標註資料保守拒收，不捏造時間。這是正式協定 library，不代表 GUI ASR、有效金鑰或雲端品質已驗收；一般 GUI 仍只準備外掛字幕。自動全文分析、其他影音輸入及播放定位待接續，Phase 09 尚未具備條件；有效金鑰留至結案固定介面手動輸入。詳見 [Phase 8](docs/phase-8.md) 與 [ADR 0045](docs/adr/0045-bounded-prepared-speech-http-providers.md)。

2026-10-13 第十一批將 OpenAI／Gemini Prepared ASR 接入一般準備視窗及原有單一背景 worker。外掛字幕只有完全空字串才走辨識，無效／損壞字幕不偷偷改傳音訊；中文本機處理，來源字幕只翻譯。新增獨立 Auto／五語音訊來源選擇，Auto 尚未建立可靠契約，原生視窗在讀金鑰前、controller 在 worker／憑證／媒體 I/O 前拒收；字幕路徑忽略此偏好。啟動前固定目的地列按實際路徑揭露短期媒體音訊及辨識原文／翻譯脈絡用途，無字幕使用同一所選服務及一次解析的金鑰建立辨識／翻譯 clients。固定雙列遮蔽金鑰取用後清除及取消／關閉清理保持一致；不預讀憑證、不默認 provider 或跨服務 fallback。GUI 使用 16 kHz 前處理及同一媒體 pin 的 run_with_recognizer，保持一個 Session、可回收 worker 及容量一結果通道，完整成功才交付 RAM Ready。沒有自動播放、保存或匯出；真實金鑰、雲端品質、多格式／URL、全文專名／詞彙與播放定位仍待完成，Phase 09 尚未具備條件。詳見 [Phase 8](docs/phase-8.md)、[固定金鑰介面](docs/api-key-ui.md) 與 [ADR 0046](docs/adr/0046-explicit-prepared-asr-gui.md)。

2026-10-13 第十二批新增有界內容辨識與 MPEG-1 Layer III 的原生 MP3 解碼 library。沿用同一唯讀 file object、IStream／MF、原生時間及資源限制；只接受兩個完整相鄰 frame 與有界 ID3，最多讀取 31 bytes，不靠副檔名或掃描任意 sync。MPEG-2／2.5、free format、AAC／MP4、播放清單及未知格式明確拒收。合成 MP3 的實際 native time 顯示 decoder delay 缺口與每 frame 100 ns 取整；兩秒檔案會觸發既有 DSP 漂移檢查。因此原生 Native 解碼與 SampleAccurate 前處理／ASR 分開，MP3 在 PCM 或要求交付前回報 UnsupportedAudioClock，不放寬 PCM 時鐘、補靜音或重設時間。既有字幕不需音訊解碼，保持優先。MP3 時間映射、正式 VBR／真實檔案、多格式、URL、詞彙與播放定位仍待完成；金鑰留至結案，Phase 09 尚未具備條件。詳見 [Phase 8](docs/phase-8.md) 與 [ADR 0047](docs/adr/0047-bounded-mp3-native-decoding-and-clock-gate.md)。

2026-10-13 第十三批新增有界 decoded sample／native 時鐘映射核心。每個 run 由累積取樣數推導 sample time，原始 PCM allocation 原樣移交，另保留各原生起點及真實 frames 的端點；真缺口依前 sample end 加 native gap 開新 run，不放寬既有 DSP 或重設 origin 隱藏漂移。每個 span 保留 source frame 起迄及 run origin，窗口獨立核對完整取樣相位，拒絕相同 Session 混入另一 normalizer 的不相容時間證據。rolling metadata 最多 512 spans／十二秒，明確 prune，超限／取消／錯身分／缺少範圍或跨 gap 均 terminal；區間以整數有理內插轉回原生時間，不外推未觀測音訊。真正合成 MP3 經未放寬的 16／24 kHz DSP 驗證；一般 MP3 Prepared／ASR 仍拒收，尚須將不可變、有界映射綁定 chunk、context 及結果合併，不能等四份十秒 ASR batch 回來才查 rolling window。本批不新增上傳、保存、快取或播放；WAVE 與字幕優先維持可用，真實相容性／雲端與完整 Phase 08 仍待完成，Phase 09 尚未具備條件。設計見 [ADR 0048](docs/adr/0048-bounded-decoded-sample-to-native-clock-map.md)。

2026-10-13 第十四批完成分段持有的不可變時間快照與原生辨識定稿合併 library。快照最多 512 spans／十二秒，保留原始相位及精確 audio／左右脈絡，窗口回收後仍可還原延遲結果；服務要求／回應維持 sample clock，先驗證唯一正文再轉 native time，任何 cue 重疊均拒收。綁定完整來源身分，register／complete 不接受等值重綁或同文件混用直接與映射模式；相鄰來源 rows／frame 相位、正文／context、共享 EOF 端點及合法左脈絡擴充皆核對，音訊 end 不倒退。source run 與 chunk run 分開，可略過實際 DSP 沒有輸出 frame 的原生 run；短尾不外推。最多八個不同準備 scopes 的 guards 去重並保留到整份交付，較早已提交內容仍可撤回，取消優先於 NoSpeech，超限明確 Overloaded。正式 native actor 的快照移交、大 block／長缺口／窗口邊界仍待接線；一般 MP3 GUI／ASR 仍在 PCM／要求前拒收，沒有部分 Ready 或上傳。WAVE／字幕優先、固定遮蔽金鑰與結案手動輸入安排保持原有方式，Phase 08 尚未結案，Phase 09 尚未具備條件。設計見 [ADR 0049](docs/adr/0049-chunk-owned-clock-evidence-and-native-finals.md)。

2026-10-14 第十五批將時鐘快照接入正式依需求拉取 native actor。MPEG-1 MP3 使用映射、WAVE 保留直接取樣精度；先以一筆原始 pending span 移交完整分段／context，再依 segment 左脈絡及 DSP 尚未交付起點的最小值回收窗口。私有單調 watermark 限十二秒可查詢範圍／512 rows，另保留一筆跨左界完整原始 span，不裁切端點、補靜音或放寬一微秒精度；公開 append 保持原有完整端點限制。真缺口先排完舊 DSP／segment run，零輸出 run 不產生假正文，先前文件可在零輸出尾 run 的 EOF 完成。命令一、回覆兩份、辨識批次四份及同執行緒 COM／MF、CPU／owner 許可與原檔 pin 均沿用。一般 Prepared／GUI 可使用內容核對通過的本機 WAVE／MPEG-1 MP3，字幕優先；公開 borrowed DSP 仍拒收壓縮封包時鐘。真正合成 MP3 經 native actor、mock 辨識／翻譯到 RAM Ready，原生時間與 pin／取消另有驗證；不代表 VBR／使用者檔案、雲端品質或播放定位。Auto、其他容器、URL、全文詞彙及播放仍待接續，Phase 09 尚未具備條件，真實金鑰留至結案固定遮蔽介面手動輸入。設計與驗證見 [ADR 0050](docs/adr/0050-native-prepared-clock-handoff.md) 及 [Phase 8](docs/phase-8.md)。

目標：

- Local file
- HTTP(S) URL
- Embedded subtitle
- ASR fallback
- Parallel translation
- Consistency pass
- Completion notification
- Cancellation
- Temp cleanup
- No persistent cache

---

2026-10-14 第十六批接入全文專名／術語分析：來源字幕及完整 ASR transcript 先以每窗口最多 64 cues／64 KiB、至多 256 個窗口遍歷完整文件，再固定同一份 SessionGlossary 供所有 Draft／Review 使用。服務逐字提出原文詞彙，本機驗證 cue 並推導 UTF-8 範圍；領域核心核對完整身分／sequence、target 本地化後的 256 bytes 與最多 64 筆容量。手動詞彙優先，同生成詞彙去重，衝突、超限、錯資料或未完成回應明確失敗，不抽樣或無聲截斷。GUI 的分析／翻譯組合契約共用同一所選 OpenAI／Gemini client、要求許可及速率 gate，中文仍本機處理；舊手動詞彙 library API 保留。三十秒窗口預算、子取消 scope、要求 token、共用 CPU lane、原媒體 pin 及真正 worker 清理保護整份 Ready，介面揭露分析用途。只完成全文詞彙整理，不宣稱完整主題／語意摘要、真實譯名品質或有效金鑰驗收；全文及詞彙仍本次 RAM。其他 Phase 08 輸入、Auto、跨句品質、播放與實際定位仍待完成，Phase 09 尚未具備條件；金鑰維持結案固定遮蔽介面手動輸入。詳見 [ADR 0051](docs/adr/0051-grounded-prepared-session-glossary.md) 與 [Phase 8](docs/phase-8.md)。

2026-10-14 第十七批新增本機 MPEG-4 AAC-LC／ADTS 的二十 bytes 固定內容門檻，核對兩個完整相鄰 frame、格式及單一 raw block。正式 Prepared／GUI 開放 48 kHz 雙聲道，使用原有 native actor、取樣／原生快照、分段合併、全文詞彙與翻譯到 RAM Ready；字幕優先及固定金鑰安排保持一致。44.1 kHz 單聲道合成檔實際觀測到四微秒的原生／PCM 推導時間重疊，一微秒驗證保持，其餘組合在 PCM／服務要求前回 UnsupportedAudioClock。共同 metadata 上限改為 640 rows／十二秒，以涵蓋最密集的 1024-frame 情形；實際 MF 可以合併多個封包，壓縮 packet 數不等於 native row 數。PCM／pending／回覆／ASR／worker、原檔 pin、取消、Session 與整份 Ready 規則沿用，沒有新增產品編碼、保存／暫存／快取、匯出或播放。公開 borrowed DSP 仍拒收封包時鐘，其他 AAC 時鐘、profile／包裝、影片容器、URL、Auto、真實檔案／雲端品質及播放定位仍待接續，Phase 09 尚未具備條件。設計及官方資料見 [ADR 0052](docs/adr/0052-bounded-adts-aac-and-native-clock-gate.md) 與 [Phase 8](docs/phase-8.md)。

2026-10-14 第十八批將上述 AAC-LC／ADTS 正式支援擴大為 48 kHz 單／雙聲道，沿用一微秒 native/frame gate、分段時間證據及同一 Prepared 管線。十三秒合成檔以直接 IMF sample time／duration 獨立觀測四種率／聲道組合；44.1 kHz 單／雙聲道即使採原生 duration 仍有最多 43 個 100 ns ticks 的重疊，維持 PCM／服務要求前 UnsupportedAudioClock，不改寫端點或放寬精度。48 kHz 單聲道以兩個所選服務的明確 mock、16／24 kHz 前處理、全文詞彙、Draft／Review、原生 cue endpoint、停止／失敗／字幕優先與原檔 pin 驗證本次 Ready。GUI 列出單／雙聲道；其他 AAC、URL、Auto、全文語意及播放定位仍待接續。金鑰仍留至結案固定介面手動輸入，Phase 09 尚未具備條件。證據與限制見 [ADR 0052](docs/adr/0052-bounded-adts-aac-and-native-clock-gate.md) 及 [Phase 8](docs/phase-8.md)。

2026-10-14 第十九批接入有限的本機單音軌 M4A／MP4：非 fragmented、AAC-LC、48 kHz 單／雙聲道。有界 metadata 門檻先核對唯一音軌、檔內 data reference、AAC 描述及所有 chunk／sample byte 範圍，再用同一 pinned file 的原生 reader；moov 最多一 MiB，音訊 payload 以 seek 跳過。影片、多音軌、加密、外部引用、edit list、封面或其他非空 metadata 目前拒收，不能宣稱完整 MP4 影片支援。native type／ASC／PCM 格式核對後沿用原有時鐘 gate、映射、ASR／全文詞彙／翻譯及 RAM Ready；沒有播放、下載、音訊保存或新增快取。44.1 kHz MP4 時鐘尚未驗收，於 PCM／服務要求前拒收。合成與明確 mock 驗證不能代替真實檔案／雲端品質；固定遮蔽金鑰維持結案手動輸入，Phase 09 尚未具備條件。設計及限制見 [ADR 0053](docs/adr/0053-self-contained-mp4-aac-prepared-input.md) 與 [Phase 8](docs/phase-8.md)。

2026-10-14 第二十批以真正 AAC／MP4 的十二種率／聲道／長度組合直接查詢 IMF timestamp、duration 及 PCM frames；44.1 kHz 本機最大 native duration 重疊為 0.1 微秒、frame-derived 端點無重疊，全部真正 EOF 且格式一致。只擴大前批 M4A／MP4 到 44.1／48 kHz 單／雙聲道，ADTS 44.1 kHz 維持拒收；原有一微秒 gate、時鐘快照、容量、取消、Session 與 pin 保持。新增 44.1 kHz 的正式 decoder tone／原生 gate，以及兩個所選服務 mock、16／24 kHz、完整詞彙／Draft／Review 到 RAM Ready 的獨立 endpoint 驗證；停止／錯誤／中文字幕優先及外部引用拒收也驗證。影片、封面、其他未支援容器、URL、Auto、全文語意與播放／實際定位仍待接續，Phase 09 尚未具備條件；有效金鑰仍留至結案固定遮蔽介面手動輸入。觀測與限制見 [ADR 0053](docs/adr/0053-self-contained-mp4-aac-prepared-input.md) 及 [Phase 8](docs/phase-8.md)。

2026-10-14 第二十一批開放單音軌 AAC-LC M4A／MP4 的有界描述標籤。已知 UTF-8 文字每筆 64 KiB、JPEG／PNG 封面 512 KiB、ilst 合計 768 KiB，moov 保持一 MiB；少數數值欄位按固定大小核對，未知、重複／多值、外部引用、freeform 及其他未支援結構拒收。借用原有 moov 做檢查，不產生 metadata model、圖片解碼／UI／抓取、詞彙或翻譯脈絡／快取。真正合成檔加標籤前後所有 PCM／native timestamp 相同；Ready／取消／錯誤／字幕優先沿用，服務 mock 檢查詞彙及翻譯要求不含標籤內容。沒有改寫原檔、產品媒體保存或放寬時鐘。一般標籤變體、影片、多音軌、URL、Auto、全文語意及播放／實際定位仍待接續，Phase 09 尚未具備條件；有效金鑰留至結案固定介面手動輸入。設計與限制見 [ADR 0054](docs/adr/0054-bounded-mp4-descriptive-tags.md) 與 [Phase 8](docs/phase-8.md)。

2026-10-14 第二十二批開放有限的本機影音 MP4：一軌 H.264 Baseline 加一軌 AAC-LC 44.1／48 kHz 單／雙聲道，仍只取音訊。每軌檔內 reference、描述／sample tables 與 mdat 範圍完整核對，借用 cursor 檢查跨軌 byte overlap；moov 一 MiB 及原有取消／worker／PCM／時鐘上限保持。視訊 avcC 限 64 KiB、尺寸限 8192，不解碼、渲染、上傳或保存視訊；Main／High、其他 codec、多音軌、無音軌、edit list、fragmented 及未知結構仍拒收。音訊允許原生 muxer 的 version 0 ctts 並核對 counts，不以 metadata 改寫 IMF 時間。沿用只選 FIRST_AUDIO 的 decoder、既有一微秒 gate、ASR／詞彙／翻譯與本次 RAM Ready；中文字幕優先及固定金鑰安排保持。純合成原生 H.264／AAC 與明確服務 mock 驗證不代表一般使用者影片或雲端品質已驗收。URL、Auto、內嵌字幕、完整語意及播放／實際定位仍待接續，Phase 09 尚未具備條件；有效金鑰留至結案固定遮蔽介面手動輸入。設計及限制見 [ADR 0055](docs/adr/0055-bounded-h264-aac-mp4-prepared-input.md) 與 [Phase 8](docs/phase-8.md)。

2026-10-14 第二十三批擴大有限影音 MP4 的 H.264 視訊描述門檻為 Baseline／Main／High；High 每個 SPS 必須核對 4:2:0／8-bit 格式欄位，使用固定上限的借用 bit reader，不配置完整 RBSP。High 可無 extension，若有只接受完整四 bytes 的 4:2:0／8-bit／零 SPS-ext 形式，其他 profile／深度／色彩格式與不明描述拒收。視訊 ctts 允許 version 0／1、零 flags、精確大小與樣本總數；音訊維持 version 0，其他 tables 不放寬。組合時間表只做結構檢查，不生成視訊時間或改寫 IMF 音訊端點；真正 B frames／其他 muxer 尚未驗收。沿用一個 AAC-LC 音軌、檔內引用、cross-track byte overlap、一 MiB moov／64 KiB avcC、唯一 FIRST_AUDIO decoder、既有 gate、詞彙／翻譯及 RAM Ready；視訊不解碼、渲染、上傳或保存。原生合成 Main／High 音訊／時鐘與明確 mock Prepared 生命週期可測，不能代替使用者檔案或雲端品質。內嵌字幕、URL、Auto、完整語意及播放／實際定位待接續，Phase 09 尚未具備條件；金鑰維持結案固定遮蔽介面手動輸入。決策見 [ADR 0056](docs/adr/0056-ordinary-h264-profiles-and-composition-tables.md) 與 [Phase 8](docs/phase-8.md)。

2026-10-14 第二十四批補上真正含 B-frame 的 Main／High 合成 MP4 音訊準備驗證。只在 cfg(test) 原生 encoder 開始寫入前設定 B-picture 上限，再以實際 slice_type=1／6、非 IDR NAL、影格數／EOF 與 presentation timestamp 倒序證明 B frames，沒有改寫壓縮資料或 ctts 製造成功。八種 profile／率／聲道、24 種原生音訊時間組合及明確服務 mock 的完整 Ready／取消／錯誤／中文字幕優先可驗證；無效視訊表格或引用不發服務要求。正式容器／decoder／時鐘／ASR／RAM Ready 不變，產品仍只取音訊、不包含 encoder／視訊 renderer；windows 套件僅啟用測試 VARIANT 所需的既有 OLE binding feature。這是固定 NV12／PCM 的本機合成案例，其他 muxer、GOP／fps／尺寸設定、使用者影片與播放／雲端品質仍待驗收。URL、Auto、內嵌字幕、完整語意及實際定位未完成，Phase 09 尚未具備條件；金鑰維持結案固定介面手動輸入。證據與限制見 [ADR 0057](docs/adr/0057-genuine-b-frame-mp4-audio-evidence.md) 與 [Phase 8](docs/phase-8.md)。

2026-10-14 第二十五批先提供 MP4 tx3g 的有界純文字樣本核心。原始 track ticks／明確語言與 Session／序號經嚴格 UTF-8／BOM UTF-16、完整長度、時間／容量及取消驗證後形成一份 ExistingText；空字串保留清除區間，任一失敗不交付部分文件，不改變音訊／播放時鐘。modifier 擴充暫時拒收，核心不推定中文地區、不提供仲裁或 alignment。與既有臺灣片語及五語全文 Standard Draft／Review 的資料相容性可用合成測試驗證；MP4 軌道探索、table／reference／edit list 讀取及 GUI 字幕優先接線尚未提供，不能宣稱一般 --ui 已讀取內嵌字幕。沒有新相依、unsafe、雲端要求、產品媒體／字幕寫入或快取；金鑰維持結案固定介面手動輸入。Phase 09 尚未具備條件。設計及限制見 [ADR 0058](docs/adr/0058-bounded-tx3g-plain-text-samples.md) 與 [Phase 8](docs/phase-8.md)。

2026-10-15 第二十六批提供同一唯讀 pin 的指定 MP4 tx3g 軌讀取 Rust 介面。共用有界 top-level／audio／video gate，以最多三軌的借用 cursors 核對各 sample tables／檔內 reference／mdat／跨軌 byte overlap，再逐份讀取指定文字樣本；保留原 ticks、清除區間及完整 ExistingText。文字描述／font table／明確語言與 mdhd、stts duration 與 movie rounding 有結構核對，零動態 display flags、無 edit list／文字 ctts／modifier 的子集合才接受，不估計時間偏移或載入原媒體字型。回傳同一原檔 pin，可由後續文字階段保留；共用 CPU lane、取消／Drop、過載及 panic 型別處理保持有界。原音訊 decoder gate 未放寬；GUI 選軌／自動優先／Ready 與真實 muxer／影片／播放仍待接續，不能宣稱一般 --ui 已讀內嵌字幕。無產品媒體／文字寫入、快取、服務要求或金鑰讀取；有效金鑰維持結案固定介面手動輸入，Phase 09 尚未具備條件。見 [ADR 0059](docs/adr/0059-selected-pinned-mp4-tx3g-reader.md) 及 [Phase 8](docs/phase-8.md)。

2026-10-15 第二十七批接通明確 MP4 tx3g 選軌的 PreparedRequest／Session／一般 GUI／Ready。單一內嵌選取與外掛字幕共用語言優先／八份總預算，同順位明確拒收；metadata-only 規劃先選文字語言再決定服務，中文不建 client 或取用金鑰，來源字幕只走同一所選服務的全文詞彙／品質翻譯，不建立 ASR。Session 在既有有界 worker 以同一 pin 讀取及沿用原始時間、取消／completion 隔離／RAM Ready；無第二個 permit／讀取管線。GUI 以既有來源／格式選項、路徑／軌 ID 欄明確選取，空白／非法 ID 不改傳音訊，雙列固定遮蔽金鑰與通知／清除流程保持。軌道探索、其他 muxer／描述／字幕軌、edit／composition、URL／Auto、完整語意／播放與真實雲端仍待驗收；Ready 不自動播放，Phase 09 尚未具備條件。有效金鑰維持結案固定介面手動輸入。見 [ADR 0060](docs/adr/0060-explicit-embedded-prepared-ready-and-ui.md) 與 [Phase 8](docs/phase-8.md)。

2026-10-15 第二十八批新增一般 Prepared 視窗的本機 MP4 字幕軌探索按鈕。背景工作共用既有 bounded CPU lane 及單一 text／tx3g 格式驗證，成功填入軌 ID；五語 ISO 提示可填入來源語言，中文／und 必須明確指定，不推測繁簡或臺灣用語。探索不取用金鑰／雲端或解碼音訊，不讀完整文字樣本；保留共用 loader 的一 byte mdat 可讀性探測。來源 pin 在探索退出釋放，準備重新驗證檔案／指定軌／本文，成功探索不是 Ready 或版本證據。唯一工作 identity／有界回覆／取消及 reaping 防止舊結果回填，來源路徑變更退休探索與其自動 ID，關閉等待 owner 退出。既有多文字軌／edit／composition／modifier 等限制保持；真實 muxer／影片、URL／Auto、完整語意／播放與雲端仍待驗收，Phase 09 尚未具備條件，金鑰留至結案手動輸入。見 [ADR 0061](docs/adr/0061-bounded-local-mp4-subtitle-discovery.md)。

2026-10-16 第二十九批將既有 text／tx3g 範圍擴大至最多八條字幕軌、一條 AAC-LC 音軌及可選 H.264，總計最多十軌；moov、每軌 sample／wire／decoded／cue 及共用 CPU／取消上限保持。全部軌的 ID／描述／時間／檔內引用及跨軌範圍仍完整核對，只讀取明確選定軌的本文。新增固定八格 catalog 與 plural discovery，原 singular API 遇多軌回 AmbiguousSubtitle。一般 GUI 提供清單，多軌不自動選第一軌或猜中文繁簡，選定後沿用同一 Session／全文處理／RAM Ready；手動 ID 保留，來源變更／重探／取消清除舊清單，metadata 不作版本或 alignment 證據。其他格式／handler／語言、多音軌、edit／文字 ctts／modifier、真實 muxer／影片、URL／Auto、完整語意／播放與雲端仍待驗收；Phase 09 尚未具備條件，正式金鑰仍留至結案固定介面手動輸入。見 [ADR 0062](docs/adr/0062-bounded-mp4-multiple-subtitle-tracks.md)。

2026-10-16 第三十批在 tx3g 文字軌接受有限 version 0 edts／elst：完整 rate=1、media_time=0 的一筆 edit，可有一筆 media_time=-1 的前置空白；不做裁切、重複、變速、dwell 或未知版本。active duration 必須精確涵蓋整份 mdhd，tkhd 是 edit 總長，mvhd 必須涵蓋；無 edit 保留舊 floor／ceil。delay 與每個媒體絕對端點以 checked u128 有理數一次 floor 成微秒，避免相位／累積取整誤差，clear gap、Session、序號及包含 delay 的 24 小時上限保持。全部文字軌先驗證，A/V edit 仍拒收；同一 Prepared 管線到 RAM Ready，不新增播放、動態 offset／seek／drift 或 Phase 09。原容量／CPU lane／取消／pin／金鑰安排不變，真實影片、URL／Auto、完整語意、播放與雲端仍待驗收；Phase 09 尚未具備條件。見 [ADR 0063](docs/adr/0063-full-track-tx3g-edit-delay.md)。

2026-10-16 第三十一批將文字 v0 rate=1 active edit 擴大為單一有效頭尾區間，可有前置 empty；media_time 非負、整段位於 mdhd 內、tkhd／mvhd 及既有格式驗證保持。半開區間與各樣本取精確有理數交集，一次 floor 成呈現微秒，重疊原文保留完整字串，完全在外側的 cue 移除，clear gap 不延長。先以原媒體時間完整驗證所有選定樣本及既有容量，再以最多8192筆數值對照原地過濾，不複製全文、不讓裁切繞過損壞或預算；只有可見文件送同一詞彙／品質翻譯／RAM Ready。只含空窗則 InvalidSubtitle，不空白 Ready 或 ASR fallback。A/V edit、v1、多 active、變速／dwell／repeat 仍拒收，取消／pin／Session／無 edit 舊時間保持；沒有啟動 Phase 09，真實影片、URL／Auto、完整語意、播放／雲端仍待驗收，金鑰留至結案固定介面手動輸入。見 [ADR 0064](docs/adr/0064-single-interval-tx3g-text-crop.md)。

2026-10-16 第三十二批為文字 elst 加入 version 1：duration／signed media_time 各 64 bits、rate 32 bits，零 flags、精確 1 或 2 筆。保留單一正常速度 active 及可選 leading empty、範圍核對、checked 總長、有理數裁切／延遲、24 小時與完整本文／容量／取消／Session／同一 pin；不截斷高位或把 unsigned 32-bit 全一誤當 v1 的 -1。只擴充 elst，mdhd／tkhd／mvhd 仍限 v0，其可表示範圍不放寬；A/V edit、多 active、文字 ctts／modifier 及未知版本仍拒收。GUI／Ready、中文本機及兩所選服務／三品質流程沿用，沒有播放、產品寫入或 Phase 09。真實影片、URL／Auto、完整語意、播放定位與真實雲端仍待接續，金鑰維持結案固定遮蔽介面手動輸入。見 [ADR 0065](docs/adr/0065-version-one-tx3g-edit-list.md)。

2026-10-16 第三十三批在既有全文詞彙分析加入有界語意摘要，每批整合 prior_summary 與原文 cues，最後以獨立分析套件封存詞彙及 4096 UTF-8 bytes 內的模型摘要，綁定完整 Session／generation、語言、品質及段數。自動來源字幕與 ASR 的所有 Draft／Review 共用同一不可變摘要，保留各品質原有鄰近脈絡與一致性校訂；缺少／空白／超長／非法控制字元或分析錯誤明確失敗，不默默退回無摘要 Ready。摘要只作可能有誤的推論提示，與系統指令隔離，以原文及明確詞彙為準，不作內容識別、版本、仲裁或播放定位證據。中文本機、明確手動 library 入口與固定金鑰安排保持；無新增 AI 要求、worker、時間軸或內容保存。真實語意品質、URL／Auto、媒體相容性及播放定位待接續，Phase 09 尚未具備條件。見 [ADR 0066](docs/adr/0066-bounded-prepared-document-context.md)。

2026-10-16 第三十四批提供獨立 HTTP(S) 媒體範圍讀取 library。明確 remote 輸入、本次 identity／sequence／offset 與取消，首個一-byte probe 核對強 ETag／total，後續同 client／URL 使用 If-Match，206／range／Content-Length／identity 編碼與實際 bytes 必須精確相符。版本改變、截斷、重新導向或忽略 range 明確失敗，不改 URL／scheme 或下載整份；無記錄私有 URL／query、原始錯誤或媒體。來源四 GiB、每段64 KiB、65536份要求、兩個 owner、全程序四份在途／下游保留 blocks（256 KiB payload），過載不排隊；取消、錯誤或 Future Drop 退休來源，保留 blocks 仍占配額至 Drop。沒有產品媒體寫入、temp／cache／export、新 CPU pool 或時間軸。尚未接上 Prepared 解碼／Ready／URL GUI，原正式路徑仍明確 UnsupportedUrl；不授予識別、版本配對或播放證據，Phase 09 尚未具備條件。見 [ADR 0067](docs/adr/0067-bounded-session-http-media-ranges.md)。

2026-10-16 第三十五批將同一遠端 range owner 接入既有內容格式前置檢查，提供 probe_remote_audio library。私有 Read／Seek adapter 每次最多64 KiB，大讀取分段、seek 限於表示長度，既有 WAVE／MP3／ADTS／有限 MP4 gate 與 metadata 上限保持。成功保留原 identity、client、強 ETag／total 及要求序號，不重新開啟來源或新增快取。blocking job 受既有遠端 owner 上限限制，解析共用原 CPU lane；等待網路釋放 CPU，到達後 try_acquire，過載不排隊。取消等待真正退出，Future Drop 取消且配額保持到實際清理；底層 typed error 不被連帶取消蓋掉。格式通過不代表完整 codec、解碼、Ready 或播放，一般 Prepared／GUI 仍拒收 URL。無產品媒體／文字寫入、temp／cache／export、新 CPU pool、COM 或 unsafe，Phase 09 尚未具備條件。見 [ADR 0068](docs/adr/0068-bounded-remote-content-preflight.md)。

2026-10-16 第三十六批新增同一遠端 owner 的實際 PCM16 WAVE 解碼 library，沿用 DecodedAudioBlock／Gate 與唯一 Session／generation。canonical RIFF、十六-byte fmt、單／雙聲道、8–192 kHz、唯一完整 data、有界 JUNK 及完整 chunk／長度／padding 均在 PCM 前驗證；其他格式不 fallback。每塊最多64 KiB，另限全程序四份在途／保留 PCM（256 KiB），退休來源仍持有保留配額；慢消費端在 I/O 前拒收。累積 frames 產生時間、維持一微秒捨入容差、二十四小時與剩餘 HTTP 要求預算；完整 data 才 EOF。網路等待不持 CPU，原 CPU lane 上有界轉換／取消，不新增每 block 執行緒。錯誤／Future Drop 撤回舊 PCM 存取，不回傳部分 block；無媒體落盤／temp／cache／export。本機 Windows MF 與壓縮路徑不變，遠端 ASR／Ready、URL GUI、其他編碼及實際媒體／雲端／播放仍待接續，Phase 09 尚未具備條件。見 [ADR 0069](docs/adr/0069-bounded-http-pcm16-wave-decoder.md)。

2026-10-16 第三十七批將同一遠端 PCM16 WAVE 接入既有 Prepared DSP／分段／辨識 library。先驗證明確所選辨識器能力，再拉取媒體；網路等待不持 CPU，一來源最多一份既有 CPU lane 上的 blocking job，每 pull 最多兩段，三／四份待辨識分段先完成批次再拉取來源。沿用16／24 kHz、十秒正文與半秒脈絡、並行上限、逾時、前文與亂序定稿合併。取消／Drop 及過載明確失敗，排隊／執行中 job 配額保留到真正回收；來源變更 typed error 不被連帶取消蓋掉。完整 EOF／DSP 尾段／全部辨識成功才回傳綁定原 HTTP owner 的原文，最後服務等待及結果保留不重開 URL，結果 Drop 退休 scope。沒有全文翻譯／Ready／URL GUI、播放、金鑰取用或內容落盤；合成／mock 不代表真實媒體與雲端品質已驗收。Phase 09 尚未具備條件，金鑰留至結案固定介面手動輸入。見 [ADR 0070](docs/adr/0070-demand-driven-remote-wave-prepared-recognition.md)。

2026-10-16 第三十八批以明確 PreparedTask 程式庫入口接通遠端 PCM16 WAVE 的完整本次準備：同一 HTTP owner／內容檢查／解碼／辨識後，共用本機原有全文摘要、詞彙、三品質翻譯、臺灣本地化及 Session RAM Ready。私有來源擁有者區分本機檔案與遠端 EOF decoder，crate-private 移交同時保留原文與來源；來源跨分析／翻譯、未 apply completion 與 Ready 持續占 owner 配額，不重開 URL 或使用媒體暫存。完整成功及目前 Session／sequence 才能 Ready，不自動播放。選定字幕的 URL 組合與本機輸入先拒收，ambiguity／舊詞彙／能力先驗證，不忽略文字或改上傳音訊；一般 URL GUI 尚未開放。MediaError::Remote 保留遠端 typed cause，子 scope 連帶取消不蓋掉 SourceChanged 等原因；停止／Drop／舊 completion 不發布部分文件。沿用原 CPU／owner／PCM／文字／並行／逾時限制，無套件、產品內容寫入、temp／cache／export、遙測或第二份時間軸。測試採實際 loopback 與明確服務 mock，真實媒體／雲端品質、URL GUI／既有字幕組合及 Auto 仍待接續；金鑰維持結案固定介面手動輸入，Phase 09 尚未具備條件。見 [ADR 0071](docs/adr/0071-remote-wave-session-analysis-translation-ready.md)。

2026-10-17 第三十九批在一般 Prepared 視窗開放有限 HTTP(S) WAVE。媒體欄以明確 http://／https:// scheme 區分網址，完整語法／容量驗證且不 trim／修復，不依副檔名猜格式；網址必須無選定字幕／內嵌模式並明確選五語與服務。metadata 規劃保留歧義，URL加文字、Auto及缺服務在金鑰／factory／worker前拒收，不默默改上傳音訊。固定揭露媒體連線及所選 AI 端點／短音訊／原文／全文摘要／詞彙／翻譯前後文，不把私有URL／query加入AI要求、目的地、狀態或通知。沿用單一背景worker、原remote PreparedTask與16 kHz／全文三品質／RAM Ready、固定雙列遮蔽金鑰、一次所選憑證與清空undo；探索限本機且直接命令也有防護，手動字幕保留，來源變更退休探索metadata。停止／關閉／Ready鎖定／清除及舊completion隔離維持，不自動播放。無新pool、依賴、永久內容、temp／cache／export或遙測；只支援既有PCM16 WAVE子集合，網站／登入／重新導向、其他遠端編碼、URL可靠字幕組合與Auto仍未開放。實際loopback與明確mock、隱藏原生視窗及窄視窗／雙倍字型量測不代表真實雲端／媒體／多螢幕／播放品質；金鑰維持結案固定介面手動輸入，Phase09尚未具備條件，達標會提醒。見 [ADR 0072](docs/adr/0072-explicit-http-wave-prepared-ui.md)。

2026-10-18 第四十批支援一般 HTTP(S) PCM16 WAVE 搭配明確本機 SRT／WebVTT。metadata先保留語言順位／歧義，所有遠端內嵌選取仍先拒收；選定外掛在既有有界worker讀取解析後才進行同一HTTP／內容／WAVE標頭核對，不拉取PCM、不做DSP或ASR，失敗不改傳音訊。中文三類本機臺灣本地化、不取金鑰；五語來源字幕只使用所選分析／翻譯服務及同一全文摘要／詞彙／三品質流程，不建立辨識器，音訊Auto不影響選定字幕。原來源owner保留跨服務／completion／Ready，既有取消／Drop、CPU／來源配額、Session／sequence隔離及完整成功才Ready不變。固定用途揭露區分來源標頭連線、中文本機與來源文字雲端；來源字幕不傳音訊，私有URL／query不加入AI要求或狀態。固定遮蔽金鑰及清空、遠端探索防護、保留手動字幕與不自動播放維持。關聯仍是使用者聲明，不製造版本／播放定位證據；未新增依賴、unsafe、內容落盤或第二份時間軸。遠端內嵌／字幕網址、其他編碼、五語音訊Auto、真實媒體／雲端及播放驗收仍待接續，Phase09尚未具備條件，金鑰留至結案手動輸入。見 [ADR 0073](docs/adr/0073-remote-wave-local-sidecar-ready.md)。

2026-10-18 第四十一批新增獨立HTTPS SRT／WebVTT完整文件讀取library，不變更本機路徑或PreparedRequest／GUI契約。明確格式／語言及真實Session／generation／sequence，不套用Live capture stamp或虛構媒體識別；原Live查詢容量／期限不變。單份GET、TLS驗證、不跟隨redirect／重試／scheme fallback或代理；核對MIME／UTF-8／identity、宣告與實際長度，有界chunked／未知長度仍限八MiB。全程序兩份要求／本文／結果持有者，解析共用原CPU lane及8192 cues／四MiB本文，網路等待不持CPU；Drop／取消保留容量至真正清理，成功結果仍屬同一可退休scope。原文時間與Existing身分保留，沒有辨識、翻譯、金鑰、Ready、內容保存、假版本／仲裁或播放證據；錯誤不含URL／query，404或壞字幕不退回ASR。一般GUI仍拒收字幕網址；Session／用途揭露／全文本地化翻譯接線與其他媒體／Auto／真實雲端／播放驗收仍待後續，金鑰維持結案手動輸入，Phase09尚未具備條件。見 [ADR 0074](docs/adr/0074-bounded-prepared-https-subtitle-reader.md)。

2026-10-18 第四十二批以明確run_with_https_subtitles Task接通HTTPS原文讀取、同Session全文處理及RAM Ready。HTTPS候選與本機外掛／內嵌共用八份上限、語言順位及歧義；只有勝出網址連線，未選網址不讀取，本機／內嵌勝出重用既有文字流程。來源缺translator、舊glossary及遠端媒體帶任一內嵌選取先拒收；先持有本機media pin或遠端WAVE標頭owner，再讀字幕，不做PCM／DSP／ASR，字幕錯誤不改傳音訊。中文本機臺灣本地化，五語重用同一所選服務之全文摘要／詞彙／三品質翻譯；URL／query／媒體路徑不加入AI要求。原文不整份複製，私有lease保留HTTP本文配額跨服務／completion／Ready，原文譯文釋放後才還lease；網路等待不持CPU，Stop／Drop／舊結果隔離及typed cause保持，無內容落盤或第二份時間軸。舊Task與一般controller在用途揭露接線前仍拒收任何HTTPS字幕候選，GUI字幕欄保持本機契約；不以Ready宣稱媒體版本／仲裁／播放定位或自動播放。一般視窗接線、其他媒體／Auto、真實雲端／播放驗收仍待後續，金鑰結案手動輸入，Phase09尚未具備條件。見 [ADR 0075](docs/adr/0075-prepared-https-subtitle-session-ready.md)。

2026-10-18 第四十三批在一般準備視窗新增明確HTTPS SRT／WebVTT模式，使用既有字幕欄輸入非空完整HTTPS網址，格式與語言由使用者聲明。新draft constructor及backend start_with_https_subtitles在對應preflight核對語法／metadata／必要服務；舊本機／內嵌模式與一般start保持拒收字幕網址，空白或失敗不改做ASR。固定用途區先說明字幕全文連線、本機媒體唯讀或遠端WAVE標頭、中文本機或所選AI端點及原文／全文摘要／專名詞彙／翻譯前後文用途；私有媒體／字幕網址與音訊不加入AI要求。中文不取password或安全憑證，五語僅建立同一所選分析／翻譯服務，音訊Auto不影響明確字幕。單一worker／runtime／completion／Session、品質、持有者配額、停止回收、Ready鎖定與舊completion隔離沿用。HTTPS字幕模式不探索內嵌軌，直接命令也防護，避免自動切模式或取代手動網址；其他本機模式的探索保持。固定雙列遮蔽金鑰、清空undo、一次通知與不自動播放不變，內容僅本次RAM。遠端媒體仍限既有PCM16 WAVE，不新增網站／登入／redirect／其他編碼或遠端內嵌，真實媒體／語意／雲端／多螢幕／播放仍待驗收；有效金鑰留至結案固定介面手動輸入。本批完整工程驗證已通過，已具備開始Phase09對齊核心的工程條件；不代表Phase08全面產品驗收結案，本批不自行啟動下一階段。見 [ADR 0076](docs/adr/0076-explicit-prepared-https-subtitle-ui.md)。

## 83. Phase 9 — Alignment

階段目標：

- Fuzzy matching
- Current position
- Seek detection
- Offset
- Drift correction

2026-10-22 第一批啟動：獨立、無 I/O 的原文／Final ASR 模糊定位候選核心，明確區分文件與擷取 Session，全文核對重複及次高差距，限制每段 256 個正規化 Unicode scalar、全文 2,097,152 個 DP cells、三秒工作與原擷取視窗五秒期限，支援取消及 epoch／ASR binding 失效檢查。編輯一致度只作候選指標，不轉內容信心、採用證據或播放位置；原精確驗證、仲裁、擷取時鐘與單一時間軸保持。尚無正式管線／有界 worker 接線，目前位置、seek／pause／restart、offset 與 drift 仍待後續，不宣稱整個 Phase 09 完成。有效金鑰與真實雲端驗收維持結案固定遮蔽介面手動輸入。進度見 [Phase 9](docs/phase-9.md) 與 [ADR 0077](docs/adr/0077-bounded-fuzzy-alignment-proposals.md)；Phase 10 尚未具備啟動條件，符合時提醒。

2026-10-22 第二批提供獨立 FuzzyCueLocator async library，owned 文件在擁有者內不可變，精確與模糊定位沿用同一份全程序 min(available_parallelism, 2) 名額，每擁有者一項，過載不等待。首次 poll 起的三秒與原視窗五秒期限取較早者，包含 Tokio blocking pool 等待且不重設；明確取消／逾時等待真實清理，Future Drop 合作取消且名額保留至輸入釋放。Stop／擁有者 Drop 與 caller 取消撤回已交付候選；不取消父 scope，不把相似度轉採用或播放證據。這是第一批核心的有界背景接線，正式 Live／Prepared controller、Ready 文件借用、多觀測錨點及目前位置／seek／offset／drift 仍待接續，不代表 Phase 09 完成。詳見 [Phase 9](docs/phase-9.md) 與 [ADR 0078](docs/adr/0078-shared-alignment-worker-and-proposal-lifetime.md)；結案金鑰安排與 Phase 10 條件提醒不變。

2026-10-22 第三批新增 Prepared Session／controller 的明確 Ready 定位 library 橋接，只接受已 apply 的五語既有原文，文件與擷取身分分開綁定。私有文字 owner 與媒體資源分離，locator／未 poll Future 只持弱參考；CPU 名額允許後才升級文字 owner，不複製全文、不傳媒體 pin／decoder 到工作。Stop／替換／Session Drop 撤回舊候選，媒體資源先釋放，進行中工作保留文字與 HTTPS 本文配額至真實清理。精確採用、單一時間軸、原有字數／DP／CPU／期限與四祖先取消上限保持；不建立假 capture stamp、不讀金鑰或自動播放。一般視窗／Live 自動接線、recognized／中文配對、多觀測錨點與目前位置／seek／offset／drift 仍待後續，不宣稱 Phase 09 完成。詳見 [Phase 9](docs/phase-9.md) 與 [ADR 0079](docs/adr/0079-revocable-prepared-ready-alignment-text.md)；結案金鑰安排與 Phase 10 條件提醒不變。

2026-10-22 第四批新增 FuzzyProposalSession／caller-owned Task，管理最新接受的 Final ASR 與定位候選；輸入／結果各一格，新修訂撤回舊工作，真實清理後才處理最新替換。舊 scope／binding／ticket 結果不能覆蓋新狀態，同 binding 的本文／語言／時間／kind 變更撤回舊證據；重複輸入不建立新觀測或重試。三秒從 offer 時計算，與原窗口五秒期限取較早，包含 Task 等待且不重設；直接 locator 舊入口契約保持。Ready 撤回及 Task Drop 喚醒管理器，子取消不影響 Ready，沿用四祖先及既有 CPU 名額；沒有新服務、capture stamp、播放操作或自動採用。一般 Live／GUI、recognized／中文配對、多觀測錨點與目前位置／seek／offset／drift 仍待接續。詳見 [Phase 9](docs/phase-9.md) 與 [ADR 0080](docs/adr/0080-latest-only-fuzzy-proposal-session.md)；Phase 10 尚未具備條件，結案金鑰安排不變。

2026-10-23 第五批讓 Prepared Session／controller 的既有 fuzzy_locator 入口接受已 apply 為 Ready 的五語 Recognized 原文；透過私有借用 view 共用原有搜尋核心，不複製全文、不將 ASR 包裝成 ExistingText／字幕出版物，不讀取譯文作證據。候選新增明確 FuzzyTextOrigin，保留 ExistingText／PreparedRecognition 身分。辨識本文仍經原有 sealed transcript 結構／身分／時間／大小驗證，全文歧義、Unicode／DP 上限、窗口與期限、弱文字 owner、四祖先取消及共享名額不變；未開始捕捉／服務／播放或採用。中文配對、跨 cue 配對、多觀測錨點、目前位置、seek／offset／drift 及一般 Live／GUI 接線仍待後續。詳見 [Phase 9](docs/phase-9.md) 與 [ADR 0081](docs/adr/0081-prepared-recognized-alignment-source.md)，Phase 10 條件提醒與結案手動金鑰安排不變。

2026-10-23 第六批在既有 FuzzyProposalSession 接上最多八筆 Final 定位 metadata 的時間一致性提示。至少三個不同且遞增的 utterance／sequence／cue、同來源種類、capture／media 各自向前且互不重疊、首尾 capture 觀測跨度至少一秒，並以 signed 計算核對全軌跡起訖差值散布不超過 250 ms；這只是具名工程門檻，非信心、verified anchor、目前播放位置或 offset／drift 校正。新舊不相容撤回舊軌跡並明示重新蒐集；修訂、較舊訊息、失敗／未匹配、原窗口期限、Ready／driver／owner 撤回使快照失效，不延長原五秒、不讓重複輸入計數。已接受 metadata 不隨正常換 utterance 的舊 job 取消而消失，仍受來源 availability、原期限與修訂撤回約束；快照不持本文或媒體、不增加取消祖先或 worker／queue／服務。原精確驗證、仲裁、擷取時鐘與時間軸不變，來源／epoch 變更須停止重建。一般 Live／GUI、中文／跨 cue、經驗證錨點及目前位置／seek／offset／drift 仍待後續。見 [Phase 9](docs/phase-9.md) 與 [ADR 0082](docs/adr/0082-bounded-fuzzy-temporal-consistency.md)，Phase 10 尚未具備條件，結案手動金鑰安排不變。

2026-10-23 第七批讓同一模糊核心比較全文所有單段及相鄰雙段，處理 Final ASR 與 cue 分句不同。每原 cue 與 ASR 仍限 256 scalar，雙段含明確空白邊界最多 513；最多 2N−1 個詞彙候選共用原 DP／工作／窗口期限，不跳過超量 cue 或依猜測播放時間排除競爭者。單段／雙段或遠處／缺時間重複同分與差距不足仍拒收；雙段唯一勝出後須時間完整、互不重疊且間隙最多 500 ms，否則明確 TimingMismatch，不回退較弱單段。候選新增完整 cue_range，cue_index 保留第一段索引；多觀測追蹤核對完整索引範圍不重疊，不把共享 cue 的雙段算為獨立支持。Existing／Recognized 來源、獨立 Session／epoch、Ready 弱參考／媒體釋放、取消與共享容量保持，未改精確驗證、仲裁、擷取時鐘或時間軸。結果仍是候選，中文、三段以上、經驗證錨點、目前位置／seek／offset／drift 及一般 Live／GUI 仍待後續。見 [Phase 9](docs/phase-9.md) 與 [ADR 0083](docs/adr/0083-bounded-adjacent-cue-fuzzy-matching.md)，Phase 10 尚未具備條件，結案手動金鑰安排不變。

2026-10-23 第八批新增明確 owned 中文主軌／五語同版本來源參考軌的同步及背景模糊候選入口。兩軌合計 8192 cues／四 MiB，保留原文完整搜尋／單段雙段／DP 與原工作期限；中文不當 ASR 原文。原文唯一勝出後，完整主軌掃描要求逐 cue 起點／長度精確且唯一對應，雙段中文索引必須連續，缺失／差一微秒／重複／不連續皆明確拒收。候選新增 PairedSourceReference 與獨立中文索引，追蹤同時核對兩軌範圍，保留原 allocation／取消／CPU 容量。SourceTextReference 抽出文件驗證，既有 query 的來源／content_id／Session 檢查保持，不製造 capture query／stamp。這是 caller 明確移入與停止的 owned library，尚無 lookup lease／Prepared 中文參考軌自動橋接，不直接輸出中文、不繞過臺灣本地化；仍不轉經驗證錨點、採用或播放位置，精確驗證／仲裁／擷取時鐘／時間軸不改接。一般 Live／GUI、目前位置／seek／offset／drift 待後續。見 [Phase 9](docs/phase-9.md) 與 [ADR 0084](docs/adr/0084-paired-chinese-fuzzy-source-reference.md)，Phase 10 尚未具備條件，結案手動金鑰安排不變。

2026-10-23 第九批新增 FuzzyCueLocator::from_lookup，弱借用既有單軌五語／中文同版本雙軌 lease。原 lookup 1024 cues／128 KiB 與四祖先上限保持；取得原 CPU 名額後才升級本文，沿用原 allocation／搜尋／時間對應，不重抓來源或建立假 stamp。工作實際期限取 offer／dispatch、capture 及原 lookup query 的最早截止，候選／每點觀測／軌跡快照均保留弱發布 guard，renew 或新 ASR 不延長舊 query；閒置管理器到期喚醒、driver 退出。lease.retire／provider 取消／降低信心撤回沿用既有通道，不取消父來源。純本文釋放可由 validate／poll 拒收，無 retire 通知的等待最遲於原期限喚醒；不保留強本文。既有 lookup clear／stop 契約不變，Live owner 仍負責原退休規則。這是明確 library 橋接，未改精確採用／仲裁／擷取時鐘／時間軸；Prepared 中文 Ready、一般 Live／GUI、可靠錨點、目前位置／seek／offset／drift 待後續。見 [Phase 9](docs/phase-9.md) 與 [ADR 0085](docs/adr/0085-weak-lookup-fuzzy-publication-lifetime.md)，Phase 10 尚未具備條件，結案手動金鑰安排不變。

2026-10-23 第十批新增明確 AlignmentController，從 Ready 或 lookup 弱來源被動建立為 Paused，resume 才使用 caller runtime 啟動一個既有 task。pause／reset／stop 先撤回舊候選與觀測，再等待 driver 和輸入清理許可；等待被取消時保留 owner、join handle 與已完成 join 結果，再次操作先完成舊清理。恢復與 reset 配置程序內控制器共用、checked 且不回繞的新 epoch；健康 resume 冪等，無效 capture reset 不動舊回合，文件與 capture 身分分離。stop 為終止，Drop 只緊急取消、不假稱同步 join。未持強本文／媒體 pin、不取消父來源、不延長原 lookup 期限。這是對齊生命週期，不操作播放器、不偵測 seek、不啟動擷取，也不產生可靠錨點／目前位置／offset／採用證據；一般 Live／GUI 與媒體時間規則仍待完成。設計與實際驗證見 [Phase 9](docs/phase-9.md) 與 [ADR 0086](docs/adr/0086-explicit-alignment-epoch-lifecycle.md)。Phase 10 尚未具備條件，結案手動金鑰安排不變。

---

2026-10-23 第十一批接通真正 Prepared Ready 與 Live 的獨立多觀測驗證。至少三筆獨立 Final／cue、950 一致度與 100 次高差距，全部端點誤差不超過 50 ms、速率差最多 ±5%，核對實際連續擷取及最新語音；有符號偏移與最近媒體位置不從牆鐘推進。只有明確 Prepared 路徑保留最多八筆／三十秒 metadata，最新位置仍受原候選五秒期限；原 hint／lookup 契約不延長。跳轉／版本差異撤回並重蒐集，小幅速率差逐筆重新校時。中文主軌可明確指定本機同版本來源參考軌與版本標識，逐 cue 精確且唯一時間對應。一般準備視窗保留 Ready 返回主視窗，使用者自行播放及啟動所選來源；定位暫停／恢復／重設／清除使用有界命令，清除後繼續 Live AI。驗證證據只經既有單一仲裁器／時間軸，不新增 STT／翻譯或保存內容。沿用單段／相鄰雙段、格式、CPU／DP／容量限制，超量與三段以上保留 Live AI。最終完整回歸 1976 項一般測試通過、零失敗，既有 15 項 ignored 不變；fmt／零警告 Clippy／Debug／Release 通過，已具備啟動 Phase 10 的工程條件。本次停止於此門檻，尚未開始 Phase 10；有效金鑰與真實媒體留至結案固定介面手動驗收，不宣稱全面產品驗收。見 [ADR 0087](docs/adr/0087-verified-prepared-live-alignment.md)、[Phase 9](docs/phase-9.md) 與 [工程門檻](docs/phase-9-gate.md)。

## 84. Phase 10 — Performance

輸出：

- Capture overhead
- P50
- P95
- Queue high-water
- Memory
- CPU
- Dropped segment count
- API concurrency

2026-10-23 使用者批准啟動 Phase 10。第一批新增有界、Session 數值限定的擷取診斷：sink 方法耗時、成功交付距 native capture estimate 的年齡、P50／P95 區間上界與 raw／normalized ring 觀測峰值下界。DEBUG／TRACE 才配置延遲 histogram，預設不啟用；callback 無配置、排序、I/O 或等待消費端，原空值／零值、無效時間與飽和明確區分。一般 capture／Live 沿用 stderr 並在清理後補最後統計；STT Session 數不冒充 API 並行數。新增獨立零值 PCM／原正規化／有界 ring 的 Release 合成 probe，不呼叫 WASAPI／使用者音源或雲端。第一批格式、零警告 Clippy、Debug／Release、Release 合成 probe 與完整回歸通過，1987 項一般測試成功、零失敗，既有 15 項 ignored 不變；完整 capture overhead、CPU／memory、STT／翻譯／UI 關聯延遲及實際 API／worker／segment 指標仍待接續，不宣告 Phase 10 完成或 Phase 11 就緒。正式金鑰仍留到結案固定介面手動驗收。設計與實際結果見 [Phase 10](docs/phase-10.md) 與 [ADR 0088](docs/adr/0088-bounded-capture-performance-diagnostics.md)。

---

2026-10-23 第二批完成 Phase 10 工程量測與驗證。同 Session／generation／來源修訂／job／輸出修訂的 STT、翻譯、時間軸採用與原生繪圖提交延遲已接通；不把通知或 mock receipt 當作實際螢幕可見時間。加入 STT／翻譯 queue、實際 provider operation／permit／owned worker、修訂與 Final segment 淘汰／停止未完成計數，以及父程序／精確 helper 的只讀 CPU／private commit／working set／handle 取樣。計時 DEBUG／TRACE 才啟用，固定 histogram 與有界數值 metadata 不含內容，鎖與程序查詢不進 capture callback。Release 三品質／並行／取消各三輪、30 項 Phase 10、11 項亂序及六次十秒自有靜音原生開／關對照全部通過。CPU 合計觀測約 0.31%～0.78% 單核心，開／關差分有噪音，不據此調整既有品質、容量或期限。最終 Release 後獨立完整回歸 2006 項一般測試通過、零失敗，既有 15 項 ignored 不變；fmt／零警告 Clippy／Debug／Release 通過。已具備啟動 Phase 11 的工程條件，本次停於 Phase 10 完成，未提前執行長時間 soak 或大量重啟。真實服務、媒體、播放器及實際可見時間仍留結案固定遮蔽介面手動驗收，不宣稱全面產品驗收或 SLA。見 [Phase 10](docs/phase-10.md)、[工程門檻](docs/phase-10-gate.md) 與 [ADR 0089](docs/adr/0089-session-numeric-pipeline-performance.md)。

## 85. Phase 11 — Hardening

2026-10-28 Phase 11 已依使用者確認範圍結案：主頁／狀態視窗與 OpenAI／Gemini Live Translate 手選語言啟動限制皆回報正常；Chrome 以外來源／播放器免測，剩餘實際雲端斷線／恢復及真實混音／串燒／歌詞亦明確指定免測、保留限制並結案。免測不宣稱通過，不變更功能或隱私規格。交付版本 E3C6C5CD… 不變；既有 2166 項一般測試及格式／Clippy／建置證據保留，本次另有原生影片停止／錯誤／中文 sidecar 單案通過。完整範圍見 [結案紀錄](docs/phase-11-closeout.md)。本規格最後階段為 Phase 11；以下為歷史紀錄，其未結案文字不作目前狀態。

2026-10-28 OpenAI gpt-live-transcribe／Standard、手選來源語言的 Chrome 法文／德文／韓文三輪各收到選項 1，確認原文與繁中持續、暫停恢復及停止保留正常。連同既有英日文與 Gemini Auto 紀錄，兩種辨識服務的五語基本流程皆已有實測正例；不推論量化完整度、延遲或其他來源。版本 E3C6C5CD… 不變，接續主頁配置、手選語言前置阻擋及其他播放器可用性確認；無程式變更或工具自動擷取，Phase 11 未結案。以下保留先前批次紀錄。

2026-10-28 使用者對 Chrome／Gemini Transcribe Live／Auto／Standard 的法文、德文、韓文三輪分別回覆選項 1，確認各輪原文與繁中持續更新、暫停恢復及停止保留正常。記為三語基本流程正例，不推論零缺漏、量化延遲或服務／本機語言依據。版本 E3C6C5CD… 保持，接續 OpenAI gpt-live-transcribe／Standard 的三語手選語言測試；其他播放器矩陣保留。無程式變更或工具自動擷取，Phase 11 未結案。

2026-10-28 更新：已收到 Gemini Live Translate／日文／Standard 十分鐘安排的人工回覆選項 1，確認觀察到輪替後恢復與字幕紀錄保留，記為本次明確條件的穩定性正例，不推論更長時間、所有來源或無縫保證。E3C6C5CD… 版不變，接續其餘三語及來源矩陣，尚須可用音源／播放器資料；Phase 11 未結案。以下保留各批歷史。

2026-10-27 Gemini Transcribe Live／Auto／Standard 以日文來源人工回覆選項 1，確認原文／繁中持續更新、暫停恢復與停止保留。沒有語言依據或數值，不推論特定判定路徑、其他三語或效能目標。Live Translate 日文清除亦已有本次正例，現接續至少十分鐘跨本機九分鐘連線期限的人工驗收，尚未回覆。版本與視窗不變，僅更新驗收文件，Phase 11 未結案；以下保留各批歷史。

2026-10-27 Gemini Live Translate／日文／Standard 人工回覆選項 1，確認手動清除後翻譯繼續、舊段不回補、下一段顯示及停止保留。只勾選該次操作，不推論長時間或其他語言驗收。E3C6C5CD… 版不變，接續 Gemini Transcribe Live 的日文 Auto 測試；服務與開始由使用者手動操作，工具未自動送出音訊。Phase 11 尚有其餘矩陣，未結案。

2026-10-27 同版補測已確認來源日文、停止後字幕紀錄保留，併同掉字較少記為本次人工正例；完整度未量化，不推論零缺漏或五語全面通過。保留 E3C6C5CD… 程式及現有視窗，接續 Gemini Live Translate／日文／Standard 的手動清除與停止保留驗收，未自動切換服務或雲端處理。工程與其他人工矩陣保持原紀錄，Phase 11 未結案。

2026-10-27 同版補測回報「掉字較少」，與先前提示後恢復正例一併記錄為相對改善；沒有來源語言、停止保留或量化完整度資料，不作零缺漏或全面驗收推論。保留 E3C6C5CD… 版，本次僅更新驗收文件，Phase 11 未結案。

2026-10-27 E3C6C5CD… 版使用者回覆選項 2：提示後持續更新並恢復，記為本次恢復流程的人工正例。提示種類、語言、確切時長、停止保留及原文完整度未補，不推論特定原因或五語全面通過。執行檔不變，本次僅更新驗收紀錄，未重跑既有 2166 項測試；Phase 11 尚有人工矩陣與完整度驗收，未結案。見 [工程與人工清單](docs/phase-11-gate.md)。

2026-10-27 因單次翻譯 Incomplete 自動停止的人工回報，新增 Live 要求層級隔離、非過期退休與明確提示；連續第三次仍停止，一般要求／Prepared 仍原子失敗，已知 content_filter 明確 Refused。五項新增、145 項 Phase 11、翻譯相關 59 項一般測試及最後獨立完整回歸 2166 一般成功、零失敗、18 ignored（集合重疊），格式／零警告 Clippy／Debug／Release 通過。來源與執行檔雜湊核對後備份／更新，PID 4420 待命；未執行真正雲端測試。實際持續翻譯與原文完整度待驗收，Phase 11 未結案。見 [ADR 0110](docs/adr/0110-isolate-incomplete-live-translation-requests.md)。以下保留歷史。

2026-10-27 OpenAI 接句缺漏追查已補上接近上限時優先使用已觀察短停頓的收尾策略；四項新增、140 項 Phase 11、65 項串流一般測試與最終獨立完整回歸 2161 一般成功、零失敗、18 ignored（集合重疊），格式／零警告 Clippy／Debug／Release 通過。備份與核對雜湊後更新並開啟 PID 13500 待命。前一字首版已回報非常顯著改善，但接句仍有缺漏；新版真實完整度與其他矩陣待人工，Phase 11 未結案。見 [ADR 0109](docs/adr/0109-prefer-observed-pause-before-openai-turn-limit.md)。以下保留歷史。

2026-10-27 OpenAI 原文缺漏追查補上有限字首音訊與突發回覆消費排程；17 項 OpenAI、136 項 Phase 11、64 項串流一般測試及最終獨立完整回歸 2157 一般成功、零失敗、18 ignored（集合重疊），格式／零警告 Clippy／Debug／Release 通過。標準版備份／雜湊核對後更新，PID 27672 待命。最新人工為英日文基本正常但原文仍缺漏，Gemini 小聲改善；新版真實長句與其他矩陣待驗收，Phase 11 未結案。見 [ADR 0108](docs/adr/0108-openai-bounded-prefix-and-consumer-fairness.md)。以下為歷史。

2026-10-23 使用者批准啟動，要求持續至本階段完成，除非必須人工排除或指示。自動工程驗證已完成：修正具名 WebSocket EOF 分類，通過自有靜音原生三十分鐘、DEBUG 三百次重啟、故障／隔離短測、本機 heartbeat、五分鐘明確 mock 管線與一百次 Session 切換。最終完整回歸 2017 項一般測試成功、零失敗、16 項 ignored；fmt／零警告 Clippy／Debug／Release 通過。實體裝置操作、真正瀏覽器／服務／媒體驗收仍需人工，不把模擬結果冒充通過。進度、數據與未完成條件見 [Phase 11](docs/phase-11.md)、[工程與人工清單](docs/phase-11-gate.md) 及 [手動操作](docs/phase-11-manual.md)。目前停於必要人工驗收，尚未結案。

後續人工回報 Gemini 按開始即中斷、尚無字幕，未通過真實服務驗收。已追加安全連線階段與數字代碼，並修正 rustls 驗證錯誤經 I/O 包裝後被誤分為一般網路中斷；不關閉憑證驗證或換服務。追加批次 fmt／零警告 Clippy／Debug／獨立 Release 與新完整回歸通過，最新 2022 項一般測試成功、零失敗、16 項 ignored；標準執行檔已備份及更新並開啟待命。追加驗證與手動重試見 [Gemini 安全診斷](docs/phase-11-gemini-diagnostics.md)，仍不宣稱本次真實失敗根因已確認或全面結案。

再依初始化 1011 及本機金鑰儲存狀態回報，完成各服務獨立「檢查本機」、主視窗儲存後讀回核對與 ServiceInternal 分類。本批最新回歸 2026 項一般測試通過、零失敗、16 ignored，fmt／零警告 Clippy／Debug／Release 通過；標準執行檔已備份及更新，新版待命視窗已啟動。本機中繼資料確認存在不代表服務權限，仍待使用者按固定介面檢查可讀性與實際雲端驗收；未改來源、模型、TLS 驗證、擷取或定位界線，Phase 11 維持未結案。

測試：

- 長時間 Live
- 多次切換 Source
- Network disconnect / reconnect
- AI timeout
- Process exit
- Audio device change
- Prepared cancellation
- Temp cleanup
- Mixed audio
- Medley
- Browser multi-process audio

---

## 86. Definition of Done

功能必須符合：

1. 符合本規格
2. 可編譯
3. 測試通過
4. 無新增 Clippy warning
5. Error handling 完整
6. Audio Capture 不阻塞
7. Queue 有界
8. Concurrency 有界
9. Task 可取消
10. Session isolation 正確
11. 無 Thread/Task/Handle leak
12. 無 Hard-coded secret
13. 無永久 Audio
14. 無永久 Subtitle Cache
15. 無永久 Lyrics Cache
16. 無永久 Translation Cache
17. Remote data Session-only
18. 簡中正確轉為臺灣 zh-TW
19. unsafe 有 SAFETY 說明
20. 文件同步
21. 已知限制明確
22. 未測試項目不得宣稱通過

---

## 87. 最終架構

```text
                     Windows Audio
                           │
                           ▼
                    Process Capture
                           │
                           ▼
                     Ring Buffer
                           │
                           ▼
                   Audio Preprocess
                    │      │      │
          ┌─────────┘      │      └──────────┐
          ▼                ▼                 ▼
     Streaming STT    Classification    Fingerprint
          │                │                 │
          └─────────┬──────┴───────┬─────────┘
                    ▼              ▼
                Content Resolver
                    │
       ┌────────────┼────────────┐
       ▼            ▼            ▼
    Live AI       Lyrics      Subtitle
       │            │            │
       └────────────┼────────────┘
                    ▼
             zh-TW Translation
                    │
                    ▼
              Stabilization
                    │
                    ▼
            TranscriptTimeline
                │        │
                ▼        ▼
            SideChat   Bottom
                │        │
                └── Dual ┘
```

---

## 88. 最高工程原則

```text
Never block audio capture.
Keep latency bounded.
Keep memory bounded.
Keep concurrency bounded.
Never persist captured audio.
Never create persistent subtitle/translation/lyrics caches.
Remote subtitles are session-only.
Always output Taiwan Traditional Chinese.
Prefer verified existing text when correctly identified and aligned.
Fall back gracefully to Live AI.
Use one TranscriptTimeline.
Use multiple renderers, not multiple translation pipelines.
Make cancellation explicit.
Make session identity explicit.
Correctness before clever optimization.
```
