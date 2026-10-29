# Phase 9 — Alignment

後續：使用者已批准啟動 [Phase 10](phase-10.md)，以下保留 Phase 09 各批工程紀錄。

2026-10-22 接續使用者「繼續作業」，完成第一批獨立模糊文字定位候選核心。基準為本機 Windows 10.0.19045.0。Phase 08 第四十三批的工程驗證已通過；歷史格式支援與人工驗收限制仍見 [Phase 8](phase-8.md)。有效 OpenAI／Gemini 金鑰及真實雲端驗收維持結案時在固定遮蔽介面手動輸入。

目前第十一批已完成工程驗證：獨立多觀測驗證、最近語音位置、有符號偏移與速率漂移，串接真正 Prepared Ready、Live 擷取觀測、單一仲裁／時間軸及一般視窗。中文主軌可明確指定同版本五語參考軌；保留 Ready 後須自行播放並啟動所選來源。完整回歸通過，已可啟動 Phase 10；本次停止於此門檻，尚未實作 Phase 10。早期未接線或尚未就緒的敘述為當批歷史紀錄。

## 第一批範圍

`subtitle::fuzzy_alignment::locate_fuzzy_cue` 是同步、無 I/O 的 CPU 核心，尚無正式呼叫者。只接受完整時間、明確五語原文與 Final ASR，返回獨立 `FuzzyCueProposal`；不產生 `ExactCueMatch`、`CandidateEvidence`、仲裁採用、偏移或目前播放位置。原精確驗證器、擷取時鐘、Live 管線、共用時間軸及 UI 沒有改接。

`AlignmentScope` 明確區分文件 Session／generation 與實際擷取 Session／generation，另帶非零 epoch；Prepared 與 Live 不必使用相同 Session，也不因此證明同一版本。Session、generation、epoch、ASR utterance／sequence／revision 與視窗 sequence 皆不得為零。文件、ASR 及擷取視窗分別核對對應身分；ASR 正時間範圍必須完整包含於真正擷取視窗，所有加法檢查溢位。文件仍通過原有來源、metadata、時間、8192 cues／四 MiB 本文／每 cue 8192 bytes 契約。

只做 ASCII 小寫與空白折疊；保留 Unicode scalar、標點、詞界及重音，不做跨語言、字元正規化或簡繁轉換。ASR 與勝出 cue 至少 16 個字母／數字；中文配對參考軌、跨 cue 合句及 Prepared 辨識全文 adapter 尚未支援，不把譯文當原文證據。

## 唯一性與資源界線

兩列固定容量的帶狀 Levenshtein 計算實際編輯距離，文字一致度為 `(最大字數 − 距離) × 1000 / 最大字數` 向下取整，屬工程指標，不是服務信心或機率。勝出門檻 900／1000，與次高差距至少 50；計算帶包含 850 以上競爭者，避免漏掉低於勝出門檻但會使差距不足的 cue。

掃描整份有界文件，遠處、重複與未定時歌詞都參與排名；不依猜測播放範圍跳過重複句，也不依時間選擇文字候選。只有唯一勝出 cue 的原生媒體時間完整才交付結果。每份 ASR 與每個 cue 最多 256 個正規化 Unicode scalar；任何後續 cue 超量也讓整次搜尋失敗，不裁切、跳過或先交付部分最佳值。實際 DP recurrence cells 全文共限 2,097,152，精確上限允許，多一格明確失敗。

工作最長三秒且不得超過真正擷取視窗原有五秒有效期；每 cue、正規化每 32 scalar 及每個 DP row 核對取消／期限。原有文件契約的一次有界驗證前後也核對。沒有建立 worker、佇列或 runtime；未來接線必須使用有界 CPU worker，不能從擷取、UI 或 async executor 直接執行，也不能以此同步介面宣稱已驗證並行容量。

候選只保留身分、完整 ASR binding、兩段原始時間、cue 索引及距離指標；沒有本文、文件 owner、Debug／Clone／Serialize。`validate` 重查取消、五秒視窗期限、兩份身分、epoch 及 ASR binding。未來 owner 在文件發布／來源／seek 或同 binding 的本文更改時，必須撤回 token 或更新 epoch／revision；核心無法發現呼叫者沿用同一身分而偷偷變更文件或本文。兩段時間差不可直接視為播放 offset。

## 驗證

2026-10-22，Windows 10.0.19045.0。本批只用合成文字與既有測試 fixture，沒有使用真實金鑰、使用者音訊／媒體或雲端。

| 實際執行命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo test --locked --offline fuzzy_alignment -- --nocapture` | 新增 23 項全部通過，執行 0.21 秒 |
| `cargo test --locked --offline subtitle::exact_alignment --quiet` | 既有精確／配對定位 24 項全部通過，執行 0.46 秒 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過，零警告，42.87 秒 |
| `cargo build --locked --offline` | Debug 通過，0.84 秒 |
| `cargo build --release --locked --offline` | Release 通過，1 分 06 秒 |
| `cargo test --locked --offline --quiet` | 1750 項 library、12 項設定、4 項探索、10 項啟動，共 1776 項一般測試全部通過，零失敗；既有 15 項 ignored 不變 |

完整測試於 Release 完成後單獨執行，命令正常結束；library 506.57 秒、設定 1.47 秒、探索 0.00 秒、啟動 1.43 秒。測試時間不是字幕延遲基準。第一輪新增測試曾因兩個語言列舉名稱不符而編譯失敗，修正測試名稱後定向及完整回歸全部通過，沒有放寬正式契約。

23 項新測試涵蓋五語 Unicode 編輯、ASCII／空白與重音／詞界保留、弱句／短句、Partial／未知或錯語言／中文拒收、雙 Session／generation／零身分、完整時間與溢位、視窗邊界、非零媒體位置原樣保留、遠處與未定時重複、低於 900 的次高競爭者、900／50／850 精確邊界、未定時勝出者、256／257 scalar、原文件 byte／cue／數量上限、全域及精確 DP 預算、取消／期限、候選 epoch／完整 ASR binding 失效。帶狀 DP 另與獨立不受限演算法比較，涵蓋空字串、不同長度、Unicode scalar、替換／插入／刪除、850 取整與最大列長。

人工程式檢查確認新核心沒有正式呼叫者、unsafe、可恢復路徑 unwrap／expect、I/O、服務、OS thread／runtime、無界 queue、內容保存或新增依賴。候選保留 metadata 與取消 token，不持有私有本文或整份文件。未新增 `.git` 或 commit，未丟棄其他既有工作；README／SPEC／Phase 8 交接與本 ADR 已同步，固定金鑰介面行為不變。

## 第二批：共用 CPU 名額與候選生命週期

2026-10-22 新增 `subtitle::fuzzy_worker::FuzzyCueLocator`，owned 文件移入私有不可變 Arc，只有身分與大小在建構時檢查，完整文字驗證及模糊搜尋在 CPU worker。文件不複製、不公開；Stop／Drop 釋放擁有者參考並撤回候選，工作持有的參考待真正清理才釋放。每份文件仍限 8192 cues／四 MiB／每 cue 8192 bytes，metadata 1024 bytes；語言與 Final ASR 契約不變。

`locate` 返回未 poll 不工作的 Send／static Future。擁有者最多一項，精確與模糊定位共用同一份全程序 `min(available_parallelism, 2)` 名額；滿載回報 Overloaded，不 await 名額、不新增 pool／OS thread。Tokio blocking pool 的 pending closure 也占名額。原 ExactCueVerifier 只抽出共用名額函式，沒有放寬精確匹配／時間／來源採用。

請求子取消同時連結擁有者與 caller，沿用最多四個扁平祖先，超限明確 InvalidData，子取消不影響父 scope。正常成功保留父 scope 連結，Stop／Drop 或 caller 取消都會讓已交付候選失效；ASR 最新 binding／epoch 仍需 caller 核對。每請求三秒由首次 poll 算起，與原擷取視窗五秒期限取較早；同一絕對 deadline 傳至核心，不在真正開始 CPU 時重設。

明確取消／逾時先撤回，等待 blocking job 真正退出再回報原 typed cause；Future Drop 同步取消／abort 排隊項，已開始工作仍合作退出並保留兩份名額。輸入欄位在 permit 前釋放，包含排隊 Drop、錯誤與 panic；panic 回 WorkerFailed，沒有 runtime 回 RuntimeUnavailable。需 timer-enabled Tokio runtime 並維持至清理，沒有宣稱能強制終止已開始的 CPU 工作。

這是背景 library，未接一般 Live／Prepared controller、Prepared Ready 借用或辨識全文；沒有使用金鑰、雲端、使用者音訊／媒體、保存／匯出或第二份時間軸。設計見 [ADR 0078](adr/0078-shared-alignment-worker-and-proposal-lifetime.md)。

### 第二批驗證

2026-10-22，Windows 10.0.19045.0，本批實際執行：

| 命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo test --locked --offline fuzzy -- --nocapture` | 41 項通過，0.49 秒；包含原核心 23 項及本批新增 18 項 |
| `cargo test --locked --offline subtitle::exact_alignment --quiet` | 24 項通過，0.46 秒；既有共享容量測試另確認使用同一份定位名額 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過，零警告，19.33 秒 |
| `cargo build --locked --offline` | Debug 通過，0.87 秒 |
| `cargo build --release --locked --offline` | Release 通過，48.66 秒 |
| `cargo test --locked --offline --quiet` | 1768 項 library、12 項設定、4 項探索、10 項啟動，共 1794 項一般測試全部通過，零失敗；既有 15 項 ignored 不變 |

