# Phase 7 — Existing Subtitle Integration

2026-10-10 依使用者「是否能啟動 Phase07」後的「繼續作業」接續實作。基準為 Windows 10 19045；先補齊 Phase 6 中斷的歌詞查詢組合及測試。Phase 6／前階段尚待的正式內容服務、雲端、播放器、DPI 與效能驗收保持待辦，有效金鑰仍於結案時在固定 OpenAI／Gemini 介面手動輸入。

## 首批：中文候選本地化

`subtitle::existing_text::ChineseTextLocalizer` 提供實際可呼叫的非同步 library 入口。輸入為當次 `TextQuery` 與 `ExistingText`，輸出 `LocalizedText` 保留原文、來源語言、content ID、Session／generation、sequence 與原始時間戳；目標字串與原 cues 共用索引，固定目標 zh-TW。沒有新增 TranscriptTimeline 或 renderer，也沒有 Debug／Serialize／匯出／內容寫檔。

- TaiwanTraditional：保留來源字串，不改寫原生臺灣字幕。
- OtherTraditional／SimplifiedChinese：使用既有 bundled OpenCC 臺灣詞彙規則，不只更換字形。例如軟件／软件 → 軟體、服務器／服务器 → 伺服器、出租車／出租车 → 計程車。
- Source（英／日／法／德／韓）：中文 localizer 回報 typed `TranslationRequired(language)`，不呼叫雲端、不將原文改標成 zh-TW。來源翻譯由下述第二批的獨立入口處理。

共用字詞轉換搬至 `subtitle::localization`；原有 HTTP 翻譯 adapter 仍使用完全相同規則。這是通用隨程式字典，不是字幕快取。規則轉換已涵蓋測試詞彙，不宣稱可取代完整語境本地化品質驗收。

## 有界處理與生命週期

每個明確建立的 Session localizer 僅有一個專用 CPU 執行緒，由同時只處理一份候選的模型決定，不建立池或每 cue 執行緒；GUI 尚不會自動建立它。`&mut self` 避免同一管理物件平行發出要求。單一 active 加最多一個 pending 的 sync channel，滿載回報 Overloaded，沒有無界排隊或隱性重試。此獨立工作不接觸 capture callback、UI 或全域鎖。

每份輸入最多 1024 cues／128 KiB，原文加目標各有獨立 128 KiB 預算，每 cue 最多 8192 bytes。先檢查尺寸，再由 CPU worker 驗證本文與時間資料；轉換膨脹後也重新檢查 per-cue／總量。保留未定時歌詞的 None，拒絕未定時字幕、零長度、overflow、錯誤內容 ID 或工作階段，不猜測播放起點或 match offset。

要求沿用原始五秒 freshness，進入／CPU 執行／交付均檢查；sequence 與單調時間不得倒退。等待最多三秒；外部取消、future Drop 或 timeout 會通知 request token，CPU 每個有界 cue 間檢查取消。完整字典初始化及單個 cue 的轉換不能強制中斷，因此沒有宣稱固定毫秒取消 SLA。管理物件 stop／Drop 關閉 channel 並 join 工作，未讀 oneshot 結果隨 receiver Drop 釋放。

## 已測及待接續

十三項新增離線測試使用原創合成文字與真實 bundled 字典／CPU lane：三種中文路徑、原文／時間戳／順序／未知時間保留、五語拒絕假翻譯、Session／ID／時間格式、過期／未來／逆序、cue／總量／控制字元、轉換字詞膨脹、取消前拒收、外部取消、future Drop／後續要求、active 加 pending 上限、panic typed error／工作恢復，以及二十次建立／停止／join。沒有真實字幕、音訊或雲端資料。

首批結束時，來源字幕翻譯及前文策略、經信心／新鮮度／定位驗證的來源仲裁、接入單一 TranscriptTimeline 與 Live AI 補洞、GUI 端到端驗證均待接續。Phase 9 的進階 seek／drift alignment 不在本批。這批本地化完成不代表字幕已與播放對齊；正式 GUI 仍使用 Live AI，未完成仲裁前不採用候選。設計理由見 [ADR 0022](adr/0022-existing-text-preparation.md)。

## 首批驗證

2026-10-10 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：341 library、12 設定、4 探索、10 啟動，共 367 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 仍由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

首次完整測試在受限執行環境的 WinCred 合成憑證寫入得到 Unavailable（340 library 通過、1 失敗）。該測試在正常登入工作階段單獨重測通過後，全套也在相同環境通過；只使用唯一 `SubtitleNow/Test/...` 名稱並於測試後刪除，沒有讀取正式 API 金鑰。沒有新增 ignored、相依套件、unsafe 或放寬 lint。Cargo offline 不禁止合成本機 HTTP 測試，既有協定測試正常執行；沒有以此宣稱真實雲端或播放器驗收完成。

## 第二批：有界來源字幕視窗翻譯

`subtitle::source_translation::SourceTextTranslator` 接受已選定及公開處理目的地的 `Translator` 借用、固定品質、當次 `TextQuery`、原始 `ExistingText` 及明確 cue 範圍。只接受英／日／法／德／韓來源；中文沿用首批入口。每次一至四個片段，超過回報 WindowSize，不截斷文件、不猜播放起點。輸入文件仍受 1024 cues／128 KiB 限制。這是 Live 候選的處理預算，不是整份影片翻譯或 Phase 8 Prepared 工作排程，也不要求使用者在 GUI 手選四句；將來由通過定位驗證的 caller 選取範圍。

輸出 `TranslatedTextWindow` 保存文件內原始範圍、品質與 `LocalizedText`；原始語言、內容 ID、工作階段、時間戳、None 及原文不變。最終譯文在既有 CPU lane 經臺灣詞彙規則與尺寸檢查後才交付。沒有新增時間軸、renderer、保存格式、檔案或雲端 fallback。

### 品質與前文

- Fast：最多一句已完成前文，單次 Draft。
- Standard：最多三句已完成前文，單次 Draft。
- High：最多八句已完成前文，每個片段執行 Draft 後再以草稿 Review。

沿用既有 `QualityPolicy`、`TranslationRequest`、Session 詞彙及固定 zh-TW 指示；既有字幕片段已定稿，無須等待 ASR debounce。同一視窗逐句處理以保留脈絡，最多一個翻譯要求執行中。前文暫存最多八句，只有完整視窗翻譯、校訂及本地化成功後才提交；後續片段失敗時不會留下半份前文。相同內容、來源及語言且 cue 範圍相鄰才沿用前文，也會核對前文原始文字及時間。跳段、回頭或前文修訂清除脈絡；內容 ID／來源／語言變更另清除 Session 詞彙。停止／Drop 清除所有暫存。

既有串流 adapter 仍接收原本的 emitter；候選準備使用 `final_only` 模式驗證部分輸出但不排入更新佇列，只使用服務完整回傳。這可避免無消費者的部分輸出阻塞或溢位，也不將尚未驗證定位的文字送入 Live AI 時間軸。正式 Live 的部分字幕更新路徑維持原狀。

### 取消、預算與服務上限

整個視窗沿用三秒翻譯總預算，並受原查詢五秒 freshness 剩餘時間限制；等候服務 permit、請求、Review 與 CPU 本地化均在同一期限內。每次服務前後、交付前檢查期限與取消，逾時不啟動下一句或 Review，不採用遲到成功。四句是上限，不保證服務能在三秒完成四句；實際服務延遲與品質仍待結案驗收。

Session／外部取消、future Drop 或 timeout 取消工作並釋放持有的 future／socket／permit。同步 provider 建立 future、非同步 future 與能力檢查的 panic 有 typed error 邊界。舊 Session、generation、sequence、倒退或未來時間均拒收；失敗不自動換服務或重送整個視窗。CPU 沿用有界通道、取消通知與 join；單次字典初始化／字詞轉換不能強制中斷。

OpenAI／Gemini 的同一個 `HttpTranslator` client 現在以共用 Semaphore 限制最多兩個執行中要求，涵蓋多個 caller 與既有 bounded retry／rate gate。client 被共用時，Live 與來源字幕各自的排程不會將網路並行數相乘。等待 permit 可取消；來源字幕入口不建立額外 client、不讀取金鑰，也不更動端點、模型或協定。

### 第二批測試與剩餘範圍

新增十七項測試：十四項來源視窗測試、一項 CPU 譯文向量／來源身分測試、兩項真正 HTTP adapter 的本機整合測試。涵蓋五語、三種品質、原始時間及範圍、前文／詞彙隔離、非法資料／範圍、過期、Session 隔離、取消／Drop、能力及 provider panic、後續 cue／Review 失敗不提交半份前文、臺灣用詞轉換膨脹拒收，以及故意超時的同步 mock 回傳後不啟動新要求。HTTP 測試驗證兩家 adapter 的前文資料與指示分離，以及三個 caller 共用兩個 permit、第三個等候者取消後不發出請求。所有本文與回應均為原創合成資料，只連線 localhost。

