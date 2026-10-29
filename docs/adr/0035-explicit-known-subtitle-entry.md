# ADR 0035：明確指定的本次影片來源字幕

日期：2026-10-11

狀態：採用；全球電影目錄及實機／雲端驗收另列待辦，外部歌曲辨識服務仍暫緩。

一般電影搜尋結果不能直接滿足 MovieCatalog 的獨立對白與實際分數契約。搜尋排名或熱門程度也不能代替辨識信心。Phase 7 已有正式 HTTPS 字幕 adapter、完整 ASR 定位及共用時間軸，卻缺少一般 GUI 可設定的來源；採用使用者明確指定已知片名及來源字幕的入口，讓這條管線可實際使用，不假裝已有全球目錄。

UI 提供本次 RAM 設定，接受片名、HTTPS 網址、明確格式及五語之一。套用不連線，開始才使用不可變快照；設定在停止、來源切換及關閉清除。簽名網址遮蔽且不進設定或日誌，主機欄只顯示來源 host／port。沿用固定 OpenAI／Gemini 金鑰介面，不要求額外辨識服務憑證。

一個 Session owner 同時提供 MovieCatalog 與 ExistingTextProvider，使用真正 TimedSubtitleProvider 下載有界來源字幕。Catalog 把當前 Final ASR 與整份實際字幕比對：至少二十四個有效字元、完整正規化相等、全軌唯一，才回傳由一致字元數計算的分數與原始下載 cue。片名由使用者明確指定；年份、完整片長及配對版號保持未知。不把 Query ASR 回送為參考，不偽造信心或媒體身分。

這個分數是文字一致率。後續仍由 MovieMetadataResolver 核對可靠所選程序標題、來源語言及獨立對白；字幕真正顯示前，沿用更嚴格的標點、完整 cue、時間及唯一性定位。各 renderer 仍消費單一 TranscriptTimeline。任何不符保持 Live AI，不以另建管線或猜測配對解除限制。

Catalog 與字幕 provider 共用一份 Session RAM document，避免同一網址重抓。單一要求及 try-acquire 不累積佇列；CPU 工作依 available_parallelism 限制到最多兩項，已執行的工作在 Future Drop 後仍持有許可並合作停止。資料只在完整驗證及回覆有效時才發布，取消／異常不留下快取。Registry 最多一個 Weak owner，跨 Session 不共用 document，也不讓舊回呼取消新工作；最後 owner Drop 撤回出版物並清空 RAM。

本機核對與遠端文字下載分開揭露。指定網站只接收文字 GET，ASR／音訊不送到字幕網站；雲端語音辨識與翻譯仍由使用者選定的服務處理。既有內容服務配置不得被此入口靜默取代。這個入口不是任意影片下載器，不取得影音、不保存字幕、不新增帳號或自動跨服務 fallback。

Phase 8 不需要全球電影目錄才能處理本機媒體，因此這個可用的已知字幕入口與既有核心通過驗證後，可進入 Prepared Mode；外部目錄與實機／雲端驗收仍須明列未完成，不因新 Phase 啟動而列為通過。