完整測試在 Release 完成後單獨執行，命令正常結束；library 520.75 秒、設定 0.40 秒、探索 0.00 秒、啟動 1.19 秒。這是本批實際回歸結果，不沿用上一批數字，也不作字幕延遲 SLA。

18 項新測試核對 current-thread runtime 上真正 worker 成功、元資料與非零媒體時間、文件所有權、共用定位／單一擁有者容量、超量文件 admission、缺 runtime、未 poll Drop、預先取消、錯視窗身分、滿載不排隊及恢復、明確取消／逾時等待真實退出、執行中與排隊 Future Drop、owner Drop 與新 epoch 隔離、成功候選的 owner／caller 撤回、panic／BudgetExceeded 後名額回收、取消祖先上限及原始期限包含 blocking pool 等待。

定向測試輸出中的 `synthetic worker panic` 是刻意注入的 worker panic，用以驗證 WorkerFailed 與 RAII 回收；該測試通過，不是隱藏失敗。測試 gate 與孤立 Semaphore 只在 test 模組，正式工作無 I/O 等待；原字數、DP、期限、15 項 ignored、精確採用條件均未放寬。本批未新增依賴／unsafe／服務、永久內容、遙測、臨時媒體或正式金鑰讀取；沒有 `.git`，未建 commit 或丟棄既有工作。

## 第三批：Prepared Ready 的弱參考文字橋接

2026-10-22 新增 `PreparedSession::fuzzy_locator(capture_identity, epoch)` 與 `PreparedController` 同名明確 library 入口。只有目前精確 completion 已 apply 為 Ready 且原文為五語 ExistingText 才可建立 locator；Idle／Preparing／未 apply／失敗／停止回 NotReady，中文及 recognized Ready 回 UnsupportedEvidence。文件 Session 由目前 Ready 提供，擷取 Session／generation 與非零 epoch 明確交付；不建立 Prepared 的假 capture stamp。

PreparedDocument 內部的私有文字 owner 與媒體 owner 分開，原文／譯文 allocation 不複製，所有公開 getter 仍只借用。原文、譯文及 HTTPS 本文 quota lease 共用文字 owner，文字在 lease 前真正釋放；媒體 pin／遠端 decoder 留在原 Ready，不交給定位 worker。

locator／未 poll Future 只持 Weak 與 Session 發布 token，取得原有兩份定位名額及每擁有者一項名額後，job 才升級文字 Arc。Stop／替換／Session Drop 撤回所有舊候選並釋放 Ready；保留 locator 或未 poll Future 不會保留文字或媒體。已開始工作僅保留文字與配額至合作清理，媒體檔案鎖定可先釋放。原三秒／五秒期限、完整字幕歧義檢查、輸入先於 permit 清理與錯誤策略不變。

每請求連結 locator、caller、Ready 三份 scope，沿用最多四個扁平祖先，過深明確拒收；子請求 Drop／取消及單一 locator 停止不會取消 Ready 或其他 locator。新入口不建服務、不讀金鑰、不傳送音訊／本文、不開始播放，不把相似度或兩段時間差當作採用／播放證據。設計見 [ADR 0079](adr/0079-revocable-prepared-ready-alignment-text.md)。

### 第三批驗證

2026-10-22，本機 Windows 10.0.19045.0；目前已實際通過：

- `cargo fmt --check`。
- `cargo test --locked --offline fuzzy -- --nocapture`：55 項通過、1.04 秒，包含本批新增 13 項 Ready 橋接與 1 項 controller 測試。
- `cargo test --locked --offline subtitle::exact_alignment --quiet`：24 項通過、0.46 秒。
- `cargo test --locked --offline native_asr_translation_publishes_ready_with_genuine_recognized_provenance --quiet`：1 項通過、0.36 秒，新增 recognized Ready 不可冒充 Existing 證據的斷言。
- `cargo test --locked --offline https_prepared_subtitle_leases_remain_bounded_across_service_wait_completion_and_ready --quiet`：1 項通過、0.45 秒，新增保留私有文字 owner 時清除 Ready 仍占本文配額，真正釋放後才可恢復的斷言。

- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：通過、零警告，46.73 秒。
- `cargo build --locked --offline`：Debug 通過，1.04 秒。
- `cargo build --release --locked --offline`：Release 通過，1 分 00 秒。

- `cargo test --locked --offline --quiet`：1782 項 library、12 項設定、4 項探索、10 項啟動，共 1808 項一般測試全部通過、零失敗；既有 15 項 ignored 不變。

完整測試於 Release 完成後單獨執行，命令正常結束；library 526.56 秒、設定 0.33 秒、探索 0.00 秒、啟動 1.17 秒。這是本批實際結果，不沿用前批數字，也不作字幕延遲 SLA。定向 `synthetic worker panic` 是既有刻意注入案例，測試通過；沒有新增 ignored、放寬契約或把 mock 譯文當真實雲端品質。

新增測試以合成 opaque 媒體、真正本機 SRT／WebVTT 及明確 Translator mock 建立 Ready，核對五語與兩格式、來源字串 allocation 位址相同、文件與擷取兩份身分、非零媒體時間、Ready 狀態不變、未完成／失敗拒收、中文拒收、原始全文歧義、錯誤擷取身分／過期、弱參考強度、未 poll／Stop／替換／Drop、執行中媒體檔案鎖定先釋放、舊候選撤回、父 scope／其他 locator 隔離、取消祖先精確界線與 controller 不額外建服務／翻譯。Windows 實際開寫入 handle 測試僅核對鎖定，不寫入媒體；沒有使用者音訊、正式金鑰、真實雲端、新依賴／unsafe、永久內容或第二份時間軸。

