# Phase 10 — Performance

2026-10-23 使用者批准啟動並要求持續至 Phase 10 結束。起點為 Phase 09 第十一批：1976 項一般測試通過，既有 15 項 ignored，fmt／零警告 Clippy／Debug／Release 通過。本次本機確認 Windows 10.0.19045.0、rustc 1.98.1（48a229cea，2026-09-01）。**Phase 10 已完成工程量測與驗證，可啟動 Phase 11；本次停止於此，尚未啟動 Phase 11。** 正式金鑰、真實服務與使用者媒體依既定安排，留到結案固定遮蔽介面手動驗收。下方第一批的進行中敘述保留為歷史，最終狀態以第二批結果與工程門檻為準。

## 第一批：擷取診斷基礎

新增有界 DurationHistogram、Capture Session 的 sink_call 與 delivered_age 分布，以及 raw／normalized ring 的觀測高水位。一般 capture／Live 的 Debug stderr 每秒及正常清理後報告本 Session 純數值；沒有音訊、字幕、私有 URL、來源名稱或金鑰，也沒有檔案／網路 sink。原 hard-coded active_requests=1 改名為 stt_sessions=1，避免把 Session 數冒充實際服務要求。

延遲計時只在建立 Session 時已啟用 DEBUG／TRACE 才配置，每 Session 兩份固定 histogram 合計小於八 KiB，不含配置器開銷；其他模式 timings=None。每份 496 個 bins，讀者才掃描分布，callback 不排序、配置、寫檔或等待消費端。這仍有時鐘與原子操作成本，不宣稱沒有量測負擔。

P50／P95 是 nearest-rank 所在區間的上界，命名 p50_upper_ns／p95_upper_ns；低於 16 ns 精確，其餘區間寬度不超過下界 12.5%。max_ns 是實際最大樣本，可能小於區間上界。沒有樣本／未啟用為空值，合法零樣本另計；未來／超長時間記 rejected_samples，不變成零值。溢位明示 saturated 並撤回分位數；並行快照不是交易，停止後穩定。設計見 [ADR 0088](adr/0088-bounded-capture-performance-diagnostics.md)。

## 量測邊界

| 指標 | 本批實際意義 | 不能宣稱 |
| --- | --- | --- |
| sink_call | CaptureSink 方法進入至返回，含驗證／複製／入列及拒收呼叫 | 完整 WASAPI／IPC／正規化或 instrumentation CPU overhead |
| delivered_age | 成功交付 block 時，距其 native capture estimate received 的時間 | STT／翻譯／畫面可見或端到端字幕延遲 |
| observed_high_water | push 後觀測的 Session ring 峰值下界，eviction 確認滿載 12 格 | 無遺漏的真實瞬間最大值 |
| dropped_frames／stale_frames | 原有淘汰／過期 frame 數；與成功延遲一起讀取 | segment 數、服務要求數或無丟失 |
| stt_sessions | 一個執行中的 STT Session | 實際 API 並行要求數 |

不同 backend 的 sink 所在位置不同，不能跨架構直接比較。成功樣本分布排除被丟棄的慢資料，必須一起看淘汰與過期計數，避免倖存偏差。DEBUG 計時也不是產品預設效能證據。沒有修改精確／模糊定位、媒體時鐘、仲裁、STT／翻譯策略或音訊持久化規則。

## 使用

沿用既有 `SUBTITLENOW_LOG=DEBUG` 或 `TRACE`，在啟動 Session 前設定；正常預設 INFO 不配置計時分布。日誌僅送 stderr，不增加檔案保存；有效金鑰仍由固定遮蔽介面手動輸入。此處未啟動任何使用者音源或真正服務。

完全離線、自有合成 PCM 的探測入口：

```powershell
cargo run --release --locked --offline --example capture_metrics_probe
```

probe 明確設定自身 DEBUG logger，依序跑 50 個 20 ms／960-frame 零值 stereo PCM blocks 的正常情境，以及 50 blocks 突發、消費端延後 100 ms 的過載情境。沿用真正有界 ring、正規化與消費端，只輸出數值 JSON，不呼叫 WASAPI／程序探索、不連網、不讀金鑰／媒體或寫內容。保留完成後 100 ms 收尾並限四秒；逾時明確失敗，不輸出成功報告。這是診斷通路測量，不是端到端、真實來源或雲端基準。

## 第一批驗證

新增 11 項一般測試，涵蓋分布邊界、空值／零值、nearest rank、時間拒收、飽和、多執行緒計數、Session 隔離、過載峰值、淘汰、normalized 入口、拒收呼叫、成功交付／停止與預設關閉。`cargo test --locked --offline phase10 --quiet` 通過 11 項，0.01 秒；`cargo test --locked --offline audio::capture --quiet` 通過 21 項，0.39 秒。

