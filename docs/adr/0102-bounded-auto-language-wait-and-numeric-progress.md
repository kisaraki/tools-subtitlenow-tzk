# ADR 0102：Auto 語言等待上限與純數值進度

日期：2026-10-26。狀態：採用，Phase 11 人工回報診斷修正。

## 證據與原因

使用者在同一 Chrome 英文來源確認手選英文有字幕，Auto 沒有，狀態僅正在聆聽／等待字幕。這將問題限縮於 Auto 路徑，但沒有正式服務回應，因此不能斷言服務是否未送轉錄或缺少語言 metadata。

新增本機回歸先實際失敗：0 成功／1 失敗（9.04 秒）。同一未知語言 partial 每三秒更新一次，一般回應計時會被重新整理，九秒後仍未過期；既有 partial 的容量有界，總等待時間卻沒有獨立上限。這是確認的本機缺口，不當作此次雲端根因。

## 決定

Gemini Auto 從本段第一個未核對語言的 partial 起算八秒總期限，沿用既有 FINAL_TIMEOUT 的數值但獨立保存起點。後續 partial、一般回應或輸入中斷 flush 不續期；同段有效五語回報才能退休等待，真正定稿後下一段重新核對。期限後的遲到語言／本文拒收，回報非重試 LanguageNotReported，仍不默選英文、放行未知語言或變更供應商。沒有任何轉錄的情況不套用這個期限，避免把靜音直接當成辨識失敗。

每個 SpeechSession 另外持有固定六個飽和原子計數及一個語言等待旗標：辨識端取出音訊包、成功送出音訊包、服務資料訊息、partial、final、交付辨識輸出佇列。沒有音訊、本文、來源標題、網址、金鑰、內容雜湊或播放時間；不新增 channel、worker、timer、鎖或依賴。音訊計數在驗證通過後增加，送出計數須等 send 成功；服務 ping／pong 不冒充轉錄。計數是非同步取樣，不作完整因果快照、語音分類或播放定位證據。

Auto 的 Live UI 使用既有每秒 report tick 取樣，經同一 latest-only UiFrame 發布；未改動字幕快照或自動跟隨。顯示尚無音訊、尚未送出、已送出但沒有服務轉錄，或轉錄待語言確認。相同數值不重送，舊 Session／generation 無法更新新畫面。錯誤保留最後一份純數值取樣並標為已停止；停止、重連、換來源／新連線清除。主頁與既有可調整狀態視窗共用同一狀態字串，不另存本文。

## 驗證與限制

測試涵蓋真實九秒總期限、同段語言解除、跨段重新等待、遲到拒收、純數值計數／飽和、舊 Session 拒收、錯誤保留／新連線清除及字幕 Arc 不變。本機 WebSocket 使用合成音訊及合成本文，確認未知 partial 不交付、取得有效語言才交付，取消可正常結束。此結果不能替代真實 Auto 測試。

沿用官方 [Live Transcribe](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe) 的空 languageCodes Auto 與 partial／final 欄位，以及 [Live API](https://ai.google.dev/api/live) 的 languageCode 契約。未改請求模型或語言提示、擷取、音訊期限／容量、一般 final 期限、有限復原、翻譯排程、ExactCueVerifier 或 LivePlaybackClock。實際命令、失敗、部署與人工結果見 [Phase 11 診斷](../phase-11-gemini-diagnostics.md)。
