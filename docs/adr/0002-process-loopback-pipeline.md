# ADR 0002：程序樹擷取與有界記憶體管線

日期：2026-10-05。狀態：採用；組建門檻決策由 [ADR 0003](0003-runtime-capture-capability.md) 取代。19045 本機核心驗收通過；執行程序部署由 [ADR 0004](0004-session-owned-capture-process.md) 補充。範圍：Phase 2。

## 決策與原因

使用 `ActivateAudioInterfaceAsync`、`VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK` 與 `PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`。範圍固定為明確指定的 PID 與子程序，跨播放裝置；不使用排除目標模式、System Mix、麥克風或視窗掛鉤。這符合 SPEC 的應用程式隔離與來源透明要求。

先開啟只含 query／synchronize 權限的程序 handle，在 Session helper 以最新探索快照確認獨立程序音訊來源；擷取父程序不建立 Core Audio 確認物件。handle 在工作階段中持續持有，防止 PID 被其他程序重用。程序結束直接停止，即使還有子程序存活也不繼續擷取。來源切換是停止並 join 舊工作，再建立全新 SessionId 與緩衝區。

工作階段背景程序內的擷取與正規化使用兩個具有固定責任的專用 worker；另有 pipe 傳送及父程序接收角色，詳見 ADR 0004。這不是依 CPU 數量擴張的計算 pool，也沒有每區塊建立執行緒。Tokio 只處理狀態、計時及取消。Windows COM 和 unsafe 封裝於 `win32`，通用緩衝、順序與格式轉換位於 `audio`，沒有 AI 供應商依賴。

使用音訊引擎提供的 PCM16、48 kHz、立體聲格式轉換，再於正規化 worker 轉成單聲道 f32。保持 48 kHz 可避免尚未確定 STT 需求就選定有損降採樣策略；未來供應商需要其他採樣率時，在預處理邊界加入明確的濾波與重採樣。

兩個預先配置的 `crossbeam_queue::ArrayQueue` 各 12 格，每格最多 960 frames（20 ms）。擷取熱路徑只檢查取消、加時間戳、複製 PCM 並推入 ring，不做配置、日誌、磁碟、網路或 UI 操作。滿載以 `force_push` 淘汰最舊格；原生緩衝、正規化前與消費端均會淘汰超過 250 ms 的資料，並保留序號缺口。此為有界延遲策略，不是硬即時 SLA。

每區塊帶 SessionId、序號、frame position、QPC 時間戳與單調時間。Windows 回報 timestamp error 時，frame position／QPC 均標示未知，設置 discontinuity；不猜造可靠時間軸。立體聲平均轉成 [-1, 1) 的單聲道，無自動增益或音量放大。

## COM、取消與失效

原生 worker 建立 MTA，COM 物件於同執行緒釋放。Windows 完成回呼使用 free-threaded marshaler；透過有界通道傳回 owning activation operation，在同一 MTA 的擷取 worker 呼叫 GetActivateResult 取得 IAudioClient。借用啟動參數的 PROPVARIANT 外層使用 ManuallyDrop，避免自動清除由 Box 擁有的記憶體。啟動參數由回呼持有的 Box 保留到 Windows 釋放回呼，避免取消後存取失效記憶體。

Windows 啟動 API 沒有取消方法。應用程式每 20 ms 檢查取消，10 秒啟動逾時後退出；晚到回呼只釋放結果，不啟動擷取。全程序最多一個待完成啟動要求，Windows 尚未釋放舊回呼時，新要求回報 `ActivationPending`，避免未完成要求無限累積。工作階段擁有的兩個 worker 均 join；系統持有的晚到回呼可能晚於工作階段停止才釋放。

執行中同時等待程序結束與樣本事件，最長 20 ms 回到取消檢查；事件沒有通知時仍查詢封包，讓服務或裝置失效可以回報。每輪最多處理 32 個封包，單一原生封包接受上限為一秒；不依賴虛擬 client 的 GetBufferSize 回傳值，也不宣稱 Windows 引擎總緩衝有此上限。`GetBuffer`／`ReleaseBuffer` 在同一 worker 配對，錯誤或 unwind 亦由 guard 釋放。停止與 Drop 清空兩個 ring，不遺留本程式的 worker。Windows 同步呼叫本身不能被強制中斷，停止延遲仍需實機量測。

擷取或正規化 worker 發生 panic 時，由退出 guard 設定取消並發布 `Stopped(WorkerPanicked)`，讓另一個 worker 停止。錯誤終止狀態不會被晚到的啟動通知或正常結束覆寫；停止仍 join 兩個 worker。即使第二個 worker 隨後也失敗，`stop()` 及重複停止仍回傳已發布的第一個錯誤。這避免正規化 worker 已失效、擷取端卻繼續運行及顯示 Running 的情況。這個處理依賴 Rust 預設的 panic unwind；程序中止型錯誤不在可恢復範圍。

## 平台限制

最初依 Microsoft 文件列出的最低支援組建 20348 加入硬性版本檢查。後續查核 OBS 官方指南與實作，發現有 Windows 10 2004 以上使用同一 Process Loopback API 的證據；因此更正先前「本機 19045 不支援」的結論。本機測試只觸發本程式的門檻，沒有嘗試原生擷取。

新的平台決策見 [ADR 0003](0003-runtime-capture-capability.md)：以本機 19045 為驗收基準，改由實際 API 結果判斷，保留程序隔離、COM 壽命、有界管線與取消設計。門檻已移除，19045.6456 的真實 PCM、程序樹隔離及停止／重啟已通過。本機原生 client 反覆建立仍留下 Timer，故正式後端採工作階段專用背景程序回收資源；早期 100 次父程序 handle 維持 214；後續把來源確認亦移入 helper，最終三批共 300 次維持 163。詳細結果與未測範圍見 [Phase 2](../phase-2.md)。

依據：[官方範例](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)、[Activation 參數](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_activation_params)、[非同步啟動與回呼壽命](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-activateaudiointerfaceasync)、[GetBuffer 時間戳與生命週期](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)。
