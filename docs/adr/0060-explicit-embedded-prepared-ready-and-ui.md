# ADR 0060：明確內嵌字幕的 Prepared／GUI／Ready

日期：2026-10-15。狀態：Phase 08 第二十七批採用。

後續：第二十八批已加入目前支援單一軌的本機探索，見 [ADR 0061](0061-bounded-local-mp4-subtitle-discovery.md)；下列手動入口與當批限制是歷史記錄。

## 問題與決策

前批能讀取指定 MP4 tx3g，但一般準備流程只處理 sidecar 或 ASR。把明確文字來源加入同一 PreparedRequest／Session，讓現有詞彙／翻譯／本地化／Ready 共用該原文與原檔 pin；不為內嵌來源建立第二套全文管線或 renderer。

PreparedRequest 最多一份 Mp4SubtitleSelection，和 sidecars 共用八份總選擇預算。依原有語言順位比較，等順位報錯讓呼叫者選定，不假設 embedded 比 sidecar 更可信。沒有字幕才計畫 ASR；已選文字的 parse／read／provider 錯誤不轉換為音訊辨識或跨服務 fallback。

metadata-only text_language 計畫在 SelectedPreparedServiceFactory 之前進行。中文直接本地化、不解析或讀取金鑰、不建 client；來源文字只建立所選 provider 的 Translator／GlossaryAnalyzer，音訊語言 Auto 與這條路徑無關。固定遮蔽金鑰規則維持。

## 同一所有權與 CPU 邊界

既有 Session blocking load 先開啟 OpenedMedia，再使用 read_pinned helper。該 helper 只複製同一 File handle，不重開 path；不從已持 CPU permit 的 worker 呼叫另一個 async extraction 或再次取得 permit，避免單核心／飽和時自我過載。獨立 library async extractor 也重用 helper，仍維持原 API。

完整 ExistingText／原始 times 及 pin 進入既有 LoadedDocument；來源文字經全文 glossary／品質策略 Draft／Review，中文經 Taiwan phrase conversion。雲端等待期間不持 CPU 許可，最終本地化／validation 回到既有 bounded lane。只有當前 identity／sequence 的完整 completion 成為 Ready；stop／clear／Drop 釋放 RAM、pin 與長任務，沒有部分發布或播放。

## 一般使用者介面

沿用既有格式 combo，新增「MP4 內嵌 tx3g（填軌 ID）」；同一輸入欄在該明確模式為 track ID，其他模式為 sidecar path。這可保留原有控制項數量、scroll／DPI 幾何、固定雙列 password edits、通知及 Ready 鎖定流程。暫時沒有自動探索清單，使用者需已知軌 ID。

格式控制項在文字空白時也可選取。embedded 模式無論 ID 是否有效，都屬已選文字，不啟用音訊路徑；空字串、零、符號、空白、小數、全形數字或 u32 溢位拒收，不能自動 ASR。原音訊語言 Auto 僅限制沒有文字的 ASR；文字語言仍需明確選擇。來源模式切換更新欄位及 transmission disclosure，中文保持本機，來源語言只揭露選定主機的文字／脈絡傳送。

操作：開啟一般 --ui 的本次準備視窗，填本機媒體路徑，選 MP4 內嵌 tx3g，在字幕欄填已知非零 track ID，再選字幕語言及品質；來源文字才需要服務／金鑰。按準備後只處理該選取，Ready 不播放；清除／取消／關閉會釋放本次資料。有效金鑰仍留至結案固定介面由使用者手動輸入。

## 驗證與限制

測試使用合成 MP4 結構與實際文字 samples，音訊／視訊 bytes 是明確 gate fixture，不代表 playable codec 或 muxer 相容性。覆蓋語言順位／同順位、八份總預算、三種中文不碰 provider／key、五語 × 三種品質 × 兩個所選服務的 mock glossary／Draft／Review、原始文字／時間與 pin、錯誤無 ASR、pending 取消與 CPU 回收、舊 completion、過載及復原。hidden native GUI 驗證模式／語言／品質／disclosure／keys 與真實中文 controller 的 Ready／clear／錯軌／修正重試；通知使用明確 mock，不主張真實 toast 顯示已驗收。結果見 [Phase 8](../phase-8.md)。

容器門檻保持 [ADR 0059](0059-selected-pinned-mp4-tx3g-reader.md)：單一指定 text／tx3g、既有 AAC-LC／可選 H.264、無 edit list／文字 ctts／modifier／外部引用。未提供多文字軌、其他 handler、樣式或自動 language／版本判定；原音訊 probe 不因 UI 接線開放含文字軌的 ASR。content ID 不當作 alignment／arbiter 或版本 fingerprint。

軌道探索清單、一般 muxer／使用者檔案、完整語意、HTTP(S)／Auto、實際播放同步及雲端品質仍待接續。新功能沒有正式程式 unsafe、套件安裝、產品寫入／temp／cache／export、第二條 STT／翻譯時間軸或自動播放。hidden fixture 的新增 unsafe 僅查詢同一測試執行緒仍存活的自有 child HWND 是否啟用，沒有 pointer／buffer／ownership 逸出，並附 SAFETY 說明。Phase 09 尚未具備條件。