## 第四批：最新定位候選狀態管理

2026-10-22 新增 `session::fuzzy_proposals::{FuzzyProposalSession, FuzzyProposalTask}`。移入 owned 或 Ready locator，返回 owner 與 caller 持有／poll 的 Task；輸入、結果各一格，不替每筆更新建立 Task。來源、文件發布或 seek epoch 改變時須停止並建立新 owner。

只保留最新接受的 Final ASR，最多 8192 UTF-8 bytes；sequence／utterance／revision 與擷取窗口水位皆受控。同完整 binding 不得偷偷改變本文、語言、時間或 kind，含超量／Partial 等錯誤變更也同步撤回舊要求；較舊要求回 Obsolete。重複 payload 不建立 ticket、不重試、不延長期限，也不作獨立觀測。失敗／過期後只留一份去重快照至下一筆／停止，沒有歷史清單。

新修訂取消舊工作，driver 等待 blocking job 真正退出才處理 watch 裡最新替換，不因多次更新累積 CPU 工作。結果再次核對 ticket、scope、完整 binding、取消及窗口有效期；舊成功或錯誤都不能覆蓋新要求。明確呈現 Waiting／Locating／Proposed／Unmatched／Failed／Cancelled／Stopped，只有有效 Proposed 可借用候選 metadata。

三秒從 offer 成功起算，與原窗口五秒期限取較早，涵蓋未 poll／舊工作清理／blocking pool 等待；同一絕對 deadline 傳給 worker，不從 CPU 開始時重算。直接 locator 的舊公開入口仍從首次 poll 起算。共用定位容量、每 locator 一項、Overloaded 明確拒收及四祖先取消界線保持，沒有新 pool／runtime／OS thread。

Stop／owner Drop 清除輸入／結果並撤回候選，caller await driver 才代表執行中工作已清理；丟棄 changed 等待 Future 不取消 owner。Ready 撤回及 driver Drop 透過只讀 availability token 喚醒 idle／pending／已交付 owner，不把額外 token 加入 job 祖先鏈。取消本管理器不取消 Ready 或其他 locator，原弱文字／媒體生命週期維持。設計見 [ADR 0080](adr/0080-latest-only-fuzzy-proposal-session.md)。

### 第四批驗證

2026-10-22，本機 Windows 10.0.19045.0。本批只用合成文字／媒體及明確 mock，正式金鑰與真實雲端維持結案固定遮蔽介面手動驗收。目前已實際通過：

| 命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo test --locked --offline fuzzy -- --nocapture` | 76 項全部通過，0.96 秒；含本批新增 21 項 |
| `cargo test --locked --offline subtitle::exact_alignment --quiet` | 24 項全部通過，0.47 秒 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過，零警告，40.01 秒 |
| `cargo build --locked --offline` | Debug 通過，15.09 秒 |
| `cargo build --release --locked --offline` | Release 通過，1 分 02 秒 |
| `cargo test --locked --offline --quiet` | 最終重跑：1803 項 library、12 項設定、4 項探索、10 項啟動，共 1829 項一般測試全部通過，零失敗；原有 15 項 ignored 不變 |

完整回歸在 Release 完成後單獨執行。首輪 library 為 1638 通過／165 失敗／15 ignored，423.89 秒；第一個錯誤為既有 `video_mp4_stop_error_and_chinese_sidecar_preference_release_resources` 在 `prepared_asr_tests.rs` 的五秒 `terminal_with_timeout` 等待逾時，其後多項共用媒體測試因 TEST_OWNERS 的 PoisonError 連鎖失敗。首輪因此不是通過結果，未執行到整合測試。

保持程式與五秒門檻不變，單獨執行 `cargo test --locked --offline video_mp4_stop_error_and_chinese_sidecar_preference_release_resources -- --nocapture`：1 項通過，14.55 秒（包含兩組合成來源及完整流程，不等於單一等待時間）。隨後僅設定 `RUST_BACKTRACE=1`，重跑原本完整命令：1829 項一般測試通過、零失敗，命令正常結束；library 514.23 秒、設定 0.65 秒、探索 0.00 秒、啟動 1.41 秒。沒有和建置並行，也未放寬測試期限、停用警告或新增 ignored。

最終工程檢查通過，但本批沒有找出首輪 MP4 五秒等待逾時的確切觸發原因，仍保留為間歇性測試／原生流程問題，不宣稱已修復或不會再發生。上述時間為測試執行時間，不是字幕延遲或效能 SLA。

本批新增 19 項 owner／driver 測試及 2 項真正 Ready 流程測試。涵蓋 20 次修訂在 driver 啟動前合併、30 次修訂在 blocking pool 排隊時合併、舊 closure 取消／清理後才 admission 最新工作、只交付最新 cue、舊成功失效、舊錯誤／ticket／epoch／binding 不污染目前結果、同 revision 偷改文字／語言／時間／kind、較舊 revision／sequence／utterance、視窗倒退、錯 Session／generation、非 Final／未知語言／零值／缺時間／溢位／範圍、重複輸入不延長期限、不重試過載或未匹配、offer 原始期限、空等逾時、等待 Future Drop、driver abort／Drop、owner Drop／Stop、ticket 溢位及輸入／結果清除。

Ready 測試以合成 opaque 媒體、真正本機 SRT 及明確 Translator mock，驗證 idle／尚未 poll／已交付三種狀態都會在 Ready 撤回時被喚醒，文字與媒體 pin 可釋放；停止本候選管理器不影響 Ready 或另一 locator。Windows 寫入 handle 只核對鎖定、沒有寫入媒體。新 availability listener 未加入 job 祖先，已有四祖先界線、共享容量、真正開始工作的清理及全文歧義測試也通過。

第一輪測試 helper 的 String／Box<str> 編譯錯誤與 Clippy 巢狀條件提示已修正；未放寬正式證據／資源契約、停用警告或新增 ignored。定向輸出 `synthetic worker panic` 仍是既有刻意注入的通過案例。人工檢查新狀態層沒有 unsafe、可恢復路徑 unwrap／expect、I/O、無界 queue、隱藏 spawn／runtime、金鑰讀取、服務要求、永久內容、遙測、新依賴或第二份時間軸。未建立 `.git`／commit，未丟棄既有工作。

## 第五批：Prepared 辨識原文定位

2026-10-23 擴充原有 Session／controller 的 `fuzzy_locator(capture_identity, epoch)` library 入口，接受已 apply 為 Ready 的五語 Recognized 原文。私有 AlignmentText 分開借用 ExistingText／PreparedTranscript，只取原文 cue、原生時間及明確語言，不重建整份文件／本文，不虛構 ExistingSubtitle、content_id 或辨識可信度。

ASR 文字重新通過原有 sealed transcript 結構、文件 Session／generation、正原生時間與順序、8192 cues／四 MiB／每 cue 8192 bytes 契約。借用 view 共用原有搜尋核心，因此全文歧義、256 scalar、900／50／850 門檻、DP、取消與三秒／五秒期限全部保持；遠處重複或後段過長 cue 仍會使整次要求失敗。獨立 capture 身分與真正 WindowStamp 不變，不拿媒體時間冒充擷取時間。

候選新增 `FuzzyTextOrigin` 與唯讀 origin getter，明確區分 ExistingText／PreparedRecognition。成功匹配不把 ASR 變成來源字幕或同版本證據，也沒有轉 ExactCueMatch／CandidateEvidence、目前播放位置或 offset 的路徑。公開同步 ExistingText 入口、精確驗證、來源仲裁、擷取時鐘與時間軸不變。

Ready 的弱文字／獨立媒體 owner、取得名額後才升級原文、停止／替換／Drop 撤回、四祖先取消、共用 CPU／單 locator 一項及最新輸入管理皆沿用。尚未 apply 回 NotReady；中文 Existing 仍 UnsupportedEvidence，譯文不當原文。本次搜尋不再呼叫辨識／翻譯服務，不取金鑰，不開始 capture 或播放。設計見 [ADR 0081](adr/0081-prepared-recognized-alignment-source.md)。

### 第五批驗證

本批新增 8 項純 ASR 借用／核心測試與 6 項合成 WAVE／真正 Ready／worker 測試，共 14 項；既有 native ASR Ready 測試另新增候選來源及原生時間、停止失效斷言，Existing Ready 測試核對來源仍為 ExistingText。

| 實際執行命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo test --locked --offline fuzzy -- --nocapture` | 90 項通過，2.11 秒；含本批 14 項 |
| `cargo test --locked --offline native_asr_translation_publishes_ready_with_genuine_recognized_provenance --quiet` | 1 項通過，0.45 秒；包含 Recognized 定位來源及停止撤回斷言 |
| `cargo test --locked --offline subtitle::exact_alignment --quiet` | 24 項通過，0.46 秒 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過、零警告，21.35 秒 |
| `cargo build --locked --offline` | Debug 通過，0.74 秒 |
| `cargo build --release --locked --offline` | Release 通過，1 分 00 秒 |
| `cargo test --locked --offline --quiet` | 1817 項 library、12 項設定、4 項探索、10 項啟動，共 1843 項一般測試通過、零失敗；既有 15 項 ignored 不變 |

