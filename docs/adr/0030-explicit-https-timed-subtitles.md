# ADR 0030 — 明確配置的 HTTPS 定時字幕來源

日期：2026-10-10。狀態：Phase 7 第九批採用。

Phase 7 已有文字出版物、配對來源證據與仲裁，但尚未有 ExistingSubtitle 的正式網路 adapter。既有 LRCLIB 提供歌詞，沒有可直接用來推定字幕語言或配對版本的欄位；單純加入設定開關無法補齊這些資料。另查核 [AcoustID 官方 API](https://acoustid.org/webservice) 與 [FAQ](https://acoustid.org/faq)，該服務要求完整音檔時長，並明確不支援短片段識別，因此不以十二秒 Live 視窗冒充完整歌曲資料來串接。

新增 TimedSubtitleProvider，接受 caller 已知的 Session、內容 ID、字幕 HTTPS 位置、格式與語言；可另外提供來源參考軌及兩個相同版本 ID。這些資料是明確配置的聲明，不是 adapter 自行產生的辨識證據，不能從網址、檔名或 STT 選項猜測。它只讀字幕本文，不取得影音檔案、不辨識內容、不提高確認信心；最終仍須通過原有 Final ASR、唯一 cue、時間、內容信心及實際擷取游標檢查才可採用。

格式依據 [SubRip 格式說明](https://www.loc.gov/preservation/digital/formats/fdd/fdd000569.shtml) 與 [W3C WebVTT 草案](https://www.w3.org/TR/webvtt1/)，本版僅支援明確標示的純文字子集。接受 UTF-8 可選 BOM、LF／CRLF、正數 SRT 索引、WebVTT 標頭及可選 cue ID、多行文字與明確起訖時間。保留順序、原始時間、重複及重疊 cue，交由定位器拒絕歧義；不排序、去重、猜測結尾或重分段。樣式、定位設定、註解區塊、時間映射、標記、實體與 ASS 控制碼明確拒收，不靜默剝除語意。普通文字中的裸 & 保留；不承諾完整 SRT／WebVTT 相容性或其他編碼。

正式建置僅允許 HTTPS 與有效 TLS，禁止網址 userinfo、fragment、空白／控制字元及超過 4096 bytes；不跟隨重新導向。位置可能含簽署 query，因此位置、track、provider 與出版物沒有 Debug／Serialize，不顯示或記錄網址及 reqwest 原始錯誤。只有測試編譯的私有 literal 127.0.0.1 HTTP 入口可使用本機 fixture，未停用正式 TLS 驗證。配置本身不送要求；caller 必須在使用前揭露所有實際處理目的地。

每個 provider 同時最多一個 lookup，容量不足立即回報 TextOverloaded，沒有等候佇列；同一份配對最多兩個依序 GET，每秒最多啟動一份 lookup。HTTP 僅接受完整 200、相符 MIME 與無 charset 或精確 UTF-8 charset；其他編碼與壓縮明確拒收。Content-Length 與累積 chunk 均限制每軌 256 KiB，兩軌暫存合計最多 512 KiB；解析後兩軌合計沿用 1024 cues／128 KiB，每 cue 8192 bytes。參考軌 404、非法格式或任何失敗都使整份配對失敗，不能只交付中文本文冒充未配對成功；主軌 404 回報沒有來源。

一份 lookup 使用三秒預算與原 query 五秒期限較早者，兩個 GET 與解析不能各自重設期限。外部取消、Session 替換或 future Drop 釋放網路 future，通知獨立工作取消訊號，不反向取消 caller。CPU 解析在 spawn_blocking 執行，process-wide permit 依 available_parallelism、最多二個，飽和立即拒收；closure 持有 permit 至結束，逐行檢查取消與原期限。Drop abort 尚未開始工作，已開始工作協作停止，不宣稱固定毫秒取消延遲。

429 沿用 server Retry-After 秒數或日期，缺少標頭使用六十秒，非法或溢位值則在此 provider 生命週期內停止新要求。其他錯誤使用固定型別訊息，沒有自動重試、跨來源備援、持久內容或額外金鑰。兩軌解析與總預算都通過後，才透過 ExistingTextBundle 原子交付至原有 ExistingTextSession／lease／準備／仲裁介面。

本批完成真實 transport 與解析 adapter，沒有建立預設內容服務、GUI 網址設定或 Movie／ExistingSubtitle 管理物件。正式辨識服務、來源 metadata、可靠配對位置／版本及 Live GUI 組合仍待完成，不能以合成 HTTP／ASR 測試宣稱真實媒體已驗收。Prepared 影音輸入不在本批；金鑰與真實服務驗收維持結案時固定遮蔽介面手動輸入。

後續擴充：Phase 7 第十批依 [ADR 0031](0031-live-subtitle-owner-and-shared-text-routing.md) 完成可設定 Live／GUI 的字幕管理、服務提示及 Session metadata 核對。一般 GUI 仍不預設設定服務；正式來源及實機驗收保持待辦。
