# ADR 0037：有界的 Prepared 全文字幕翻譯

日期：2026-10-11

狀態：採用；Phase 08 第二批。GUI、媒體解碼、ASR、播放與實際音訊定位另列待辦。

Live 原文字幕翻譯只有一至四句且受三秒處理期限與五秒擷取視窗限制。全文 Prepared 翻譯不建立假的擷取 stamp，也不延長 Live 有效期限；採用獨立的文件工作生命週期，逐要求有三十秒上限，整份文件則由 Session 的停止／取代／Drop 取消。

PreparedTask 提供明確的 run_with_translator 入口。caller 傳入本次選定及已告知使用者的 Translator、品質及可選的本次詞彙表；不讀取憑證、不自行選服務或做跨供應商 fallback。中文最高優先來源仍只做本機臺灣本地化，不驗證或呼叫不必要的供應商。原有 run() 在來源字幕缺少翻譯服務時繼續回報 TranslationRequired。

解析與最終本地化／合併在共用有界 CPU lane；原媒體的唯讀 handle 於翻譯期間保留，CPU 許可在網路要求前釋放。翻譯每批最多等於服務宣告的 max_concurrent，上限四；一批全部完成並還原序號後才排下一批。HttpTranslator 的共用 Semaphore 與速率限制仍限制所有 caller，不能以多個協調器乘上網路並行量。每個 provider future 由 caller 輪詢與持有，不另產生 detached 工作。

每句沿用文件的 Session、序號與原始媒體時間。共用 TranslationRequest 新增可選 PreparedContext，明確分開前文來源、後文來源與相鄰草稿；Live 要求不帶這個欄位。前一批已完成的譯文及同一份 SessionGlossary 隨要求傳遞，詞彙表 accessor 核對完整 Session 身分。所有來源、前後文、草稿及詞彙都放在不受信任的 JSON 資料中，系統指示只要求翻譯當前 cue，不能把脈絡當新字幕。

Fast 使用一個前後文窗口並只做 Draft；Standard 使用三個，High 使用八個，後兩者先完成整份草稿，再做整份 Review。校訂可查看跨批次的相鄰原文及草稿，並使用先前已完成的校訂譯文；如此維持有界並行與前後文，品質模式影響實際策略。這是有界脈絡分析與一致性校訂，不宣稱已完成自動全文專名抽取或自動詞彙表建立。

每個要求持有取消 token 的 Drop guard，取消、逾時、panic 或任何批次失敗都停止繼續排程並撤回尚在進行的要求；服務在取消後仍回傳成功也被拒收。單句譯文要求非空、合法控制字元且最多 8192 bytes；每個完整 pass 最多四 MiB。最終 CPU 合併再次核對 cue 數量、原文／時間結構、臺灣片語轉換後的單句與總容量。只有整份成功，才產生可由目前 owner 明確 apply 的 Ready 文件，沒有部分 Ready 或自動播放。

Prepared HTTP 要求的 transport timeout 為三十秒；原有 Live 三秒 transport 期限保留。High 最多八句前文、八句後文、八組已完成譯文與十六組鄰近草稿，單一要求 payload 明確限制四 MiB，以涵蓋資料 JSON 放入 user text 後再序列化外層 JSON 的兩層 escaping；Live 仍限制 256 KiB。實際服務 token／配額限制仍可能回報錯誤，不以本機容量驗證宣稱所有最大文件都可由雲端接受。

原文文件、兩個翻譯 pass、要求脈絡及媒體身分只留本次 RAM。沒有內容快取、字幕匯出、原始音訊寫入、URL 下載或新暫存檔。PreparedDocument 保留原文為 ExistingSubtitle，不把未對齊的文件當作播放器定位證據；通知、GUI 及同一 TranscriptTimeline 的播放接線仍需後續實作。
