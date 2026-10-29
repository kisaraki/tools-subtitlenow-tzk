# ADR 0042 — 本機離線辨識的原生拉取與資源存活

日期：2026-10-12  
狀態：採用於 Phase 08 第七批；正式辨識 adapter、翻譯／Ready 與 GUI 尚待接續。

## 問題

本機 RIFF/WAVE 解碼、前處理及離線辨識核心已具備。舊公開 callback 只借用當前音訊，不能為接雲端而複製留下 PCM 或等待佇列；整份解碼持有的 CPU 許可也不適合涵蓋漫長的服務等待。原生 COM／MF 物件必須固定在同一工作執行緒，取消 Future 不代表原生呼叫或銷毀已結束。

## 決策

新增私有 PreparedLocalSource。建構與未 poll Future 不開檔；首 pull 以 try_acquire 取得共用 CPU 與 native-owner 許可後才排入 blocking worker。原生解碼器、FIR、分段器只在此 worker 內建立與使用，沒有 COM 物件跨執行緒。每份 Pull 命令攜帶 CPU 許可與 oneshot 回覆，命令通道容量一且沒有 PCM；可變借用確保只有一個未完成 pull。

work::native_owners 使用 available_parallelism，最多兩個存活的原生解碼 owner。新 actor 與舊正式 decode_local_audio／prepare_local_audio factory 共用此池；舊 factory 以 OwnedDecoder 持有許可，inner 解碼器先銷毀，最後釋放許可。mock factory 的隔離 lane 保持測試用途，不把容量測試混入真正全域池。

CPU 許可只包住本次 open／decode／FIR／分段，以及正常 EOF／錯誤時的銷毀；送回結果前釋放。actor 閒置時只等待命令或取消，保留 COM／唯讀檔案 pin／有界 DSP 狀態及 native-owner 許可，不繼續讀檔。閒置取消的同執行緒 RAII 清理必須可直接進行，不因 CPU 過載拒絕釋放；此清理仍受 native-owner 上限約束。

Idle 段在 blocking worker 使用捕獲的 Handle.block_on，僅 select 有界 channel 與 watch cancellation，沒有網路、timer 或 relay task。這些等待不依賴 current-thread runtime 的 I/O／timer driver；相關限制見 [Tokio Handle::block_on](https://docs.rs/tokio/1.53.2/tokio/runtime/struct.Handle.html#method.block_on)。外部取消可直接喚醒閒置 actor，不能只等 blocking_recv。

私有 push_owned／finish_owned 直接移交原有 DecodedAudioBlock，不複製借用 PCM。公開 push／finish 仍只暫借音訊，原契約不放寬。每次 owned output 立即 push 分段器並排空 ready 到本次 reply，最多兩份 chunk；完整處理當前 native block 後停止拉取。六十四 KiB block 在最低 8 kHz 單聲道最多 4.096 秒，gap／EOF 仍可產生最多兩份尾段；超過容量明確失敗，沒有靜默丟棄。

EOF 先完成 FIR flush、分段器 finish 及正文尾端閉合，再銷毀 native core、釋放 CPU 許可、送回 eof。caller await 真正 worker Join 後才接受 eof 回覆；不是把一筆 queued reply 當作已回收。取消與未 poll／已開始 Future Drop 會取消 scope、關 command sender 並 abort queued worker；已開始 worker 必須合作停止，許可持到真正退出。[Tokio spawn_blocking](https://docs.rs/tokio/1.53.2/tokio/task/fn.spawn_blocking.html) 的 abort 不等於已開始工作結束。

公開 recognize_local_media 要求 caller 明確提供 PreparedSpeechRecognizer；取消／Session／capability 核對先於檔案 I/O。只有 batch 尚有兩份空間時才 pull，累積三或四份即等待辨識，維持每批最多四份 PCM；沒有服務等待期間的背景前讀。所有結果仍交付單一 PreparedRecognitionSession，只有完整 EOF、時間／身分／語言核對及有序合併成功才回傳獨立 PreparedTranscript。

## 結果與限制

本批將真正本機 WAVE 路徑接到可由 caller 提供的離線 recognizer，測試辨識仍是明確 mock，沒有假 production provider 或隱含 fallback。未新增 crate、unsafe、安裝、音訊落盤、快取、金鑰讀取、真正雲端、播放或 GUI ASR。

音訊與本文僅在 RAM。native 檔案 pin 在 EOF 實際回收時釋放；回傳的 transcript 不是保留媒體的 Prepared Ready，也不是字幕／媒體配對或播放定位證據。原外掛字幕 Ready 流程仍保留其既有唯讀媒體 owner，不額外解碼。

句子跨正文仍回報 BoundaryMismatch，不猜測 word timing。正式 ASR adapter、全文脈絡／glossary、翻譯／一致性、Ready／GUI、其他影音格式、URL 與播放定位仍待接續。Phase 09 尚未具備啟動條件；真實金鑰維持結案固定遮蔽介面手動輸入。實際測試與檢查記錄於 [Phase 8](../phase-8.md)。
