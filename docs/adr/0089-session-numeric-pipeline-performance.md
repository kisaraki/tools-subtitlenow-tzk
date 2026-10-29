# ADR 0089：Session 數值管線關聯與程序資源量測

日期：2026-10-23。狀態：採用；Phase 10。

## 原因

只有 capture ring 的交付時間無法分辨辨識、翻譯、排隊與畫面提交的負擔。Session 數也不能代表真正執行中的服務操作。必須量測實際生命週期，同時避免保存本文、增加擷取端鎖或以 mock 宣稱真實服務速度。

## 決定

沿用 DEBUG／TRACE 開關，每個 Live Session 建立獨立 `LiveDiagnostics`。九種延遲沿用 ADR 0088 的固定 histogram；metadata 最多 2048 個音訊時間錨點、64 個 utterance、128 個語音送出標記及 128 個翻譯輸出標記。只保存身分、修訂、工作編號、時間及狀態，不保存文字、音訊、URL 或憑證。metadata 的短同步鎖僅在 capture 下游，沒有跨 `.await`、網路或 native capture callback。

辨識時間從真正送入 STT 的音訊區段映射到 received 時間；缺少有效錨點就記缺樣，不改用 Session 開始時間。語音及翻譯的送出／消費分開計時。翻譯採用須經原時間軸檢查，繪圖 receipt 再核對 Session、generation、來源修訂、job、pass 及輸出修訂；舊 frame、重複 receipt、來源替換或退休不能冒充新結果。UI receipt 只代表成功的原生繪圖提交／同步 paint；不代表 compositor 或螢幕實際發光時間。隱藏、最小化、搜尋或回捲中的 SideChat 不計最新列提交。明確 mock receipt 只驗證管線。

以 RAII 計數真正 poll 中的 STT／翻譯 provider operation；另計已持有的翻譯 semaphore permit，未 poll 的 job 也會占 permit，但不能算執行中的 API。這些是邏輯操作，不是 HTTP／TLS 連線數。queue 高水位為觀測下界，捕捉角色 worker 數從尚未結束的 owned join handle 取得。resolver 沿用既有 pool 的實際 worker 數，不稱為整個程序執行緒數。

frame、audio block、輸出修訂、工作替換、final segment 與診斷 metadata 淘汰分別計數，不加總成一個「丟失字幕」數字。High 模式依既有策略抑制 Review partial；這類 update 拒收不是 final segment 丟失。取消未完成的要求不產生假 completion。

資源取樣在協調端用只讀 Win32 查詢，父程序與精確 owned helper 分開。helper 在 DEBUG／TRACE 啟動時複製其現有 handle，沒有依來源 PID 讀取使用者程序。handle 由 RAII 釋放；擷取 callback 不查程序資源。第一筆 CPU 為空值，身分／時間／計數倒退回傳具型別錯誤。`cpu_core_ppm` 的一百萬代表一個核心；`cpu_machine_ppm` 再除以 `available_parallelism()`。短樣本可能受 OS 計時粒度影響而為零，不能推論沒有成本。

CPU 使用 kernel＋user 的差值；其單位與多核心意義依 [GetProcessTimes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes)。`private_commit_bytes` 與 `working_set_bytes` 分開報告，依 [PROCESS_MEMORY_COUNTERS_EX](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex) 與 [GetProcessMemoryInfo](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo) 定義，不把 working set 當作私有提交量。

## 代價與限制

計時、原子操作、下游 metadata 鎖、程序查詢、診斷 helper handle 與有界配置都有成本。以 Release 三輪合成管線及自有靜音 capture 的開／關對照報告；不是通用硬體、雲端或播放器 SLA。短 native 對照包含測試 harness、IPC、正規化及消費端；不包含來源程序及 Windows 音訊服務，因此不稱為純音訊驅動 overhead。

沒有變更精確驗證、來源仲裁、媒體時鐘、品質策略、queue 容量或過期期限；沒有新增持久內容、export、遙測或服務 fallback。真實服務、媒體及跨播放器驗收依使用者約定留到結案固定遮蔽介面手動執行。結果與門檻見 [Phase 10](../phase-10.md) 及 [工程門檻](../phase-10-gate.md)。
