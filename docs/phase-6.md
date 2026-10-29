# Phase 6 — Content Resolver

2026-10-09 依使用者「批准啟動phase 06」開始。基準仍為本機 Windows 10 19045；正式金鑰與真實服務測試留至結案互動輸入。Phase 5 的 [待驗收項目](phase-5-acceptance.md) 保留，沒有因啟動下一階段而改列通過。

## 首批範圍

src/resolver 現在包含 contracts、window、candidates 與 session；沒有 Win32、renderer、檔案或供應商 JSON。ContentType 固定 Unknown／Speech／Music／MovieOrShow／Mixed，SubtitleSource 保留五種概念來源；沒有直接操作 UI 或新增 timeline。正式 GUI／命令列仍執行原有 Live STT／翻譯，尚未建立內容服務或額外傳送音訊。無命令啟動回報 phase 6 與 content_resolver_configured=false。

ContentIdentifier 表示服務／演算法的證據，不由核心猜歌曲或製造信心分數。FingerprintExtractor 是有界 opaque 指紋介面，尚無正式演算法／目錄 adapter；CPU 指紋工作必須由有界 blocking worker 執行。SongCandidate 包含服務內穩定 ID、歌名／演出者、信心與可選 match offset；MovieCandidate 保留來源線索與 metadata，只是候選，不是已驗證字幕。分類、歌曲、電影與取消能力明確檢查，核心不按供應商名稱分支或跨服務降級。

ExistingTextProvider 共用歌詞／字幕查詢契約，結果限於同一 Session／content ID／來源。支援原生 zh-TW、其他繁中、簡中及固定五語的 source language 標記；歌詞可未定時，字幕須有合法順序／時間／長度。單次最多 8192 cues、4 MiB 文字、每 cue 8192 bytes。這批只驗證資料契約，簡中原始來源還未本地化、翻譯、校時或呈現；Phase 7 才將查詢結果整合進單一 timeline／來源仲裁，沒有提前啟用該階段。

## 有界視窗與要求

SlidingAudioWindow 只接收既有預處理 worker 的 16 kHz mono i16 ResampledAudio。初值為 12 秒、每 3 秒產生一次 192000-sample 視窗，符合 SPEC 約 8–15 秒／2–5 秒的設計區間；這是工程初值，尚無真實辨識效能 benchmark。保留原生未知時間為 None，沿用 FIR 延遲補償；順序缺口、discontinuity、時間跳動或過期輸入清除舊視窗，不把缺口拼成連續音訊。最多 1024 packet 時間記錄，過多微小封包重設而非增加記憶體。

滑動 PCM 上限約 375 KiB；每個 immutable 視窗約 375 KiB，resolver 最多一個進行中的要求與一個最新待處理視窗，輸出也只留最新通知。呼叫端須在有界 CPU／orchestration worker 組窗，不能從 capture callback 組窗／等待服務；正式分流 worker 尚待實作。Session／generation／sequence、monotonic capture estimate 與可選來源時間隨視窗傳遞。

ResolverTask 由 owner 持有並 poll，核心不 spawn／detach 工作。每次要求最多三秒、視窗 freshness 五秒；新視窗取代未完成要求時，drop future 並取消該要求專用 token。Session 取消、Session Drop 或 worker future Drop 都回收 input 與要求。服務錯誤發布安全的 Failed／Unknown，不取消 STT、不自動換供應商；本批沒有自動 retry，下一個新視窗才再次詢問同一服務。adapter 必須在取消／drop 時釋放自身遠端／CPU 工作。

## 候選策略

五個近期視窗內三票才 Confirmed；歌曲信心至少 800/1000，第一／第二候選至少相差 100。分類門檻 700/1000。票保留最多十五秒，既有 confirmed 最多六秒；一次弱 miss 可維持既有候選，兩次移除。可信的新歌曲與既有歌曲衝突或候選接近時，呈現 Transition／Mixed 並取消確定歌曲輸出，累積足夠新票才切換；Speech／明確 Mixed 會清除歌曲票。低信心、不同 Session、舊順序與過期結果不會形成新確認。

這些門檻只控制服務證據的採用策略，沒有宣稱是真實概率或已量測準確度。歌曲確認也不能跳過文字來源的 alignment／confidence 檢查；Movie metadata 不會直接採用字幕。設計理由見 [ADR 0014](adr/0014-session-content-resolver.md)。

## 已測與未完成

首批新增十四項離線測試：滑動視窗的實際 PCM 順序／間隔／上限／未知時間、來源時間與 discontinuity、缺口／過期／非法／不同 Session、三票與 crossfade、弱 miss／expiry／Speech／舊序列、候選／指紋大小契約、Movie metadata、既有文字契約、有界輸入／輸出、服務錯誤、要求 supersession、三秒 timeout、Session Drop／pre-cancel，以及 worker future Drop 釋放音訊。慢速 MockContentIdentifier 與實際 SpeechSession 核心同時運作，後者的 cfg(test) recognizer 能輸出 Partial／Final；只證明這個核心邊界可以並行，不外推正式 WASAPI／GUI 接線或端到端延遲。

