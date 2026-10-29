# ADR 0006 — 所選供應商的有界串流與重建

日期：2026-10-09。狀態：採用；真實連續辨識／翻譯驗收依 Phase 11 手動流程進行。

2026-10-24 後續人工仍回報傳送佇列滿，Gemini 改採連續轉錄，取代本 ADR 原人工斷句及 Final 屏障；未知精確擷取時間不作來源定位，詳見 [ADR 0091](0091-gemini-continuous-transcription.md) 與 [Phase 11 診斷](../phase-11-gemini-diagnostics.md)。OpenAI 與共用安全限制保持。

Phase 3 使用者指定 OpenAI／Gemini 可選，且禁止永久保存音訊、跨供應商靜默 fallback 與舊 Session 結果污染。既有 Process Loopback 仍維持獨立 helper；組合層只把 RAM 正規化區塊交給 SpeechRecognizer。

採用單一持續 WSS、TLS 系統根憑證驗證、一個有界 CPU 預處理 worker，以及固定端點與安全憑證管理員。握手先於擷取，以免連線延遲累積舊音訊。網路失敗與滿載都先停止舊工作、清空緩衝，再以新 Session 重建同一服務；重試上限三次，服務太快要求重建也受相同限制。

OpenAI 以 committed acknowledgement／item_id 關聯已提交音訊；Gemini 依 ADR 0091 持續送 PCM，由服務自動斷句，回覆段落識別只表示文字排序。缺少可靠擷取關係時時間保持 None，不以回覆時間或任意 PCM 序號偽造證據，也不繞過精確驗證／仲裁。移除 Final 屏障不增加音訊容量或年齡，未知時間仍可顯示即時文字並翻譯，實際品質與停止待人工驗收。

OpenAI 的客戶端能量端點策略使用 400 ms 靜音與 4 秒上限；它只分段，不聲稱是語意 VAD，也不承諾辨識延遲。Gemini 使用服務自動斷句。Auto 五語提示與回報檢查、未知 Partial 暫存、未知／不支援 Final 拒絕保持。

選擇互動式終端機顯示當前 Session 的辨識修訂，正式命令拒絕 stdout 內容重導向。Phase 4 翻譯與 Phase 5 UI 不在本次範圍。詳細限制與驗收見 [Phase 3](../phase-3.md)。
