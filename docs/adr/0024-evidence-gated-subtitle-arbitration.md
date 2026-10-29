# ADR 0024 — 字幕來源仲裁須綁定明確定位證據

日期：2026-10-10。狀態：Phase 7 第三批採用。

文字取得及翻譯成功不能證明它與目前音訊同步。來源仲裁因此獨立於 resolver、文字準備與 renderer，接受實際 LocalizedText／TranslatedTextWindow 加上外部定位證據；不建立第二條字幕時間軸、不以確認歌曲或任意 offset 製造定位成功。

每份定位證據綁定 Session／generation／epoch、文字 query、內容 ID、來源及精確 cue 原文與媒體時間，並將一個完整 cue 映射至已觀測的 capture 範圍。仲裁僅檢查這份映射，不外推其他 cue、播放位置或 drift。正式 ASR fuzzy match／播放定位證據產生仍待實作；測試證據均明確為合成資料。

內容與定位信心使用具名保守門檻 850／900 permille，沒有機率校準宣稱；來源翻譯須達到實際策略品質。通過新鮮度與可用性後按規格文字偏好排序，同級維持目前來源。沒有涵蓋實際觀測位置時回傳 LiveAi，讓後續唯一 TranscriptTimeline 可以補洞。

最多四份候選，固定八個來源／語言優先級 watermark。相同種類的新 query 替換舊候選，滿載的新種類回報 Overloaded；取消／過期後保留 watermark，以拒絕遲到回覆。兩個原始 freshness 分別檢查，不用新定位更新舊文字期限。候選 token 必須獨立，淘汰及 owner Drop 取消並清除本文。決策保留裁切候選及原文件的兩種索引、原始媒體起點與捕捉範圍，避免後續時間軸接線混淆來源位置。

來源／內容轉換、音訊缺口、seek 或重啟須由 caller invalidate，遞增 epoch 並提高 query sequence 下限。舊證據無法重新啟用來源。同步、無鎖、無背景工作及借用決策讓界線保持有界且可測；正式採用與來源切換仍須由後續 Session 管理物件接入原有時間軸，resolver 不直接控制 UI。
