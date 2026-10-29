# ADR 0074：Prepared 的有界 HTTPS 字幕讀取

日期：2026-10-18。狀態：Phase 08 第四十一批採用。

## 決策與原因

新增獨立 `media::remote_subtitle` 程式庫入口，讀取使用者明確選定的 HTTPS SRT／WebVTT，再用既有 `parse_prepared` 解析完整文件。這是後續網址字幕接入 Session 的前提；本批不修改 `PreparedRequest`、本機路徑契約、控制器或一般視窗，不將不支援的字幕網址當成本機檔案或音訊辨識要求。

不呼叫 Live 的 `TimedSubtitleProvider::lookup`。該入口需要實際 capture stamp、短 freshness lease、查詢期限及已識別媒體資料；Prepared 不應為了重用它而製造時間或內容識別證據。只重用既有 URL 語法防護與完整文件解析器；Live 的三秒／256 KiB 規則保持原樣。

## 輸入與網路

`PreparedSubtitleUrl` 只接受明確 HTTPS，格式與三類中文或五語來源語言由呼叫者指定，不依 URL 副檔名、MIME 或本文猜語言。沿用 4096 bytes、拒絕 userinfo、fragment、空白、控制字元、反斜線及空 authority 的防護。HTTP 例外只存在測試建置的 literal 127.0.0.1 fixture。

一次明確動作只有一份 GET，不重試、不跟隨重新導向、不改 scheme、不加入帳密、cookie 或代理伺服器。不停用 TLS 驗證；URL／signed query／原始 reqwest 錯誤與本文沒有 Debug、日誌或序列化出口。HTTP 401／403、404、429、重新導向、網路、逾時及壞回應均回報不含私有來源的 typed error，不回傳「沒有候選」後轉為 ASR。

明確格式對應 plain-text MIME，字元集限 UTF-8，內容編碼僅 identity；拒絕多重或矛盾的控制標頭。宣告 Content-Length 時驗證值與實際長度，沒有宣告或使用有效 chunked 時仍逐塊核對實際八 MiB 上限與完整 HTTP 訊息結束。連線期限十秒、要求期限三十秒，獨立於 Live 音訊時鐘。

## 容量、取消與身分

全程序最多兩份要求／本文持有者，過載立即拒收。許可跨網路、原始 body、解析工作及成功結果保留，直到真正持有者退出或結果銷毀，不能透過丟棄 future 反覆繞過容量。網路等待不持 CPU；解析沿用 `media::work` 的有界 CPU lane，不建立新的 pool 或 OS thread。

解析由 blocking job 擁有原始 body、CPU 與本文許可，開始後直到真正退出才釋放；排隊工作 Drop 會中止，已開始者合作取消。解析後釋放原始 bytes，只保留既有最多 8192 cues／四 MiB 的 `ExistingText`。沒有字幕、翻譯或遠端 metadata 的永久保存、temp、cache 或匯出。

呼叫者交付真實非零 Session／generation／sequence，開始網路前核對。內部 content ID 只綁定本次工作，不是網站提供的版本或媒體身分。Future 在未 poll 時被 Drop 也會退休自己的子取消 scope，不能取消父 scope；父取消會向下退休工作及結果。成功結果沒有 Clone／Debug／Serialize，存取原文前核對取消狀態，舊結果不能成為新 Session 的資料。

## 範圍與驗證

本批只交付遠端字幕原文及實際原始時間；不建立媒體來源、辨識器、翻譯器、金鑰讀取、Ready、時間軸、仲裁或播放定位證據。網路原文仍需後續同 Session 的臺灣本地化／翻譯與來源規劃，不能直接宣稱為 zh-TW 準備結果。

測試限自有 loopback HTTP、合成字幕、注入容量與同步 barrier。正式傳輸維持 HTTPS；測試例外不提供使用者設定。結果與限制記於 [Phase 8](../phase-8.md)。一般 UI 的網址字幕仍拒收；正式金鑰維持結案固定介面手動輸入，Phase 09 尚未具備條件。