完整測試在 Release 完成後單獨執行，命令正常結束；library 527.15 秒、設定 0.42 秒、探索 0.00 秒、啟動 1.24 秒。本輪設定 RUST_BACKTRACE=1 保留失敗診斷，沒有與建置並行或新增 ignored，這些時間不是字幕延遲／效能 SLA。第四批 MP4 五秒等待逾時本輪未重現，原紀錄仍保留待追查，不宣稱已修復；未修改該案例門檻或底層原生媒體實作。

本批定向輸出中的 synthetic worker panic 仍是既有刻意注入的通過案例，非隱藏失敗。新加入的 14 項測試與既有精確定位、生命週期／容量／取消回歸都通過；未放寬全文、來源或資源契約，未停用警告。

純測試使用真正 PreparedTranscriptMerger 交付的 sealed 合成原文，涵蓋五語 Unicode／編輯距離、字串 allocation 位址、獨立 Session／generation、完整原生時間、未知／錯語言／Partial／缺時間拒收、跨 chunk 遠處重複、8192 cues 全文掃描、後段 257 scalar、取消／期限／DP 上限。沒有為測試開放任意建立 transcript 的正式 constructor。

Windows 整合使用本機合成 WAVE 與明確 recognizer／translator mock，經真正解碼、原生時間合併、翻譯、completion apply 與 Ready。核對五語、來源位址不變、Recognized 身分、原生 4000–16000 µs 原樣保留、未 apply 拒收、Stop／Session Drop／替換、未 poll 不保留本文或媒體、已交付候選撤回及最新候選管理被喚醒。另一份足夠長的臺灣中文合成譯文在相同語言欄位下仍 NoUniqueMatch，避免短譯文只因字數被拒的無效反例；錯語言也拒收。原文正確配對後 recognizer／translator 要求數仍為準備時的 1／2，定位沒有額外服務要求。

上述為合成及 mock 工程證據，不宣稱實際辨識品質、遠端來源定位或播放相容性。沒有使用真實金鑰／私有媒體／雲端、永久內容、音訊保存、第二份時間軸、新依賴／unsafe／服務／runtime／thread／queue，未建立 `.git` 或 commit。有效金鑰維持結案固定遮蔽介面手動輸入。

## 第六批：多觀測時間一致性提示

2026-10-23 新增 `subtitle::fuzzy_track`，由既有 `FuzzyProposalSession` 私下接受驗證成功的 Final 定位結果。公開 `track_status()` 與 sealed `track_proposal()` 提供獨立狀態／可撤回快照，不改變單次文字候選的意義，也不接上精確採用、來源仲裁、LivePlaybackClock 或共用時間軸。

最多保留八筆 metadata，至少三個不同且遞增的 utterance／sequence／cue、同一來源種類、capture 與 media 各自向前且互不重疊，首尾 capture 範圍至少一秒才有 Consistent。首尾範圍包含空隙，不能當連續播放證據。每點起訖端點以 signed i128 計算 media 減 capture，全部端點差值散布最多 250,000 微秒；完整軌跡都須符合，不能以相鄰差小掩蓋累積漂移。三筆／一秒／250 ms 是具名工程門檻，不是統計信心、播放位置、offset 估計或 drift 校正。

單筆長度不一致回 Rejected(TimingMismatch)；與舊軌跡不相容則撤回舊支持、只用有效新點重新蒐集並回 Restarted，單次文字結果仍可為 Proposed。這不宣稱偵測到 seek。相同目前 binding／payload 不增加觀測或延長期限；修訂先撤回舊 utterance 支持。較舊 utterance 的非目前完全重複訊息也保守撤回，即使 offer 回 Obsolete，目前單次候選仍不被舊訊息改寫。新工作失敗、無唯一匹配、過期／停止會撤回舊軌跡；未成功交付的新要求不算觀測。

每點沿用原 capture WindowStamp 的五秒期限；快照取最早貢獻點期限，新增／修訂／到期／重建／停止／Drop 撤回 revision。`changed()` 會在較舊支持到期時喚醒，即使最新文字候選尚有效。已接受的 Final metadata 跟隨文件 availability 及原期限，正常換到下個 utterance 時不隨被取消的舊 per-job token 消失；舊單次候選本身仍照舊失效。快照獨立檢查來源及 revision token，不增加四祖先取消鏈。

快照不持本文／文件／原生媒體、不提供 Clone／Debug／Serialize 或任意 constructor，沒有 offset／目前位置／CandidateEvidence 轉換。Ready 撤回、driver Drop／abort、owner Stop／Drop 使其失效；來源／文件／epoch 變更須由 caller 停止並重建管理器。owner 最多掃描八筆，不新增工作池、runtime、thread、佇列、I/O、服務、依賴或 unsafe。所有內容仍本次 RAM；靜音、牆鐘與擷取持續不推算媒體前進。設計見 [ADR 0082](adr/0082-bounded-fuzzy-temporal-consistency.md)。

### 第六批驗證

本批新增 17 項 metadata 核心、7 項實際 worker／管理器及 1 項真正合成 SRT Ready 撤回測試，共 25 項。另在既有 Recognized Ready 管理測試核對首筆 metadata 接受。純測試涵蓋重複／修訂、三點／一秒／250 ms 邊界、全軌跡漂移、反向／範圍重疊、八筆上限、signed 大數、期限、epoch、取消／Drop 與快照失效。整合涵蓋舊 job 取消後保留獨立觀測、舊訊息撤回、不匹配／過載／過期、較早支持到期喚醒，以及持有快照時 Ready 仍可釋放文字與媒體 pin、不增加翻譯服務要求。

