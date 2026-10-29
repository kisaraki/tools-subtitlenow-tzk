# ADR 0067：Session 內有界 HTTP(S) 媒體範圍讀取

日期：2026-10-16。狀態：Phase 08 第三十四批採用。

## 範圍與決策

Prepared 的 remote MediaInput 先前只有語法驗證。新增獨立的 `media::remote_http::RemoteHttpMedia` Rust library，實際以 HTTP(S) 讀取指定 byte ranges，供後續遠端解碼接線使用。它不是整份下載、播放、媒體快取或 Ready；一般 GUI、Prepared producer／decoder 及原 `open_media` 仍明確拒收 URL，不能以本批宣稱一般介面可準備遠端影片。

格式語意參照 [RFC 9110 If-Match](https://www.rfc-editor.org/rfc/rfc9110.html#section-13.1.1)、[Content-Range](https://www.rfc-editor.org/rfc/rfc9110.html#section-14.4) 與 [206 Partial Content](https://www.rfc-editor.org/rfc/rfc9110.html#section-15.3.7)。以下限制是本產品的保守子集合，不是宣稱所有合法 HTTP 回應都必須符合這些限制。

## 來源與回應驗證

caller 明確提供通過既有語法驗證的 MediaInput、本次唯一 StreamIdentity 與取消 scope。只有 remote 輸入可開啟；建構 Future 不啟動 I/O。首次 GET `Range: bytes=0-0`，只取一 byte 核對服務是否提供實際分段、完整長度及強 ETag，probe byte 隨即釋放。不上傳音訊、不開 HEAD／Files API，也不從副檔名或 MIME 推定 codec。

Future 須在啟用網路及 timer 的 Tokio runtime 執行；沒有目前 runtime 時明確回 RuntimeUnavailable，且不開始連線。

明確設定 `reqwest::retry::never()`，停用目前 reqwest 0.12.28 預設的協定 NACK 重試；沒有把「應用程式未寫 retry loop」誤當成底層一定只發一次要求。

只接受 206、精確單一 Content-Range、明確 Content-Length、identity／無 content encoding 與單一強 ETag。強 tag 限 256 ASCII bytes，必須完整雙引號，拒收 weak tag、列表與非法字元；只用 HTTP validator，不以 Last-Modified 或無驗證 fallback 代替。HeaderMap 可見的重複必要欄位拒收。未知 total、multipart、chunked／Transfer-Encoding、壓縮、範圍或 body 長度不符都拒收；合法但不符合本子集合的服務明確不可用。

後續每次 GET 使用相同 client／URL 與 `If-Match`，要求回應仍有相同強 ETag、total 及精確 range。412／416、版本或長度改變回 SourceChanged，已取得的舊資料不與新版本混用。不追蹤 Location、不退回整份 200、不改用其他 URL／scheme。HTTPS 使用原有驗證 TLS，失敗不退回 HTTP；HTTP 僅按 caller 明確提供的 URL 使用。停用環境 proxy，不加入 Authorization、Cookie 或 Referer，沒有帳號、憑證或 key lookup。URL／query、ETag 與原始錯誤不進 Debug、設定或日誌；typed error 不含私有內容。

HTTP validator 是伺服器提供的表示版本資訊，不是本機 OS 檔案 pin、密碼學內容證明、字幕版本配對或播放定位證據。後續播放／來源仲裁仍須獨立驗證。

## 容量、取消與生命週期

每份來源最多四 GiB，每次 1–64 KiB；range request sequence 為 1–65536，帶 identity／generation、順序及 byte offset，不捏造媒體時間。offset＋length checked，完整位於已驗證 total 才發送。這是位元組讀取，不是音訊 sample 或字幕時間軸。

全程序最多兩個連線 owner；`&mut self` 保證每 owner 同時只有一份 read。獨立於 CPU lane，以 `try_acquire_owned` 取得 owner 配額，沒有等待佇列或新執行緒／CPU pool。同一 client 可重用持續連線。

全程序另限四個讀取中／下游保留的 range buffers，各最多 64 KiB，最多 256 KiB 已擁有 payload，另有最多兩份一-byte probe 及 transport／metadata 開銷。buffer 持有自己的許可，來源退休後仍計入配額，必須由下游 Drop 才釋放，不能靠切換來源規避記憶體上限。慢消費端耗盡配額立即 Overloaded，不等待或累積 stale 工作。產品不另存 byte cache，不保留整份媒體。

每個要求總上限三十秒，包含 body，連線上限十秒；每個 chunk 檢查取消並合作 yield。沒有自動重試；Network／Timeout／RateLimited 明確標為 retryable，重試必須重新開啟並驗證來源。任何 read 失敗或 Future Drop 都取消此 source scope，後續讀取及 range 的新 `bytes()` 存取回 Cancelled；取消不代表 caller 已 Drop source／舊 buffers。owner Drop 釋放連線配額，buffer Drop 釋放 payload 配額。caller 宜使用 Session 的子 scope，且在消費邊界繼續核對 identity／取消，不能把先前借用的 bytes 當成新 Session 資料。

沒有 unsafe、recoverable unwrap／expect、新相依、安裝、產品磁碟／temp 寫入、永久 metadata、cache、export、telemetry 或 provider 選擇。

## 驗證與下一步

以實際 TCP loopback 驗證精確 range／If-Match、持續連線、最大 block、版本改變與新 Session、重新導向不接觸 Location、200 不退回下載、錯誤範圍／body 截斷、取消／未 poll／執行中 Future Drop、deadline、owner 過載與慢消費端的 buffer 配額。HTTPS 對本機非 TLS 端點只發 TLS handshake，失敗不改發 HTTP 或暴露 query；沒有以此取代真實憑證／網際網路服務相容性驗收。結果見 [Phase 8](../phase-8.md)。

下一步仍在 Phase 08：把此非同步 range owner 接入受控媒體讀取／解碼，驗證取消橋接、來源一致性、格式探測、EOF 與 RAM Ready，再加入明確 URL GUI 與來源目的地揭露。目前沒有遠端 Prepared／播放成功或真實網站相容性證據，Phase 09 尚未具備條件。API keys 仍留至結案固定介面手動輸入。