最終版本實際執行結果如下：

| 實際執行命令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過、零警告，最終 1.21 秒 |
| `cargo test --locked --offline phase10 --quiet` | 11 項通過，0.01 秒 |
| `cargo test --locked --offline audio::capture --quiet` | 21 項通過，0.39 秒 |
| `cargo build --locked --offline` | Debug 通過，最終 0.85 秒（增量） |
| `cargo build --release --locked --offline` | Release 通過，首次 56.48 秒，最終 0.78 秒（增量） |
| `cargo run --release --locked --offline --example capture_metrics_probe` | 正常退出；兩個合成情境實際結果見下表 |
| `cargo test --locked --offline --quiet` | 1961 項 library、12 項設定、4 項探索、10 項啟動，共 1987 項一般測試通過、零失敗；既有 15 項 ignored 不變 |

完整回歸在最後一次 Release 結束後單獨執行，RUST_BACKTRACE=1，正常退出。library 496.84 秒、設定 0.33 秒、探索 0.00 秒、啟動 1.16 秒。這些是檢查耗時，不是產品延遲。沒有新增 ignored、放寬期限／容量、停用警告或修改媒體 codec；既有歷史 MP4 五秒逾時本輪未重現，不宣稱已修復。

檢查本批正式邏輯無新增 unsafe、recoverable unwrap／expect、同步鎖、worker 或相依，沒有媒體／本文寫入、cache、export、遙測或正式金鑰取用。新增原子計數與固定儲存不讓 capture 等待消費端，取消與 Session 隔離維持。實際測試只用合成資料、明確服務 mock、隱藏自有視窗與既有隔離／loopback fixture，未啟動使用者音源或真實服務。本機仍無 .git，沒有初始化、commit 或丟棄既有工作。

第一批工程驗證完成；Phase 10 仍在進行，尚未具備 Phase 11 條件。

### Release 合成探測的實際結果

`cargo run --release --locked --offline --example capture_metrics_probe` 正常退出，example 建置 3.12 秒。兩個情境各產生 48,000 raw frames／50 blocks；下列為單次實際數值，非多輪統計或 SLA，沒有原生擷取／辨識／翻譯／畫面工作。後一情境與前者的排程、緩衝及快取狀態不同，不以方法耗時差異推論效能改善。

| 情境 | 成功 blocks | sink P50／P95 上界 ns（50 次呼叫） | 交付年齡 P50／P95 上界 ns | 實際交付最大 ns | raw／normalized 觀測峰值 | 淘汰／過期 frames |
| --- | --- | --- | --- | --- | --- | --- |
| steady，20 ms 節奏 | 50 | 1919／3071 | 7864319／16777215 | 16478500 | 1／1 | 0／0 |
| burst，消費延後 100 ms | 12 | 511／831 | 100663295／100663295 | 100370500 | 12／12 | 36480／0 |

sink_call 實際最大分別為 4100 ns／1400 ns。兩個情境的 rejected_samples 均 0，saturated 均 false。過載的成功樣本僅 12 筆，必須連同 36,480 frames 淘汰一起解讀；沒有藉此宣稱產品字幕達成一秒或 UI 一百毫秒目標。報告僅 stdout 數值，沒有產品檔案保存。

## 第二批：完整管線與資源診斷

在既有 Live 協調器、STT／翻譯介面、單一時間軸及 UI bridge 接上可選 `LiveDiagnostics`，預設 INFO 不建立。只保存有界數值 metadata：2048 個音訊時間錨點、64 個 utterance、各 128 個語音及翻譯送出標記，另有固定九份 histogram。診斷鎖僅在擷取下游，不在 callback，也不跨 `.await`。詳細理由見 [ADR 0089](adr/0089-session-numeric-pipeline-performance.md)。