第一輪 112 項定向測試通過後補入三個案例，115 項第一次執行找到新 SRT 整合案例的合成時間單位錯誤：fixture 的 cue 相隔十分鐘，新輸入卻相隔十秒，正確回 Restarted(1)，其 panic 導致同 lane 後續十項 PoisonError。修正這個新案例的 capture 座標以符合 fixture 後，115 項全部通過；沒有修改正式判定、mutex 中毒處理、原生媒體或任何等待期限。初次 Clippy 的重複清理分支提示也已合併修正，未停用警告。

| 實際執行命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo test --locked --offline fuzzy -- --nocapture` | 修正合成座標後 115 項通過，1.56 秒；含本批 25 項 |
| `cargo test --locked --offline subtitle::exact_alignment --quiet` | 24 項通過，0.46 秒 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 最終通過、零警告，14.18 秒 |
| `cargo build --locked --offline` | Debug 通過，0.94 秒 |
| `cargo build --release --locked --offline` | Release 通過，1 分 06 秒 |
| `cargo test --locked --offline --quiet` | 1842 項 library、12 項設定、4 項探索、10 項啟動，共 1868 項一般測試通過、零失敗；既有 15 項 ignored 不變 |

完整回歸在 Release 建置結束後單獨執行，設定 RUST_BACKTRACE=1，命令正常結束；library 489.18 秒、設定 0.63 秒、探索 0.00 秒、啟動 1.16 秒。這些是測試耗時，不是字幕延遲／效能 SLA。第四批記錄的 MP4 五秒等待逾時本輪未重現，保留既有待追查紀錄，不宣稱已修復，也未調整原生媒體測試期限。

本批僅使用合成資料與明確 mock，未使用真實金鑰／私有媒體／雲端；這些工程測試不代表播放定位或實際辨識品質已驗收。定向輸出的 synthetic worker panic 仍是既有刻意注入的通過案例。沒有新 ignored、依賴、unsafe、永久內容、第二份時間軸、`.git` 或 commit；未丟棄既有工作。有效金鑰維持結案固定遮蔽介面手動輸入。

## 第七批：相鄰雙段模糊定位

2026-10-23 擴充同一搜尋核心，處理一句 Final ASR 對應兩個相鄰原文 cue 的分句差異。每個單段及相鄰雙段都參與同一全文排名，最多 2N−1 個候選，不合併重疊候選票數、不偏袒單段。另一單段／遠處雙段同分或差距不足仍回 NoUniqueMatch；雙段即使缺時間或時間很遠也參與詞彙競爭，不能依猜測播放區間排除。

ASR 與原始每 cue 仍限 256 個正規化 Unicode scalar；每 cue 只正規化一次，上一段與本段加入一個明確空白邊界，合併最多 513 scalar，DP 右側固定 514 格。沿用大小寫／空白正規化、標點／重音／字界保留、900／50／850 與至少 16 字母／數字門檻。所有單段與雙段共用原有 2,097,152 cells、三秒工作及原窗口五秒期限；無裁切／跳過超量原 cue，後段超量仍使整次失敗。不支援非相鄰、三段以上或任意子字串。

唯一勝出的雙段須各有完整正時間，原順序互不重疊且間隙不超過 500,000 微秒，包含邊界；超限／缺時間回 TimingMismatch，不回退較弱單段。media range 是第一段起點至第二段終點，保留間隙；capture range 沿用 ASR 原範圍，不生成 stamp 或聲稱連續播放。單段／相鄰短片段若產生近似競爭仍保守拒收，沒有偷偷省略候選。500 ms 是工程門檻，不是播放驗證。

`FuzzyCueProposal::cue_range()` 新增半開原始 cue 索引範圍，`cue_index()` 相容保留第一段索引。多觀測 metadata 改保留完整範圍，核對 `new.start >= old.end`，避免兩個共享 cue 的雙段成為獨立支持。ExistingText／PreparedRecognition 共用此路徑，來源種類、文件／擷取身分、epoch／binding、弱文字 owner、媒體釋放、取消與原期限維持。沒有新增服務／金鑰取用／依賴／unsafe／worker／queue／runtime／保存功能；精確採用、擷取時鐘及單一時間軸不改接。見 [ADR 0083](adr/0083-bounded-adjacent-cue-fuzzy-matching.md)。

### 第七批驗證

新增 13 項合成核心、2 項 sealed Recognized、2 項多觀測、1 項實際 worker／管理器、1 項真正合成 SRT Ready 測試，共 19 項。涵蓋雙段索引／原始時間、單段保留、跨型別／遠處／缺時間／重疊雙段歧義、時間間隙邊界、時間重疊／缺失拒收、非相鄰／三段拒收、五語字界、513 scalar 與後段超量、共用 DP 精確預算、擴大陣列與 unrestricted oracle 比較、跨 chunk 重複、完整範圍追蹤、停止撤回與 CPU／Ready 資源回收。

| 實際執行命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo test --locked --offline fuzzy -- --nocapture` | 134 項通過，1.78 秒；含本批 19 項 |
| `cargo test --locked --offline subtitle::exact_alignment --quiet` | 24 項通過，0.47 秒 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過、零警告，45.42 秒 |
| `cargo build --locked --offline` | Debug 通過，0.78 秒 |
| `cargo build --release --locked --offline` | Release 通過，59.53 秒 |
| `cargo test --locked --offline --quiet` | 1861 項 library、12 項設定、4 項探索、10 項啟動，共 1887 項一般測試通過、零失敗；既有 15 項 ignored 不變 |

完整回歸在 Release 建置結束後單獨執行，設定 RUST_BACKTRACE=1，命令正常結束；library 560.89 秒、設定 0.64 秒、探索 0.00 秒、啟動 1.42 秒。這些是測試耗時，不是字幕延遲／效能 SLA。第四批記錄的 MP4 五秒等待逾時本輪未重現，仍保留待追查，不宣稱已修復；未調整原生媒體或等待期限。人工檢查新增正式邏輯沒有 recoverable unwrap／expect、unsafe、I/O、隱藏 spawn、無界佇列或新增取消祖先。

只使用本機合成文字／媒體及明確服務 mock，沒有真實金鑰／私有媒體／雲端；不代表實際播放或辨識品質驗收。定向輸出的 synthetic worker panic 仍是既有刻意注入的通過案例。沒有新 ignored、依賴、unsafe、產品內容寫入、第二份時間軸、`.git` 或 commit；未丟棄既有工作。結案金鑰維持固定遮蔽介面手動輸入，Phase 10 尚未具備條件。

## 第八批：中文同版本參考軌模糊候選

2026-10-23 新增同步 `locate_paired_fuzzy_cue` 與 `FuzzyCueLocator::new_paired`，接受明確移入的 ExistingTextBundle：中文主軌加上供應端宣告同版本的五語來源參考軌。共用原有同版本／身分／內容／來源與文件驗證，不猜測參考軌，不以中文或 Live 譯文冒充 ASR 原文，也不將 edition 宣告當實際版本或播放驗證。

SourceTextReference 新增文件驗證入口，供 owned 配對使用，不製造 TextQuery／capture stamp。既有 query 驗證仍要求 query.source／content_id／Session 與主軌一致，新增回歸測試保留此契約。雙軌合計最多 8192 cues／四 MiB，每 cue 8192 bytes；constructor 只做長度／主軌身分 admission，全文驗證與配對在同一有界 CPU lane，沒有在 UI 或 capture callback 做搜尋。

