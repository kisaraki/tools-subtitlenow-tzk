# ADR 0068：有界遠端內容格式前置檢查

日期：2026-10-16。狀態：Phase 08 第三十五批採用。

## 決策與範圍

RemoteHttpMedia 已驗證 HTTP 表示版本及 byte ranges，但尚未檢查媒體內容。新增 `media::remote_probe::probe_remote_audio`，接收已開啟的唯一遠端 owner，以私有 Read／Seek adapter 執行既有 `local_audio_format::probe`。不依 URL、副檔名、伺服器 MIME 或名稱猜測格式，不複製另一套 WAVE／MPEG-1 MP3／AAC-LC ADTS／有限 AAC-LC MP4 gate。

成功只表示內容通過現有格式前置檢查。WAVE 沿用十二-byte RIFF／WAVE 標頭 gate，並非完整 fmt／data 或 codec 驗證；MP3／ADTS 的兩完整相鄰封包與 MP4 原有 metadata／sample tables gate 也不取代真正解碼。沒有 PCM、ASR、文字、Ready、播放或 alignment 證據，GUI 與 Prepared producer／decoder 仍拒收 URL。

## 同一來源與錯誤

成功的 ProbedRemoteAudio 擁有原 RemoteHttpMedia，保留同一 identity、URL、client、強 ETag、total、連線配額及已前進的要求序號。into_media 轉移原 owner，後續依 next_sequence() 繼續；不重新開啟來源、重設序號、建第二個 client、建立本機 pin 或留下 byte cache。metadata accessor 只提供 identity、長度與格式，原始 URL／query、ETag、媒體 bytes 沒有 Debug／Clone／Serialize 或日誌輸出。

每次 read 最多 64 KiB，只讀當次需要的範圍，複製到既有 parser 的受限 buffer 後釋放 RemoteMediaRange。大筆 read_exact 拆成多次有界要求，不預讀或保留完整媒體。seek 僅移動 checked byte cursor，限於 0..=total，不發網路要求；空 read／檔尾回 0，取消與既有 terminal error 仍先檢查。非法 seek、範圍或順序不允許後續操作恢復成成功。

RemoteProbeError 分開保留 RemoteMediaError 與 MediaError。adapter 記住第一個底層錯誤，避免 parser 的通用 I/O 映射或 range 失敗的連帶取消蓋掉 SourceChanged／RateLimited／Network。取消明確 Cancelled，格式不支援明確 UnsupportedMedia；失敗退休來源，不交付部分格式結果或改連其他位置／服務。

## CPU、網路與生命週期

只有持有 RemoteHttpMedia 才能啟動 blocking job；既有全程序兩個 owner 上限也限制此工作的在途數量。每次工作以原 Prepared CPU semaphore 的 try_acquire_owned 取得配額，不新增 CPU pool、每-block 執行緒或應用程式命令佇列。CPU 容量仍由 available_parallelism 決定。

私有 adapter 只在 spawn_blocking 工作使用 Handle::block_on 執行同一 async range future；呼叫端 await 該工作，原 runtime 持續驅動 I/O／timers，current-thread runtime 也採此方式。沒有在 UI、capture 或 async executor worker 同步等待網路。[Tokio Handle::block_on](https://docs.rs/tokio/1.53.2/tokio/runtime/struct.Handle.html#method.block_on) 要求 runtime 保持運作；本函式要求 runtime 在工作退出前存活，沒有目前 runtime 則在新讀取／排程前回 RuntimeUnavailable。

網路等待前釋放 CPU 許可，資料到達後重新 try_acquire，取得後才複製到 parser buffer 及解析；若被其他 Prepared 工作占滿則明確 Overloaded，不同步等待 CPU 或持有 CPU 等網路。HTTP owner／buffer、四 GiB、65536 要求及單要求三十秒等限制沿用 ADR 0067。四份 range payload 的 256 KiB 上限不包含既有 parser 的 metadata：MP4 moov 仍最多一 MiB／工作，其他 box、sample tables 與結構上限不放寬。暫時 parser 資料隨工作釋放，不留於結果或磁碟。

未 poll 的 Future Drop 也取消並釋放來源。正常取消喚醒網路 future，await 真正工作退出後回覆；沒有以取消通知假裝已清理。Future Drop 取消 scope 並 abort 尚未開始的 job；已開始的工作在檢查點合作退出，原 owner／CPU 許可留至實際釋放。worker panic 由 JoinError 映射為 WorkerFailed，RAII 釋放來源與配額。沒有 unsafe、COM 或永久／temp 媒體與文字寫入。

## 驗證與後續

以實際 TCP loopback 和合成結構資料驗證四種既有 gate、錯誤內容、同一來源續讀、版本變更 typed error、大 read 分段、seek／EOF、取消與 Future Drop、CPU 邊界、過載、panic 及 current-thread runtime。合成 MP3／AAC／MP4 bytes 不視為 playable codecs，WAVE 標頭不視為完整檔案；沒有讀取使用者媒體或使用正式金鑰。結果見 [Phase 8](../phase-8.md)。

下一步仍在 Phase 08：把受控遠端來源接入原生解碼，保持 COM 同執行緒、取消／網路等待與 CPU 配額邊界，再完成 EOF、ASR／Ready 與 URL GUI 的目的地揭露。五語 Auto、真實媒體／雲端與播放定位仍待驗收。Phase 09 尚未具備條件，金鑰仍在結案固定遮蔽介面手動輸入。
