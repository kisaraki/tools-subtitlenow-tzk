# ADR 0036：Prepared 文件生命週期與本機輸入

日期：2026-10-11

狀態：採用；Phase 08 首批，本機外掛字幕準備。影音解碼、ASR、全文來源翻譯及播放接線另列待辦。

Live 字幕處理核對實際擷取視窗，只有五秒有效期限，來源翻譯也只處理一至四句。Prepared 可能處理整份影片文件數分鐘，不能用假擷取時間或不斷續期讓 Live 介面勉強運作。採用獨立 Prepared document scope、較大的有界文件預算及 caller-owned 工作；共用嚴格字幕語法與臺灣用語字典，保留 Live 的原有期限及容量。

Live 與 Prepared 共用 SessionId 的 checked atomic allocator。每個新 Prepared 要求分配唯一 Session、sequence 與實際完成時間，撤回上一輪；只有目前精確身分與序號的 completion 可以提交。新工作、停止、owner Drop 或執行 Future Drop 都取消；舊完成結果不能覆蓋或清除新資料。完成結果與 Ready document 不提供 Clone、Debug、Serialize 或匯出。

準備輸入是使用者明確指定的本機媒體與外掛字幕，不由檔名猜語言或配對。最多八個明確選項，依原生 zh-TW、其他繁中、簡中、五語原文排序；相同最高優先級多份資料要求明確選取。沒有字幕回報 ASR 尚未接入，原文字幕回報全文翻譯尚未接入，URL 只完成 HTTP(S) 輸入驗證並回報實際預處理不可用，不假裝已完成或偷偷改抓另一個來源。

本機 I/O 只在有界工作上執行。路徑有界、絕對且不包含裝置、UNC、alternate streams 或特殊名稱；Windows worker 在查檔前核對磁碟類型，拒絕遠端映射與未知磁碟。先核對各路徑元件的 reparse／symlink，再開啟唯讀一般檔案，使用 SHARE_READ 與 OPEN_REPARSE_POINT，開啟後再核對 handle metadata。媒體檔案只持有原檔 handle 與大小，不讀取影音、不宣稱解碼或配對成功。

字幕每八 KiB 核對取消，限制八 MiB 原始資料並最多探測上限後一 byte。嚴格 SRT／WebVTT 解析最多 8192 cues、四 MiB 原文；本地化結果另限四 MiB，保留原始 cue／時間。處理完解析即釋放 raw buffer。中文以既有 OpenCC 臺灣片語字典處理，不能只做繁簡字形轉換。結果只由目前 owner 借用，停止或 Drop 釋放；沒有磁碟快取或永久字幕保存。

File I/O、解析與本地化共用依 available_parallelism 限制到最多兩項工作的 CPU lane。try-acquire 不排佇列，過載回傳型別錯誤。Future Drop 取消目前工作並嘗試 abort 未開始工作；已開始的 blocking worker 仍持有許可，逐 chunk／line／cue 合作檢查取消，直到工作真正退出。磁碟系統呼叫本身不能由此介面強制中斷，取消在系統呼叫返回後的下一個檢查點生效。

Ready 在此只表示外掛字幕已完成解析、本地化及結構／時間驗證，媒體關聯是使用者聲明，不能當作播放定位或仲裁證據。這批不自動播放、不建立第二個 renderer 或 Timeline；之後須接入完成通知及既有時間軸，依實際音訊重新核對位置。

這條路徑不需要媒體暫存檔，故不建立無用 temp 或掃除其他檔案。內嵌字幕、URL 讀取、音訊抽取、ASR、全文脈絡／一致性、通知與播放仍是 Phase 08 後批；Phase 09 的 fuzzy／drift 不在本批實作。