五語參考軌經原有全文單段／相鄰雙段模糊核心，沿用語言、Final、256／513 scalar、900／50／850、全文歧義與 DP 預算；中文本文不搜尋、不複製。原文唯一勝出後，對每個參考 cue 掃描完整中文主軌，要求唯一且起點／長度完全相同；差一微秒或沒有對應回 TimingMismatch，重複時間回 NoUniqueMatch。雙段中文索引須相鄰，不支援一對多／多對一／跳過中間 cue。遠處原文重複不能靠中文時間補救。

候選新增 PairedSourceReference 來源種類與 `paired_primary_cue_range()`；`cue_range()` 永遠索引實際搜尋的來源軌，兩軌索引不混用。非配對候選的新 getter 為 None。多觀測 metadata 同時保留兩軌範圍，修訂／向前不重疊檢查也核對中文索引；media range 保留精確對應時間，沒有新增 offset 或位置投影。

配對驗證、搜尋與數值映射共用原始工作截止時間，不重設三秒或原窗口五秒；映射每 cue 檢查取消／期限。owned Arc 保留原 allocation，停止／Drop 撤回工作與已交付 metadata，候選不保留整份 bundle；取消／過載／最新替代沿用原路徑。未 poll 與執行中工作遵循原 owned locator 生命週期。這不是外部 lease 訂閱器：來源撤回／替換／epoch 變更須 caller Stop 並重建，lookup lease 與 Prepared 中文參考軌 Ready 橋接仍待接續。沒有新增服務／金鑰取用／依賴／unsafe／worker／queue／runtime／I/O／保存功能。

本入口不輸出中文本文，簡中與其他繁中未繞過原有臺灣本地化。結果仍是模糊候選與時間一致性提示，不產生 ExactCueMatch／CandidateEvidence／可靠錨點／目前播放位置；精確驗證器、仲裁、擷取時鐘與單一時間軸不改接。見 [ADR 0084](adr/0084-paired-chinese-fuzzy-source-reference.md)。

### 第八批驗證

新增 17 項配對核心／契約、4 項實際 worker／管理器測試，共 21 項。涵蓋五語與三種中文分類、來源／中文不同索引、雙段連續對應、中文不能冒充原文、缺參考／錯身分／來源／版本／語言、時間差一微秒／重複／不連續、遠處原文重複、原 allocation、雙軌合計 count／bytes、後段原文超量、取消／期限／DP、query 契約、容量回收、三個獨立觀測與 Stop／Drop／父 scope 隔離。

第一輪新增 paired enum 後，補齊兩個既有測試 helper 的 exhaustive match；Clippy 的巢狀條件提示已合併修正，未停用警告。155 項定向測試首次通過後，將新 allocation 測試改用獨立測試 CPU 名額，避免干擾既有全程序容量斷言；最終版本再次 155 項通過。正式工作容量不變。

| 實際執行命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo test --locked --offline fuzzy -- --nocapture` | 最終 155 項通過，2.15 秒；含本批 21 項 |
| `cargo test --locked --offline subtitle::exact_alignment --quiet` | 24 項通過，0.46 秒 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 最終通過、零警告，18.33 秒 |
| `cargo build --locked --offline` | Debug 通過，0.93 秒 |
| `cargo build --release --locked --offline` | Release 通過，1 分 10 秒 |
| `cargo test --locked --offline --quiet` | 1882 項 library、12 項設定、4 項探索、10 項啟動，共 1908 項一般測試通過、零失敗；既有 15 項 ignored 不變 |

完整回歸在 Release 完成後單獨執行，設定 RUST_BACKTRACE=1，命令正常結束；library 572.70 秒、設定 0.34 秒、探索 0.00 秒、啟動 1.59 秒。這些是測試耗時，不是字幕延遲／效能 SLA。第四批記錄的 MP4 五秒等待逾時本輪未重現，仍保留待追查，不宣稱已修復；未調整原生媒體或等待期限。人工檢查新正式邏輯沒有 recoverable unwrap／expect、unsafe、I/O、隱藏 spawn、無界佇列或增加取消祖先。

只用合成 RAM 文件與明確測試工作，不使用真實金鑰、私人媒體、來源服務或雲端；不宣稱實際版本配對、辨識品質或播放相容性已驗收。定向輸出的 synthetic worker panic 仍為既有刻意注入的通過案例。沒有新增 ignored、依賴、unsafe、產品內容寫入、第二份時間軸、`.git` 或 commit；未丟棄既有工作。

## 第九批：lookup 發布生命週期橋接

2026-10-23 新增 `FuzzyCueLocator::from_lookup(scope, &lease)`，借用已驗證的 ExistingTextLease 單軌五語或同版本中文／來源雙軌。crate-private 弱快照只保留原 query metadata Arc、兩軌 Weak 與原 availability，不複製整份本文、不增加 lookup／翻譯要求，也不製造 capture query／stamp。保留原 lookup 合計 1024 cues／128 KiB 與既有來源宣告／全文匹配契約，不因 owned 配對上限較大而放寬。

取得原有 CPU／local 名額後才同時升級兩軌，任一失敗即釋放另一軌。單軌共用原搜尋核心；雙軌以唯讀 parts view 共用第八批同版本／精確時間映射，不重新包裝或複製 bundle。weak locator、未 poll Future、已交付候選與軌跡快照均不保留強本文；開始中的 job 可保留借用至真正清理。原 allocation、媒體無關界線及 worker 輸入先於許可釋放的順序保持。

實際工作截止時間包含排程等待，取 offer／dispatch 三秒、capture 五秒及原 lookup query 五秒的最早者。單次候選、每點觀測期限與軌跡快照也核對原發布 guard；新的 ASR／成功／重複輸入／同信心 renew 不延長舊 query，取得新 lease 快照須新建 locator／epoch。管理器在 offer／state／poll／changed 檢查來源，閒置 driver 也在原發布期限退出，來源到期回 Stale，不只靠收到新 ASR 才清理。

lease.retire、較低確認信心撤回舊 availability、provider guard 取消，均透過原取消通道喚醒等待中的工作／管理器，不取消父 provider。來源原有祖先計入既有四祖先上限，超限明確 InvalidData；弱 guard 與 timer 不新增祖先、relay task 或 runtime。沒有調整既有 lookup clear／stop 對獨立 lease 的契約，Live owner 仍需使用原 retire／provider guard 管理來源撤回。

只釋放所有本文但未送出 retire 訊號時，validate／poll 立即拒收；Weak 沒有 Drop 通知，已等待的 changed 最遲在原發布期限喚醒，不假稱立即通知。本文仍可直接釋放；工作清理後會再次驗證，不能交付已失去來源的結果。這是明確 library 橋接，尚未自動接上一般 Live／GUI 或 Prepared 中文 Ready，不產生可靠錨點、播放位置或字幕採用證據，原精確驗證器／仲裁／擷取時鐘／時間軸不改接。見 [ADR 0085](adr/0085-weak-lookup-fuzzy-publication-lifetime.md)。

### 第九批驗證

新增 14 項真正 ExistingTextSession＋明確 RAM provider／worker／管理器測試。涵蓋單軌與雙軌原 allocation、零額外 lookup、未 poll／已交付雙軌釋放、lease 與 provider 退休、閒置／待處理／已交付等待喚醒、純本文釋放的 validate／poll、來源先於新 capture 到期、舊 query 不隨 renew 延長、信心下降撤回、工作原截止時間、開始中取消／逾時等待真正清理及許可回收、四祖先與父 scope 隔離、錯文件身分。

第一輪編譯補齊既有測試 helper 對 Lookup enum 的明確分支；未放寬正式限制、停用警告或新增 ignored。

| 實際執行命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo test --locked --offline fuzzy -- --nocapture` | 169 項通過，1.68 秒；含本批 14 項 |
| `cargo test --locked --offline subtitle::exact_alignment --quiet` | 24 項通過，0.46 秒 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過、零警告，1 分 09 秒 |
| `cargo build --locked --offline` | Debug 通過，0.92 秒 |
| `cargo build --release --locked --offline` | Release 通過，58.38 秒 |
| `cargo test --locked --offline --quiet` | 1896 項 library、12 項設定、4 項探索、10 項啟動，共 1922 項一般測試通過、零失敗；既有 15 項 ignored 不變 |

