# ADR 0043 — 辨識來源的共用全文翻譯與獨立結果

日期：2026-10-12  
狀態：採用於 Phase 08 第八批；正式 ASR adapter、Ready 同一媒體 owner 與 GUI ASR 尚待接續。

## 問題

本機 WAVE 已能透過 caller 提供的 recognizer 交付有界 PreparedTranscript。既有 Prepared 全文翻譯只接受 ExistingSubtitle 的來源字幕；若將辨識結果包裝成 ExistingText 或虛構 content ID，會錯置來源證據，讓後續字幕優先序、Ready 或播放定位誤認為已取得可靠出版物。

另建 ASR 翻譯引擎會重複品質、脈絡、批次、校訂、取消與容量邏輯。直接在 async executor 做整份臺灣本地化，則會把 CPU 工作混入服務排程；僅看翻譯成功也不足以確認最後詞彙替換後的時間、句數與容量。

## 決策

新增私有 PreparedTranslationInput borrowed enum，分別接收 ExistingText 或 PreparedTranscript。兩種來源各自驗證：來源字幕保留原有 ExistingSubtitle／來源語言契約，ASR 使用 sealed transcript 的身分、五語、真正時間、順序、二十四小時及全文容量檢查。Cue view 只借用本文與時間，不建立共同 publication owner，也不複製整份原文。

新增 translate_transcript 內部入口，與既有 translate_document 共用 run_pass／make_request。只有目前有界 batch 的要求與有限脈絡會複製；所有要求保留原 Session、cue 序號及原生媒體時間，不使用 Live capture stamp 或更新 Instant 掩蓋過期。

Fast／Standard／High 使用一／三／八句前後文。Fast 做一次 Draft；Standard／High 在全篇 Draft 完成後才做 Review，校訂目前及鄰近草稿、前後原文與已完成譯文。倒序完成按原索引還原；每批依能力限制一至四個要求，整批完成後才排下一批。每要求三十秒離線期限不變，翻譯 client 仍須在跨 caller 的共用許可下限制並行及速率。

SessionGlossary 必須匹配來源 transcript 的 SID 與 generation，錯誤 glossary 在服務要求前拒收。所有本文、脈絡、草稿與 glossary 都是不可信任的資料；沿用固定 zh-TW 指示及已有 prepared_context HTTP 邊界，不向 provider 新增虛構來源證據或 fallback。

新增公開 translate_recognized_transcript，caller 明確提供 owned transcript、品質、同 Session glossary、所選 Translator 與 cancellation。BeforeFuture guard 使未 poll 的 Drop 也取消 scope 並釋放本文。服務要求不持 CPU 或 native-owner 許可，不預設服務或讀金鑰。

所有翻譯成功後，才在共用 CPU lane 進行臺灣用語本地化與最終驗證。原文維持 8192 cues／四 MiB；原始譯文及本地化譯文各限單句 8192 bytes、全文四 MiB，並核對非空、控制字元、相同句數、完整來源身分及不可改動的原始時間。過載明確失敗；已開始 worker 持有許可與本文至真正退出，abort 不代表同步工作已回收。

完整結果封裝為 PreparedTranslatedTranscript，持有原 ASR、同索引 boxed zh-TW 譯文與實際品質。本文只在本次 RAM，沒有 Debug／Clone／Serialize、內容匯出或持久化 cache。取消、timeout、provider error／panic、Future Drop、本地化或最終驗證失敗，不返回部分本文。

## 結果與限制

共用引擎保留現成字幕的嚴格來源契約，也保留辨識來源的獨立 provenance。結果是已翻譯的 ASR transcript，沒有 ExistingSubtitle publication、媒體 pin、Ready、來源仲裁或播放定位證據；不由工作直接寫 UI。

目前 native producer 會在真正 EOF／取消回收時釋放唯讀媒體 handle。後續 Ready 組合仍須設計同一媒體 handle 的身分與生命週期，不能重新開檔代替原 pin。可靠外掛字幕優先路徑不額外解碼；一般 GUI 無字幕仍回報 AsrUnavailable。

本批支援 caller 提供的 Session glossary，尚未完成自動全文脈絡／專名分析及 glossary 建立。正式 ASR adapter、GUI ASR、其他容器、URL 與播放定位亦待完成；既有 adapter 的本機 transport 測試與 mock provider 不能代替真正辨識／翻譯品質驗收。

沒有新增 crate、unsafe、安裝、真正雲端、產品音訊／字幕／翻譯寫檔、暫存媒體、快取或匯出。Phase 09 尚未具備啟動條件；有效 OpenAI／Gemini 金鑰及雲端驗收維持結案固定遮蔽介面手動輸入。實際指令與測試數於 [Phase 8](../phase-8.md) 完成驗證後記錄，不由此決策預先宣稱通過。
