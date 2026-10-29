# ADR 0007 — 翻譯修訂與單一時間軸

日期：2026-10-09。狀態：Phase 4 共用核心及正式翻譯 adapter 已接入；真實服務驗收待結案。

STT Partial 會回退，平行要求會亂序完成，High 可能需要第二次校訂。若只按回應時間追加翻譯，會顯示舊文字或多次覆寫已提交內容；不同 renderer 各自處理也會重複要求。

採用單一 TranscriptTimeline，以 Session／generation、utterance、來源修訂、Draft／Review 與串流修訂驗證。較新的來源取消舊工作；結果只能套用當前來源修訂，Final 是終止狀態，commit_prefix 必須符合 UTF-8 邊界且後續不可覆寫。High 的校訂保留 Draft 顯示至完整 Review，以減少閃爍。

品質以 debounce、上下文上限與有條件 Review 實際改變處理。來源、上下文與 SessionGlossary 都是 RAM 資料，不作為 provider 指令；固定臺灣繁中指南由 adapter 分開送出。能力限制與 Semaphore 控制 1–4 個要求，64 格資料與 3 秒來源修訂總期限使記憶體及延遲有界。過期／失敗明確停止，不能假裝保留完整翻譯。

呼叫端必須持有與 poll 每個 job、消費更新並檢查取消原因；核心不建立背景孤兒工作。正式 app 已透過 LiveTranslation 接線，服務驗收仍待完成，不能以 mocks 宣稱真實翻譯或 zh-TW 品質。

固定前綴只能在目前來源修訂的翻譯完成且結果已套用後提交。待排程、執行中或完成事件尚在佇列時回報 PrefixBusy，保持原有前綴及 Session，不取消翻譯。理由是每個已配置的 job 持有不可變前綴快照；執行途中修改 timeline 的前綴會使其合法的下一筆結果變成不相容。確認前綴是呼叫端的語意決策，核心不將重複出現的文字自動判定為穩定。後續來源修訂的要求會攜帶已提交前綴；取消後拒絕再變更。