第二批結束時，唯一來源仲裁、信心／新鮮度／播放定位驗證、單一 TranscriptTimeline 的來源切換與 Live AI 補洞，以及 GUI 端到端接線均待接續。正式 GUI 尚不呼叫本批翻譯入口或採用候選，Phase 6 正式服務／播放器矩陣及效能待辦不變。沒有讀取正式 API 金鑰、擷取音訊或使用真實雲端／字幕服務。設計理由見 [ADR 0023](adr/0023-bounded-source-subtitle-translation.md)。

## 第二批驗證

2026-10-10 第二批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：358 library、12 設定、4 探索、10 啟動，共 384 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

完整回歸在正常 Windows 登入工作階段執行，既有 WinCred 測試只建立及刪除唯一合成 `SubtitleNow/Test/...` 項目。初次 Clippy 指出測試的固定 chunks 用法與刻意反向範圍表達，已依建議修正，沒有略過 lint。沒有新增 ignored、unsafe、相依套件、永久內容保存或正式金鑰讀取；Cargo offline 測試仍包含 localhost HTTP fixture，不代表真實雲端或播放器已驗收。

## 第三批：有界字幕來源仲裁

`subtitle::source_arbiter::SubtitleSourceArbiter` 為每個 Session 提供一個同步、無工作執行緒的仲裁入口，持有最多四份已完成的 `LocalizedText`／`TranslatedTextWindow`。它沿用前兩批的實際輸出，不接受任意字串冒充已翻譯候選，也不建立另一條時間軸。所有本文及原文只在 RAM；決策以借用提供所選 cue、候選內與原文件內索引、原始媒體起點及捕捉範圍，沒有 Debug／Clone／Serialize／匯出或內容寫檔。

### 明確定位證據

每個候選須附上內容信心、獨立定位信心及 `CueAlignment`：同 Session／generation／仲裁 epoch、文字 query sequence、content ID、來源、cue 索引、所匹配原文、媒體原始起點，以及實際捕捉時鐘中的起點。定位證據的 `WindowStamp` 也必須新鮮且具有明確 capture 起點。媒體起點與原始 cue 必須一致；完整 cue 的 capture 範圍須位於該證據的捕捉視窗內，拒絕 overflow、未知／不完整時間或不同本文。

此核心驗證證據的身分、資料與政策，不執行 ASR fuzzy match，也不產生信心分數。正式定位證據產生元件仍待完成，測試中每份證據都明確為合成資料。不能把歌曲確認、中文轉換成功或範例 offset 當成已定位。仲裁僅選擇證據明確映射的單一 cue，不把它的 offset 延伸到其他 cue，不假設播放從零開始，不依牆鐘推估播放位置。

### 政策、可用性與缺口

內容信心至少 850／1000、定位信心至少 900／1000。這是具名的保守工程門檻，不是已校準的命中機率或已經真實內容驗收的參數。來源翻譯的實際策略品質須達到當次所選 Fast／Standard／High；中文本地化不冒用雲端品質等級。通過所有門檻後按原生 zh-TW、其他繁中本地化、簡中本地化、來源翻譯排序；同等優先度維持目前可用來源，以避免反覆切換。

`PlaybackObservation` 必須由 caller 提供實際 capture 位置、單調時間與 sequence，不接受舊工作階段、未來／過期時間、重複／倒退 sequence 或倒退 capture 時間。只有觀測落在 `[capture_start, capture_end)` 且不早於定位證據時才選用候選，否則回傳 LiveAi 決策。這是後續 Live AI 補洞的政策入口，尚未將 Hybrid 寫入正式時間軸。

文字查詢與定位證據各自受原始五秒 freshness 限制；較新的定位不能延長舊文字期限。submit／有效 select 時清除取消或過期候選並釋放 RAM。每份候選使用獨立 availability token，不能與其他候選或 Session 共用；替換、淘汰、invalidate、stop／Drop 會取消該 token。沒有背景 timer，caller 必須持續提供觀測或停止仲裁。

兩種既有文字來源乘四種語言優先級形成八個固定 watermark，不保存 content ID 的無界索引。相同來源／語言優先級的新 query 替換舊候選，query sequence 必須前進、時間不得倒退；watermark 在候選取消／過期後仍拒絕遲到結果。四個槽位滿載時，新種類回報 Overloaded，不偷偷淘汰有效候選。本文每份沿用最多 1024 cues／128 KiB 原文與獨立 128 KiB 目標預算。

caller 遇到來源／內容切換、Transition、音訊缺口、seek 或 provider 重啟須呼叫 invalidate；該操作清除候選、取消 token、遞增 epoch，並要求比已接收文字更新的 query sequence。舊 epoch 回覆不能重新填入。stop／Drop 清空所有本文，epoch overflow 進入 Stopped。整個核心沒有 await、網路、音訊 callback 或鎖。

### 第三批測試與剩餘範圍

十七項新增離線測試以真實中文 CPU localizer／來源翻譯管理物件和明確 MockTranslator 產生候選；定位證據及本文全部為原創合成資料。涵蓋無證據 LiveAi、非零 capture／media 起點及半開區間、裁切翻譯保留原文件索引與媒體時間、四種文字偏好、兩個信心門檻、query／文件／來源／本文／cue／時間範圍不符、實際翻譯品質門檻、同等偏好保持來源、字幕缺口、取消／過期、獨立 freshness、Session／generation、非法播放觀測、epoch 失效、替換與過期 watermark、四槽滿載、未定時歌詞拒絕假定位、停止／Drop／epoch overflow。

第三批結束時已完成仲裁政策 library，尚未配置正式定位元件、字幕服務或 GUI 候選提交，時間軸採用亦待接續。Prepared 優先來源屬 Phase 8，進階 seek／drift 修正屬 Phase 9。正式 GUI 仍使用既有 Live AI，未宣稱已完成實際來源切換或同步。金鑰保持結案時透過固定遮蔽介面手動輸入。設計理由見 [ADR 0024](adr/0024-evidence-gated-subtitle-arbitration.md)。

## 第三批驗證

2026-10-10 第三批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：375 library、12 設定、4 探索、10 啟動，共 401 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 仍由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

初版十六項仲裁測試及完整 400 項通過後，整合檢查發現裁切字幕須帶出原文件索引；補齊索引及原始媒體時間、加入第十七項測試後，再執行格式、Clippy、全套測試及兩種建置，皆通過。完整回歸在正常登入工作階段執行以支援既有 WinCred 合成測試，只使用唯一 `SubtitleNow/Test/...` 名稱並刪除。沒有新增 ignored、unsafe、相依套件或 lint 放寬；沒有讀取正式金鑰、捕捉音訊或啟動真實雲端服務。

## 第四批：仲裁採用與共用時間軸

`LiveTranslation` 現在持有同 Session 的 SubtitleSourceArbiter，提供 `sources_mut()` 明確提交候選與證據、`arbitrate(binding, observation)` 採用及 `reconcile_sources()` 失效回復介面。預設 GUI 不提供候選，也不建立新的內容／字幕服務或讀取額外金鑰。實際選擇仍經第三批的信心、品質、freshness、epoch 及明確 capture 位置檢查。

`TimelineBinding` 必須完全吻合原有 ASR 項目的 Session／generation、utterance、sequence、revision。只有具完整合法時間的 Final ASR 可採用既有文字，完整 ASR 範圍須在定位 cue 的捕捉範圍內；Partial、未知時間、範圍越界、舊修訂與已淘汰項目回報 typed InvalidBinding。相同內容 ID、文件 cue 索引與 capture 範圍不能重複採用到第二個項目。此介面更新已有項目，不製造新的 ASR 事件、sequence、假時間戳或重複歷史列，也不推估定位。

### 同一項目的顯示與 Live AI 狀態

TranscriptTimeline 的每個項目仍保存原有 ASR、Live 譯文、穩定度、前綴與修訂；另可保存一份有界 ExistingDisplay，包含所選原文／zh-TW、來源、content ID、原文件索引、媒體起點與 capture 範圍。顯示 accessor 選用尚有效的既有文字，底層 Live 修訂及翻譯前文保持獨立，避免採用候選造成既有前綴驗證失敗。既有文字顯示中拒絕提交 Live 前綴，回報 PrefixBusy。

