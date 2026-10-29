# ADR 0047 — 有界 MP3 原生解碼與 Prepared 時鐘限制

日期：2026-10-13  
狀態：採用於 Phase 08 第十二批；MP3 到 Prepared DSP／ASR 的時間映射仍待完成。

## 問題

目前本機音訊入口只接受 RIFF/WAVE，無法逐步驗證壓縮音訊。直接讓 Media Foundation 猜測任意檔案可能啟用未支援格式；依副檔名或私人來源路徑選 handler 也不足以證明格式。已有的 DSP／分段契約則要求準確的 PCM frame 時鐘，不能假設 MP3 的封包時間與解碼後取樣數具有同樣的精度。

## 決策

增加純 Rust 的有界內容門檻，仍使用已開啟、唯讀 pinned 檔案的同一 file object。接受原有 RIFF/WAVE，或 MPEG-1 Layer III：32／44.1／48 kHz、單聲道／立體聲。MP3 必須具有兩個完整相鄰 frame，取樣率與聲道一致；bitrate／padding 可變，不搜尋任意 junk 中的 sync。Free format、MPEG-2／2.5、其他 layer、保留欄位、playlist、AAC／MP4 及未知格式明確拒收。

可選 ID3v2.2／2.3／2.4 前綴只核對版本、flags、synchsafe 長度及可選 v2.4 footer，一 MiB 上限包含 header／footer。v2.2 compression 不支援；不解析或保存 tag 文字／圖片，不將未知 ID3 當作 MP3 證據。最多讀取 31 bytes 及固定次數 seek，成功回復 cursor 到零；取消在每次 I/O 前後檢查，重複 Interrupted 有限，不建立全檔案或 metadata buffer。此門檻不保證完整音訊有效，真正格式及 PCM 仍由原生 decoder 驗證。ID3 長度／footer 依 [ID3v2.4 structure](https://id3.org/id3v2.4.0-structure) 與 [ID3v2.3 specification](https://id3.org/id3v2.3.0)。

原生 reader 接收 trusted audio/wav 或 audio/mpeg MIME，仍從 IStream／IMFByteStream 建立，沒有 URL、私人 filename 或 origin attribute。沿用第一音訊串流、PCM16 協商、原生格式／時間、有界 native sample／64 KiB block、同執行緒 COM／MF 及 cancellation／RAII。公開 decode_local_audio 使用 native 時間；它只交付借用 PCM callback 與 metadata summary，不是字幕 Ready 或播放定位。Windows 格式與 API 依 [supported media formats](https://learn.microsoft.com/en-us/windows/win32/medfound/supported-media-formats-in-media-foundation)，沒有採用已不受支援的 MP3 byte-stream plugin API。

本機合成 MP3 實測顯示，MF 的 PCM 初段移除 decoder delay，但後續封包保留 native timestamp，留下約 11–16.5 ms 的回報缺口。44.1 kHz 的 MPEG frame duration 另以 100 ns 單位獨立取整；兩秒測試已觸發既有 DSP 的累積漂移檢查。不能透過放寬所有來源的精度、補靜音或重設 origin 隱藏這個差異。壓縮檔原生時間與取樣時鐘的明確映射需另行設計。

因此解碼明確區分 Native 與 SampleAccurate 要求。原生 library 可解碼 MP3；prepare_local_audio、私有 Prepared producer／ASR 仍要求原有 sample clock，內容門檻確認 MP3 後、MF／PCM／服務要求前回報 UnsupportedAudioClock。GUI 顯示安全 typed 錯誤；不將 MP3 上傳一部分後才失敗，也不改用另一 decoder 或服務。已有字幕仍不需要解碼或此時鐘，因此優先流程保持可用。

## 驗證與限制

測試專用 encoder 以合成 440 Hz PCM、Windows MP3 sink／encoder 及 HGLOBAL RAM stream 產生真正 MP3；固定 sink 的既有串流先配置再建立 writer，Finalize／釋放 writer／Shutdown sink 的順序明確。只有 cfg(test) 具備 writable stream／編碼 helper；產品沒有錄音、輸出檔、保存或快取。API 依 [MP3 audio encoder](https://learn.microsoft.com/en-us/windows/win32/medfound/mp3-audio-encoder)、[MP3 sink](https://learn.microsoft.com/en-us/windows/win32/api/mfidl/nf-mfidl-mfcreatemp3mediasink) 及 [sink writer lifetime](https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-mfcreatesinkwriterfrommediasink)。測試合成輸入檔使用自有 RAII temp，與產品暫存或 captured media 無關。

取樣數、RMS／頻率、native gap、完整壓縮 frame 的結束時間與每 frame 最多一個 100 ns 單位的量測容差分別驗證，不用原始 PCM 長度冒充 MP3 檔時間。容差只用於測試量測，正式 DecodedAudioGate／DSP 時鐘精度不變。CBR 合成資料、ID3 前綴、取消、pin 及清理可離線核對；VBR 的門檻可接受，不代表實際 VBR、任意使用者 MP3 或播放器相容性已驗收。

真正雲端、金鑰、其他容器、Auto、全文詞彙及播放定位仍待完成。Phase 09 尚未具備啟動條件；金鑰維持結案固定遮蔽介面手動輸入。實際指令及測試數見 [Phase 8 第十二批驗證](../phase-8.md#第十二批驗證)。
