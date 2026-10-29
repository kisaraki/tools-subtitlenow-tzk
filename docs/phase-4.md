# Phase 4 — Translation

啟動：2026-10-09，依使用者「進行下一個作業」接續。有效 API 金鑰的互動輸入與真實服務測試留至結案；本批不要求金鑰、不連雲端、不傳送私人內容。產品規格依 SPEC 第 7、20–28、78 節。

目前狀態：第二批已接入實際 HTTPS／SSE adapter 與 STT → 翻譯 → 單一時間軸，透過 --translate-pid 明確啟動；第三批補上前綴提交的執行狀態保護。第一批是下方歷史紀錄。工程實作與離線驗證已推進，未把人工伺服器文字當成真實翻譯品質。

## 第一批已實作

src/ai/translation.rs 定義 Translator、能力、要求、串流 emitter、可取消 job 與 TranslationCoordinator。目標透過固定方法取得 zh-TW，沒有任意 target 參數；來源語言沿用五語型別。要求帶原始 Session／generation、utterance、source revision、sequence 與時間，以及前文、詞彙表與臺灣用語指南。沒有正式 dummy translator；所有人工翻譯只在 cfg(test)。

三種品質影響實際排程與要求：

| 品質 | Partial 合併等待 | 前文上限 | Final 校訂 |
| --- | --- | --- | --- |
| Fast | 60 ms | 1 個已穩定句子 | 單次翻譯 |
| Standard（預設） | 160 ms | 3 個已穩定句子 | 單次翻譯，附詞彙表 |
| High | 300 ms | 8 個已穩定句子 | 有前文／詞彙表或曾發生來源回退時，安排 Draft → Review |

Final 立即可排程，不等待 Partial debounce。High 的 Draft 完成先標 Stable，Review 只在完成時一次替換，避免逐 token 重寫已顯示的稿件。尚未確認的尾端可回退；沒有把「文字連續出現兩次」假裝成語意穩定。呼叫端只有在確認時才能 commit_prefix；UTF-8 邊界、前綴不可縮短／覆寫與 Final 保護均由時間軸檢查。

工作由呼叫端持有並 poll／drop，沒有 detached task。Semaphore 依供應商能力限制同時要求，允許 1–4 個；不建立任意 CPU worker。每個 utterance 只保留最新來源修訂，較新的修訂取消舊 job；舊 Session／generation／revision 的排隊結果拒絕套用。64 格更新佇列滿載明確取消，不默默遺失 Final。來源修訂從 ingest 到翻譯／校訂的總期限為 3 秒；待排程過期或活動要求逾時會停止 Session，避免累積數十秒延遲。這是有界失敗策略，不是達成字幕效能目標的宣告；後續正式管線需接上可見狀態與同供應商復原。

src/subtitle/timeline.rs 建立單一 TranscriptTimeline，包含 source、translated、Tentative／Stable／Final、已提交前綴與獨立翻譯修訂。依 source sequence 排序，最多 64 項，淘汰後舊來源不能再出現。所有未來 renderer 共用它，不各自啟動辨識／翻譯。SessionGlossary 只有當次 identity 的 RAM，最多 64 項、每個來源／目標詞最多 256 bytes；相同詞更新，不做永久術語資料庫。

## 第一批邊界與當時後續

目前只有共用核心與可測試排程，尚未加入 OpenAI／Gemini 的實際翻譯 HTTP／串流 adapter，也未把 TranslationCoordinator 接到 --stt-pid。現有命令仍只顯示 ASR 來源文字，不能把品質策略測試解讀成正式翻譯已可用。臺灣用語指南是供 adapter 採用的明確要求，尚不能證明真實模型輸出自然 zh-TW；需在結案使用五語內容與術語案例驗證，不能只驗證繁簡字形。

後續同階段工作：依官方 API 實作兩種翻譯 adapter、固定 locale／資料與指令隔離、串流解析、錯誤與速率分類；串接單一 STT → 翻譯 → timeline 管線，加入有界同供應商復原；檢查 source language、Taiwan localization、詞彙一致性與字幕穩定性。Prepared Mode、內容辨識及字幕視窗不在本批範圍。

## 互動金鑰測試安排

為落實使用者安排，既有 STT 加入 --interactive-key。只在明確啟動時出現隱藏的本機輸入提示，不透過聊天工具索取金鑰、不預先測試。金鑰不寫入認證管理員或設定；當次重連仍使用同一個 RAM 金鑰，停止時清除本程式的配置。TLS／HTTP 套件內部副本不宣稱全面擦除。正式結案步驟見 Phase 3 測試操作；翻譯 adapter 完成後延伸同一個互動測試流程。

## 本批驗收

2026-10-09 已執行並通過 cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked、cargo build --locked --release。一般測試共 127 項通過（103 library、12 設定、4 探索、8 啟動），新增 14 項翻譯核心測試。十三項硬體／手動／外部服務測試仍預設略過，本批沒有重跑或計為通過。

新增測試涵蓋品質的等待／上下文與有條件第二次要求、來源修訂取消、亂序完成、已排隊舊結果／舊 generation、64 項歷史與詞彙表上限、UTF-8 前綴與回退、pending provider 的取消／逾時、能力／並行上限、輸出滿載及待排程過期。使用明確 MockTranslator／PendingTranslator／FailingTranslator，沒有將人工字串呈現為正式辨識或翻譯。--help 已檢查互動旗標；沒有觸發金鑰輸入提示。未改動原生擷取／helper／COM，不新增套件。

架構理由見 [ADR 0007](adr/0007-translation-revisions-and-timeline.md)。本階段尚未結案，真實翻譯、臺灣用語品質與延遲均待後續正式 adapter 及結案互動測試。

