# ADR 0015 — 本機 Chromaprint 與可回收的 CPU worker

日期：2026-10-09。狀態：Phase 6 第二批採用；實際歌曲目錄與 Live／GUI 音訊分流尚待接線。

指紋產生是 CPU 工作，且產生指紋不等於已識別歌曲。直接在 Tokio future 計算或每視窗 spawn_blocking，可能讓取消後仍有未回收工作；讓每個 oneshot 保有完整結果，則慢消費端可以累積許多指紋。

採 rusty-chromaprint 0.3.0 的固定 test2 configuration，壓縮結果明確標成 ChromaprintTest2Compressed；generic opaque 格式不假裝可送至這種目錄。Fingerprinter 不支援跨執行緒移動，因此每次在 CPU worker 內建立、使用及銷毀，不以 unsafe 強制 Send／Sync。不使用原生 DLL、音訊檔或外部命令列解碼器。程式庫與範圍限制見 [套件說明](https://github.com/darksv/rusty-chromaprint) 與 [Fingerprinter API](https://docs.rs/rusty-chromaprint/0.3.0/rusty_chromaprint/struct.Fingerprinter.html)。

同一 Live Session 只有一個 resolver 要求，CPU budget 以 available_parallelism 為輸入、受單一 in-flight 約束限制為一個固定 worker。pending queue 容量一，新視窗取消 active／淘汰 queued 工作。每份工作有自己的 token，drop await future 會取消該份工作；stop／owner Drop 取消 active、清除 queue／completed 並 join。提交與停止、提交與結果發布共用短 metadata lock，避免停止後又入列或被取代後發布成功。

completed 指紋只存在一個私有欄位。oneshot 回覆只含完成／錯誤訊號，取結果時重新檢查最新 sequence、Session、age；保留許多未讀回覆不會各自保留一份指紋。取用移交所有權，drop await future 清除尚未取用的同份結果。payload、PCM 與指紋不提供 Debug／Serialize，也沒有檔案／日誌出口。

每 100 ms 的輸入片段之間檢查取消，但程式庫內部還有 FFT／resampling batching，不能把片段長度當成取消時間 SLA。RMS gate 只排除近乎靜音資料，不是 Speech／Music 分類器。第三方 panic 轉為 typed WorkerPanicked，並在 owner stop 回收執行緒；核心不轉送 panic payload 作為服務錯誤。本機合成測試的預設 Rust panic hook 仍會印固定測試訊息。

CatalogIdentifier 串接本機 CPU job 與呼叫端明確提供的 FingerprintCatalog。它只接受支援固定格式／取消的目錄，輸出普通 resolver evidence，仍經 candidate tracker，不直接採用歌詞或操作 UI。沒有預設網路目錄，不把 STT 的 OpenAI／Gemini 選擇當成額外服務授權；本批以 mock 目錄驗證這段流程，不能宣稱實際歌曲識別／目錄相容性已通過。