候選不另建 TranscriptTimeline。SideChat、BottomSubtitle 與 Dual 均由原有 TimelineSnapshot／UiPublisher 消費所選來源；DisplayRow 帶出 origin。每份快照使用同一個單調時間重查候選 lease，取消／過期後的快照不再複製失效文字，即使 owner 尚未執行下一輪回復。狀態為「使用既有字幕／歌詞」，同時提示即時辨識仍使用所選雲端服務。Win32 跟隨來源螢幕的 active 狀態也涵蓋此狀態，沒有新增 renderer 或原生 unsafe。

### 要求取消與回復

採用時取消該 utterance 的執行中翻譯，停止 pending Draft／Review；其他 utterance 的 Live 翻譯繼續，支援已定位片段與 Live AI 缺口共用時間軸。取消的 future 由既有 caller-owned async 輪詢回收，owner Drop 則清空全部要求。來源 lease 沿用原始文字與定位的五秒期限及 candidate 專用 availability token；替換、取消、invalidate 或停止會使 lease 失效。

核心在原有十毫秒排程輪詢中檢查 lease，移除失效顯示並發布同一時間軸的更新；caller 不等待 changed 時可明確呼叫 reconcile_sources。LiveAi 的仲裁決策也清除所綁項目的來源顯示。已有 Final Live 譯文直接回復；未完成的要求只在原本三秒期限尚有餘裕時恢復，不延長期限、不把陳舊 ASR 重新上傳。若無可回復譯文且期限已耗盡，保留該項目原來的空／部分 Live 狀態，後續新 ASR 仍正常處理，沒有假譯文或默默重送。

為避免同一 ASR 修訂恢復後誤收被取消要求的舊訊息，每次排程配發新的 Session 內 job ID，更新須符合目前 job ID、來源修訂與 pass，suspended 要求不接收更新。時間軸的翻譯排序亦納入 job ID，恢復時不會因服務輸出 revision 從一重新開始而被擋住；High 恢復原 Review 與草稿。job ID overflow 回報型別錯誤並回收 permit。若非配合式 provider 在取消時同時回報網路錯誤，已取消要求不使仍運作的 Session 失敗。

時間軸仍最多 64 個項目；每個新增顯示最多原文／譯文各 8192 bytes、content ID 1024 bytes，沒有無界索引／佇列或持久內容。淘汰項目釋放其顯示與 Live 狀態，Session Drop 清除仲裁候選與所有要求。沒有捕捉執行緒呼叫、磁碟、額外 worker 或新雲端協定。

### 第四批測試與剩餘範圍

新增十四項離線測試：十二項真實 LiveTranslation／仲裁／時間軸／UiPublisher／Presentation 整合，兩項翻譯要求生命週期測試。使用真實中文 CPU localizer 與明確 MockTranslator，所有本文、定位與 PID 都為合成資料。測得同一項目保留 Live 回復、三種模式共用同一 Arc 快照、來源狀態與雲端提示、快照立即拒用取消內容、重複決策不新增項目、pending／active 取消與 permit 回收、缺口與其他 utterance 繼續 Live、同修訂恢復後的舊佇列訊息隔離、High Review 保留草稿、非法 binding／時間／重複 cue 拒收、過期不重送舊 ASR、invalidate 更新、停止後拒收、慢 UI 消費／新 generation、64 項淘汰與 Drop，以及取消時 provider 失敗與 job ID overflow。

第四批結束時已完成到共用快照與顯示層的採用介面，查詢文字到準備的串接、正式定位證據與 GUI 候選排程均待接續；既有 Phase 6 正式服務與播放器／DPI／效能待辦保留。進階 fuzzy match、seek／drift 校正仍屬 Phase 9，不以合成證據宣稱已同步真實媒體。金鑰與真實雲端測試仍留至結案手動輸入。設計理由見 [ADR 0025](adr/0025-single-timeline-source-adoption.md)。

## 第四批驗證

2026-10-10 第四批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：389 library、12 設定、4 探索、10 啟動，共 415 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 仍由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

來源整合十二項與翻譯核心十九項 targeted 測試通過後，完整回歸在正常 Windows 登入工作階段執行，支援既有 WinCred 唯一合成 `SubtitleNow/Test/...` 項目的建立及刪除。沒有新增 ignored、unsafe、相依套件或放寬 lint。所有新來源／定位／服務測試均為合成資料，未讀取正式金鑰、捕捉真實音訊、啟動真實雲端或字幕服務；這些結果不代表實際播放器同步、端到端延遲或定位元件已驗收。

## 第五批：查詢文字到候選準備的串接

LiveLyrics 現在持有最多一份 `ExistingTextLease`，LiveContent 透過 `ready_lyrics()` 提供仍有效的唯讀工作階段文字。只有真實 Available 查詢出版物可建立它；身分、query、文字來源、內容 ID、時間及 Session 文字預算均重新檢查。clone 分享 Arc 原文，不新增歷史、匯出、Debug／Serialize 或磁碟內容。清除、Transition、來源／metadata 切換、查詢失敗及停止會同步撤銷舊出版物，外部持有的舊 handle 也失效。

`TextPreparationSession` 接受此介面、所選品質與已公開處理目的地的 Translator 借用，使用同一 SourceTextTranslator／CPU lane：原生 zh-TW 保留、其他繁中／簡中做臺灣詞彙轉換，中文路徑不發送翻譯要求；五種來源語言須提供明確一至四個 cue 的範圍並沿用既有 Draft／Review、前文及翻譯預算。每個管理物件同時只處理一個 mutable future，沒有 detached task、額外 client、無界排隊或假定位。

`PreparedTextCandidate` 為封裝輸出，保留原文、媒體時間及文件範圍，透過 `submit(flow, evidence)` 進入同一 LiveTranslation 的仲裁。仍須由呼叫端提供實際定位證據，準備成功不自動顯示或取消 Live AI；仲裁與 TimelineBinding 規則保持有效。未定時歌詞可以準備文字，但不能冒用定位證據採用到時間軸。

### 確認、新鮮度與信心

同一已確認歌曲及 provider 不重新查詢。當且僅當 LiveLyrics 收到另一份已通過順序、身分及新鮮度檢查的明確 Confirmed resolver 結果時，才對 RAM 中相同文字建立新的選取 query，使用該真實捕捉視窗的 sequence／時間。這不是以牆鐘或 UI 輪詢刷新，也不是新下載或持久快取。外部已持有的 handle／候選維持原本 query 與期限；新定位或新選取不延長舊候選期限。

這使同一首歌曲的不同來源翻譯視窗可用不同明確選取 query 依序處理，保留相鄰前文而不重抓整首歌詞。相同 query 的重複準備仍回報 Stale，不隱性重送；重複拒收不擦掉先前已完成前文。沒有新確認時沿用原五秒期限，逾時不得提交候選。

文字介面保留實際 Confirmed 歌曲信心。低於仲裁門檻時準備在送雲端前拒收；提交時以此值限制呼叫端的 content confidence，不能提高辨識服務分數。新確認信心下降時撤銷舊 handle 的 parent token，再保留同一 RAM 文字給新的確認；先前的準備及顯示結果隨之失效。沒有產生新的信心分數或命中機率。

### 三層取消與有界生命週期

每個仲裁候選維持獨立 availability token，另可附帶查詢出版物與準備管理物件的兩個 parent token。候選淘汰只取消自己的 token；查詢清除或準備 stop／Drop 則使旗下全部準備／顯示失效。這是固定三個有界訊號，沒有背景監看 task 或可任意延伸的取消樹。SourceLease 與快照 accessor 均檢查 parent，既有文字消失後仍由第四批回復原 Live 狀態。

準備期間外部取消、parent 撤銷、Session 停止與 future Drop 會釋放要求；最後交付及提交前再檢查身分、原期限與取消。撤銷造成的未交付脈絡會清除，服務錯誤或重複 query 不丟失前次有效前文。CPU 沿用原有 bounded channel／request guard／stop join；單一字典初始化或 cue 轉換不可強制中斷，沒有毫秒級取消保證。正式 UI 不會因新增文字 getter 自動送內容至未設定的服務。

### 第五批測試與剩餘範圍

新增十二項離線整合測試，經實際 LiveLyrics／ExistingTextSession 查詢與真實 CPU 本地化，使用明確 MockLookup／MockTranslator 和合成定位。涵蓋三種中文零翻譯要求、五語與來源視窗索引、查詢撤銷後未提交與已採用結果失效、子候選取消不影響共用查詢、準備中取消／future Drop／要求回收、非法範圍要求與 foreign generation／預先取消、明確確認更新不重抓且舊時間不變、重複 query 不擦掉相鄰前文、信心下降／門檻、provider 錯誤及 panic、準備 stop／Drop、原視窗期限取消慢要求。既有 LiveContent 測試另驗證暫時 Resolving 保留有效介面、settled Unknown 清除並撤銷外部 handle。

