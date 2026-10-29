# ADR 0003：以實際 API 結果判定程序擷取能力

日期：2026-10-05。狀態：已實作，19045.6456 本機核心驗收通過；30 分鐘與 300 次重啟已通過；完整應用程式矩陣仍待人工。更新：2026-10-08。範圍：Phase 2。

## 問題與證據

使用者已確認以目前本機 Windows 10 22H2、組建 19045 為基準。登錄資訊為 19045.6456、Education；`RtlGetVersion`／作業系統版本顯示的組建與累積更新修訂號須分別記錄。

原實作在呼叫 Process Loopback API 前，以 `MIN_PROCESS_LOOPBACK_BUILD = 20348` 拒絕低組建。先前實機測試的 `UnsupportedWindows` 是本程式產生的結果，不能證明 API 不可用。

[Microsoft 參數文件](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_activation_params) 仍將 20348 列為最低支援組建。[OBS 官方指南](https://obsproject.com/kb/application-audio-capture-guide) 則列 Windows 10 2004 以上可使用應用程式音訊擷取；[OBS 28.0.0 的註冊邏輯](https://github.com/obsproject/obs-studio/blob/28.0.0/plugins/win-wasapi/plugin-main.cpp) 從組建 19041 啟用程序音訊來源，且註明與 Microsoft 門檻的差異。[同版本的擷取實作](https://github.com/obsproject/obs-studio/blob/28.0.0/plugins/win-wasapi/win-wasapi.cpp) 使用 `ActivateAudioInterfaceAsync` 與包含目標程序樹的參數。

推論：19045 值得直接驗證同一原生 API，不能僅憑文件門檻宣告不可用。OBS 的相容性不是本專案的測試結果，也不代表 Microsoft 對較低組建的支援承諾。

## 決策

1. 以 Windows 10 22H2、19045、x86_64 為主要開發與擷取驗收基準；先測本機 19045.6456。Windows 11 列入延伸驗證，不推廣為所有 Windows 10 組建皆支援。
2. 移除僅依 `< 20348` 拒絕擷取的邏輯；不以另一個未經本專案驗證的組建數字取代。版本讀取只供診斷，讀不到時標示未知；實際擷取錯誤由原生呼叫回報。
3. 能力檢查與使用者選定來源的正式啟動共用一次流程，依序確認 activation、初始化、Start、PCM 接收。不在啟動程式或列舉來源時暗中試抓音訊，也不先開一條探測串流再開正式串流。
4. 保留所選 PID 與子程序的擷取範圍、既有來源確認及生命週期。某來源的拒絕存取、消失、格式問題、無聲或服務失效，不應轉成整個作業系統永久不支援。
5. 保留啟動／完成回呼的錯誤階段及 HRESULT、取消與單一待完成要求上限。沒有聲音與初始化失敗分開呈現；Start 成功不等於收到有效音訊，更不等於跨程序隔離驗收通過。
6. 維持 [ADR 0002](0002-process-loopback-pipeline.md) 的 MTA、RAII、有界佇列、時間戳、工作階段隔離及記憶體限定政策。若實機證據要求調整格式或旗標，先針對該錯誤設計最小修正與測試。

## 替代方案與取捨

| 方案 | 判斷 |
| --- | --- |
| 保留 20348 門檻，要求升級 Windows | 與使用者的 19045 基準不符，也未反映 OBS 的實作證據 |
| 只把門檻改為 19041 | 能避開目前阻擋，但版本通過仍不能證明初始化、格式與隔離正確 |
| 一般 endpoint loopback | 擷取播放端點混音，不能取代程序隔離；不作自動相容方案 |
| 虛擬音效線、注入或自製驅動程式 | 增加安裝、路由或安全邊界，不納入本次設計 |
| 實際 API 啟動與分層驗收 | 採用；以可重現的本機結果決定支援範圍 |

一般 loopback 的混音範圍依 [Microsoft Loopback Recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)。探索到只有一個活動 Session 或把其他來源靜音，都不能當成未選來源永遠不會進入混音的隔離保證。

代價是低於 Microsoft 文件支援門檻的行為需由本專案持續驗證；任何相容性標示都應註明實際組建、修訂號及測試範圍。若本機失敗，記錄具體階段後修正或明確回報限制，不能宣稱已完成、偷偷擴大擷取範圍，或跳到下一 Phase 掩蓋缺口。

## 驗收

實作與驗收順序見 [Windows 10 19045 擷取設計](../windows-10-capture-plan.md)。本 ADR 只取代 ADR 0002 的平台門檻決策；其餘管線設計繼續有效。目前已移除硬性門檻，完成本機原生 API、非靜音、程序樹隔離及來源退出測試；最終 300 次正式重啟與 30 分鐘連續擷取的資源驗證亦通過。背景程序資源隔離見 [ADR 0004](0004-session-owned-capture-process.md)，完整紀錄與未測項目見 [Phase 2](../phase-2.md)。
