# ADR 0041 — 有界離線語音辨識與辨識來源保留

日期：2026-10-12  
狀態：採用於 Phase 08 第六批；native producer、正式服務與 GUI ASR 尚未接入。

## 問題

本機解碼與前處理已產生有界、原生媒體時間的 16／24 kHz 單聲道 PCM。現有 Live STT 以 capture stamp、牆鐘 freshness、partial／final 串流及 provider 連線生命週期運作，不能把整份離線媒體裝成新的 Live capture 來繞過音訊年齡。Prepared 需要在有限的記憶體及雲端並行下保留 overlap context、還原完成順序，並對跨分段的句子保持真正時間。

公開 decode_local_audio／prepare_local_audio callback 只能借用當前 PCM 做短、有界 CPU 工作，不得保留音訊複本、連網或等待佇列。如果為接辨識而放寬 callback，native COM worker 可能等待 provider、失去背壓規則或累積整部媒體；目前的 callback 也不是可交付所有權的 demand-driven producer。

ASR 產生的是辨識來源，沒有 ExistingSubtitle 的出版物身分或配對證據；合併成功也不能直接代表 zh-TW 翻譯、完整 Ready 或實際播放定位。用同一來源型別代替會誤導後續字幕優先序及仲裁。

## 決策

新增獨立 portable PreparedSpeechRecognizer 契約與 caller-owned PreparedRecognitionSession；正式 adapter 必須由 caller 明確選定，不從憑證或 provider 名稱猜測能力，不提供假 production 成功或跨供應商 fallback。能力明列取消、原生定稿時間、英／日／法／德／韓、限制五語的 Auto、16／24 kHz 及一至四個並行要求。Live 的 audio age、capture stamp、重連與翻譯 timeout 不變。

PreparedSegmentBuilder 消費 owned DecodedAudioBlock，每個連續 run 的正文為十秒半開區間，左右各最多五百毫秒真實音訊 context；單份 owned chunk 最多十一秒。首段沒有左 overlap，EOF／gap 不補音訊。Working buffer 最多十一秒、ready 最多兩份，另有當前六十四 KiB input；未排空 ready 時 push 明確 terminal Overloaded。EOF／gap 可留下完整正文及短尾兩份，finish 超限也拒收並清除，不靜默丟掉正文。主要 PCM buffer 移交 chunk，只複製下一份所需的有界 overlap／真實後續片段，沒有整部媒體音訊容器。

每個 block 核對完整 Session／generation、checked 序號、rate／mono 格式、原生時間與二十四小時界限；每個 run 使用原生起點與累積 frames 的 checked clock，只容許一微秒取整差。Discontinuity 必須描述 gate 的實際 forward-gap，不能用假 reset 掩蓋累積漂移；首 run 第一份 false，真實 gap 先完成舊 run 再清除 overlap，新的 checked run ID 僅第一份 chunk 為 true。分段不使用 Instant 或 Live capture stamp。

Request 帶 chunk 正文範圍、明確來源語言及最多八筆／六十四 KiB 已提交前文。Response 只接受真正原生定稿時間，單份最多 512 cues／128 KiB、單句最多 8192 bytes；核對完整身分、正長度、有序、音訊範圍及語言。每句須完整位於唯一正文；跨界句回報 BoundaryMismatch，沒有 word timing 時不能裁句、改時間或猜測去重。Overlap 僅提供語音 context，不授權重複正文。

PreparedTranscriptMerger 最多四筆 pending metadata／response slot，不留 request PCM；倒序完成只有在前項到齊後按序提交。同 run 正文連續，跨 run 有 checked 下一份 ID、真實 gap 且沒有前 run overlap。切新 run 前及整份 finish 時，前／最後 chunk 的 audio_end_us 必須等於 body_end_us；最後真實音訊須已有正文，不得將尚未交付正文的右 context 視為完整 EOF，不符就 typed error，沒有 partial transcript。提交的原生時間全程不倒序或重疊，全文最多 8192 cues／四 MiB；context 只來自同 run 已提交的八筆／六十四 KiB，未提交結果及前 run 不入 context。Auto 非空結果若跨來源語言，整份回報 MixedLanguages；無語音回報 NoSpeech，不以空結果猜測語言。

Async runner 每批最多四份 owned chunk，按所選 provider 能力啟動一至四個要求，完整批次結束才接下一批；單一要求使用六十秒 offline timeout。呼叫端 owns／polls Future，不建立 detached provider 工作、每句 OS thread 或整份 cue 數量的待執行 queue。Provider adapter 仍須提供自身跨 caller 共用的並行／速率限制並遵守取消；宣告能力不代表本批已有正式雲端 adapter。

取消、錯誤、timeout、provider panic、晚到或不符身分／時間、超量結果及 Future Drop，停止後續排程並取消已開始要求，釋放 PCM 與累積本文，session 成為 terminal，不交付部分 transcript。未 poll 的 batch Future Drop 亦持有 owner 的取消 guard。完整 EOF 及全部結果有效後才交付獨立 PreparedTranscript／PreparedTranscriptCue，明確保留 ASR 來源，不包裝成 ExistingSubtitle、Ready 或播放定位證據；本文只在本次 RAM，沒有 Debug、Clone、Serialize、匯出或快取。

容量上限須包含已保留的配置，不只 Vec 的可見長度。PCM 與前文 owner 使用 boxed slices，回應及 batch 移除多餘 reserved capacity；超過四份的 input batch 在建立 Future 前立即釋放，不等待首次 poll，避免未 poll Future 仍保留超量音訊。

## 結果與限制

本批只提供可由明確合成 provider 測試的分段、能力核對、並行／取消與時間合併核心。沒有新增 crate、unsafe、音訊落盤、正式服務、金鑰讀取、GUI 音訊接線或播放。現有外掛字幕優先路徑不額外解碼；一般 GUI 無字幕仍回報 AsrUnavailable，不能把 library 或 mock 合併成功列為檔案 ASR 可用。

公開 decode／preprocess consumer 的原契約保留。後續接線需另建私有、依需求拉取的 native actor，使 PCM 所有權移交有明確容量與 backpressure，並保留 COM 執行緒、解碼 CPU 許可、EOF、cancel 及真正退出生命週期。此 actor 尚未實作，本批不以 callback 留存或等待網路替代。

跨界完整句無法在缺少 word timing 下可靠合併，本批採明確失敗而非做 fuzzy 裁切；後續 adapter 必須交付正文內的原生 final。PreparedTranscript 仍需全文脈絡／glossary、翻譯、Consistency Pass 及 timeline validation 才能成為 Ready，播放定位需另有實際證據。

測試涵蓋合成 PCM 分片、overlap／正文唯一、長串流容量、EOF／gap、時鐘漂移、Session、取消、倒序及 typed errors；實際執行與數字另記於 [Phase 8](../phase-8.md)。正式媒體／雲端品質、檔案 producer、ASR adapter、GUI、其他容器、URL 與播放均未據此驗收。Phase 09 尚未具備啟動條件，真實 OpenAI／Gemini 金鑰仍留到結案由使用者在固定遮蔽介面手動輸入。
