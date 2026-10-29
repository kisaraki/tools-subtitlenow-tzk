# ADR 0088：有界的擷取診斷分布與觀測峰值

日期：2026-10-23。狀態：採用，第一批工程驗證通過。使用者已批准啟動 Phase 10，實際結果見 [Phase 10](../phase-10.md#第一批驗證)。

## 原因

既有每秒 queue depth 只能顯示當下狀態，不能代表過去峰值；單一耗時也不能代表 P50／P95。診斷必須有界，不在音訊 callback 排序、鎖住消費端或寫檔，也不能把未量測值或合成資料冒充真實服務效能。

## 決定

新增一般化 DurationHistogram，每份固定 496 個 u64 原子 bins，每個二倍數區間八個細分；低於 16 ns 精確，其他分位數回報所在區間的整數上界，區間寬度不超過下界的 12.5%。P50／P95 使用 nearest rank，欄位明確命名 p50_upper_ns／p95_upper_ns；max_ns 保留實際最大觀測，不必大於區間上界。沒有樣本時為 None，合法零耗時另計為一筆零值。

Duration 超出 u64 nanoseconds 或未來 Instant 不能冒充有效樣本，分開記 rejected_samples。計數 checked 飽和，不回繞；飽和或快照總數超出 u64 時明確 saturated 並停止提供分位數。原子讀取不是跨欄位交易，並行 snapshot 的 max 可能包含更新的樣本；停止工作後可讀穩定結果。讀者掃描固定 bins，沒有成長中的 samples、排序、額外 worker、I/O 或內容。

Capture Session 建立時，只有既有 DEBUG／TRACE 診斷啟用才配置兩份固定 histogram（合計小於八 KiB，不含配置器額外成本），其餘模式 timings=None。擷取執行緒只讀時鐘、更新固定原子計數，不等待網路、UI、磁碟、佇列空間或鎖；單一 sink writer 及單一 consumer 各自使用不同 histogram。診斷會影響觀測耗時，不以此宣稱零量測負擔。

sink_call 是 CaptureSink 方法進入至返回，包含驗證、複製及入列，也包含拒收／取消等呼叫；不包含進入前的 WASAPI、IPC、原生正規化及最終 histogram 記錄成本。不同 capture backend 的 sink 所在位置不同，不能將其當成完整 WASAPI callback、CPU 使用率或整體 capture overhead。

delivered_age 是真正交付 NormalizedAudio 時，距原 native capture estimate received 的時間。只統計成功交付的 block，不把被淘汰／過期／無效資料混入成功延遲；必須同時看 dropped_frames／stale_frames，不能利用丟棄慢資料宣稱延遲改善。這不是 STT、翻譯或畫面可見延遲，也不推進媒體時間。

兩個 ring 每次成功 push 後更新 observed_high_water，成功入列至少曾有一格；force_push 交回舊值代表實際滿載，記錄容量 12。並行 dequeue 可能使讀取低估短暫峰值，因此明確標為觀測下界，不能冒充完全精確的歷史 maximum。停止清空 queue 不抹去本 Session 數值，下一 Session 從空值重新開始。既有 frame 淘汰計數意義不變，不把 frame 數改稱 segment 數。

一般 capture／Live 的既有 Debug stderr 路徑報告這些純數值，停止／重連退休時補最後一份。無內容、來源名稱、私有 URL、金鑰、音訊或字幕，也無檔案 sink／網路／遙測。原 hard-coded active_requests=1 改名為 stt_sessions=1：這是執行中的 STT Session 數，不是量測到的雲端要求並行數。實際 API 並行儀表留待後續。

## 驗證與範圍

核心測試涵蓋完整 bucket 邊界、整數極值、nearest rank、無樣本、零值、未來／過長時間、飽和、多 writer 計數、Drop 與 Session 隔離。Capture 測試涵蓋過載淘汰、峰值在 drain／stop 後保留、normalized 入口、拒收呼叫、真實交付時計時及預設關閉。合成 probe 只跑自有零值 PCM／原有正規化與消費端，完全不呼叫 WASAPI、探索使用者程序或連線服務；結果不能代表產品端到端效能或相容性。

本批不優化未測出的瓶頸，不新增相依。CPU／memory、完整 capture overhead、STT／翻譯／UI 關聯延遲、各 queue／worker／API 並行與 segment 淘汰仍屬 Phase 10 後續；Phase 11 尚未就緒。正式金鑰與真實媒體依既定安排留到結案固定遮蔽介面手動驗收。
