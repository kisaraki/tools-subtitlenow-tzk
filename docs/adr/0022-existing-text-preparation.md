# ADR 0022 — 分開歌曲查詢、文字本地化與字幕採用

日期：2026-10-10。狀態：Phase 6 補件與 Phase 7 首批採用。

已確認歌曲不等同歌詞與當前播放時間相符。LRCLIB 又不回傳來源語言，將未知語言默認成英語或臺灣繁中會造成錯誤處理。因此 LiveContent 只在明確配置時建立 LiveLyrics，由每個 Session 的 binder 取得已有 metadata；LRCLIB 配置要求已知語言，整首時長不可用音訊視窗長度代替。缺少語言回報安全型別錯誤，不發出查詢。

LiveLyrics 持有 ExistingTextSession 與 caller-polled future。歌曲 ID／曲名／歌手相同時沿用 binding 與服務速率限制，不因 offset／信心更新重抓；歌曲 metadata 變更、Transition、Unknown、音訊缺口或錯誤清除查詢選擇。同步 factory／binder／supports 及非同步 provider panic 都有邊界保護。失敗不取消 Live STT、不替換服務。獨立 UI disclosure 納入同一啟動可見性確認；狀態更新沿用現有 snapshot。

Phase 7 的本地化回傳原文加 zh-TW 候選，與採用字幕分開。中文轉換在一個有界 CPU lane 執行，原文與譯文索引一致，保留所有時間值及 None。使用既有通用 OpenCC 臺灣詞彙規則，與 HTTP 翻譯共用實作；來源字幕需要翻譯時明確回報 TranslationRequired。沒有新增字幕時間軸或假定播放從零開始。

最終採用仍須由後續唯一 source arbiter 驗證 Session、freshness、信心與實際播放定位，再整合到原 TranscriptTimeline。現階段 UI 不顯示原始候選，沒有把轉換成功當作內容驗收或正確定位。所有內容只在 Session RAM，沒有快取、檔案、匯出、額外金鑰讀取、遙測或供應商 fallback。