| 延遲指標 | 起點 → 終點 | 限制 |
| --- | --- | --- |
| SpeechFirstPartial／SpeechFinal | 真正音訊 received＋區段偏移 → 第一筆 partial／final emit | 每 utterance 各一筆；無 native 時間錨點就缺樣 |
| AudioToSpeechConsumer | block received → provider 成功取到音訊 | 未成功交付的淘汰不在分布內 |
| SpeechOutputQueue | 每個修訂 emit → 協調器接收 | partial 及快速 final 分別保留標記 |
| TranslationFirstOutput | 真正 provider 操作開始 → 首筆成功送出 | 每個 Draft／Review 要求各計，不含排程等待 |
| TranslationRequestFinal | provider 操作開始 → 要求完成輸出 | High 的 Stable Draft 也屬要求完成，不冒充 Final 字幕 |
| TranslationApply | 輸出標記建立 → 通過時間軸採用 | 送入 queue 前先建立標記，避免跨執行緒競爭漏樣 |
| TranslationToRenderSubmission | 該輸出送出 → 同 binding 的原生繪圖提交／同步 paint | 不是 compositor 或實際螢幕可見時間 |
| CaptureToFirstUsefulSubmission | 真正音訊起點 → 該 utterance 首次有效譯文提交 | 同 utterance 只計一次；缺樣不使用 Session 開始時間 |

UI receipt 核對 Session／generation／來源修訂／job／pass／輸出修訂，舊 frame、重複 receipt、退休與來源替換不能增加新樣本。原生視窗只在最新列可見的 SideChat 完成同步 paint，或 overlay 原生提交成功後回報；隱藏、最小化、搜尋／回捲中的 SideChat 不計。probe 使用明確 mock receipt，輸出 `native_ui=false`，不能據此宣稱 UI 一百毫秒或整體一秒目標已通過。

queue 指標包含 STT audio／event、翻譯 update／pending；沿用非交易快照與觀測峰值下界。STT／翻譯 operation 用 RAII 追蹤真正 poll 的 provider future；翻譯 permit 另計已取得但可能未 poll 的工作。取消／Drop 回收實際所有權後才報清理狀態。捕捉 worker 從 owned join handle 取得，resolver 回報既有 pool 的 worker 數；不代表程序全部 OS 執行緒或原始 HTTP／TLS 連線數。

淘汰按單位分開：原 capture frames、STT audio blocks、語音 events／Final 拒收、翻譯修訂取代／過期、update 拒收、Final segment 過期／未完成即退休，以及診斷 metadata 淘汰／缺樣。`FinalSegmentsUnfinishedAtStop` 是停止時仍未採用完成譯文的 Final segment 庫存，可能與過期原因重疊，不能和原因計數相加成唯一丟失總量。High 的 Review partial 依既有策略被抑制，不等於 Final segment 丟失。

父程序與精確 owned capture helper 分開取樣 CPU／private commit／working set／handle；helper query handle 只在計時開啟時建立。首筆 CPU 為空，無效時間／計數／程序身分回傳具型別錯誤；短樣本 CPU 零值可能是 OS 計時粒度。`cpu_core_ppm / 10000` 為單核心百分比，`cpu_machine_ppm` 再依可用邏輯處理器正規化。程序資源查詢、格式化與報告不在 capture callback，未新增內容檔案或網路 sink。

離線 Release 管線 probe：

```powershell
cargo run --release --locked --offline --example pipeline_metrics_probe
```

每輪使用不同 synthetic Session，三品質各八個 utterance，另有八段並行與四個等待中要求取消；共三輪。使用真實 STT／翻譯有界協調、時間軸及 UI bridge，辨識／翻譯明確 mock，輸入為 RAM 合成 PCM。每輪結束查程序資源，成功完成與取消分開報告，不寫內容檔。

原生擷取使用既有 ignored 的 `win32::soak_tests::native_process_capture_slow_consumer_stays_bounded`，只啟動測試自有靜音程序，三輪各跑一次 `SUBTITLENOW_PERF_TIMING=0` 與 `1`。最終每次父程序 idle 一秒、穩定消費十秒、故意停讀一秒，再驗證新鮮音訊與完整清理。最初一秒穩定區間的六次試跑全部成功，但僅一個 helper 樣本有非零 CPU，其餘父／helper CPU 均零；因此只將它視為解析度不足的試跑，延長正式樣本而沒有降低測試門檻。native 工作負擔包含父程序測試消費端、IPC、正規化與 helper，不包含來源程序及 Windows 音訊服務；開／關對照時間短且有排程噪音，不能稱為純 driver overhead 或通用 SLA。Phase 11 的長時間 soak 與大量重啟未在此提前執行。

### 三輪 Release 合成管線實測

2026-10-23 在同一個 probe 程序內依序完成三輪，每個情境都有獨立 Session。以下範圍是「三次各自分位數上界的最小～最大」，不是將原始樣本合併後重算分位數。所有時間單位 ns；辨識／翻譯／render receipt 都是明確 mock，並非服務或 native UI 實測。

