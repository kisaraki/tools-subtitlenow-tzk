# ADR 0031 — Live 字幕管理與共用候選處理

日期：2026-10-11。狀態：Phase 7 第十批採用。

Phase 7 已有 HTTPS 字幕 adapter，但正式 Live 只管理歌曲歌詞。查詢字幕需要可靠的內容選取、出版物撤銷與啟動前服務提示，不能將一份成功下載的本文直接當成定位或新增另一個時間軸。因此加入可明確配置的 LiveSubtitles，沿用 ExistingTextSession 及同一 LiveTextPipeline／TranscriptTimeline。

電影／影集查詢使用辨識服務原有分數與多個線索。從最多四個合法候選中選最高分，不假設輸入排序；信心至少為原仲裁門檻 850／1000，與其他候選的差至少為既有 100 門檻，並須具備 Dialogue、RemoteMetadata 及 WindowTitle 或 Filename。這些是保守工程條件，不是已校準機率或真實內容驗收。弱、模糊、Unknown 或 Mixed 結果同步撤銷舊出版物；非法 metadata、重複 ID／signal 或不合法年份回報固定錯誤。舊或跨工作階段的 stamp 不得清除有效現任。

SubtitleServiceFactory 每個 Session 建立一個 SubtitleMovieBinder，只將已選取的電影候選交給 binder。相同 ID／片名／年份保留同一 binding，不因分數或輪詢重新建立服務、下載字幕或延長已交付候選期限。新的明確可靠確認才可對 RAM 文字建立新 query；信心沿用服務實際值，下降時撤銷舊 parent。停止、來源切換、缺口、故障、過期及 Drop 同步退休外部 lease，不等待非同步工作再次輪詢。factory、binder、supports、lookup 與 future 的 panic 都在原有邊界處理，失敗不自動重試或取消 Live STT。

正式 HTTPS 組合使用 SubtitleMetadataSource 讀取已知 Session metadata，禁止同步網路及語言／版本猜測。metadata 必須帶出目前 StreamIdentity 與所選電影 ID，binder 核對後才建立 TimedSubtitleProvider；舊 Session metadata 不得建立新 Session 的 provider。字幕位置、語言及配對版本仍須由實際來源提供，這些欄位是來源聲明而非獨立驗證。metadata、簽署網址與本文不加入 Debug／Serialize、一般設定、日誌或持久內容。

ContentConfiguration 可附加字幕及歌詞兩種設定。settled MovieOrShow 與 Music 選取互斥，各自持有最多一份出版物，ready_text 只交付目前仍有效的來源。兩者共用一個候選 task、所選 Translator、仲裁器與原有時間軸，沒有第二條 STT 或視圖專用翻譯管線。新的來源會取消舊準備並清除脈絡；字幕採用仍須通過完整 ASR／cue 證據及實際擷取位置，查詢成功不直接顯示。

UI 新增獨立的字幕 scalar 狀態，使用相同的有界文字狀態模型。prepare_subtitles 與既有來源／內容／歌詞服務共用可見確認版本；兩個 GUI 啟動入口都在確認後才建立 Live 服務。字幕讀取提示不聲稱將片名、對白或原始音訊送往字幕來源。狀態依目前 candidate source 與 display_origin 投射，避免字幕處理顯示成歌詞處理。開始、重連、停止、關閉及 publisher Drop 重設／停止狀態，狀態更新不替換共用 TimelineSnapshot。

本批完成可設定 Live／GUI 的字幕管理接線與正式 HTTPS factory，未提供預設內容辨識服務、正式 SubtitleMetadataSource、目錄或使用者服務設定資料。一般 --ui 仍傳入 None，不新增目的地或讀取額外金鑰。所有新增來源與 native fixture 都是合成測試；沒有以隱藏視窗測試宣稱實際可見性、DPI、播放器同步或延遲已驗收。Prepared 與進階 fuzzy／seek／drift 不在本批，金鑰與真實服務維持結案時固定介面手動輸入。