完整回歸在 Release 完成後單獨執行，設定 RUST_BACKTRACE=1，命令正常結束；library 579.69 秒、設定 0.81 秒、探索 0.00 秒、啟動 1.42 秒。這些是測試耗時，不是字幕延遲／效能 SLA。第四批記錄的 MP4 五秒等待逾時本輪未重現，仍保留待追查，不宣稱已修復；未修改原生媒體實作或等待期限。人工檢查新正式邏輯沒有 recoverable unwrap／expect、unsafe、產品 I/O、隱藏 spawn／runtime 或無界佇列；來源既有祖先計入原四祖先上限，未放寬限制，音訊擷取路徑未改動。

只使用本機合成文字與明確 mock lookup，不使用真實金鑰、私人媒體、來源服務或雲端，不宣稱實際來源品質或播放相容性已驗收。定向輸出的 synthetic worker panic 仍為既有刻意注入的通過案例。沒有新依賴／unsafe／產品 I/O／永久內容／第二份時間軸／`.git`／commit，未丟棄既有工作；結案金鑰仍由固定遮蔽介面手動輸入。

## 第十批：明確對齊暫停／恢復／重設

2026-10-23 新增 `session::alignment_control::AlignmentController`，由已套用的 Prepared Ready 或既有 lookup lease 明確建構為 Paused。只保留弱文字來源與原有許可，不持媒體 pin、強本文或額外服務；建構不需要 runtime，也不啟動 task、擷取或播放器。`resume().await` 才使用呼叫端 timer-enabled Tokio runtime 啟動一個既有 FuzzyProposalTask，沿用原 latest-only bounded watch、CPU 與四祖先限制。

健康執行中的 resume 冪等；暫停後恢復、reset 或異常後明確恢復均配置新 epoch。控制器間共用程序內 checked AtomicU64，從非零值遞增、用盡即明確失敗，不回繞重用。文件與 capture Session／generation 保持獨立；reset 先驗證 capture，再撤回、清理及重新綁定，相同 capture 也產生新 epoch。呼叫端仍須交付實際 capture 範圍及 Final ASR，不能把文件時間當成擷取 stamp。

pause／reset／stop 一旦被 poll，先停止 owner、撤回舊候選／軌跡，再等待 driver 和 local 許可屏障。AdmittedWork 在輸入與 CPU 許可之後釋放 local，因此 driver 被 abort 也不能提早宣稱清理完成。等待 Future 被 Drop 時，舊 owner／JoinHandle／已完成 join 結果留在控制器，後續操作先接續清理，不重複 poll 完成的 JoinHandle、不提前啟動新工作。reset 中途取消保留舊 capture 並暫停；stop 中斷後仍為終止，可再次 stop 等待。Paused／Stopped 狀態僅表示輸入已封鎖，完成清理以 await 成功返回為準。

stop 為終止操作，完成後放掉弱來源；不取消父 provider、其他控制器或 Prepared Session。Controller Drop 緊急撤回並 abort driver，已開始 CPU 工作仍合作退出並持有許可至真正清理；Drop 不同步 join，呼叫端應維持原 runtime 存活並等待 stop。錯誤保留型別，driver 異常明確回 WorkerFailed，不假裝成功；處理過錯誤後，尚有效來源可再明確恢復新 epoch。

原 lookup query 到期、退休或本文釋放仍拒收，pause／resume／reset 不更新舊 query／guard，不重抓來源。Ready 停止後能釋放本文與媒體 pin；控制器不持有它們。這些是對齊生命週期操作，不暫停或跳轉播放器、不啟動擷取、不推測 seek／目前媒體位置，也不把模糊提示轉成可靠錨點或採用證據。原精確驗證器、仲裁、capture-only 時鐘與共用時間軸未改接。見 [ADR 0086](adr/0086-explicit-alignment-epoch-lifecycle.md)。

### 第十批驗證

新增 17 項生命週期測試（16 項控制器、1 項真正 Prepared Ready），涵蓋被動建構、冪等恢復、新 epoch、雙軌／單軌舊快照撤回與重新累積、capture 重綁、無效 reset 保留舊回合、控制器／父來源隔離、終止後拒收、弱本文釋放、lease 退休、原始期限、缺少 runtime、序號用盡、等待 join 前取消、已 join 後等待輸入清理時取消、reset／stop 中斷、driver abort 錯誤也必須清理，以及 Ready 本文與媒體 pin 釋放。只用合成文字／檔案、真正 RAM lookup 發布及明確 translator mock；local 許可 gate 只模擬清理尚未結束，不冒充正式定位或雲端證據。

未放寬正式限制、停用警告或新增 ignored。最終版本已執行結果如下：

| 實際執行命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過、零警告，1 分 57 秒 |
| `cargo test --locked --offline alignment_control --quiet` | 17 項通過，0.57 秒 |
| `cargo test --locked --offline fuzzy --quiet` | 170 項通過，1.80 秒；包含本批 Ready 案例 |
| `cargo test --locked --offline subtitle::exact_alignment --quiet` | 24 項通過，0.46 秒 |
| `cargo build --locked --offline` | Debug 通過，0.75 秒 |
| `cargo build --release --locked --offline` | Release 通過，1 分 07 秒 |
| `cargo test --locked --offline --quiet` | 1913 項 library、12 項設定、4 項探索、10 項啟動，共 1939 項一般測試通過、零失敗；既有 15 項 ignored 不變 |

完整回歸在 Release 結束後單獨執行，RUST_BACKTRACE=1，命令正常結束；library 567.16 秒、設定 0.34 秒、探索 0.00 秒、啟動 1.68 秒。這些是測試耗時，不是字幕延遲或效能 SLA。第四批記錄的 MP4 五秒等待逾時本輪未重現，仍保留待追查，不宣稱已修復；本批未改原生媒體實作或期限。

人工檢查新正式邏輯沒有 recoverable unwrap／expect、unsafe、產品磁碟／網路 I/O 或無界佇列；唯一新增的 driver spawn 位於明確 resume，沿用原工作容量、期限與取消祖先上限。音訊擷取、精確採用及共用時間軸不改接。沒有新依賴、安裝、永久內容、第二條管線、`.git` 或 commit，未丟棄既有工作。測試不代表真實媒體／雲端品質或播放相容性已驗收，固定遮蔽介面與結案手動金鑰安排保持。

## 第十一批：經驗證的 Prepared／Live 定位