第五批結束時已串起查詢出版物到準備、明確證據提交、仲裁及同一時間軸的介面。正式定位證據產生、GUI 中選取 cue 範圍與準備／採用排程、實際字幕服務及端到端媒體驗收仍待完成。進階 fuzzy match／seek／drift 不在本批；正式 GUI 預設仍不配置內容服務、不產生定位或自動採用歌詞，沒有以合成證據冒充實際同步。金鑰保持結案手動輸入。理由見 [ADR 0026](adr/0026-session-text-preparation-leases.md)。

## 第五批驗證

2026-10-10 第五批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：401 library、12 設定、4 探索、10 啟動，共 427 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 仍由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

新增十二項準備測試及既有十一項歌詞測試均通過；最後加入相鄰前文斷言後再執行格式、Clippy、準備測試及全套回歸。完整回歸在正常 Windows 登入工作階段執行，WinCred 只使用唯一合成 `SubtitleNow/Test/...` 項目並刪除。沒有新增 ignored、unsafe、相依套件或放寬 lint；沒有讀取正式金鑰、擷取音訊或使用真實雲端／字幕服務，未宣稱真實定位或媒體同步已驗收。

## 第六批：最終 ASR 的保守完整 cue 驗證

新增 ExactCueVerifier，接受已有內容確認的查詢文字、當前仲裁版本、實際捕捉視窗與最終 ASR。五種來源語言須明確一致；只接受完整 cue，僅放寬 ASCII 大小寫與空白，保留標點及詞界。至少八個字母／數字、全文唯一命中才可定位；重複副歌、短句、未定時歌詞、未知／不同語言、Partial、中文候選或無完整命中均維持 Live AI。

時間採實際 ASR 起點與 cue 原始媒體起點，沒有零秒假設或牆鐘外推。ASR 長度不得超過 cue，長度比值至少 900／1000，而且完整映射 cue 須位於捕捉視窗；比值是實測時間一致程度，不是命中機率。內容信心沿用 Confirmed 服務原分數，不因完整文字命中提高。

全文掃描在 spawn_blocking 執行，process-wide CPU permit 依 available_parallelism、上限二個，飽和直接回報 Overloaded，不另建等候佇列。取得 permit 的工作可能等待 Tokio blocking pool，但只保留有界工作及原期限。查詢原五秒期限及驗證三秒預算取較早者；parent／外部／owner 取消與 future Drop abort 尚未執行工作，已執行工作逐 cue 協作取消，permit 保留至 closure 釋放。沒有新增執行緒／工作於每個音訊 block，也不阻塞擷取或 UI；單個有界 cue 的正規化沒有強制中斷保證。

ExactCueMatch 封裝出版物、完整 ASR 綁定與仲裁版本。TextPreparationSession.prepare_matched 在雲端前驗證，僅翻譯命中 cue；定位器停止會取消準備中的要求。PreparedTextCandidate.submit_matched 再核對當前時間軸修訂、原文／語言／時間、文件範圍及 query，不允許把證據套到另一個 cue 或新仲裁版本。最後仍由實際 PlaybackObservation 經原有仲裁採用，使用同一 TranscriptTimeline，不新增第二條翻譯管線。

新增十三項定位測試與四項準備整合測試，全部採明確 MockLookup／MockTranslator 與合成 ASR。涵蓋五語、大小寫／空白、詞界／標點、全文重複、Partial／未知語言／中文／短句、長度與 overflow、未知／越界時間、Session／新鮮度／順序、取消／容量、逐 cue 中止、future Drop 保持有界及 permit 回收、CPU 等候仍遵守原期限。整合涵蓋唯一 cue 翻譯與同一時間軸採用、查詢撤銷回復 Live、舊 ASR 綁定／仲裁版本、錯誤範圍、定位器停止前不送雲端及進行中取消。

這是可由實際輸入計算的單一片段定位元件，尚未完成真實媒體驗收。正式 GUI 候選排程、當前 ASR／捕捉證據分流、實際內容服務配置、中文證據及跨 cue 持續定位仍待接續；Phase 9 的 fuzzy match／seek／drift 保留。正式 GUI 預設仍不配置內容服務；金鑰維持結案互動手動輸入。設計見 [ADR 0027](adr/0027-conservative-exact-cue-evidence.md)。

## 第六批驗證

2026-10-10 第六批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：418 library、12 設定、4 探索、10 啟動，共 444 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 仍由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

新增十七項定位／準備測試通過後，完成全套回歸及 Debug／Release 建置。完整回歸在正常 Windows 登入工作階段執行，WinCred 只建立及刪除唯一合成 `SubtitleNow/Test/...` 項目。沒有新增 ignored、unsafe、相依套件或放寬 lint。所有辨識、查詢、翻譯及時間輸入都是合成測試資料，未讀正式金鑰、擷取真實音訊或連接真實雲端／字幕服務；未宣稱 GUI 排程、真實媒體定位、同步或端到端延遲已驗收。

## 第七批：有界 Live／GUI 候選排程與擷取游標

Live 迴圈現在會在已明確配置歌詞服務及翻譯 client 的分支建立 LiveTextPipeline／LiveTextTask。以查詢出版物及最新最終 ASR 排程完整 cue 驗證、命中 cue 準備、仲裁及時間軸採用；與原有辨識、Live 翻譯及內容查詢平行輪詢，沒有在音訊 callback／drain 中等待網路。一般 GUI 與命令預設的內容配置仍為 None，沒有新增服務目的地、client 或金鑰讀取。

最新輸入 watch 保留最多一個 active 與一個 replacement；結果 mpsc 容量一，進度 watch 僅含固定大小狀態。新最終 ASR 取消舊工作，舊未讀結果清除，完成後再次核對 ticket、Session、查詢 parent 與仲裁版本。相同 query 不重送，新的歌曲確認不重啟相同 cue 的 pending 要求或延長期限。成功準備後保留一筆有界 metadata watermark，即使查詢撤銷後再選取，也不重新翻譯同一 ASR 綁定；停止釋放。相鄰來源 cue 保留品質所需前文；明確撤銷攜帶脈絡重設，不因 watch 合併中間 None 而漏掉清除。

LivePlaybackClock 只讀實際 NormalizedAudio 的 frame_position／frames／sequence／received，保存一份游標、不保存 PCM、不以牆鐘外推。時間須保持原 250 ms 音訊新鮮度；缺口、discontinuity、未知／非法時間、overflow、順序或時間倒退立即撤銷舊定位版本及歌詞出版物。準備完成時必須用這份實際觀測，只有游標仍在已驗證 cue 內才提交／採用；未知或已播放完的位置維持 Live AI，不用 ASR 起點假裝當前位置。

核對／準備／採用／無可靠定位及 typed error 接入既有歌詞狀態列，選定歌詞服務目的地與所選翻譯服務提示保留。候選分支失敗不停止或重連 STT／Live 翻譯；採用仍走原有單一 TranscriptTimeline 和共用快照。停止、來源切換、重連或關閉釋放 caller-owned future，取消未完成要求，原 CPU lane 回收，已採用候選的準備 parent 隨之失效。

新增十六項離線測試，採明確 MockLookup／MockTranslator、合成 ASR 及音訊 metadata，不擷取真實音訊。涵蓋三品質、正常相鄰前文、當前 cue 採用與 UI publisher 快照、未知／越界／已播完位置拒收、無完整命中零雲端要求、服務錯誤不毒化 Live、取消 wait 保留工作／clear／stop 回收、最新 ASR 取代、舊完成結果拒收、慢消費者容量及連續替代只執行最後一份、不因確認更新重啟或刷新期限、成功後撤銷再確認不重送、Task Drop 回復 Live、watch 合併時的明確脈絡清除、Session／版本／Partial／超量本文／倒序輸入不撤銷新工作／ticket overflow、實際 frame 游標與時間／缺口檢查、UI 狀態與音訊缺口立即撤銷出版物。

已完成可設定 Live／GUI 路徑的候選排程接線，尚未提供正式內容／字幕服務的預設配置或宣稱實際媒體同步。保守完整 cue 驗證可能因文字／長度門檻、時間不明及處理延遲而無法採用，屆時維持 Live AI；沒有延遲 SLA 或真實播放器相容性承諾。中文定位證據與正式服務配置仍待完成，進階 fuzzy／seek／drift 保留 Phase 9。金鑰與實機／雲端測試維持結案手動輸入。設計見 [ADR 0028](adr/0028-live-text-scheduling-and-capture-cursor.md)。

### 第七批的下一階段判定

依使用者要求，每批完成會檢查並提醒下一 Phase 的啟動條件。第七批結束時尚不建議啟動 Phase 8：當時 Phase 7 仍有中文定位證據與正式來源服務配置的實作缺口。正式金鑰／實機驗收按既有安排留至結案，不因尚未輸入金鑰就擅自改變此安排；本批也未開始 Prepared Mode。

