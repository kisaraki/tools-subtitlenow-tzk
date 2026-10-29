# ADR 0032：當前 Final ASR 的電影線索分流

日期：2026-10-11

狀態：採用；正式目錄、視窗 metadata 取得及使用者配置仍待完成。

MovieMetadataResolver 已能交叉核對片名、目錄對白參考、語言、年份及完整媒體片長，但 Live 的 ContentIdentifier 只收到 AudioWindow。不能從字幕歷史抽取一段文字當成當前對白，也不能用到達時間替缺少的 ASR 時間背書。

ContentConfiguration 預設不分流對白。明確啟用時，factory 必須實作 create_with_movie_hints；未支援就回 UnsupportedCapability，不能悄悄退回只處理音訊的服務。MovieCatalogFactory 在每次擷取／重連建立新的 Session 目錄；MovieMetadataSource 是同步、有界的 RAM 查詢，必須核對所選程序及 metadata 新鮮度，不能在這個 callback 做網路或長時間工作。

Live 僅在原有 translation／transcript ledger 接受事件後傳入 Final。MovieHintBuffer 只保留一筆最多 4096 bytes 的對白、原生時間、真正已知語言及取消 token。Partial、未知時間、非法字串與舊 Session 不替代有效提示；sequence、同段 revision 及原生起訖時間水位阻擋較舊結果，清除仍保留水位。缺少時間不推算，超限不截斷。

電影 adapter 先核對實際 AudioWindow 的原始 stamp 與五秒期限，再要求 Final 的完整正時間範圍落在擷取視窗內。對白不足 24 個字母／數字、過期或不在視窗內時，不呼叫 metadata／目錄。metadata 必須逐欄等於該視窗 stamp；檔名只接受 basename，完整片長與年份由明確來源提供，不能用十二秒視窗長度代替媒體 runtime。adapter 不把 PCM 傳給 MovieCatalog。

一個提示 token 代表一份有效對白。替代、音訊缺口、停止與 Drop 同步清除及取消；已開始的查詢同時監看使用者與提示取消，丟棄 Future 也取消目錄子要求。回覆仍通過原 MovieMetadataResolver 的對白參考比對及原始期限，保留目錄實際信心，不把搜尋排名或查詢成功當成分數。較新提示取代查詢中的舊提示會回 Stale，保守撤回該次內容結果，等待下一個有界滑動視窗，Live STT 不等待或重試目錄。

等待目錄時也監看原始捕捉及提示 receipt 的較早期限，到期就丟棄查詢並取消子要求；CPU 比對完成後再次核對期限。不能只在服務回覆後拒收，讓過期要求繼續占用三秒查詢預算。

啟動前的同一可見確認 gate 會列出目前已定稿辨識文字、已設定媒體線索及目的服務。執行中不得改變這項已呈現設定；狀態只含旗標與服務摘要，不包含對白、片名、檔案位置或目錄資料。普通 --ui 仍未配置內容服務。

這個接線沒有選定新的遠端供應商，也沒有提供虛構的識別結果。正式 MovieCatalog、可靠的當前視窗 metadata、字幕／歌詞 metadata source、使用者服務設定與實機驗收仍待完成；它們不因合成測試通過而視為已交付。所有提示與結果只在當次 RAM，沒有新增磁碟內容或相依套件。