新增獨立 AlignmentVerifier，至少三筆獨立 Final／cue，文字一致度至少 950、次高差距至少 100；checked 有理數模型核對全部端點誤差最多 50 ms、速率差最多 ±5%。VerifiedAlignment 為可撤回的封裝證據，保留來源／修訂／epoch／擷取連續性與期限，經同一 SubtitleSourceArbiter 進入同一 TranscriptTimeline。原 ExactCueVerifier 不放寬，raw fuzzy 提示仍不能直接採用。

最近位置是實際新語音匹配的媒體終點，signed offset 支援負值。靜音及持續擷取不推進媒體時間。實際 CaptureCursor 核對完整連續擷取歷史、250 ms 新鮮度與最多兩秒辨識落後；沒有實際窗口就明確失敗。只有此 Prepared 路徑保留最多八筆／三十秒 metadata，最新位置仍受原候選五秒期限限制。原 hint／lookup 不延長有效期。

一致的小幅速率差逐筆重新校時；前／後跳、章節、重啟與時間不連續先撤回，等待清理並換新 epoch。廣告、歧義與版本差異保留 Live AI。Prepared 中文參考軌使用明確本機 SRT／WebVTT、來源語言與同版本標識，逐 cue 精確時間對應，中文仍先做臺灣本地化。來源字幕保留實際三品質策略資訊，不以較低品質冒充較高品質。

一般準備視窗新增參考軌欄與「保留」操作，真正移交 Ready 擁有者；關閉仍清除，不自動播放或捕捉。主視窗提供定位暫停／恢復／重設與清除 Ready，命令透過有界 watch，清除後 Live AI 繼續。正式 Live 協調與測試共用同一接線；worker 通知與 UI tick 不會漏掉採用或重複交付。詳見 [ADR 0087](adr/0087-verified-prepared-live-alignment.md) 與 [工程門檻](phase-9-gate.md)。

### 第十一批驗證

新增 37 項一般測試：10 項獨立驗證器、9 項真正 Ready／worker／仲裁／時間軸生命週期、3 項正式 Live 協調接線、4 項 UI 命令／身分隔離、5 項明確參考軌規劃與 6 項隱藏原生視窗。涵蓋負偏移、每個端點、過期／修訂／來源撤回、連續擷取不推進媒體、四秒長句與五秒以上語句間隔、正負速率差、跳轉／章節／重啟、廣告後恢復、取消等待、品質不足、UI tick 先消費結果、缺少／不符擷取時間，以及真正 Ready 移交／清除及資源釋放。

`cargo test --locked --offline alignment --quiet` 實際通過 158 項，7.69 秒；包含既有精確定位與本批核心／整合／視窗測試。`cargo test --locked --offline prepared_dialog --quiet` 最終通過 68 項，68.39 秒。準備視窗第一次回歸曾因舊「播放尚未提供」文案、舊捲動末端與控制項索引斷言失敗，並使後續共用測試鎖中毒；已依新介面更新斷言與索引，重跑整組通過，沒有放寬正式逾時或容量。

最終版本的實際執行結果如下；測試 fixture 也已調整為先釋放 Ready 媒體 pin，再由 TempDir 清除合成檔案。

| 實際執行命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過、零警告，13.44 秒 |
| `cargo test --locked --offline alignment --quiet` | 158 項通過，7.69 秒 |
| `cargo test --locked --offline prepared_dialog --quiet` | 68 項通過，68.39 秒 |
| `cargo test --locked --offline fuzzy --quiet` | 170 項通過，1.67 秒 |
| `cargo build --locked --offline` | Debug 通過，最終 0.85 秒（增量） |
| `cargo build --release --locked --offline` | Release 通過，首次 1 分 04 秒，最終 0.90 秒（增量） |
| `cargo test --locked --offline --quiet` | 1950 項 library、12 項設定、4 項探索、10 項啟動，共 1976 項一般測試通過、零失敗；既有 15 項 ignored 不變 |

完整回歸在最後一次 Release 結束後單獨執行，RUST_BACKTRACE=1，正常退出。library 506.90 秒、設定 0.62 秒、探索 0.00 秒、啟動 1.15 秒。沒有新增 ignored、停用警告或放寬正式門檻；第四批歷史 MP4 五秒逾時本輪未重現，仍不宣稱已修復。

檢查新增核心／協調邏輯無 recoverable unwrap／expect、unsafe、產品內容落盤或無界佇列；實際捕捉 callback 不做匹配、UI 或網路等待。沿用原有 CPU lane、單格工作與有界服務管線，停止／取消等待真實清理。沒有新依賴、安裝、永久音訊／字幕／翻譯、遙測、第二條辨識／翻譯管線、Git 初始化或 commit，未丟棄既有工作。測試只用合成檔案、明確服務 mock 與隱藏自有 Win32 視窗；尚未使用有效金鑰、私有音訊／影片或真實供應商。測試耗時不是字幕延遲或效能 SLA。

依 [工程門檻](phase-9-gate.md) 已可啟動 Phase 10 — Performance。本次依使用者停止條件結束於工程就緒，未開始效能量測或宣告全面產品驗收。

## 結案人工驗收仍待執行

目前工程測試不證明真實供應商或播放器已驗收。依使用者既有決定，結案時由使用者在固定遮蔽介面手動輸入 OpenAI／Gemini 金鑰，再檢查實際辨識、臺灣譯文與可見來源／雲端揭露。以明確授權的媒體測試 Ready 保留、手動播放、從非零位置開始、暫停、前／後跳、章節、重啟及廣告後重新定位，並核對清除 Ready／更換來源後沒有舊字幕。既有格式及版本限制須保留，不將合成測試視為 VLC、PotPlayer、Chrome、Edge、Spotify、Media Player 或遊戲的相容性證據。Phase 10 的延遲、P50／P95、佇列、CPU／記憶體與服務並行量測尚未開始。

## 接續工作（第十批時的歷史狀態）

已提供候選核心、背景擁有者、Ready 文字生命週期橋接、最新候選狀態層、Prepared recognized 借用、多觀測時間一致性提示、相鄰雙段、中文參考軌、lookup lease 橋接及明確 epoch 生命週期控制器，不代表整個 Phase 09 完成。仍需 Prepared 中文參考軌 Ready、Live 實際擷取／一般視窗接線、三段以上／任意跨 cue 配對、經驗證的多觀測錨點、目前媒體位置、播放器暫停／seek／重啟的觀測、offset 與 drift 的明確規則及測試；持續擷取或靜音不證明媒體播放前進。Prepared Ready 不自動播放。Phase 10 尚未具備啟動條件，符合時提醒使用者。有效金鑰與真實雲端／私有媒體維持結案固定遮蔽介面手動驗收。設計見 ADR [0077](adr/0077-bounded-fuzzy-alignment-proposals.md)、[0078](adr/0078-shared-alignment-worker-and-proposal-lifetime.md)、[0079](adr/0079-revocable-prepared-ready-alignment-text.md)、[0080](adr/0080-latest-only-fuzzy-proposal-session.md)、[0081](adr/0081-prepared-recognized-alignment-source.md)、[0082](adr/0082-bounded-fuzzy-temporal-consistency.md)、[0083](adr/0083-bounded-adjacent-cue-fuzzy-matching.md)、[0084](adr/0084-paired-chinese-fuzzy-source-reference.md)、[0085](adr/0085-weak-lookup-fuzzy-publication-lifetime.md) 與 [0086](adr/0086-explicit-alignment-epoch-lifecycle.md)。
