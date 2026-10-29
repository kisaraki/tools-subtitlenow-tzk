# ADR 0018 — 電影 metadata 必須交叉核對獨立線索

日期：2026-10-09。狀態：Phase 6 第五批採用；實際 MovieCatalog 與正式 GUI 接線尚未完成。

播放器／視窗標題或檔名只能提示候選，不能證明目前正在播放該作品。既有 MovieCandidate 契約也不能自行證明線索互相吻合。本批加入 MovieMetadataResolver，實際比對目前視窗／檔名的片名線索與目錄中該作品的對白參考；沒有全球電影音訊指紋假設，不直接採用字幕。

MovieQuery 帶同一 Session／generation／sequence、目前音訊視窗的時間，以及 caller 明確提供的視窗標題、basename、當前 ASR 對白與可選的語言／年份／實際整片時長。不能用舊 Session 對白、歷史全文或十二秒視窗代用片長。檔名拒絕目錄分隔符、冒號／URL、控制字元。缺少片名線索或對白少於二十四個英數／Unicode 字元時直接回傳 Unknown，不查詢目錄；這是保守的證據門檻，沒有宣稱辨識準確度。

MovieCatalog 必須由 caller 明確提供，表示可取消且具備候選作品實際對白參考的資料來源。搜尋排名／人氣不能當成辨識信心，回覆不能只重複 query 對白當成目錄證據。核心沒有正式 provider、虛構作品、always-success fallback、金鑰讀取或網路連線。實作與測試只新增真實的文字／metadata 比對演算法；測試目錄全部在 cfg(test) 明確宣告為 mock。

比對採 Unicode 小寫、標點／空白正規化及完整 token 邊界；支援目錄明確提供的片名別名。視窗片名和檔名可能是相同 metadata 的複本，因此兩者相加也不能代替獨立對白；至少其中一者與足夠長的 ASR 對白同時相符。已提供的年份、語言與片長必須在回覆中存在且相符，片長僅容許五秒差異。缺少資料、配音版本或不同剪輯會保守地回到 Unknown，不能推論字幕時間軸相符。

每個回覆最多四部作品，各作品最多四個片名別名、四個 4096-byte 對白摘錄；沿用 1024-byte metadata 上限，沒有全文字幕查詢或大量 CPU 工作。候選信心至少沿用既有分類門檻 700／1000；返回原始目錄信心，不因線索數量增加而加分。多部相符作品全部保留並依原信心／ID 排序，不強制選第一部或建立已識別字幕。

每個 resolver 只允許一個 lookup，並行直接回報 RateLimited，沒有排隊、自動 retry 或 detached task。lookup 最多三秒；query 進入和回覆後均檢查五秒 freshness、工作階段與順序。回覆必須完整帶回相同 stamp；不同 Session、時間、序列、重複作品或超限資料拒收。每次 request 使用獨立取消 token；取消、timeout、future Drop 或成功結束都回收 request，且不取消 caller 的 STT／Session token。

結果仍是 ContentEvidence／MovieCandidate，能交由既有 CandidateTracker，但沒有 renderer／TranscriptTimeline 寫入。Phase 7 才處理字幕來源、本地化、定位與仲裁。實際服務必須另外完成有界協定 adapter、資料品質與涵蓋率驗收；本批不能宣稱實際電影辨識、模糊對白或播放定位已通過。
