# ADR 0004：以工作階段專用背景程序隔離原生音訊資源

日期：2026-10-07，更新：2026-10-08。狀態：採用；核心功能、30 分鐘連續擷取與最終 300 次重啟驗證通過。範圍：Phase 2。

## 原因與證據

解除錯誤版本門檻後，Windows 10 19045.6456 可真正擷取指定程序音訊。實測另外揭露兩個程式錯誤：`AgileReference<IAudioClient>` 無法建立（0x80040155），以及 `windows` 0.62 的外層 `PROPVARIANT::Drop` 自動清除借用的 Box，導致提前釋放與 heap corruption。已改為同一 MTA 內傳遞完成 operation，在擷取 worker 取得 client；借用 variant 的外層亦使用 `ManuallyDrop`，Box 仍由回呼負責釋放。

在上述錯誤修正後，同一程序 100 次原生啟動仍增加約 200 個 Timer handle。診斷確認 client 最後一次 Release 回傳 0；PCM16 與不使用格式轉換的浮點 client 都能重現。WinDbg 建立堆疊涉及 AudioSes 的電源通知與 thread pool。單獨 100 次探索及模擬 worker 重啟均不增加 handle。這是本機實測觀察，尚不能外推到所有 Windows 組建或認定是哪個系統版本修正。

單純更換格式、Reset、延後等待或保留 DLL 的實驗未解決此現象；正式程式沒有保留這些試驗。程式不操作或強制關閉原生元件內部的 Timer，也不要求使用者升級作業系統或更改擷取範圍。

## 決策與資料流

每個 Live 擷取工作階段最多一個背景程序，使用同一份應用程式執行檔。背景程序只處理所選來源的 Process Loopback 與正規化；沒有額外安裝、網路服務、驅動程式或注入。

```mermaid
flowchart LR
    A[父程序：指定 PID／持有 handle] --> B[目前 Session 的背景程序]
    B --> C[MTA 擷取 → 12 格 PCM ring]
    C --> D[正規化 worker → 12 格 mono ring]
    D --> E[有界匿名 pipe／本機 RAM]
    E --> F[父程序接收 worker → 12 格 mono ring]
    F --> G[目前 Session 消費端]
```

1. 父程序先固定使用者指定的 PID、持有來源 handle 並顯示 PID／程序樹範圍。背景程序在收到父程序啟動訊息前不探索或擷取；收到後以最新 Core Audio 列舉確認同一 PID 為獨立音訊來源，成功後才建立原生 capture client。此確認不在擷取父程序建立 Core Audio 物件。父程序持續持有 handle，避免 PID 重用。
2. 以匿名 pipe 傳遞版本標記、工作階段身分、sequence、frame position、可靠 QPC、接收時間及 mono f32。內容只在目前 Session 的 RAM／本機 pipe，沒有音訊檔案、磁碟 cache 或雲端傳輸。父程序不重複正規化。
3. 每則訊息上限 4096 bytes，音訊最多 960 samples，來源處理 ring 與父程序 ring 均固定 12 格。pipe 背壓可阻擋傳送角色，原生擷取仍只推入有界 ring，滿載淘汰舊資料。父程序重新檢查接收年齡，保留 250 ms 逾期策略。
4. 解碼必須核對 Session ID、訊息長度、樣本範圍／有限值與 sequence；只接受固定語意的錯誤名稱，不將背景程序文字提升為指令。晚到資料無法進入新的工作階段。
5. 背景程序加入唯一的 Job Object，啟用 `KILL_ON_JOB_CLOSE`；Job 不包含使用者選取的來源或其子程序。父程序退出時 OS 回收背景程序。正常停止先關閉 pipe，讓背景程序停止／join 兩個音訊 worker；500 ms 內未退出則終止自有 Job，回收原生資源。
6. Job 配置失敗則中止啟動，不以不受管理的程序替代。stdio 必須是 pipe，背景模式不向 console 或檔案輸出 PCM。啟動期限 10 秒，仍可能受 Windows 同步呼叫與排程影響；不是硬即時保證。

## 取捨與驗收

增加一個程序啟動與本機傳輸邊界，固定角色共四個：背景程序的擷取、正規化及傳送，加上父程序接收。父程序的通用原始 ring 仍配置但未使用，兩端 ring 樣本配置合計最多 180 KiB；pipe、訊息、stack、metadata 及 Windows 自有緩衝另計。這些是有界資源，未宣稱完整 working set 上限。

背景程序內的 Windows Timer 在程序退出後由 OS 回收；產品不支援直接於常駐父程序反覆建立原生 client。低階原生函式僅供目前背景程序使用，不作通用公開後端。

100 次正式隔離路徑重啟：父程序 handle 維持 214；停止 P50 44.2963 ms、P95 47.5164 ms、最大 59.52 ms；來源確認到第一個 PCM P50 127.1971 ms、P95 174.0568 ms、最大 207.2467 ms。這是當次 Debug、本機、自建靜音來源的結果，不是 STT／翻譯／UI 延遲或 SLA。

合成音 A／B 隔離、切換來源、既有／新增子程序及根程序退出而子程序仍在時停止均通過。一般測試亦驗證背景程序在未啟動擷取時可回收，關閉 Job 可在 stdin 尚未 EOF 時終止背景程序。詳見 [Phase 2 驗收](../phase-2.md)。30 分鐘記憶體／CPU 與兩個播放端點已驗證；實體拔除、服務中斷及完整應用程式矩陣仍待人工。

本 ADR 取代 [ADR 0002](0002-process-loopback-pipeline.md) 的同程序兩 worker 部署方式；程序樹範圍、來源隔離、PCM16、單一正規化、有界佇列、取消與不保存內容的決策繼續有效。

## 2026-10-08 擴充來源確認的程序邊界

反覆重啟時，父程序每次完整建立 Core Audio 來源確認也會使 handle 計數增加；200 次時 216 → 218 → 220，超過原有兩個 handle 的容許量。只移除通知註冊未解決；較長閒置期間會回收部分資源，尚不宣稱找到 Windows 內部所有權缺陷。

最終把最新 Core Audio 列舉、Scope 確認及原生擷取全放進既有 Session helper。父程序仍指定與固定 PID，持續檢查程序存活；helper 必須 fresh-confirm，不能使用舊來源資料或改抓另一個 PID。READY 只在來源確認與 client Start 成功後發布；不存在音訊工作階段的程序被明確拒絕，沒有音訊輸出。

正常流程三批共 300 次均維持父程序 handle 163；沒有放寬門檻或把診斷用的舊快照重用當成正式行為。先前同程序來源確認與停用通知試驗均已移除。來源列表／持續探索的 Phase 1 模式保持獨立；其 native 物件不進入擷取父程序的啟動流程。