## 第七批驗證

2026-10-10 第七批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：434 library、12 設定、4 探索、10 啟動，共 460 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 仍由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

新增十六項測試通過後執行完整回歸；最後入口大小與倒序防護加入既有測試後，重新執行格式、Clippy、全套回歸及 Debug／Release 建置，確認最終版本通過。完整回歸在正常 Windows 登入工作階段執行，WinCred 只建立及刪除唯一合成 `SubtitleNow/Test/...` 項目。未新增 ignored、unsafe、相依套件或放寬 lint；未讀正式金鑰、擷取真實音訊或連接真實雲端／字幕服務。所有新候選及擷取位置都是合成資料；本機 UI publisher／資源測試不等於真實播放器同步、實際服務或延遲驗收。

## 第八批：中文候選的同版本來源參考軌

ExistingTextProvider 新增可選 lookup_bundle，回傳中文本文及來源語言參考軌。既有 provider 的預設仍只呼叫原 lookup 一次，不額外查詢或取得另一個服務。兩軌須同 Session、來源及內容 ID，並由 provider 分別聲明相同版本 ID；ID 沿用 1024-byte、非空及無控制字元限制。相同內容 ID 不能代替版本證明，此欄位是 provider 聲明的來源資訊，正式 adapter 必須取得實際同版本資料。

TextLookupUpdate／ExistingTextLease 原子保留兩軌，合計最多 1024 cues／128 KiB 原文，不擴大 Session 預算。參考軌與中文本文共用 query、出版物 parent、確認信心及期限，沒有獨立續期或第三份查詢。明確新確認可沿用 RAM 文字，但先前交付結果保留原始期限；撤銷、信心下降或停止時兩軌同時失效，不增加取消樹、背景 task、磁碟內容或永久快取。

### 原文核對與精確時間對應

最終 ASR 必須以已知的五種來源語言之一，在完整參考軌唯一命中完整 cue；沿用至少八個字母／數字、僅 ASCII 大小寫及空白正規化、實際時間長度比值、內容信心與捕捉視窗限制。參考文字含重複副歌時拒收，包括其他時間或未定時的重複。再於中文本文找唯一具有完全相同媒體起點及長度的 cue；文件索引可以不同，一微秒時間差、重複時間範圍、重新分段或缺少對應都不能採用。未提供參考軌的中文仍維持 Live AI，Live 中文譯文不能作為原文證據。

ExactCueMatch 保存來源參考索引與中文文件索引，證據命名實際中文原 cue。準備中文沿用原有 CPU 本地化，三種品質均不發送 Translator 要求；原生臺灣字幕保留，其他繁中及簡中轉換臺灣用語。提交再次檢查 ASR 修訂、中文原文／時間、query 及仲裁 epoch，並核對準備與證據的出版物取消訊號身分，拒絕跨出版物替換。採用仍須使用實際擷取游標，由同一仲裁器更新原有 TranscriptTimeline。

排程的一筆有界 metadata watermark 納入可選配對版本與參考語言。同版本續期不重送；不同版本、參考語言或有／無參考軌切換會替換待處理工作並重設脈絡。同仲裁版本的舊 query sequence／時間或舊 ASR sequence／revision 先回報 Stale，正式排程的拒收路徑保留目前工作與進度，不能以更換版本繞過排序並取消較新工作；其他非法資料仍清除並顯示失敗。UI 新增「本機處理臺灣用語」狀態，避免中文準備被誤示為所選翻譯服務處理。

### 第八批測試與剩餘範圍

新增三十一項離線測試：八項 provider bundle／文字管理、十一項配對定位、十二項 Live 管線。涵蓋預設 provider 單次查詢與錯誤、三種中文乘五種來源語言、非法版本／模型／合計預算、Arc 共用及原期限／撤銷、全文查重、不同文件索引、重複或一微秒錯位、Partial／未知語言／缺少參考拒收、跨出版物證據拒收、三品質零 Translator 要求、實際 LiveLyrics owner 查詢至同一時間軸採用與清除、新版本／參考語言／有無配對切換、同版本去重、不同版本的舊 query／時間／ASR／修訂拒收且保留較新工作、ASR 修訂／新舊完成／epoch 隔離及停止回收。全部本文、ASR、版本與捕捉位置均為明確合成資料。

中文的保守配對證據路徑已實作，但須有正式服務提供真實同版本的兩軌及時間資料才可運作。目前 LRCLIB 仍是未配對資料，預設 GUI 的內容服務仍為 None；正式 ContentServiceFactory、LyricsMetadataSource、ExistingSubtitle 來源管理及配對 provider 配置尚待完成。沒有宣稱實際歌曲／影片或中文配對來源已驗收，進階 fuzzy／seek／drift 仍留於 Phase 9。設計見 [ADR 0029](adr/0029-paired-source-reference-evidence.md)。

### 下一階段判定

目前仍不建議啟動 Phase 8：Phase 7 的正式來源服務及配對資料配置還有實作缺口。本批已補上中文保守定位核心；不因尚未輸入金鑰而改變結案手動測試安排，也未開始 Prepared Mode。後續每批完成會繼續檢查啟動條件，條件具備時提醒使用者。

## 第八批驗證

2026-10-10 第八批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：465 library、12 設定、4 探索、10 啟動，共 491 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 仍由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

新增配對定位與 provider bundle 測試通過後執行完整回歸。最終整合檢查補上配對版本／語言去重、跨版本倒序與正式拒收路徑保留較新工作的防護；回歸測試同時確認其他非法資料仍清除並提示失敗。最後修正後再執行格式、Clippy、聚焦測試、全套回歸及 Debug／Release 建置，皆通過。

完整回歸在正常 Windows 登入工作階段執行；WinCred 只建立及刪除唯一合成 `SubtitleNow/Test/...` 項目。沒有新增 ignored、unsafe、相依套件或放寬 lint；沒有讀取正式金鑰、擷取真實音訊或連接真實雲端／字幕服務。中文本地化使用真實 bundled 字典及 CPU lane，但兩軌、版本、ASR、服務與捕捉位置都是合成測試資料；這些結果不代表真實配對服務、播放器同步或延遲已驗收。

## 第九批：HTTPS SRT／WebVTT 字幕來源

新增 TimedSubtitleProvider 正式 adapter，明確接受已知內容 ID、Session、HTTPS 字幕位置、格式及語言，可附加同版本來源參考軌。位置及版本是 caller 依來源 metadata 提供的聲明，不是辨識成功或經獨立驗證的保證；不從 STT 選項、網址或檔名猜測語言，不產生內容信心，也不取得影音檔案。

既有定位及採用政策保持有效：查詢成功不自動顯示，須由實際內容確認建立信心，再通過 Final ASR 完整唯一命中、配對精確時間、原期限與捕捉游標。主軌及參考軌在同一 lookup 依序取得，解析與合計預算都通過後，才一次交付 ExistingTextBundle；參考失敗不能靜默退成主軌成功。

### 保守格式子集與資源上限

支援 UTF-8、可選 BOM、LF／CRLF、正數 SRT 索引、WebVTT 標頭與可選 cue ID、純文字多行 cue、完整明確起訖時間。保留原始時間、順序、重複與重疊；拒絕缺少結尾、倒序起點、非法時間及 overflow，不猜測或合併片段。樣式、cue 定位、時間映射、註解區塊、標記／實體及 ASS 控制碼明確拒收，普通裸 & 保留。這是 SRT／WebVTT 純文字子集，不宣稱完整標準相容或支援其他編碼。

正式配置只允許有效 TLS 的 HTTPS，禁止 userinfo、fragment、控制字元／空白、超過 4096 bytes 的位置，不跟隨重新導向。可能含簽署 query 的位置只在 RAM，不公開 Debug／Serialize、不記錄網址或 reqwest 原始錯誤。完整 200、MIME、精確 UTF-8 charset 與 Content-Encoding 都檢查；主軌 404 表示沒有來源，參考 404／其他錯誤使整份配對失敗。每軌原始本文最多 256 KiB，兩軌合計最多 512 KiB；解析後合計仍為 1024 cues／128 KiB，每 cue 8192 bytes。

每 provider 同時一份要求，每秒最多啟動一份 lookup；容量不足立即回報 TextOverloaded，沒有新增等候佇列。429 遵守 Retry-After 秒數／日期，缺少時六十秒，非法或溢位值則停止此 provider 的後續要求。全份 lookup 共用三秒預算及原 query 五秒期限較早者，不因第二軌或解析刷新。CPU 在 spawn_blocking，以 available_parallelism 為輸入、process-wide 最多二個工作；permit 由 closure 持有至結束，取消／Drop 通知獨立工作訊號，逐行檢查期限並釋放 RAM，沒有固定毫秒停止保證。