## 第二批：服務端接線與整合

src/ai/translation_http.rs 實作 Translator。OpenAI 使用 [Responses SSE](https://developers.openai.com/api/docs/guides/streaming-responses) 與 [gpt-6-luna](https://developers.openai.com/api/docs/models/gpt-6-luna)，reasoning none；Gemini 使用 [streamGenerateContent](https://ai.google.dev/api/generate-content) 的 gemini-3.8-flash，thinking LOW。這些是官方文件確認的 API 計畫，帳號模型可用性仍待有效憑證驗證。Fast／Standard／High 的上下文與校訂策略保留，不以模型名稱當品質標籤。

固定 HTTPS host、系統根憑證、禁用重新導向及 proxy、敏感認證 header、共用連線池。兩個服務都送 store:false，不送 cachedContent、previous_response_id 或 tools；這些選項不代表供應商所有留存政策都由本程式控制。本程式沒有內容快取、請求正文日誌或檔案輸出。指令在 OpenAI instructions／Gemini systemInstruction，source、前文、詞彙、Draft 與 committed prefix 是獨立 user 資料 JSON。

單一 SSE event 上限 64 KiB，response／request 上限 256 KiB，文字仍為 8192 bytes。解析支援分割 UTF-8、LF／CRLF、多行 data 與註解；Reasoning／thought 不顯示。OpenAI 等 response.completed 且完整 output；Gemini 等 STOP。EOF、MAX_TOKENS、incomplete、refusal、未知 item／錯誤欄位均明確回報，不提交截斷 Final。每個 HTTP chunk 合併更新並讓出 executor，防止多個立即可讀區塊使消費端無法 poll；真正滿載仍取消。

共享速率閘門至少間隔 200 ms，核心最多同時兩個要求。可重試的網路／429／逾時最多一次同服務重試，退避 100–175 ms；總過期限制仍為來源修訂的三秒，取消與期限可中止等待，不能為重試延長來源壽命。

輸出用 zhconv 0.4.2／OpenCC 的 ZhTW 與 TWPhrases 轉換，包含軟體、伺服器、網路、資料夾、滑鼠、計程車、印表機與硬碟等臺灣用語。AI 原本仍被要求自然臺灣繁中及語境一致性；字詞轉換不能證明語意與自然度。關閉預設 MediaWiki feature，只啟用 OpenCC／TWP／compress，一般字詞表不是使用者內容快取；授權文字見 [NOTICE](../licenses/NOTICE.md)。

src/session/live_translation.rs 持有 caller-owned jobs、Semaphore 排程與共用時間軸，沒有 detached task。等待 changed 的 future 被取消不會丟棄既有工作；丟棄整個 flow 才取消與回收全部要求。app 的翻譯模式不再另開 TranscriptLedger；ASR-only 仍沿用原有 ledger。原文與翻譯按同一來源修訂，舊結果由 timeline 拒絕。接到 source stop／Ctrl+C／失敗時先取消 flow，回收要求、停止 capture，再以新 Session 重建同一來源與服務；最多三次有界重連。

--translate-pid 支援明確 PID、provider、五語 source 與 quality，互動旗標仍只在明確啟動時要求 RAM 金鑰。輸出只接受互動式終端機，不提供逐字稿匯出；目前是診斷修訂列，尚不是 SideChat／BottomSubtitle 視窗。Debug 只量測工作／佇列數，不輸出正文。持續 Partial 修訂保留第一個待排程 due time，避免永遠重設 debounce 而無法開始翻譯。

2026-10-09 第二批最終驗收：格式、Clippy（all-targets／all-features、-D warnings）、完整測試、Debug 與 Release 建置均通過。一般測試 141 項（116 library、12 設定、4 探索、9 啟動），新增 14 項；十三項原有硬體／手動／外部服務測試略過，未重跑或計為本批通過。新增測試包含兩個服務 × 五語的 MockSpeechRecognizer → localhost HTTP／SSE → LiveTranslation、UTF-8／CRLF、完整／截斷／拒絕、來源關聯、臺灣用語、同服務 429 重試、100 個更新的突發串流、flow Drop 回收、Partial debounce 與命令／重導向拒絕。

實測曾發現未啟用 TWPhrases 時只有繁簡轉換，以及串流在 paused consumer 前過早飽和；已啟用臺灣詞彙並在真正的 concurrent flow 驗證合併與讓出 executor。這是離線協定修正，不是翻譯準確度 benchmark。沒有讀取實際 API 金鑰、啟動輸入提示或向真實供應商送出音訊／文字。仍待有效帳號的真實五語、自然度、術語一致性、停止／重連與端到端延遲結案驗收。

## 第三批：前綴提交狀態保護

2026-10-09 檢查發現：job 持有前綴快照，若在要求執行中改寫 timeline 的固定前綴，後續合法更新會因前綴不一致而造成 Session 失敗。TranslationCoordinator 現在於待排程、執行中或完成結果未套用時回報 PrefixBusy，不變更前綴，也不取消 Session。完成結果套用後才能明確提交，下一次來源修訂會帶入固定前綴。已停止的 Session 拒絕變更；未知段落、越界或非 UTF-8 邊界均不修改時間軸。LiveTranslation 提供相同保護的入口，並未自動把重複文字視為語意已穩定。

新增兩項回歸測試涵蓋排程／執行／結果佇列三個時機、下一次來源修訂、無效提交及停止後提交。cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked 及 cargo build --locked --release 均通過。一般測試共 143 項通過（118 library、12 設定、4 探索、9 啟動）；原有十三項硬體／手動／外部服務測試仍略過。未啟動實際音訊擷取、金鑰提示或真實雲端服務。架構原因同步記錄於 ADR 0007。