所有歌曲、媒體、文字及音訊均為明確合成測試，沒有真實服務、正式金鑰或新的實際音訊擷取。cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked 與 cargo build --locked --release 全數通過。完整一般測試 220 項（194 library、12 設定、4 探索、10 啟動）；主測試 14 ignored 的組成仍為十三項既有硬體／手動／外部服務與一個由正常 parent 實際執行的資源 fixture，沒有新增略過項目。首次格式指令在測試檔尚未建立時回報找不到 tests 模組；完成測試檔後格式、Clippy 與全部驗證通過，未放寬 lint。

首批結束時，正式 fingerprint 演算法／資料目錄、Speech／Music／Mixed 的實際證據來源、歌曲／Movie provider adapter、既有歌詞／字幕查詢 adapter、Live 有界分流與狀態接線仍待完成。後續第二批如下；Phase 6 尚未結案，沒有啟動 Phase 7。

## 第二批：本機指紋與固定 CPU worker

新增實際 ChromaprintExtractor，使用固定 rusty-chromaprint 0.3.0 test2 configuration，接收既有 16 kHz mono i16、12 秒音訊視窗；壓縮結果標成 ChromaprintTest2Compressed，Opaque 保留作為其他契約格式。每個 bounded window 都在 CPU worker 內新建／使用／釋放 Fingerprinter，沒有用 unsafe 改變 !Send／!Sync，也沒有原生 DLL、解碼器、音訊檔或網路要求。API 與演算法範圍見 [套件來源](https://github.com/darksv/rusty-chromaprint) 與 [Fingerprinter 文件](https://docs.rs/rusty-chromaprint/0.3.0/rusty_chromaprint/struct.Fingerprinter.html)。

近乎靜音的 RMS gate 只排除不可用訊號（16 PCM units），不把有聲音當成 Speech 或 Music。維持原生未知時間 None；輸入、完成與取用時都檢查 Session／generation／sequence／freshness／尺寸。每 1600 samples 的 feed 前檢查取消，finish 前後也檢查；套件內部 FFT／重採樣 batching 仍可能超過一次 feed 的工作時間，沒有宣稱 100 ms 取消 SLA。指紋在 RAM，不提供 Debug／Serialize／寫檔／匯出，也沒有自動發到其他服務。

FingerprintWorker 為單一 Live resolver 要求建立一個固定 CPU lane：available_parallelism 作為 budget 輸入、上限一，pending queue 容量一。新提交取消 active token、淘汰 queued window。提交／結果發布／停止透過短 metadata lock 保持順序；stop 與 owner Drop 取消 active、清空 queue／completed、unpark 並 join，停止後不能再入列。所有 lock 都在 CPU 計算／await 前釋放，capture callback 不呼叫這個 API。

completed 指紋只有一個欄位；oneshot 回覆只存完成／錯誤狀態。新工作清除舊 completed，取用時再檢查最新序列／時間，舊回覆回報 Stale；保留未讀回覆不會保留多份結果。drop await future 取消自己的工作並清除同份未取用結果。panic 轉成固定 typed WorkerPanicked，測試驗證 worker 仍可回收；cfg(test) 人為 panic 的 Rust 預設 hook 會印固定 synthetic extractor panic 訊息，不把該訊息當成測試失敗。

CatalogIdentifier 連接真實本機指紋、呼叫端明確提供的 FingerprintCatalog 與原 ResolverSession／CandidateTracker。目錄必須支援固定格式與取消；只回傳歌曲／Unknown evidence，不偽裝語音／電影分類能力，不建立第二條 STT、不直接改字幕或 UI。測試以真實 Chromaprint 產生指紋，再由明確 mock 目錄回覆候選，驗證三票確認與取消 lookup；實際目錄服務尚未選定／接線，沒有把這條 mock 測試當成真實歌曲識別或目錄相容性驗收。

新增十一項測試：實際演算法 determinism／不同合成音訊／feed chunk 與上游完整 consume 的壓縮結果一致；靜音、時間／尺寸／預先取消；真正 CPU worker 結果與時間身分；50 次壓力提交僅保留 active／最新 pending PCM且不發布過期成功；future Drop／owner Drop；慢消費端單一 completed／舊回覆；人為 panic；20 次 stop／submit race；CPU budget／來源拒絕；真實指紋到 resolver 三票的完整核心路徑；目錄格式／lookup 取消。所有 PCM 與候選是測試合成內容，沒有音訊播放／實際擷取、正式金鑰或額外雲端傳送。

相依版本固定在 Cargo.lock，新增套件授權宣告與原始通知見 [授權紀錄](../licenses/NOTICE.md)。設計理由見 [ADR 0015](adr/0015-local-fingerprint-cpu-worker.md)。首次 Clippy 指出手寫 max／min 的 clamp pattern，已依建議改用 clamp；沒有放寬 lint。cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked 與 cargo build --locked --release 全數通過。完整一般測試 231 項（205 library、12 設定、4 探索、10 啟動），主測試既有 14 ignored 的組成與另由 parent 執行的資源 fixture 不變。沒有新增 ignored，也沒有把編譯或 mock 通過當成真實歌曲驗收。

目前仍待實際指紋目錄／歌曲服務、Speech／Music／Mixed 實際分類證據、Movie metadata provider、歌詞／字幕查詢 provider、Live／GUI 有界分流及狀態。真實內容矩陣、指紋魯棒性、取消與端到端效能尚待實測。現行 GUI 仍只執行原 Live STT／翻譯、不產生或查詢指紋；content_resolver_configured 仍為 false。正式金鑰與真實服務驗收仍留至結案手動輸入，Phase 6 尚未結案。

## 第三批：音訊分流核心與缺口重設

新增 ResolverAudioWorker，將 NormalizedAudio 的 48 kHz mono RAM 副本交給固定 CPU lane，使用既有 StreamingResampler 轉成 16 kHz，再由 SlidingAudioWindow 產生十二秒／每三秒的視窗。不在 Tokio executor 計算 FIR、不建立另一條 STT／翻譯；capture callback 與 Win32 模組未修改。available_parallelism 作為 budget 輸入、單一串流上限一個 worker。

input 容量十二個 block（正常 20 ms block 合計 240 ms，約 45 KiB PCM）；output 只有最新一格。滿載清除 backlog、保留最新 block；順序／來源時間跳動、原生 discontinuity、過期或非法輸入則重設，不串接缺口前後音訊。錯誤來源與已過去的序列／時間拒收，不能重設目前有效音訊。開始時刻未知仍保留 None，沿用 FIR 延遲補償；沒有把接收時刻當成播放 offset。

audio reset epoch 隨最新通知／後續視窗持續存在。ResolverSession.invalidate 清除 pending window、立即過濾舊 epoch 輸出；task 下一次被 poll 時取消 active request 並清除歌曲投票。即使 reset 通知被新視窗取代，仍不能重用 confirmed 歌曲。已完成的舊 epoch CPU block 不發布；owner 取用辨識結果前再次檢查 audio epoch。缺口狀態沿用上次視窗的真實 stamp，不製造新視窗或 fake offset。新音訊須重新收集完整十二秒才發出視窗。

LiveResolver 持有音訊 worker、ResolverSession 與 caller-polled provider future。它不自行 spawn／detach、不操作 renderer／timeline；呼叫端要將 changed() 與 STT／翻譯共同 poll。submit_live_audio 先讓 SpeechInput 接受原始 block，再提交 resolver 副本；resolver 回報錯誤不取消 SpeechSession。沒有設定 resolver 時不複製音訊或啟動 CPU lane。這是可測試的 library 組合入口，正式 app／GUI 尚未提供 resolver factory／設定及狀態，也沒有啟用額外服務。

stop／Drop 取消要求、drop provider future、清除音訊佇列／最新視窗、unpark 並 join；保留的 input handle 會拒收停止後提交。CPU panic 轉為 WorkerPanicked，保留固定安全訊息；測試 hook 的 synthetic audio worker panic 由 Rust 預設 hook 輸出，不代表測試失敗。新增 metrics 提供 queue depth、processed／dropped blocks、reset／worker count；尚未接到 GUI／app debug 輸出。新程式碼沒有 unsafe、網路、檔案、私密內容日誌或永久資料出口。

新增十三項離線測試，涵蓋真實 FIR PCM 與直接處理結果完全一致、來源時間／未知時間、三秒間隔、十二秒重新收集；50 block 壓力淘汰與舊 active epoch 排除、CPU 排隊過期、來源／舊序列／非法尺寸／NaN／原生缺口／frame 跳動、panic／Drop、20 次 stop／submit race；核心要求取消與 coalesced reset 投票清除；實際 SpeechSession 邊界的 Partial／Final 與慢 resolver 並行、STT 拒收不分流、resolver 拒收不停止 STT、owner Drop 回收，以及已完成未讀 Ready 在缺口後被過濾。音訊與結果全部是明確合成 fixture，不代表真實 STT／歌曲識別／端到端延遲或播放器相容性驗收。

首次 targeted run 有兩項測試預期不符既有契約：舊接收時間應拒收而不再次重設；Candidate 可以包含未確認歌曲 metadata。測試已修正為檢查 reset 次數與 Candidate／Confirmed 狀態，沒有放寬正式邏輯或 lint。最後將測試專用 gate 改為有界 channel／逾時／disconnect，避免 assertion 失敗時卡住 worker join；調整後再次執行全套測試。

cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked（最後再以 --quiet 重跑全套）、cargo build --locked 與 cargo build --locked --release 全數通過。完整一般測試 244 項（218 library、12 設定、4 探索、10 啟動），既有 14 ignored 及另由正常 parent 執行的隔離 GUI 資源 fixture 不變，沒有新增 ignored。沒有執行新的真實擷取／雲端或播放器矩陣，也不把這些測試時間當成端到端效能 benchmark。

實際歌曲目錄、分類／Movie／歌詞／字幕 provider、正式 GUI 組合與狀態仍待完成。[AcoustID 官方參數](https://acoustid.org/webservice) 要求整首音訊時長，不能以十二秒 sliding window 代用，所以本批沒有建立會送出錯誤 duration 的 adapter。設計理由見 [ADR 0016](adr/0016-resolver-audio-fanout-reset.md)。沒有讀取／索取正式金鑰、實際擷取或額外雲端要求；結案時的互動輸入安排不變，Phase 6 尚未結案，未開始 Phase 7。

## 第四批：LRCLIB 歌詞 adapter

新增 LrcLibService／LrcLibLyrics，實作 ExistingTextProvider 的歌詞查詢；不是 mock HTTP 回覆的正式替代。固定 [官方 HTTPS endpoint](https://lrclib.net/docs)、驗證 TLS、停用重新導向，只有明確配置此服務後才可查詢 Confirmed 歌曲的歌名／演出者，以及可選的已知專輯／整首時長。現行 app／GUI 未建立這個 Service，無額外正式查詢；STT 的 OpenAI／Gemini 選項不自動選取它。未知時長不使用十二秒視窗代填。

各歌曲 binding 共用同一 Session Service 的一個 request permit 與冷卻狀態，不排入等待佇列。最小間隔 500 ms，要求三秒逾時；429／503 依 Retry-After 秒數或 HTTP 日期冷卻，沒有欄位時六十秒，非法／溢位欄位停止該 Service 後續查詢至 owner 回收。沒有自動 retry／跨服務 fallback／背景 task。查詢 future Drop、取消或逾時釋放 permit 與傳輸；結果取用前再次檢查 Session／generation／content ID／時間，遲到的 404 也不能作為目前來源狀態。

回覆要求 JSON、最多 128 KiB，header 與 chunked body 都檢查上限。metadata 再比對歌名／演出者及已知的專輯／整首時長；只正規化大小寫與空白，錯誤版本不剝除後綴來湊匹配。ContentMismatch、取消、逾時、限制速率、傳輸／格式錯誤都用固定訊息，不回傳或記錄服務錯誤本文、private URL 或 lyrics payload。

LRC 保留原始行級時間、小數秒、chorus 多個標記、相同起點與空白時間標記形成的空隙。cue 尾端依下一個不同標記或回覆的實際整首時長計算，不設 fake duration／播放起點。parser 最多 1024 個標記／cues、展開文字 128 KiB、每 cue 8192 bytes；不支援非零 offset、enhanced word timing／角括號或非法標記，回報錯誤而非改成 plain。沒有 synced 時才回傳明確未定時的 plain lyrics。純演奏、404 與無文字回報沒有候選；純演奏卻含歌詞等矛盾資料拒收。新增 httpdate 1.0.3 的固定相依與原始授權通知，沒有安裝額外 Windows 元件。

來源語言由 caller 提供已知 TextLanguage，API 本身不提供語言；不能將未知語言默認為英語或 zh-TW。原始簡中文字也仍標示為來源，未直接顯示。臺灣本地化／ASR 定位／來源仲裁／timeline 呈現仍屬 Phase 7，沒有提前完成或啟用。回覆歌詞／metadata 只在 Session RAM，沒有寫檔、快取、Debug／Serialize、匯出或遙測。

新增十四項測試，全部是合成資料及本機 HTTP：Confirm／metadata／固定 endpoint；URL 編碼與沒有音訊／指紋／source name／憑證；已知／未知整首時長；synced／plain／純演奏／404；錯誤歌曲／專輯／時長；外部 Session generation／舊 content ID／來源／預先取消；時間小數／chorus／同時起點／空隙；損壞 synced 不降級；cue／展開大小／型別；MIME／Content-Length／chunked 上限／非法 JSON；跨歌曲的 Retry-After 秒數／日期／缺省／非法欄位；並行拒收、取消／Drop 的真正 socket EOF／permit 回收；三秒逾時、遲到成功與 404；服務錯誤與 redirects 不轉送 metadata。沒有讀取真實音訊、歌詞或正式金鑰，也沒有測試真實 LRCLIB 服務。

首次十四項 targeted 測試通過；首次 Clippy 指出測試中的 MutexGuard lifetime 無法由顯式 drop 證明已在 await 前結束，已改為 lexical scope，沒有放寬 lint。cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked --quiet、cargo build --locked 與 cargo build --locked --release 全數通過。完整一般測試 258 項（232 library、12 設定、4 探索、10 啟動），既有 14 ignored 的組成及另由正常 parent 執行的隔離 GUI 資源 fixture 不變，沒有新增 ignored。Debug／Release 建置通過不代表真實 LRCLIB 或端到端同步已驗收。設計理由見 [ADR 0017](adr/0017-session-only-lyrics-provider.md)。

Phase 6 仍待實際歌曲目錄、Speech／Music／Mixed 分類證據、Movie metadata resolver、字幕查詢 provider、正式 GUI 組合及狀態，以及真實歌曲／歌詞／播放器矩陣與效能驗收。歌詞 adapter 的本機協定測試不代表實際歌曲已識別或來源已定位；正式金鑰與真實服務測試安排仍為結案互動輸入。

## 第五批：電影／影集 metadata 交叉核對核心

新增 src/resolver/movie.rs 的 MovieMetadataResolver／MovieQuery／MovieCatalog。query 只接收同一視窗／Session 的有界片名線索、basename、當前 ASR 對白與可選的年份／語言／實際整片時長，沒有 PCM、指紋、完整路徑或歷史逐字稿。實際目錄須由 caller 明確提供，本批沒有正式服務 adapter、金鑰讀取、網路要求或 GUI 接線。

實際文字比對支援大小寫／標點／空白正規化、完整 token 邊界與目錄片名別名。至少一個視窗標題／檔名線索與二十四個英數／Unicode 字元以上的 ASR 對白同時符合；相同標題複製到檔名不能當成兩個獨立來源。已知年份／語言／片長在目錄缺漏或不符會回到 Unknown，片長容許五秒差異，未知時不從擷取視窗推導。這是保守的 exact phrase 子集，不是模糊比對、已量測準確率或字幕 alignment。

目錄須支援取消並提供作品實際對白參考；不能把 query 回聲、人氣或搜尋排序當成證據／辨識信心。每次最多四個作品，每作品最多四個 metadata 別名／四個 4096-byte 對白參考，metadata 沿用 1024-byte 上限。低於原有分類門檻的候選拒收；採用候選保留原信心，不因線索較多而製造分數。多個相符作品仍是多個候選，不自動採用第一個，也不建立字幕或播放 offset。

每個 resolver 只允許一個進行中要求、無 pending queue，並行直接回報 RateLimited；每次最多三秒，沒有自動 retry 或 spawn。sequence／capture estimate 須單調，query 和結果重新檢查工作階段／五秒 freshness；reply 的完整 stamp 必須相同。request token 在取消、timeout、future Drop 與完成時回收，並行名額釋放，不取消 Live STT 或 caller 的 Session token。所有內容僅在 Session RAM，沒有 Debug／Serialize、檔案、快取、遙測或 unsafe。

新增十六項測試，全部為原創合成 metadata／對白與明確 MockCatalog：能力要求；query 路徑／控制字元／大小及不代用視窗時長；獨立對白／片名與原信心；檔名別名／五語資料與 Unicode；弱線索不查目錄；重複 metadata／完整 token；年份／語言／時長不符／缺漏；有界模糊候選／排序；reply 身分／順序／時間／重複 ID／欄位上限；舊順序／時間／外部 Session／預先取消；並行拒收與取消釋放；future Drop 只取消 request；三秒 timeout／不自動重試；查詢過程變舊；型別化失敗；真正比對結果流入既有 CandidateTracker。沒有實際音訊／電影／目錄、正式金鑰或雲端驗收。

十六項 targeted 測試通過。cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked --quiet、cargo build --locked 與 cargo build --locked --release 全數通過。完整一般測試 274 項（248 library、12 設定、4 探索、10 啟動），既有 14 ignored 的組成與另由正常 parent 執行的隔離 GUI 資源 fixture 不變，沒有新增 ignored；沒有放寬 lint 或新增相依套件。這些測試不代表正式目錄／電影／播放器相容性或端到端效能已驗收。設計理由見 [ADR 0018](adr/0018-movie-metadata-verification.md)。

Phase 6 仍待實際歌曲／電影目錄 adapter、Speech／Music／Mixed 分類證據、字幕查詢 provider、正式 GUI 組合及狀態，以及真實內容矩陣與效能驗收。本批完成電影 metadata 比對核心，沒有將合成測試列為實際電影辨識通過。Phase 7 尚未啟動，正式金鑰與真實服務測試仍依安排留至結案互動輸入。

## 使用者指定的固定金鑰介面

依本次明確要求，主視窗改為固定的 OpenAI（ChatGPT）與 Gemini 獨立遮蔽欄位，各有明確儲存至 Windows 認證管理員的按鈕；預設輸入仍只供本次啟動。這是使用者指定的 UI 修正，不是 Phase 7 或內容服務啟用；resolver／目錄／歌詞服務沒有額外傳送。現行操作、安全邊界與驗證見 [固定金鑰介面](api-key-ui.md)。正式金鑰仍留至結案手動輸入，未因固定介面改成開發期間使用。

## 第六批：已確認歌曲與有界文字查詢生命週期

新增 src/resolver/text_session.rs 的 ExistingTextSession／ExistingTextTask，以同一 ExistingTextProvider 接收明確選擇的歌詞／字幕要求。select_confirmed_lyrics 將 settled 的 Music／Confirmed 候選轉為歌詞 query；Candidate／Transition／Mixed／Unknown／Speech 清除舊文字，缺少 provider 或歌曲 ID／信心損壞回報錯誤。caller 不能將 ResolverStatus::Resolving 的暫時 Unknown 餵入這條線；adapter 仍須明確綁定該歌曲、來源語言及實際 metadata。

一個進行中 lookup、一個最新 pending 選擇、一個最新 publication，沒有 FIFO 堆積或背景 spawn。同一 content ID／source／provider Arc binding 的重複 fresh 視窗只延長六秒選擇 lease，不重抓、不取消進行中 lookup、不修改原 query 時間。Unavailable／Failed 不自動 retry，明確 refresh 或換 binding 才重新安排。新歌曲／來源／provider、clear／reset、stop 或 Drop 都取消 request；舊文字即刻從 output slot 清除，版本門檻與短暫 input publication gate 防止 check／write race 留下隱藏舊文字。

每次最多三秒，進入與結果驗證五秒 query freshness、Session／generation／sequence／capture estimate。poll／next 在沒有 fresh 選擇超過六秒時清除並回報 Stale，不靠永遠持有第一個 Confirmed 來延用文字。這個 Live owner 只接受最多 1024 cues／128 KiB，先檢查總 byte budget 再做字元／時間驗證；通用較大字幕契約尚須其他有界 CPU lane，沒有宣稱長片／Prepared 處理已完成。epoch 溢位 fail closed，不回繞、不保留待處理文字。

新增十七項測試：十四項 MockTextProvider 情境（重複 confirmed、50 次替換、Transition／弱候選、未讀文字回收／binding／來源更換、明確 refresh、timeout／過期、Session／Task Drop、pre-stop、不同 Session／順序／clock／能力、缺設定／損壞確認、非法／超限回覆、poll／next lease、epoch overflow），一項 200 次真正跨執行緒 clear／publication 競爭，以及兩項真正 LRCLIB adapter／local HTTP 路徑。後者驗證完整有界查詢到 LRC、同首不重抓、Transition 清除，以及 clear 後 server EOF／permit 回收。沒有真實音訊／歌曲／歌詞、正式金鑰或額外雲端要求。

首輪編譯的 Result error type 與測試 Barrier Arc ownership 已修正；未使用的 sequence 改為實際斷言查詢身分。並行檢查發現單靠結果版本過濾仍可能讓舊回覆覆寫空槽，因此加入短暫 publication gate 與共享槽位競爭測試，沒有放寬 lint。十四項 owner targeted 與兩項 HTTP 整合所在的 resolver 測試批次通過；新增競爭測試後，cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked --quiet、cargo build --locked 與 cargo build --locked --release 全數通過。完整一般測試 297 項（271 library、12 設定、4 探索、10 啟動），既有 14 ignored 與另由正常 parent 執行的隔離 GUI 資源 fixture 不變，沒有新增 ignored 或相依套件。沒有以這些測試宣稱真實服務、來源辨識或效能已通過。設計理由見 [ADR 0019](adr/0019-bounded-existing-text-selection.md)。

正式 app／GUI 尚未建立這個 text owner／內容服務，content_resolver_configured 仍為 false；固定 OpenAI／Gemini 金鑰介面維持不變，沒有在開發期讀取正式憑證。實際歌曲／電影目錄、分類證據、字幕 provider、GUI 組合與真實內容／效能驗收仍待完成。Phase 6 尚未結案；文字本地化／仲裁／定位／呈現仍為未啟動的 Phase 7。

## 第七批：正式 Live 內容分支與 GUI 狀態

新增 app/content.rs 的 ContentConfiguration／ContentServiceFactory／LiveContent，將第三批的 LiveResolver 接入正式 app/live.rs 音訊 drain 與 select loop。STT 先接收原有 NormalizedAudio，只有明確配置內容分支才複製固定大小 RAM 音訊；內容查詢與 STT／翻譯共同 poll，不等待辨識結果才產生字幕。未配置時不建立 resolver CPU lane、不複製音訊、不呼叫內容 factory；原有 capture callback／Win32 capture 未修改。

run_controlled_with_content 是正式 GUI 的明確組合入口，每次新 capture／同服務重連都以新 StreamIdentity 建立 identifier，不沿用舊分類或候選。factory 同步 create 不得做網路或長 CPU 工作；實際 adapter 須有界並支援取消。本批沒有新增正式 provider 或選取控制；一般 --ui、既有命令及無參數啟動仍是 None／content_resolver_configured=false，OpenAI／Gemini 的選擇不自動啟用內容服務。

UiFrame 新增有界處理狀態與本機／遠端服務提示，名稱為編譯期固定值、最多 96 bytes、非空且無控制字元。提示版本與 source disclosure 共用可見門檻；更新內容服務提示後，舊 acknowledgement 不能提前放行。畫面呈現後才繼續 STT 憑證／連線與後續內容 factory，沒有新增使用者確認問題。狀態只含分類、SongState 及固定型別錯誤，不含作品名稱、ID、歌詞、字幕、指紋或 service payload。

狀態共用最新 mailbox、同值去重、依 Session／generation 拒絕舊更新，保留原有 Arc<TimelineSnapshot>，不重建辨識或翻譯。歌曲 Confirmed、Transition、Movie／Mixed 候選明確標示字幕仍使用即時翻譯；內容狀態沒有直接採用來源或修改文字。Ready／Resolving 再驗證 stamp／順序，六秒沒有 fresh 視窗時主動顯示過期；沒有新音訊也會清除。音訊缺口回到收集狀態，不延用舊分類。

初始化失敗／panic、provider future panic、CPU 分支失敗只回收內容分支，不取消或重連 STT／翻譯。未設定／已停止／初始化失敗的 changed 保持 pending，避免 busy loop；一般服務錯誤仍受既有三秒要求、有界視窗及 adapter 限制。stop、來源切換、重連、Drop 取消 request、清除音訊並 join；UI 停止／重連同步清除內容狀態。Debug 增加內容音訊 queue／processed／dropped／reset／worker 數，沒有私密 payload 日誌或持久內容。

新增十三項測試：九項 app 組合情境（未設定零 worker／不通知、factory 失敗／panic、20 次新 Session 建立／回收、STT 拒收不分流／resolver 停止不取消語音、無新視窗過期、非法提示拒收、真正 FIR／組窗／LiveResolver 與 MockSpeech／MockTranslator 平行、provider panic 回收、UiWorker 等待可見提示才使用 factory）；三項 mailbox／提示門檻／舊 Session／未公開服務／snapshot／Drop 測試；一項本程式隱藏原生視窗的狀態呈現／共用字幕／停止測試。音訊、辨識、文字與目錄皆是明確合成 fixture，沒有擷取其他應用程式、讀取正式金鑰或新增雲端要求。

首輪新增 app 七項及後續包含新測試的 content 十八項 targeted 批次通過。測試 borrow 衝突與 Clippy 的 nested if 已修正，沒有放寬 lint；factory／provider 的 synthetic panic 由 Rust 預設 hook 輸出，已驗證捕捉、取消與回收。遠端提示使用「內容資料」，避免把僅傳送指紋的 adapter 誤稱為上傳原始音訊。設計理由見 [ADR 0020](adr/0020-live-content-composition-status.md)。

最終 cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked --quiet、cargo build --locked 及 cargo build --locked --release 全數通過。完整一般測試 310 項（284 library、12 設定、4 探索、10 啟動），既有 14 ignored 與另由正常 parent 執行的隔離 GUI 資源 fixture 不變，沒有新增 ignored 或相依套件。最後文字調整後已再執行 Clippy／全套測試與 Debug／Release 建置；沒有以合成測試或建置時間宣稱真實辨識、播放器相容性、效能 SLA 或 Phase 6 已驗收。

仍待實際歌曲／電影目錄、分類證據、Movie hints、字幕 provider、文字 owner 的正式 GUI 組合、服務設定及真實內容／播放器／效能驗收。本批完成可配置的正式 Live 分支與狀態，不宣稱預設已能識別真實歌曲。固定 OpenAI／Gemini 金鑰欄位與結案手動輸入安排不變；Phase 6 尚未結案，Phase 7 尚未啟動。

## 第八批：取消與過期結果在共享槽位立即清除

唯讀複查發現 ResolverSession 尚有三個具體缺口：外部取消後、task 未再次輪詢時，poll 仍可取用未讀 Ready；結果超過原有五秒 freshness 後未重新檢查；invalidate 只改 input，output 仍持有舊 metadata。正式 LiveContent 的二次過濾不能代替這個公開介面的共同契約，因此本批先修正生命週期，而未建立新的內容服務。

Session 新增同步 stop，替換／invalidate／stop 清除共享 output 與待處理視窗，並直接取消該 input 的獨立 request token。poll 取消後僅傳回空 Stopped／None；Ready／Resolving 過期時取消、推進 reset epoch、回報 Failed(Stale)，並清除音訊／metadata。next 同時等待通知、取消與原始 stamp 的五秒 deadline；沒有新輸入或 task 未再輪詢也能醒來。idle／pre-stop 不捏造時間戳，epoch 溢位 fail closed。

owner 的 input replacement／output clear 與 task publication 使用相同短暫 gate，固定 input → output 順序，核對 epoch／最新 sequence／取消狀態。不允許舊回覆填回空槽，也不會讓延遲 clear 擦掉新有效回覆；allocation 與 PCM／metadata Drop 在 gate 外，不跨 await、不接觸原生 capture callback。新 input 在 gate 內再檢查取消，避免 task Drop 與新提交競爭後留存 pending PCM。

task 在 reset 時只清除投票，不重發 owner 已交付的重設通知；task Drop 保留同 epoch 已有 Stopped，否則優先採用 queued window 的真實 stamp，避免以較舊 current 覆蓋新時間或產生重複終態。LiveResolver.stop 改用同步 Session.stop，再回收 future／音訊 worker。外部 token 仍是取消通知；共享槽位與 token 可同步清除，task 的私有投票、borrowed PCM／provider future 須由下一次輪詢或 Drop 回收，caller 自己保留的 Arc 亦不能由 Session 強制清掉。

新增二十項回歸測試，全部為原創合成 fixture；包含取消／替換 token 即時通知、未讀 metadata Weak 回收、同 epoch 舊 sequence 發布拒收、Ready／Resolving 已讀與未讀過期、next 取消／deadline 喚醒、stop／owner Drop／task Drop、未執行 task 的真實 queued stamp、重設與終態不重複／時間不倒退、epoch overflow，以及三組各 200 次真正跨執行緒競爭。有界 channel 加逾時，不把 slow task 與 actor 未輪詢混同。既有重設測試改為驗證同步空狀態與只交付一次，沒有放寬辨識門檻或 lint。

十五項首輪回歸測試通過；複查後補入 reset／Drop 重發與取消 admission 的測試，最後內容辨識相關 105 項測試全數通過，包含真正既有 FIR／指紋、LRCLIB localhost、Movie 與 LiveResolver 路徑。設計理由見 [ADR 0021](adr/0021-resolver-result-lifecycle.md)。

最終 cargo fmt --check、cargo clippy --locked --offline --all-targets --all-features -- -D warnings、cargo test --locked --offline --quiet、cargo build --locked --offline 及 cargo build --locked --offline --release 全數通過。完整一般測試 330 項（304 library、12 設定、4 探索、10 啟動），既有 14 ignored 及另由正常 parent 執行的隔離 GUI 資源 fixture 不變；沒有新增 ignored、相依套件或放寬 lint。離線參數只限制 Cargo 相依取得，本機 HTTP 合成測試仍正常執行。

正式內容服務、Movie hints、字幕 provider、文字 owner GUI 組合與實際內容／播放器／效能驗收仍待完成。沒有使用正式金鑰、真實擷取或新增雲端要求；固定 OpenAI／Gemini 欄位與結案手動輸入安排不變，Phase 6 尚未結案、Phase 7 尚未啟動。

## 第九批：補齊歌詞管理與正式 Live 組合

2026-10-10 接續上次中斷的實作，先修復 `app::lyrics` 引用不存在的 `lyrics_tests.rs` 所造成的 all-targets 編譯失敗。本批完成 LiveLyrics、LRCLIB 明確配置及正式 Live／GUI 接線，再依使用者要求銜接 [Phase 7](phase-7.md)；Phase 6 的正式服務與實機驗收不改列通過。

ContentConfiguration 可選擇附加 LyricsConfiguration，未設定時不建立 ExistingTextSession 或額外服務。UI 在連線前分別呈現內容辨識與歌詞服務的目的地，兩者共用原有可見性確認；每個新的擷取 Session 都建立新的 binder／服務。LRCLIB configuration 使用既有固定 HTTPS adapter，metadata source 僅讀取已有的 Session 資料；來源語言未知時回報 UnknownTextLanguage，不猜語言或以音訊視窗代替整首時長。沒有新增預設服務、金鑰或正式網路測試。

只有 settled Ready／Music／Confirmed 才查詢；同一 ID／曲名／歌手沿用同一 binding，不因信心或 offset 更新重抓。歌曲 metadata 變更、Transition、Unknown、缺口或錯誤清除舊選擇；中間 Resolving 通知不先移除仍有效的歌詞候選。factory／binder 錯誤與 panic、supports panic、同步 lookup panic 和 future panic 都回報固定型別錯誤。查詢缺少結果或失敗不自行重試；持續相同 metadata 的 binder 失敗也不反覆建立服務。

正式 Live 一起輪詢 resolver 與 lyrics，僅歌詞狀態變更也會發出 UI 更新；音訊仍先送 STT，歌詞錯誤不取消 STT／翻譯。停止或 resolver 無法繼續時同步取消並 Drop 文字 future。狀態沒有攜帶歌詞／曲名／指紋；原有 TranscriptTimeline snapshot 保持相同，沒有直接顯示或採用原始歌詞。

新增／補齊二十四項離線測試：十一項歌詞管理、兩項 LRCLIB 配置、兩項 Live 組合與九項 UI／原生狀態測試。涵蓋同曲去重、metadata 重綁定、切換取消、錯誤／panic／timeout、舊 Session／逆序時間隔離、factory 隔離、服務提示的可見性確認、終止清除及維持單一 snapshot。所有 provider payload 均為原創合成資料；LRCLIB 配置測試只建立 client／binding，沒有發送正式要求。

目前正式 GUI 可接受明確配置的歌詞服務並顯示狀態，預設 `--ui` 仍是未設定內容服務。實際歌曲／電影目錄、分類證據、Movie hints、字幕 adapter、內容服務設定入口與真實內容／播放器／效能驗收仍待完成。固定 OpenAI／Gemini 金鑰介面與結案手動輸入安排維持；設計理由見 [ADR 0022](adr/0022-existing-text-preparation.md)。

本批與 Phase 7 首批一起完成全套驗證：367 項一般測試、格式、Clippy、Debug／Release 建置通過。首次完整測試在沙箱的 WinCred 寫入合成憑證時回報 Unavailable；同一測試在正常登入工作階段通過後，已在相同環境重跑全套並通過，沒有忽略或放寬該測試，也沒有讀取正式憑證。完整指令與結果見 [Phase 7 驗證紀錄](phase-7.md#最終驗證)。
