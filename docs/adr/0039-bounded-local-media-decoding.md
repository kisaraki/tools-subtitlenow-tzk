# ADR 0039：本機媒體解碼的有界 RAM 與原生資源生命週期

日期：2026-10-12

狀態：採用；Phase 08 第四批。本機 RIFF/WAVE 的獨立解碼基礎，GUI／Prepared ASR／播放及其他容器另列待辦。

Prepared Mode 的外掛字幕準備已能處理整份文件，但未提供無字幕時的音訊抽取。沿用本機 Windows 19045 及已驗證的唯讀輸入邊界，先建立可拉取、可取消且 Session 所有的解碼介面，讓後續 ASR 不需要保存音訊或把離線內容偽裝成即時擷取。

採用 Windows 內建 Media Foundation，從已開啟的 pinned File 建立自訂唯讀 IStream，再透過 MFCreateMFByteStreamOnStream／MFCreateSourceReaderFromByteStream 建立來源。避免將 pathname 再傳入 decoder 而失去已選檔案身分，亦不引入安裝、子程序、輸出檔案或私人檔名／URL metadata。入口先核對固定十二 bytes 的 RIFF／WAVE header，範圍限這個本機容器；一般播放清單、URL、其他容器明確不可用，不能宣稱任意媒體解碼已完成。

IStream 複本共用被 pin 的檔案，但各自保留 logical cursor。所有讀取以 position 再 file 的順序同步，保護整段 seek／read；每次要求最多一 MiB，每六十四 KiB 檢查取消，不以假 EOF 實作容量限制。禁止任何寫入、改大小或輸出複製；Stat 不配置或回傳私人名稱。MF 包裝可能從自己的 MTA 執行緒呼叫 stream，因此 mutable 狀態同步且 COM vtable 捕捉 Rust panic，固定錯誤不包含媒體內容。

decoder 在有界 blocking worker 初始化 COM MTA 與 MFSTARTUP_LITE，不初始化 socket 子系統；所有原生 owner 保留於該執行緒且不具 Send／Sync。只選第一音訊串流，關閉其他串流，協商 PCM16 後驗證實際 rate、channels 及 frame alignment。先限制 native sample 總長至一 MiB，再取得 contiguous buffer；每次 pull 至多交付六十四 KiB 的完整 frames。Lock 成功立即建立 Unlock guard，使用小端 byte pairs 避免指標對齊假設；最後 sample／reader／byte stream／IStream 先釋放，再 MFShutdown 及 CoUninitialize。

DecodedAudioBlock 是不提供 Clone／Debug／Serialize 的 RAM PCM，保留完整 StreamIdentity、checked 序號、實際媒體 timestamp 及 sample-derived duration。輸出保留原生取樣率與聲道，不預設已轉為任何供應商格式。DecodedAudioGate 只保留常數大小的時間與序號 metadata，拒收舊 Session、格式變更、倒序、跳號與實際重疊；前進缺口明確呈現，不補靜音。取整允許一微秒差異，媒體時間上限二十四小時；摘要只保留完整身分、格式與計數，不延長任何 PCM 生命週期。

解碼與既有 Prepared 檔案讀取／解析／本地化使用同一 process-wide CPU lane，以 available_parallelism 為輸入，最多兩個工作並行。工作無等待佇列，過載明確拒收；一份 block 同步交付短且有界的 consumer，釋放後再 pull，不讓整份音訊堆入記憶體。consumer 不得網路、磁碟、UI、阻塞佇列或保留 PCM 複本。後續服務工作須另建有界交付契約，不能把網路等待直接塞入這個 CPU callback。

caller 必須持有並 poll Future；未開始的 Future Drop 亦取消，已開始工作則在本機讀取／MF 呼叫返回後合作取消。工作失敗、consumer 失敗或停止均不回傳部分成功摘要。許可由實際 worker 持有到原生資源銷毀及真正退出，JoinHandle::abort 不作為同步工作已停止的證據。這個邊界不承諾固定取消時間，亦不允許從 UI 或 capture 執行緒直接呼叫同步 decoder。

既有可靠外掛字幕流程仍優先處理文字，沒有因此多跑解碼或 ASR。解碼入口尚未接入 GUI；一般準備視窗仍不擷取或上傳媒體音訊，Ready 只代表本次字幕文件。Prepared ASR 後續需獨立的離線要求、能力與媒體時間契約，不將 received 更新成 Instant::now 規避 Live 新鮮度檢查。沒有新增產品暫存檔、音訊輸出、內容快取、供應商 fallback 或自動播放。

本批驗證範圍只包含合成輸入的原生 PCM WAV、唯讀 IStream 及離線 decoder fixtures。這可以核對真實 MF 接線、PCM／時間、取消及 handle 回收，不能取代使用者媒體、其他 WAVE 編碼、MP3／AAC／影片容器、URL、Prepared ASR、播放同步或雲端品質驗收。有效金鑰維持結案固定介面手動輸入；Phase 09 尚未具備啟動條件。

Microsoft primary 參考：

- [MFCreateMFByteStreamOnStream](https://learn.microsoft.com/en-us/windows/win32/api/mfidl/nf-mfidl-mfcreatemfbytestreamonstream)：以 IStream 建立 MF byte stream。
- [使用 Source Reader 處理媒體](https://learn.microsoft.com/en-us/windows/win32/medfound/processing-media-data-with-the-source-reader)：COM／MF 初始化、byte stream 來源及輸出格式協商。
- [MFStartup](https://learn.microsoft.com/en-us/windows/win32/api/mfapi/nf-mfapi-mfstartup)：MFSTARTUP_LITE 與 NOSOCKET 相同，每次成功初始化須配對 MFShutdown；不能從 MF work queue 執行緒初始化或關閉。
- [IMFSourceReader::ReadSample](https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsourcereader-readsample)：成功但沒有 sample、native timestamp、狀態 flags 與 EOF。
- [IMFMediaBuffer::Lock](https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfmediabuffer-lock)：buffer pointer／長度與 Unlock 配對。
