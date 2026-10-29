# ADR 0071：遠端 WAVE 的 Session 全文處理與 RAM Ready

日期：2026-10-16。狀態：Phase 08 第三十八批採用。

## 決策與原因

在 PreparedTask 增加明確的 `run_with_remote_wave_recognizer` 程式庫入口，接收 HTTP(S) 媒體、呼叫端選定的辨識器與具全文分析能力的翻譯器。沿用同一 RemoteHttpMedia／probe／PCM16 WAVE decoder／Prepared 辨識，再進入原有全文摘要、詞彙、翻譯、臺灣本地化、完整文件驗證與 PreparedCompletion／Session Ready。

PreparedDocument 原先只持有本機唯讀檔案，不能假裝這個檔案 pin 代表遠端來源。改為私有 PreparedMediaOwner，明確區分 Local 與 Remote；遠端 variant 持有同一 EOF decoder，內部仍是原 client／URL／強 ETag／total／sequence owner。只新增 crate-private 的辨識結果所有權移交，先核對取消再同時移動 source 與 transcript，沒有公開取走原文的入口。媒體大小由對應 owner 回報，不重開 URL 或以本機暫存檔取代來源。

本機與遠端辨識後共用一份 finish_recognized，避免逐漸分叉的摘要、詞彙、品質與本地化流程。來源在分析、所有 Draft／Review、最後本地化、完成結果尚未 apply 與 Ready 期間持續保留；任何失敗釋放來源，不交付部分文件。只有目前 Session／sequence 的完整成功能進入 Ready。Ready 是本次 RAM 文件，不是識別、版本配對或播放定位證據，沒有自動播放。

## 輸入與服務邊界

此入口只處理明確 remote 且規劃為 AsrRequired 的要求。本機輸入回 InvalidInput；有選定字幕回 UnsupportedUrl，同順位先回 AmbiguousSubtitle，全部在媒體／服務要求前拒收。這個有限入口尚不能將 URL 與現有字幕組合準備，但不忽略選定文字、不把壞字幕當成缺字幕、不改上傳音訊。既有一般本機／GUI Prepared 入口仍拒收 URL；URL 操作、用途揭露及相關字幕支援留待後續。

舊 Session 詞彙及辨識能力在 HTTP 連線前驗證。沒有讀取金鑰、建立預設服務、跨服務 fallback 或 GUI 相依；呼叫端須明確選定並揭露服務。正式 OpenAI／Gemini 金鑰仍留至結案固定遮蔽介面手動輸入，不以本批 mock 宣稱服務品質已驗收。

遠端自動全文分析是必要步驟；沒有提供省略分析後假裝完成相同 Ready 的新入口。所有來源原文完成後先分析摘要與詞彙，分析成功才送 Draft／Review，沿用 Fast／Standard／High 的既有策略與同一不可變文件摘要。空摘要、服務錯誤或錯身分不能降級成無摘要文件。

## 容量、取消與錯誤

不新增 worker pool、原始 PCM／分段配額、無界佇列或時間軸。沿用既有 available_parallelism CPU lane、遠端兩個 owner、range／PCM 配額、Prepared 辨識／全文處理的並行、逾時與文字容量。HTTP、辨識與翻譯等待不持有 CPU；保留 EOF source 不預讀或重複驗證遠端。

Session 父取消連結本次子 scope；底層錯誤可退休子 scope，卻不能覆蓋原本的 SourceChanged／RateLimited 等原因。新增 MediaError::Remote 保存原有安全 typed transport error，Cancelled／Stale 映射既有 Session 狀態；沒有將 URL、query、本文或 provider 原始錯誤帶進訊息。Future Drop（包含未 poll）、停止、替換來源與舊 completion 沿用同一 Session 隔離。

來源一致性只涵蓋本次實際讀取的 HTTP 表示；Ready 後不再發網路要求，強 ETag 不構成使用者日後播放版本或目前播放位置的證明。沒有媒體或文字落盤、temp／cache／export、記錄私有內容或遙測。

## 驗證與後續

以實際 loopback HTTP、自有合成 PCM、明確辨識／全文分析／翻譯 mock 驗證兩種辨識取樣率、三品質、zh-TW、原時間與 provenance、摘要共用、來源 owner 跨分析／翻譯／completion／Ready 的配額、錯誤／取消／Drop／舊 Session／過載及無隱式上傳。結果見 [Phase 8](../phase-8.md)。不擷取使用者音訊、不用正式金鑰或真實雲端。

後續仍在 Phase 08：URL GUI 與用途揭露、URL 搭配可靠字幕、其他遠端編碼、五語 Auto、真實媒體／語意／雲端與播放驗收。Phase 09 尚未具備啟動條件；符合時會提醒。
