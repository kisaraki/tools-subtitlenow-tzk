# ADR 0051 — Grounded Prepared Session Glossary

2026-10-14，Phase 08 第十六批，Windows 19045 基準。

## 決策與原因

來源字幕及完整 ASR transcript 在翻譯前，先由本次明確選定的 OpenAI／Gemini 分析專名與術語。每個原文窗口都處理完後，才固定一份 SessionGlossary 供全部 Draft／Review 使用，讓較後段發現的詞彙也能影響前段翻譯。這是一輪全文詞彙整理；不宣稱已完成全文主題／語意摘要或真實譯名品質驗收。

`PreparedGlossaryAnalyzer` 使用獨立要求／回應契約，不把分析 JSON 偽裝成 TranscriptEvent、字幕或翻譯。GUI 的 `PreparedTranslator` 組合契約要求同時提供分析及翻譯；正式 HttpTranslator 使用同一選定 provider、憑證、Client、連線池、兩份要求許可及速率 gate。既有手動 glossary 的 library API 保持明確分開，不默默降級或更換服務。

## 原文證據與合併

服務只能回傳 cue_index、逐字複製的 source 與 taiwan_translation。本機查核 source 確實是該 cue 的連續原文，推導 UTF-8 半開區間；模型不用計算多位元組文字的位置。領域回應再核對完整 Session／generation、窗口 sequence、cue 及合法 UTF-8 範圍。HTTP adapter 的身分來自本機要求，不相信雲端傳回的 Session，也不把身分、路徑或金鑰放進提示資料。

空候選是合法分析結果；不存在的來源、空白、控制字元、錯身分、超限或 malformed JSON 都明確失敗。所有生成 target 在共用有界 CPU lane 做繁簡與臺灣片語本地化，轉換後仍核對 256 bytes。Caller 明確提供的手動詞彙有優先權；相同生成原文與相同譯名去重，不同生成譯名回報 InvalidData，不猜測哪一個正確或覆寫已接受的詞彙。

## 資源與生命週期

全文沿用最多 8192 cues／4 MiB 原文。每個分析窗口最多 64 cues／64 KiB 原文，依原始順序逐窗口處理，最多 256 個窗口；不抽樣、跳過尾段或在詞彙表滿後停止分析。已接受的至多 64 筆詞彙帶到下一個窗口，單筆 source／target 各最多 256 bytes。超過 64 個不同詞彙回報 Overloaded，沒有無聲截斷或假成功。

Serialized 要求最多 512 KiB；候選 JSON 最多 64 KiB／64 筆，SSE 單事件及完整傳輸另沿用 64／256 KiB 上限。窗口三十秒預算包含 gate、要求、重試、解析與 CPU 合併；網路等待不持 CPU 許可。瞬時網路／限速／timeout 最多兩次嘗試，錯誤 body 不讀取／記錄。Live 的文字／期限上限保持原有方式。

Caller-owned futures、連結的子取消 scope、要求 token 及 worker Drop guards 共同處理停止、換來源、關閉、timeout、panic、錯資料與 Future Drop。已開始的 CPU worker 持有許可至實際退出並合作檢查取消；不新增 detached task、每句 OS thread 或內容佇列。原媒體 pin 跨分析與翻譯等待持續存在。整份成功後才 apply Ready；詞彙表只在本次工作 RAM，錯誤／取消／完成時釋放，不存檔、匯出或跨啟動保存。

中文既有字幕繼續本機本地化，不建立分析 client、不讀金鑰或呼叫分析。準備視窗在明確動作前揭露所選主機及專名／詞彙整理與翻譯用途；固定遮蔽欄位、清空及結案手動輸入安排沿用。

## 官方協定資料

OpenAI Responses 的結構化輸出使用 `text.format`／JSON Schema，串流仍核對真正完成或拒絕狀態。[OpenAI Structured Outputs](https://developers.openai.com/api/docs/guides/structured-outputs?api-mode=responses)

Gemini 沿用既有 streamGenerateContent，指定 `application/json` 及 `responseJsonSchema`。結構化串流片段先完整組合再解析，未完成或拒絕不產生詞彙。[Gemini Generate Content reference](https://ai.google.dev/api/generate-content)、[Gemini Structured Outputs](https://ai.google.dev/gemini-api/docs/structured-output)

## 驗證與限制

使用合成全文、明確 mock、真正本機 TCP loopback 及現有 Prepared controller 驗證；有效金鑰、雲端模型是否接受目前全部 schema／上限、五語實際譯名品質與費用仍留至結案驗收。原文存在只能證明來源對應，不能證明語意或譯名正確。衝突及容量拒收是目前明確限制；沒有以假信心數字掩飾。

Phase 08 的 Auto、其他容器／內嵌字幕、URL 媒體讀取、完整語意脈絡、跨句品質、實際檔案相容性及播放／音訊定位仍待接續；Phase 09 尚未具備條件。