### 第九批測試與剩餘範圍

新增三十五項離線測試：二十三項格式解析、十二項真實 localhost HTTP／工作階段整合。涵蓋五語與三種中文的明確語言保留、BOM／換行／多行／原始時間、重複及重疊、語法／編碼／樣式拒收、時間 overflow 與所有尺寸上限、URL／版本配置、兩次 GET 原子配對、Session 至 Live 管線的臺灣本地化與同一仲裁／時間軸採用、零 Translator 要求、身分／來源／內容不符、HTTP 狀態／MIME／完整回應、合計預算、取消／future Drop 關閉連線、網路／CPU 容量、原期限與 429 冷卻。本文、版本、ASR、信心、位置及伺服器皆為明確合成資料，HTTP fixture 例外僅在測試編譯的私有 127.0.0.1 入口。

正式 ExistingSubtitle transport 已實作；正式內容服務、來源 metadata、可靠配對資料與 Live GUI 字幕管理組合仍待完成。一般 GUI 仍為內容配置 None，沒有新增預設目的地或要求輸入額外金鑰；真實 HTTPS 服務、字幕來源及播放器同步尚未驗收。Prepared Mode 未啟動，進階 fuzzy／seek／drift 留於 Phase 9。設計見 [ADR 0030](adr/0030-explicit-https-timed-subtitles.md)。

### 下一階段判定

目前仍不建議啟動 Phase 8：正式服務配置與 ExistingSubtitle 的 Live GUI 來源管理還有實作缺口。金鑰與真實服務測試維持結案時手動輸入；後續每批完成會檢查啟動條件，具備條件時提醒使用者。

## 第九批驗證

2026-10-10 第九批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：500 library、12 設定、4 探索、10 啟動，共 526 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 仍由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

三十五項聚焦測試先通過；最終審查補上 charset 引號及網址正規化前的 userinfo／分隔符檢查，加入既有測試的拒收案例後，再執行格式、Clippy、全套測試及 Debug／Release 建置，皆通過。解析與 HTTP／取消／隱私審查未發現剩餘重大問題，沒有新增 unsafe、相依套件、ignored 或放寬 lint。

完整回歸在正常 Windows 登入工作階段執行，WinCred 只建立及刪除唯一合成 `SubtitleNow/Test/...` 項目。新增網路測試僅連線 127.0.0.1 合成伺服器；沒有讀正式金鑰、擷取真實音訊或連接真實雲端／字幕來源。本機 HTTP 驗證不等於正式 HTTPS 目的地、實際來源版本／語言、播放器同步或延遲驗收；這些驗收仍待結案手動操作。

## 第十批：Live 字幕管理與 GUI 共用接線

新增 LiveSubtitles、SubtitleConfiguration 及正式 HTTPS factory，將 ExistingSubtitle 的查詢生命週期接入可設定的 Live／GUI。只從合法電影／影集候選選取最高實際信心，要求至少 850／1000、與第二名差至少 100，並具備對白、遠端 metadata 及片名視窗／檔名線索；不假設排序，不因查詢成功或標記相符而提高分數。模糊、弱、未知、混合或非法資料會撤銷舊出版物，舊 stamp 不能清除目前有效選取。

同電影 ID／片名／年份沿用一個 binding，不因新確認反覆建立服務或下載。只有新且可靠的 settled 確認可更新選取 query，外部舊 lease 維持原期限；信心下降會撤銷舊 parent。factory／binder／provider 的同步與非同步失敗、panic、取消及停止都有固定錯誤與回收路徑，沒有自動重試、備援服務或持久字幕資料。

SubtitleMetadataSource 僅讀已有的工作階段資料，回應必須帶出目前 StreamIdentity 與所選電影 ID，HTTPS binder 核對後才建立正式 provider。未知語言、網址或 ASR 偏好不能代替來源 metadata，配對版本仍須由真實服務提供。這是明確來源聲明及 Session 隔離，不是獨立辨識證明；新 Session 拒絕使用上一輪 metadata。

### 共用候選工作及服務提示

LiveContent 管理字幕與歌詞兩份可選狀態；可靠電影及已確認歌曲的選取互斥，ready_text 只交付有效來源。兩者使用一個 LiveTextPipeline／task、原有 Translator、同一仲裁與 TranscriptTimeline。字幕讀取、準備及採用仍與 STT／Live 翻譯平行輪詢，沒有新增每 block 工作、擷取 callback 等待或第二條顯示管線。

新字幕 scalar 狀態接入 UiFrame，服務目的地透過 prepare_subtitles 加入原有可見確認；兩個 GUI 啟動入口均在提示確認後才建立服務，終端機亦先呈現目的地。UI 依目前來源及實際 display_origin 顯示核對、本機臺灣用語處理、所選翻譯服務準備、採用及失敗，字幕與歌詞狀態不互相污染。提示只說明讀取已設定的字幕位置，不聲稱字幕服務收到片名、ASR 或音訊。

缺口、過期、來源切換、內容失敗及停止會同步撤銷兩類出版物與已採用顯示，回復原 Live 狀態。重連／關閉／publisher Drop 清除或停止 scalar 狀態；狀態更新保持同一 TimelineSnapshot，native 手動停止亦清除字幕讀取狀態。沒有新增 Win32 unsafe、金鑰欄位、內容匯出或磁碟資料。

### 第十批測試與剩餘範圍

新增三十三項測試：十七項字幕管理、三項 HTTPS factory、五項 Live 組合、八項 UI／bridge／native。涵蓋候選門檻與未排序選取、metadata／signal／年份／尺寸驗證、同 binding 更新與信心下降、查詢去重及原期限、失敗／panic／取消／Drop、舊 stamp 隔離、配對中文準備及撤回、不同 Session metadata 拒收、歌詞／字幕互斥切換、缺口／故障／過期／停止退休、正式排程與來源狀態投射、唯一時間軸採用、服務提示 gate、舊 Session／未揭露服務拒收、共用快照與 native 停止。所有本文、線索、分數、版本、ASR、PID 與目的地皆為合成資料；沒有擷取音訊或使用真實服務。

已完成可設定 Live／GUI 的字幕來源管理接線。正式內容辨識服務、Movie hints、SubtitleMetadataSource／LyricsMetadataSource 實作、目錄與實際服務設定資料仍待完成，一般 --ui 仍為內容配置 None。隱藏 native 視窗測試不等於實際操作、DPI、播放器或同步驗收；有效金鑰與真實雲端仍留至結案手動輸入。設計見 [ADR 0031](adr/0031-live-subtitle-owner-and-shared-text-routing.md)。

### 下一階段判定

目前仍不建議啟動 Phase 8：字幕的 Live／GUI 管理接線已補齊，但正式內容來源、metadata 及使用者服務配置仍有實作缺口。金鑰未輸入不是本批的阻擋原因，不改變結案手動測試安排；Prepared Mode 未啟動。後續每批會檢查條件，具備下一階段條件時提醒使用者。

## 第十批驗證

2026-10-11 第十批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：533 library、12 設定、4 探索、10 啟動，共 559 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 仍由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

管理及狀態聚焦測試通過後，補上正式 Live 排程／publisher 整合與 metadata Session 拒收案例。Clippy 首次指出兩處可合併條件，修正後格式與 Clippy 通過，並執行全套回歸及 Debug／Release 建置。沒有新增 unsafe、相依套件、ignored 或放寬 lint。

完整回歸在正常 Windows 登入工作階段執行；WinCred 只建立及刪除唯一合成 `SubtitleNow/Test/...` 項目。所有新增線索、內容、分數、Session、版本及位置皆為合成資料，native 測試只使用本程式的隱藏測試視窗；沒有讀正式金鑰、擷取真實音訊或連接真實雲端／字幕服務。不以這些結果宣稱正式 metadata、實際 GUI 操作／DPI、播放器同步或延遲已驗收。

## 第十一批：當前 Final ASR 到電影查詢

加入明確啟用的 MovieHintBuffer 與正式 movie composition factory，將原有 ledger 已接受的 Final ASR 分流給 MovieMetadataResolver。普通內容配置不複製對白；開啟此功能的 factory 必須支援這個入口，不能默默忽略設定。每個擷取／重連建立新的 Session 目錄及最多一筆 4096 bytes 的提示。

只接受完整正的原生時間；sequence、同段 revision、原生起訖水位阻擋舊結果，Partial、未知時間、超限、外 Session 與非法字串不取代有效提示。整段 Final 必須位於新鮮實際擷取視窗；太短、不在視窗內或沒有可用提示時，不呼叫 metadata／目錄。提示到達時間不更新 capture stamp 或原期限。