| 情境（每輪八個 utterance） | STT partial P50／P95 上界 | 翻譯 first P50／P95 上界 | 要求 complete P50／P95 上界 | apply P95 上界 |
| --- | --- | --- | --- | --- |
| Fast | 18,431～30,719／36,863～98,303 | 15,728,639～16,777,215／16,777,215 | 31,457,279～33,554,431／33,554,431 | 30,719～40,959 |
| Standard | 20,479～24,575／49,151～180,223 | 15,728,639～16,777,215／16,777,215 | 31,457,279～33,554,431／33,554,431 | 28,671～65,535 |
| High | 22,527～30,719／40,959～53,247 | 15,728,639～16,777,215／16,777,215 | 31,457,279～33,554,431／33,554,431 | 22,527～28,671 |
| Standard，八段並行 | 40,959～53,247／49,151～65,535 | 15,728,639／15,728,639～16,777,215 | 31,457,279～33,554,431／33,554,431 | 49,151～73,727 |

三品質每輪 STT partial／final 都各八筆；Fast／Standard 各八個翻譯要求、16 筆採用，High 依既有上下文策略形成 15 個 Draft／Review 要求、23 筆採用，七筆 Review partial 被刻意抑制。每情境八次 partial→final 修訂替換不是八筆 Final 丟失；正常情境所有 Final 拒收／過期／未完成退休／停止未完成、音訊拒收／過期、診斷缺樣與鎖錯誤均零。

順序情境真正 STT／翻譯 operation 峰值各一，audio／event queue 峰值 1／2，翻譯 update／pending／permit 峰值皆一。並行情境翻譯 operation／permit 峰值皆四、audio／event 峰值 8／16、update 峰值四、pending 峰值四至五，未超既有容量。所有情境結束後上述 current 都為零。另三次取消各證實四個真正等待中的操作降為零、permit 歸零、停止未完成 Final 為四且假 completion 為零。

mock receipt P95 上界：Fast 73,727～114,687 ns、Standard 65,535～163,839 ns、High 53,247～65,535 ns、並行 65,535～90,111 ns。順序每輪 first useful 共八筆，P95 上界 16,777,215 ns；並行只為最後目前列產生一筆 first useful（P95 上界 50,331,647 ns），其餘段落仍正常完成，不能將該列樣本當成八段 UI 延遲。mock 的兩次 5 ms sleep 受 Windows 排程粒度影響，實測並不等於指定 sleep 時間。

三輪物件清理後 private commit 分別 847,872／864,256／868,352 bytes，working set 4,345,856／4,362,240／4,366,336 bytes，handle 均 77。所有活動情境 private commit 範圍 1,036,288～1,167,360 bytes；這是短 mock 程序結果，沒有足夠證據聲稱長期無洩漏。短取樣 CPU 回報零，保留為 OS 計數解析度限制，不稱為零成本。

### Release 原生靜音擷取實測

最終三輪開／關各一次，共六次正常退出。每次父程序 idle 一秒的 CPU 觀測均零；以下穩定消費區間各十秒，CPU 單位為單核心百分比，父程序／helper 分開。不是全機 CPU 百分比，也不包含來源程序及 Windows 音訊服務。

| 輪次／計時 | 穩定 blocks | CPU 父／helper（單核心 %） | private commit 父／helper（bytes） | 父／helper handles |
| --- | --- | --- | --- | --- |
| 1／關 | 1001 | 0.1562／0.4686 | 2,248,704／5,136,384 | 234／313 |
| 1／開 | 1001 | 0／0.6248 | 2,260,992／5,083,136 | 235／315 |
| 2／關 | 1001 | 0／0.3124 | 2,281,472／5,083,136 | 234／313 |
| 2／開 | 1000 | 0.4687／0.3124 | 2,244,608／5,111,808 | 235／315 |
| 3／關 | 1000 | 0.3124／0.3124 | 2,244,608／5,091,328 | 234／315 |
| 3／開 | 1001 | 0.1561／0.4685 | 2,248,704／5,103,616 | 235／315 |

父＋helper 觀測合計 0.3124%～0.7811% 單核心。每輪「開減關」合計差分為 0、+0.4687、−0.0002 個單核心百分點；保留負值，不截成零或宣稱改善。分開程序、順序執行、OS CPU 計時粒度及排程噪音限制，尚不能從三對樣本確立 instrumentation 的因果 CPU 成本。可確認計時開啟的父程序多持有一個真正 helper query handle，關閉時沒有配置該 probe；helper handle 的差異保留為觀測，不歸因於父程序診斷。

開啟計時時的最終分布如下；單位 ns。包含穩定消費、故意停讀與恢復，因此成功交付分布必須連同淘汰一起讀，不能只看 P95。

