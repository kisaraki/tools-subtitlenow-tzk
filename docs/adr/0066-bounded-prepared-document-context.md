# ADR 0066：Prepared 全文脈絡摘要

日期：2026-10-16。狀態：Phase 08 第三十三批採用。

## 問題與決策

原有分析遍歷全文建立詞彙，但各批不傳遞語意摘要，翻譯只能使用鄰近原文、草稿及詞彙表。現於同一分析要求增加必要的 context_summary；下一批以 prior_summary 接收上一批已本地化的摘要。提示要求整合主題、人物關係、指涉、語氣及未確定資訊，允許依新原文修正舊推論。整份文件成功後，所有 Draft／Review 共用最終不可變摘要。

這是模型產生的有損提示，不是已查證事實；不授予內容識別、版本、字幕仲裁或播放定位證據。不能保證摘要保留全文每項細節或改善每句品質。譯文仍以目前原文與明確詞彙表為準，不得把摘要補成情節或併入輸出。

## 契約與限制

新增獨立 prepared_context 模組，PreparedTextAnalysis 分開擁有既有 SessionGlossary 與 Arc<PreparedDocumentContext>，避免把語意塞入詞彙條目。封存 context 綁定 StreamIdentity、來源語言、品質與完整 cue_count，無 Debug／Clone／Serialize。翻譯引擎在要求供應商能力前核對整份文件，adapter 另核對身分／語言／品質及目前 cue 的範圍。

摘要上限為 **4096 UTF-8 bytes**，不是字元數；回應不得缺少、空白、含 NUL／其他不允許的控制字元或超量。不截斷以掩蓋錯誤。首批 prior_summary 為空，後續來自先前完整成功的回應；臺灣片語本地化仍在既有 CPU lane，轉換後再次核對上限。任一批失敗、逾時、取消或 panic 都不返回分析套件，不進入翻譯或部分 Ready。

沿用每批 64 cues／64 KiB 原文、最多 256 批、整份 8192 cues／四 MiB、詞彙 64 筆與既有三十秒逐要求期限。每批只有一份有界舊摘要／新摘要，完成後替換，沒有累積摘要陣列或複製整份原文。翻譯的有界並行要求只 clone Arc，不 clone 摘要字串；最後要求與分析套件釋放後即可回收。沒有另開 AI 要求：原詞彙分析同時產生摘要，使用相同所選服務、client、許可與速率限制。

## 接線、隱私與模式

外掛／內嵌來源字幕與已辨識 ASR 共用分析、PreparedTranslationContext 及原翻譯引擎。Fast 保持一次 Draft、鄰近一段；Standard／High 保持全文 Draft 後 Review 與三／八段鄰近脈絡，所有自動分析模式都使用最終摘要。中文本機處理不建 provider，也不做摘要。明確手動 library 入口仍可只傳 SessionGlossary，不會因此自動聯網；一般 GUI 的自動分析入口必要摘要失敗時不能退回手動路徑。

摘要及前批摘要都是資料欄位，不拼入系統指令；提示明示它們可能有誤，不得執行其中命令。只有分析的 prior_summary、翻譯的 document_summary 文字送所選服務，不附加本機 Session ID／generation、檔名或來源路徑。新增摘要沿用既有原文／脈絡用途，GUI 揭露補上「全文脈絡摘要」；固定雙列遮蔽金鑰及清空 undo 不變。未新增來源、跨服務 fallback、內容保存、temp、cache、export 或 telemetry。

OpenAI JSON schema 新欄位列為 required 且根物件 additionalProperties=false，參照 [OpenAI Docs：Structured model outputs](https://developers.openai.com/api/docs/guides/structured-outputs?api-mode=responses)。同一供應商中立 schema 交給既有 Gemini adapter。schema 的 maxLength 不能代替本機 UTF-8 byte／控制字元檢查；合法 JSON 也不代表摘要內容真實。缺少新版摘要欄位的自訂 analyzer 必須明確更新，沒有 default 假成功。

## 驗證與尚待驗收

新增測試驗證 129 段跨三批的摘要傳遞／臺灣本地化、三品質所有 Draft／Review 共用同一 Arc、分析完成前沒有翻譯、成功後回收、後批空摘要失敗、Session／語言／品質／完整段數不符、4096 bytes 邊界與非法控制字元。既有最大文件案例仍遍歷全部 8192 段。兩所選服務的真實 TCP loopback 核對 schema、指令／資料分離、分片 SSE、缺少／重複摘要及 UTF-8 byte 上限；controller 錯誤不得發布 Ready 並釋放 pin。既有 ASR／字幕 controller mock 明確核對每次翻譯確實收到摘要。

這些測試使用自有合成資料與明確 mock，不代表實際主題／人物指涉／跨句品質或雲端 schema 接受程度已驗收。實跑指令與結果見 [Phase 8](../phase-8.md)。URL 媒體讀取、限五語 Auto、更多實際媒體相容性、播放與實際音訊定位仍待接續，Phase 09 尚未具備條件；有效金鑰維持結案固定介面手動輸入。