MovieMetadataSource 在合格提示存在時才讀取有界 RAM metadata，必須回傳精確同一個 capture stamp；所選程序與 metadata 新鮮度由明確來源驗證。片名、檔名、年份及完整媒體片長不從 UI label／路徑／音訊長度猜測，PCM 不傳給 MovieCatalog。回覆沿用既有獨立對白參考核對與實際信心。

提示替代、擷取缺口、停止及 Drop 同步取消；查詢 Future 被丟棄也取消目錄子要求，不留下 task 或重試。較新 Final 使舊查詢回 Stale 時，先撤回那次字幕選取，再等下一個滑動視窗；較新的提示保留，STT／翻譯繼續運作。啟動前的服務提示明示已定稿辨識文字及媒體線索，同一個可見版本 gate 核對後才建立服務，執行中不允許改變旗標。

本批新增 37 項合成測試，涵蓋時間／Session／水位／容量、取消／Drop／原期限／逾時、metadata 全 stamp、無合格提示零查詢、明確啟用及 GUI 服務提示。設計見 [ADR 0032](adr/0032-current-final-asr-movie-hints.md)。正式目錄與實際視窗 metadata source 沒有因這個接線而完成，沒有新增依賴、unsafe、內容持久化或外部服務。

### 下一階段判定

Phase 7 的電影對白分流已補齊，但正式歌曲／電影目錄、可靠當前來源 metadata、字幕／歌詞 metadata source 與使用者服務配置仍待完成，一般 --ui 仍為 None。因此目前仍不建議啟動 Phase 8，Prepared Mode 尚未啟動。金鑰與正式雲端驗收維持結案時固定介面手動輸入；條件具備時會提醒使用者。

## 第十一批驗證

2026-10-11 第十一批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：570 library、12 設定、4 探索、10 啟動，共 596 項一般測試；既有 14 ignored 不變，隔離 GUI 資源 fixture 仍由正常 parent 執行。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

電影聚焦測試 56 項通過。首次時間邊界測試使用 Windows 時鐘無法可靠表示的一奈秒差距，改為一毫秒並保留精確期限邊界測試，沒有放寬實作。完整回歸通過後，審查補上查詢中原始擷取／提示期限到期立即取消及完成後再核對；將測試改為三十秒停滯目錄、原期限剩二百毫秒、外層一秒限制，驗證到期取消及零 active requests。其後再次執行全套回歸、格式、Clippy、Debug／Release 建置，均通過。

本批新增 37 項合成測試，沒有新增 ignored、unsafe、相依套件或放寬 lint。完整回歸在正常 Windows 登入工作階段執行；WinCred 只建立及刪除唯一合成 `SubtitleNow/Test/...` 項目。沒有讀取正式金鑰、擷取實際音訊或連接真實目錄／雲端／字幕服務；UI 測試驗證狀態及可見 gate，不視為人工操作、DPI 或播放器同步驗收。一般 --ui 仍未啟用內容服務，金鑰仍留至結案時固定介面手動輸入。

## 第十二批：所選程序的可靠視窗 metadata

新增可明確配置的 Windows 電影來源 factory，Live 以實際 pinned 程序的 PID／建立時間做純設定綁定，每次擷取／重連建立新的 Session reader。綁定失敗不停止 STT／翻譯，也不取消或改換雲端；預設 --ui 仍沒有內容服務。

獨立工作執行緒每 250 毫秒讀取所選程序直接擁有的唯一合格標題，process-wide 最多一個工作。它持有 query／synchronize-only handle，核對建立時間與存活，拒收同程序多視窗、截斷、不完整列舉、長度競態、無效 UTF-16、控制字元及超過 1024 bytes。沒有 label 解析、前景／最大視窗排名、程序父子推論或 hooks；本程式 PID 也拒收，避免同程序同步訊息阻塞 UI join。

SourceMetadataBuffer 只保留一筆標題與原始掃描時間，兩秒有效性獨立於五秒 capture／ASR 期限。舊序號／觀測時間不清除新資料；變更、失蹤、模糊、錯誤、停止與 Drop 撤回舊 token。相同標題的新掃描不延長既有 lease，只更新未來查詢；metadata 同步方法只讀 RAM。MovieIdentifier 以三者的原始最早期限及取消 token 限制目錄工作，完成後再核對；檔名、年份與完整片長保持未知，信心仍來自目錄對白參考。

Native reader 停止會同步清除、喚醒及 join；暫時無法核對視窗仍繼續觀察，來源退出／啟動失敗／panic 保留有型別的錯誤。這個 Windows metadata 實作及 native 查詢有隔離合成子程序的生命週期測試，不視為實際電影／播放器／DPI／同步驗收。設計與官方 Win32 依據見 [ADR 0033](adr/0033-verified-selected-process-caption.md)。

### 下一階段判定

可靠所選程序標題的 metadata 來源已補齊；正式歌曲／電影目錄、字幕／歌詞 metadata source、使用者服務配置及真實服務／媒體驗收仍待完成。目前仍不建議啟動 Phase 8，Prepared Mode 尚未啟動；金鑰仍留至結案時固定介面手動輸入。條件具備時會提醒使用者。

## 第十二批驗證

2026-10-11 第十二批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：604 library、12 設定、4 探索、10 啟動，共 630 項一般測試，15 項 ignored。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

新增 34 項一般測試：18 項 RAM metadata／期限／錯誤管理、7 項電影 adapter 與取消整合、2 項來源設定綁定、7 項 Win32 查詢與生命週期。另新增一個僅供隔離子程序的 ignored fixture，由正常 parent 實際執行；原有 14 項略過不變，因此總數為 15，沒有新增未執行的人工驗收項。隔離程序用 CREATE_NO_WINDOW 啟動，只建立本程式自己的離螢幕、非啟用合成視窗；實際驗證建立身分不符拒收、單一標題、變更撤回、相同標題雙視窗拒收、消失／復原、Drop／重啟、程序退出與工作名額釋放。

第一次 Clippy 指出純測試 HWND 的人工 dangling pointer，改用標準 pointer helper 後通過，沒有放寬 lint。unsafe 審查另補上視窗樣式零值／last-error 核對，避免把查詢失敗當成非工具視窗；完整回歸及後續 Debug／Release 建置都使用修正後版本。

完整回歸在正常 Windows 登入工作階段執行，WinCred 僅建立及刪除唯一合成 `SubtitleNow/Test/...` 項目；沒有讀正式金鑰、擷取音訊、操作既有使用者視窗或連接真實目錄／雲端／字幕服務。新增 native unsafe 已附 ownership、指標範圍、callback／thread lifetime 的 SAFETY 說明，沒有新增相依套件或內容持久化。這些結果不是實際播放器、瀏覽器音訊、DPI、電影定位或延遲驗收；金鑰仍留至結案固定介面手動輸入。

## 第十三批：Session 文字 metadata 與出版物生命週期

新增正式 SessionTextMetadata 與唯讀 reader，同時實作 LyricsMetadataSource／SubtitleMetadataSource。每個擷取工作階段最多保留一筆歌曲、一筆電影資料；明確的 LRCLIB／HTTPS configuration 接到原有 factory，不搜尋、猜測語言、產生辨識分數或自動啟用服務。歌曲完整片長、專輯、字幕位置、語言及配對版本必須由實際來源提供。中文主軌與可選來源參考軌仍以一份 metadata 原子交付。

發布核對 StreamIdentity、內容 ID、歌名／演出者或電影片名／年份，以及新鮮 capture stamp 的 sequence／時間水位。字串有界，歌長限一秒至一小時；非法、舊版、不同 Session 或外來 cancellation token 不覆蓋有效資料。撤回後保留水位，舊非同步回覆不能重新填入。歌詞 binder 補上完整 Session 與內容 ID 核對，不能沿用上一輪的語言、專輯或片長；未知語言仍拒收。

同一份資料的新發布只更新水位，保留 revision 與 token，避免重抓文字；語言、專輯、片長、位置、配對版本或完整內容身分改變才撤回舊版。歌詞／字幕 owner 讀取 revision，讓原先缺 metadata 或未知語言的 binding 在資料到達後重試。不同資料或取消中的 provider 不再被當成同一 binding；來源回呼 panic 轉成有型別錯誤，不停止 Live STT／翻譯。

Metadata 取消一路連到 provider、查詢及已完成的 ExistingTextLease。更新、移除、停止或 owner Drop 立即讓舊出版物無效，現有定位／準備／翻譯消費者可由同一 token 喚醒。重新確認不能續期已撤回 metadata；單一出版物撤回不反向取消 metadata 或其他出版物。取消 token 使用最多四個相異祖先的平坦集合，等待最多五個 watch future，不建立接力 task、遞迴等待或無界 token 樹。原子配對結果保留，查詢取消時丟棄進行中的 Future。

