# ADR 0104：缺少服務語言標記時的本機文字判定

日期：2026-10-26。狀態：採用，Phase 11 Auto 修正。取代 ADR 0101–0103 中「缺少服務標記就無法交付」的部分。

## 問題與證據

人工重試收到／送出音訊各 348 包、服務訊息 7、暫定 5、真正定稿 1、交付 0、語言收尾要求 1；收尾有效，但該次定稿仍沒有程式可核對的 languageCode。不能再假設每段服務轉錄必有語言標記。官方 [SDK Transcription 型別](https://raw.githubusercontent.com/googleapis/python-genai/main/google/genai/types.py) 亦將 language_code 表示為 optional；這不代表所有帳戶／回應永遠缺少標記。

## 決定

仍以服務的明確語言標記優先，沿用五語、地區標籤及同一 utterance 的核對。標記存在但不合法／支援外時直接拒收，不能被本機結果覆蓋。只有 Gemini Transcribe Live／Auto 缺少標記且沒有同段服務語言時，才把目前辨識文字送到本機判定，不增加雲端要求、不讀取額外金鑰、不改供應商／模型。

使用精確固定的 whatlang 0.18.0，停用額外 features，使用其全部內建語言模型後才篩選五語，不用五語 allowlist 強迫外語對應到其中一種。至少十二個字母字元並通過函式庫 is_reliable 才可採用，結果只映射英、日、法、德、韓；其他可靠判定回報不支援，短句／不確定／CPU 忙碌不交付。is_reliable 是模型內部門檻（該版大於 0.9），不是校準的正確率，不在 UI 宣稱百分比保證。演算法及限制見 [上游說明](https://github.com/greyblake/whatlang-rs/tree/v0.18.0)。

每個缺標記的新修訂都重新判定，不把本機估計寫入服務語言 context，也不沿用上一段。只交付實際被判定的那份文字；先前未判定 partial 捨棄，不以新估計回填舊字。真正 final 若仍不確定，明示累計略過該段並退休其有界狀態，繼續後續語句；不是把不確定文字當成成功，也不捏造 Final。沒有真正 final 的持續未確認狀態仍遵守原八秒期限與限次收尾。

本機估計不代表服務／音訊語言證據。Gemini 事件仍不建立可靠音訊關聯時間，維持 start_us／duration_us=None；不修改 ExactCueVerifier、LivePlaybackClock、既有字幕仲裁或媒體定位。ASR 真正完成標記、Session／generation、時間軸及翻譯品質策略保持。

## CPU、取消與隱私

判定只在原有全程序 CPU lane（依核心數、最多兩份工作）內 spawn_blocking；不建立另一個執行緒池或每音訊包工作。每份工作最多 partial／final 各八 KiB 文字，取不到名額立即回傳 Busy，總預算四十 ms，包含派發；逾時或結果過期不得採用。音訊擷取／前處理不等這份 CPU 工作，既有二百四十 ms 音訊佇列及二百五十 ms 年齡不變。

工作綁定 Session／generation、服務訊息序號、partial／final、精確本文及期限。相同文字重複出現也不能重用舊訊息結果。取消／Drop 中止未啟動工作，已開始的有界工作持有 CPU permit 直到真正完成；完成後再檢查取消，舊 Session 不能更新新狀態。文字及結果只有本次 RAM，不存檔、不記錄本文、不建內容 cache。模型資料及授權是一般演算法資產，不是使用者內容。

UI 在開始前揭露本機估計及不確定略過政策，開始不因這則資訊提示而停用；處理中顯示服務標記／本機文字估計的依據、本機採用與語言未確認略過數。使用者可手選語言以避免估計。不自動改語言或重存金鑰，其他模型維持既有能力界線。

## 相依套件與驗證

Cargo.lock 新增 whatlang 0.18.0 與四個純 Rust 相依套件，未更新既有解析套件，沒有 build script、額外 Windows 元件／DLL 或執行時下載。原始授權與版本／來源見 [授權紀錄](../../licenses/NOTICE.md)。

測試涵蓋五語、支援外語不強制映射、短句、忙碌、取消、逾時、真正釋放 permit、錯誤／舊訊息／期限拒收、服務標記優先、逐修訂重新判定、略過後下一段及同一 WebSocket 無標記英／日連續交付。合成 CPU probe 不代表真實字幕延遲或模型準確率；短句、專名、混語或錯誤辨識仍可能不確定或估錯，需實際五語驗收。實際命令、首次測試期待修正及部署見 [Phase 11 診斷](../phase-11-gemini-diagnostics.md)。
