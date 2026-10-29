# ADR 0016 — 獨立音訊分流與不會遺失的缺口重設

日期：2026-10-09。狀態：Phase 6 第三批核心採用；正式 GUI／服務組合尚待接線。

內容辨識需要 16 kHz 連續滑動視窗，STT 的既有預處理則依服務使用 16 或 24 kHz。核心不依賴某個 STT adapter 的私有輸出，也不建立第二條辨識或翻譯流程。orchestration 先將原始 NormalizedAudio 送入 SpeechInput；只有明確提供 LiveResolver 時，才將固定大小的 RAM 副本交給獨立的內容辨識分支。STT 拒收不提交 resolver；resolver 拒收不取消 STT。

重採樣與組窗由單一固定 CPU lane 執行，available_parallelism 作為 budget 輸入，上限一。佇列容量十二個 20 ms block，沿用 240 ms budget；滿載時清除 backlog、保留最新 block，重設 FIR／視窗並清除舊候選，而不是拼接缺少的音訊或堆積延遲。短 metadata gate 序列化提交、重設、發布與停止；FIR／視窗 PCM 建立、服務要求與 await 都不持有 gate。這個入口只供 orchestration 呼叫，不放入 WASAPI callback。

缺口必須在下一個十二秒視窗形成前取消舊查詢。watch mailbox 只保留最新資料，單獨的 Reset 訊息可能被後續視窗取代，因此 audio mailbox 將 reset epoch 持續附在每個結果上；ResolverSession 也將自己的 epoch 附於輸入與輸出。owner 先傳遞 epoch 改變，再提交視窗；即使中間多次通知合併，舊票仍會清除。epoch 是同一 Session 內的音訊連續性版本，不取代 Session／generation／sequence／來源時間。

正在計算的舊 block 若完成時 epoch 已改變，不得發布。ResolverSession 在 invalidate 後立即過濾舊 epoch 的未讀結果，task 下一次被 poll 時取消要求並清除投票；音訊停止期間也清除 CPU 本地視窗／FIR。owner 在取用結果前再查看 audio epoch，處理 worker 與 async select 的發布競爭。缺口通知引用上次實際視窗時間，不捏造新的音訊時間或來源 offset；沒有前次視窗時不製造辨識結果。

LiveResolver 持有 caller-polled provider future、ResolverSession 與音訊 worker。changed() 要由 Live owner 與 STT／翻譯共同 poll；不 spawn／detach 網路工作。stop／Drop 取消 resolver、drop provider future、清除 pending／latest 音訊、unpark 並 join CPU lane。CPU panic 成為固定 WorkerPanicked，不能當成功結果；stop 與提交使用同一 gate，殘留 input handle 也不能在停止後排入 PCM。

音訊、指紋與內容只在 Session RAM，無寫檔／快取／序列化／輸出日誌。metrics 只包含佇列深度、處理／丟棄 block 數、reset 數與 worker 數，未新增 telemetry 或 GUI 預設啟用。

目前不直接啟用 AcoustID Live lookup。[官方介面](https://acoustid.org/webservice) 要求 duration 是整首音訊的秒數；十二秒滑動視窗不是整首歌曲，來源時長未知時不能假裝符合契約。這個限制只影響後續 adapter 的選擇／輸入設計，沒有改變現有 STT／翻譯服務，也沒有向新目錄傳送指紋或索取正式金鑰。
