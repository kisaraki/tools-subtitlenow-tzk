# ADR 0014 — 與 Live STT 分開的 Session 內容辨識核心

日期：2026-10-09。狀態：Phase 6 首批採用；正式服務／音訊分流尚待接線。

歌曲指紋需要約數秒至十多秒的滑動音訊，不能沿用 STT 的短佇列並等待指紋才開始辨識。現成歌詞或影集 metadata 也不是可信字幕：錯歌、組曲、版本差異或廣告插入可能使一個高分結果採用錯誤文字。

採獨立 resolver 邊界，輸入為有界預處理音訊視窗，ContentIdentifier／FingerprintExtractor／ExistingTextProvider 隔離供應商。識別證據只形成候選與分類；來源仲裁、alignment 與唯一 TranscriptTimeline 仍由後續字幕整合邊界負責。resolver 不持有 HWND、UI callback 或 capture client，不生成假的識別內容。

輸入／輸出使用一格 watch mailbox，單一 caller-owned task 最多一個服務要求。新視窗取代舊要求會取消專用 request token、drop future；stop／source switch 使用獨立 resolver cancellation，不影響同時運行的 Live STT。每次要求 deadline、輸入 age、結果 sequence／Session 與投票 age 都有明確界線。沒有每視窗 OS thread 或未持有的 spawn 工作。

候選以近期三票、信心門檻、差距與短期 hysteresis 保持穩定；可信衝突切入 Mixed／Transition 而非續用已知不可靠的歌曲。只有服務實際提供的信心才進入規則，規則分數不是任意生成的概率。

本批僅提供抽象契約與核心，不預設任何指紋目錄／內容網路服務。選擇 OpenAI／Gemini 做 STT 不會自動授權把音訊或私人視窗線索送給另一家內容服務。後續 adapter 須明確設定、使用安全 transport／憑證與有界 CPU worker，不能把 session-only RAM 變成磁碟快取。正式金鑰依使用者安排在結案互動輸入。