| 輪次 | sink 樣本 | sink P50／P95 上界／實際 max | delivered age P50／P95 上界／實際 max | 停讀後累計淘汰 frames |
| --- | --- | --- | --- | --- |
| 1 | 1102 | 1919／7167／40,900 | 25,165,823／33,554,431／137,287,200 | 42,240 |
| 2 | 1102 | 1919／6655／30,300 | 25,165,823／33,554,431／136,293,800 | 42,720 |
| 3 | 1103 | 1791／6655／53,700 | 25,165,823／33,554,431／132,597,800 | 42,720 |

六次 captured frames 為 528,960～529,440，淘汰 42,240～42,720，stale frames 都零；normalized 觀測峰值全部 12。關閉計時的 samples／P50／P95／max 全為空值，不是零耗時。原測試對停讀後 queue 容量、確有淘汰、新鮮音訊、同 Session、stop／清空與 worker 歸零的斷言全數通過，沒有放寬原容量或過期限制。穩定消費 blocks 是測試消費端收到的 block 數；淘汰 frames 是原捕捉計數單位，不能相減成 segment 數。

沒有數據顯示需改動既有工作數、buffer 容量或品質策略，本批保留原管線並提供可再量測的診斷。真實服務延遲、字幕品質、真實 native 可見時間、使用者媒體、跨播放器及長期記憶體穩定性均不在以上 mock／靜音數據所能證明的範圍。

### 最終驗證與完成判定

第二批新增 19 項一般測試；與第一批合計 30 項 Phase 10 測試。覆蓋有界數值 metadata、partial／final 快速送出、同 Session／generation／修訂關聯、缺樣、重複／過期 UI receipt、隱藏原生視窗不計提交、未 poll 工作的 permit、真實 emitter／consumer 採用標記、取消與程序資源時間／計數錯誤。沒有新增 ignored、停用警告或放寬原 queue／期限。

| 最終實際執行 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` | 通過、零警告；最後 15.17 秒 |
| `cargo test --locked --offline phase10 --quiet` | 30 項通過，0.40 秒；完整回歸另包含最後的測試程式區間調整 |
| `cargo test --release --locked --offline phase10 --quiet` | 30 項通過，0.13 秒 |
| `cargo test --release --locked --offline out_of_order --quiet` | 11 項通過，0.03 秒 |
| `cargo build --locked --offline` | Debug 通過，最後 0.76 秒（增量） |
| `cargo build --release --locked --offline --examples` | 兩個 Release example 通過，1 分 14 秒；後續僅調整 ignored 測試程式 |
| `cargo build --release --locked --offline` | Release 通過，最後 0.74 秒（增量） |
| Release `pipeline_metrics_probe.exe` | 三輪全部通過；十二個正常情境、三個取消情境，逐輪取樣資源 |
| Release 指定 native slow-consumer ignored 測試 | 初始一秒試跑六次與最後十秒對照六次全部成功；正式數據採後者 |
| `cargo test --locked --offline --quiet` | 1980 項 library、12 項設定、4 項探索、10 項啟動，共 **2006 項一般測試通過、零失敗**；既有 15 項 ignored 不變 |

完整回歸於最後 Release 與六次 native 對照完成後獨立執行，`RUST_BACKTRACE=1`，正常退出。library 530.27 秒、設定 0.69 秒、探索 0.00 秒、啟動 1.16 秒。既有歷史 MP4 五秒逾時本輪沒有重現，不宣稱已修復。ignored 中只明確選取本節的自有 fixture／慢消費測試，不宣稱其他 ignored 都已驗收。

正式新增的 native 查詢 unsafe 限於正確大小結構、有效 owned／pseudo handle 與可寫數值指標，附 SAFETY 理由；沒有跨 COM 指標或變更 capture IPC。診斷鎖只在下游，不跨 await；操作／permit 用真實 RAII 所有權，取消後才報收尾。未新增套件、內容持久化、字幕匯出、音訊錄製、遙測、跨服務 fallback、正式金鑰讀取或使用者來源擷取；精確驗證、來源仲裁與媒體時鐘不變。本機沒有 .git，未初始化、commit 或丟棄既有工作。

[工程門檻](phase-10-gate.md) 全數滿足。Phase 10 以可重複工程量測、邊界檢查及完整回歸完成；Phase 11 的長時間 Live、反覆來源切換、斷線／逾時、程序／裝置變更等強化工作已可開始，尚未執行。真實服務品質、使用者媒體、跨播放器、實際螢幕可見延遲及長期記憶體穩定性仍待依原約定驗收，不能從本次結果推論通用 SLA 或全面產品結案。
