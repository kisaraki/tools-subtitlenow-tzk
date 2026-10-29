# ADR 0101：Gemini Auto 語言中繼資料與可調整狀態視窗

日期：2026-10-26。狀態：採用，Phase 11 人工回報修正。

## 問題與證據

使用者在 Gemini Transcribe Live／Auto 播放英文，仍收到五語限制錯誤；另要求狀態與錯誤欄位可以調整寬、高。沒有收集真實服務本文或原始回應，因此不能由錯誤文字斷定雲端失敗根因。

本機三項新增回歸先重現：Auto 送出五語提示清單、合法英文地區標籤被拒，以及同一句 partial 已回報語言但 final 省略標籤時被拒。官方 [Live Transcribe](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe) 說明空 languageCodes／省略代表自動偵測，非空值是語言提示；[Live API](https://ai.google.dev/api/live) 的 languageCode 使用 BCP 47。

## 決定

Gemini Auto 送出空 languageCodes；明選語言仍送出單一 locale。服務可能偵測更多語言，但本機只放行英、日、法、德、韓，不把服務提示清單誤當硬性白名單。語言標籤接受這五個 primary language 的大小寫、一般 script／region／variant 形式，限 64 ASCII bytes、固定上限 variant 槽且拒絕重複 variant；不是完整 BCP 47 extension／private-use parser。不做文字內容猜測，不新增來源語言。

只在同一服務 utterance 內保留已核對的語言，讓省略標籤的後續 partial／final 使用該 metadata。真正 final 退休 utterance 後即釋放，不套用到下一段或另一個 Session。明確回報不支援或格式錯誤的標籤仍拒收，不能用前一個有效標籤掩蓋。Auto 的無語言 partial 保持既有有界等待；無法核對 final 時以 LanguageNotReported 顯示手選五語提示，不自動改英文、模型、供應商或重試。原 Gemini 無可靠音訊關聯的時間仍為 None，不能製造播放定位證據。

主頁狀態標題改為「狀態／錯誤（展開）」按鈕，選單亦可開啟單一 modeless 原生視窗。視窗使用可調整外框，獨立調整寬與高、最小尺寸及 DPI 字型／內框布局；關閉只隱藏，重開保留本次尺寸。狀態文字沿用既有主頁快照，沒有字幕本文、第二個時間軸或後端操作。主頁預覽仍保留，設定頁不新增重複框。主視窗結束同步銷毀子視窗並回收獨立字型，不保存狀態或視窗本文。

## 界線與驗證

不改擷取、佇列容量／期限、重連、翻譯排程、ExactCueVerifier、LivePlaybackClock 或字幕歷史／手動清除。無新依賴、金鑰讀取、內容保存或正式網路要求。測試使用本機合成協定、本機 WebSocket 及隱藏原生視窗；不替代 Chrome 英文 Auto 的真實驗收。實際命令結果、失敗與部署記於 [Phase 11 診斷](../phase-11-gemini-diagnostics.md)。
