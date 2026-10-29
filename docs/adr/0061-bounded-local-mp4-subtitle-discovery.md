# ADR 0061：本機 MP4 字幕軌探索

日期：2026-10-15。狀態：Phase 08 第二十八批採用。

後續：第二十九批擴大至最多八條文字軌及明確選擇清單，見 [ADR 0062](0062-bounded-mp4-multiple-subtitle-tracks.md)。下列單軌範圍記錄當批行為。

## 問題與決策

前批 GUI 要求先從外部取得軌 ID。新增「探索 MP4 字幕軌」，在所選本機檔案內尋找既有讀取器支援的單一 text／tx3g 軌，自動填入 ID；保留手動 ID 入口。這是既有嚴格格式範圍的探索，不是一般多字幕軌清單或最佳字幕自動選擇。

探索和抽取共用同一 selected_track 驗證：moov／metadata 上限、AAC-LC／可選 H.264、唯一非零 ID、tx3g description／font table、檔內引用、時間／sample tables／bytes 容量、各軌範圍及跨軌 overlap 均保持。探索不讀取完整文字 samples；共用 loader 仍有最後一 byte mdat 的可讀性探測，因此不宣稱完全不碰 payload。損壞的文字內容可能在後續準備時才被拒收，探索成功不代表字幕有效或 Ready。

mdhd ISO 語言只作提示。eng／jpn／fra／deu／kor 可填入對應五語；zho 只知中文，不能當成臺灣繁中或判斷繁簡，故清空語言讓使用者指定；und 同樣需要明確指定。其他語言／mul／非法 packed 值維持拒收，不推測、不自動上傳辨識。格式依據沿用 [ADR 0059](0059-selected-pinned-mp4-tx3g-reader.md)。

## 所有權與隔離

library async discovery 共用既有 Prepared CPU semaphore 及取消／Drop／blocking 許可生命週期，不另建 CPU pool。不接受 URL，不建 decoder／provider／credential client，不輸出字幕或產品暫存。開檔、結構讀取與驗證都在 bounded blocking lane，來源 pin 在探索退出前釋放。後續按準備重新開檔並完整驗證指定 ID／語言／本文，沒有把一次探索視為之後檔案版本不變的證據。

UI owner 每次只允許一份探索，分配唯一 StreamIdentity，一份 bounded terminal channel，poll 只在 thread 已結束時 join，核對 identity 後才應用結果。Stop 先退休結果，即使成功已在 channel 內也不填回；舊 thread 真正退出／runtime 回收 blocking work 前拒絕重啟。Drop 取消並 join，不留分離工作。錯誤保留型別，沒有默默 ASR 或跨服務補救。

探索期間鎖定來源／準備與服務欄，目的地明示純本機；兩列金鑰及 undo 在探索開始即清除、不讀取。完成只填 ID／格式／可確認語言，不通知 Ready、不自動準備或播放。語言／ID／格式編輯退休探索標籤；來源路徑變更會退休探索及其自動填入的 ID／語言，程式化來源改動也取消 pending scan。取消／關閉等待兩種 owner 停妥才銷毀視窗。新增按鈕位於媒體路徑旁，沿用 scroll／DPI／固定動作列。

## 驗證與限制

合成容器測試核對七種語言提示、拒收無效 tables／range／description／語言、所有截斷及預先取消、不讀完整本文、探索成功仍可能在準備拒收損壞本文。owner 測試以 controlled channels 驗證 pending／buffered 成功取消、唯一 identity／不重疊重啟、typed error／panic 復原、Drop 回收，以及真實自有檔案探索釋放 pin。hidden Win32 測試完成探索→中文明確選擇→真實本機 Ready→清除，及五語提示、來源變更、錯誤、關閉；沒有正式金鑰或真實雲端。

合成 AAC／H.264 bytes 是結構 fixture，不代表一般 muxer／實際影片相容性。探索維持單文字軌，其他 handler／codec／多文字軌／edit list／文字 ctts／modifier 仍未支援；沒有版本或播放 alignment 證據。更多容器、URL／Auto、完整語意、實際檔案與雲端品質／播放仍待接續。Phase 09 尚未具備條件；有效金鑰仍留至結案固定遮蔽介面手動輸入。
