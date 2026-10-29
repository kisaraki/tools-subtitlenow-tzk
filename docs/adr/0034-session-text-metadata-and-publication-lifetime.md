# ADR 0034：Session 文字 metadata 與出版物生命週期

日期：2026-10-11

狀態：採用；正式資料發布者、電影目錄與使用者服務設定待接入，外部歌曲辨識服務依使用者選擇暫緩。

原有歌詞 metadata 介面未帶 Session 身分，可能讓新工作階段讀到上一輪的語言、專輯或片長。字幕已有 Session 核對，但 metadata 更新不能自動撤回已完成的文字；owner 只比較內容 ID，也無法察覺相同內容的資料更新或在缺資料到達後重試。這些風險不能靠重新整理 UI 或延長查詢期限處理。

採用單一 SessionTextMetadata owner 與可共用的唯讀 reader，實作兩個原有 metadata trait。每個 Session 僅保留一筆歌曲及一筆電影，發布端提供完整內容身分與真實來源事實；不建立歷史、辨識信心、語言猜測或網路查詢。歌詞回應新增 StreamIdentity／content_id，兩個正式 binder 都核對後才建立 provider。HTTPS 位置、語言及配對版本仍由來源提供，不能當作獨立驗證。

發布核對新鮮 capture stamp、sequence 及觀測時間水位；移除保留水位，拒收晚到的舊回覆。相同資料保留 revision 與 cancellation scope，使有效查詢及 publication 可以重用。資料改變撤回舊 token，建立新版本；owner 同時比較 revision、內容事實及 provider guard，允許同內容重新綁定。只有修正後的已知語言才能解除 UnknownTextLanguage，不從 STT 設定推論。

Cancellation 是父到子有效性關係。Session 停止、metadata 更新或移除必須取消進行中的查詢，也必須讓已完成 publication、定位及翻譯不可再使用；子 publication 失效不能反向取消 metadata 或兄弟。ResolverCancellation 保持小型單一 Arc handle，平坦收集最多四個相異 ancestor watch channel，cancelled 最多等待五個 future。沒有 relay task、遞迴 token tree 或持鎖 await；超過邊界拒收或 fail closed。

Provider wrapper 保留原子 ExistingTextBundle，啟動前、等待中與回覆後核對 metadata／查詢取消。TextSession 快取 guard，避免在共享 input lock 內呼叫外部 provider；完成出版物的 leaf scope 仍連到 metadata。續期可以替換自身 leaf，但不能重啟已撤回的祖先。停止及 owner Drop 撤回並清空資料，reader Arc 不保留有效 publication。

這個入口可組合正式 LRCLIB／HTTPS adapter，但不自動啟用，也不替代正式目錄或使用者服務設定。一般 --ui 保留既有 Live AI。沒有新增音訊上傳、永久 metadata／字幕／歌詞快取、金鑰讀取或跨供應商 fallback；結案時仍由使用者在固定金鑰介面手動驗收。