這是可用的 Session metadata 管理及正式 adapter 組合入口，仍需實際目錄／資料發布者與使用者服務設定接入。reader 只能讀 RAM；停止／Drop 撤回並清空，持有 reader 不會延長有效性。沒有新增 unsafe、相依套件、金鑰欄位、音訊／文字持久化或跨供應商 fallback。設計見 [ADR 0034](adr/0034-session-text-metadata-and-publication-lifetime.md)。

### 下一階段判定

使用者本批選擇暫緩外部歌曲辨識服務；沒有接入或啟用 ACRCloud，也沒有要求金鑰。字幕／歌詞 metadata source 已完成，但正式電影目錄、實際 metadata 發布者及一般 GUI 的服務設定仍待完成，預設 --ui 仍使用 Live AI。因此目前仍不建議啟動 Phase 8；Prepared Mode 未啟動。有效金鑰與真實雲端驗收維持結案時固定介面手動輸入，未輸入金鑰不是本批阻擋原因；具備下一階段條件時會提醒使用者。

## 第十三批驗證

2026-10-11 第十三批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：651 library、12 設定、4 探索、10 啟動，共 677 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

新增 47 項一般測試：15 項取消／出版物、11 項歌詞 binding、11 項字幕 binding、10 項共用 metadata 管理。涵蓋跨 Session／內容拒收、非法資料保留目前資料、倒序與撤回水位、相同資料去重、更新／移除／Drop、缺資料到達後重試、未知語言、配對原子性、查詢零 I/O 及取消丟棄 Future、完成後喚醒、續期拒收、子 token 隔離、祖先去重與容量限制、回呼 panic。

首次編譯發現 helper import 及舊測試 fixture 欄位未同步，修正後 Clippy 指出取消 token 增大既有 enum；改成單一 Arc 的小型 handle，未放寬 lint 或修改來源仲裁 enum。完整回歸、格式、Clippy 與 Debug／Release 建置均以修正後版本執行。

完整回歸在正常 Windows 登入工作階段執行，WinCred 僅使用唯一合成 `SubtitleNow/Test/...` 項目；原有隔離 GUI 子程序 fixture 仍由正常 parent 執行。新增測試只用合成 metadata、mock 與不實際輪詢的正式 adapter 設定；沒有讀取正式金鑰、擷取音訊、操作既有使用者視窗或連接真實服務。這些結果不視為正式目錄、外部 metadata、實際 GUI／DPI、播放器同步或延遲驗收。

## 第十四批：本次指定的影片來源字幕

一般 --ui 加入固定「字幕來源（本次）」設定入口。使用者明確填入片名、HTTPS 字幕網址、SRT／WebVTT 格式及五種來源語言之一；沒有 Auto、任意目標語言或中文配對版號輸入。套用只驗證設定，不擷取、不下載；明確按開始後才使用鎖定的設定快照。網址遮蔽，唯讀主機欄可查看完整目的地主機與連接埠，排除路徑及簽名參數；非法設定不套用且錯誤不回送輸入。原有 OpenAI／Gemini 金鑰欄位不變。

設定只在 RAM，停止、自然結束、來源切換／失蹤與關閉清除；忙碌時鎖定設定，過期操作不覆寫正在使用的資料。欄位長度有界，拒收程式設值造成的超限、截短或無效 UTF-16。自有 title／URL 字串與原生輸入暫存會清除，編輯復原紀錄也清空；未加入偏好設定、日誌或歷史。本程式專用 modal 視窗由 UI 執行緒建立及銷毀，停止或關閉不留下對話框或工作。

已知字幕入口不是全球電影搜尋。每個 Session 建立不透明內容 ID，只接受使用者明確片名與來源語言；所選程序的可靠標題仍須包含該片名，且當前 Final ASR 必須至少二十四個有效字元、與下載的完整來源 cue 正規化後相同、在整份字幕只有一處命中。參考本文取自實際下載 cue，不回送查詢 ASR。分數是實測完整文字一致率，不是辨識機率、電影目錄評分或使用者輸入的信心值。未知語言、重複台詞、部分命中與不同片名不產生可靠候選。

下載與解析沿用正式 TimedSubtitleProvider：HTTPS、驗證 TLS、不跟隨重新導向、嚴格純文字 SRT／WebVTT、三秒要求及速率／容量限制。單一 Session 最多一份 document，最多 1024 cues／128 KiB 本文；原始傳輸另限 256 KiB。內容核對及字幕 provider 共用這份有界 RAM document，成功後不重新下載；未成功、取消或異常不留下快取。一次要求、零等待佇列；比對與複製使用依核心數限制、最多兩項工作的 CPU pool。已開始 CPU 工作在 Future Drop 後仍持有容量許可，逐 cue 核對取消、原始五秒 capture 期限及要求期限，不提前釋放名額。

Factory registry 只保留目前 Session 的 Weak owner；字幕 binder 與電影核對共用同一份 owner。另一個仍存活的 Session 不會被舊回呼取代或取消；停止／Drop 撤回並清空，重連建立新 ID 及新 document。Binder 核對 ID／片名／未知年份；原有 MovieMetadataResolver 再核對標題、語言與獨立對白。取得候選後仍須原有 ExactCueVerifier 的完整台詞、標點、時間與唯一性核對，再由同一 LiveTextPipeline 翻譯、仲裁及顯示；不放寬最終採用條件，失敗沿用 Live AI。

UI 分開揭露「本機核對指定字幕」及「指定 HTTPS 文字下載」。對白與片名比對留在本機，只對指定字幕位置做 GET，不將 ASR 或音訊送到字幕網站；音訊辨識及翻譯仍使用使用者選定的 OpenAI／Gemini。Library 已配置的其他內容服務不能被這個入口靜默替換。設計見 [ADR 0035](adr/0035-explicit-known-subtitle-entry.md)。

### 下一階段判定

已知字幕有可用的 GUI 設定、正式下載、Session 管理、定位及共用翻譯／顯示路徑。Phase 8 的本機媒體／Prepared 流程不依賴全球電影目錄；本批驗證已完成，可啟動 Phase 8，並依使用者要求提醒。這不代表所有自動辨識或實機驗收完成：外部歌曲服務維持暫緩，正式全球電影目錄、中文可靠配對資料、播放器／DPI／效能及真實雲端驗收仍列待辦。金鑰維持結案時固定介面手動輸入，本批不直接跨入 Prepared Mode。

## 最終驗證

2026-10-11 第十四批實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：685 library、12 設定、4 探索、10 啟動，共 711 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

新增 34 項一般測試：17 項共享 document／內容核對／Live 整合、4 項 factory owner、3 項不可變設定模型、10 項原生設定介面。涵蓋完整唯一台詞、短句／部分／重複／超長參考拒收、實測一致率與原始 cue、未知／不符語言、片名／年份／片長核對、Session／內容拒收、取消／Drop／逾時／panic、零等待飽和、CPU 工作在 Future Drop 後保留許可、本機 HTTP 實際一次下載共用及取消後連線關閉。整合測試從實測電影候選進入 LiveSubtitles、取得已確認 lease、完成原有 ExactCueVerifier，再驗證停止會撤回 lease 及證據。

原生測試只建立本程式的隱藏合成視窗，驗證遮蔽網址、固定格式／五語、超限與非法值拒收、取消／清除／復原紀錄、唯讀主機提示不包含簽名路徑／參數、忙碌鎖定、啟動快照及自然結束／來源切換撤回。第一次加入主機預覽後，修正 fixture 欄位數未同步；定位整合 fixture 原先省略原始 cue 的逗號與句點，被 NoUniqueMatch 正確拒收，改為同一完整原句，沒有放寬正式定位邏輯。編譯修正 Win32 常數匯入及 Clippy 條件寫法後，完整回歸與所有品質檢查通過。

完整回歸在正常 Windows 登入工作階段執行，WinCred 僅建立及刪除唯一合成 `SubtitleNow/Test/...` 項目；既有隔離子程序 fixture 仍由正常 parent 執行。新增網路測試只使用 literal loopback 的合成 HTTP 伺服器，HTTP 例外僅存在 cfg(test)，正式入口仍要求 HTTPS／有效 TLS。沒有讀取正式金鑰、擷取音訊、操作既有使用者視窗、連接實際字幕網站或真實雲端。本批沒有新增相依套件或 ignored；新增 Win32 unsafe 附自有 HWND、借用 Box／指標、UTF-16 範圍及 callback／UI thread lifetime 的 SAFETY 說明。

這些結果不是實際鍵盤／滑鼠操作、DPI、播放器同步、完整影片相容性或翻譯品質驗收；金鑰及真實服務仍留至結案固定介面手動輸入。
