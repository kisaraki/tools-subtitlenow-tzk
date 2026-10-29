# ADR 0090：Process Loopback 零裝置位置的擷取時鐘

日期：2026-10-24。狀態：採用，驗證結果見 [Phase 11 診斷紀錄](../phase-11-gemini-diagnostics.md)。

## 原因

Gemini 英文初始化成功後，使用者刻意選擇 Chrome 英文來源，收到辨識端取出音訊時的不連續標記。原生自有靜音程序重現：100 個 480-frame 封包的 QPC 每次前進 100,000 個 100 ns ticks，但取樣位置持續為零；原程式因位置不等於前一塊結尾而產生 99 個後續不連續標記。舊原生測試只確認音訊交付與資源，沒有驗證這項連續性；新增明確原生回歸最初失敗，不能將舊長測當成時鐘正確的證據。

[Microsoft GetBuffer 文件](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer) 說明 QPC 回覆採 100 ns 單位及原生錯誤旗標。[Chromium 的 Windows 擷取實作](https://github.com/chromium/chromium/blob/main/media/audio/win/audio_low_latency_input_win.cc) 明確區分 Process Loopback 並對其裝置位置檢查為零，與本機觀測一致。不能把一般硬體端點的位置契約直接套用至此虛擬介面。

## 決定

只在 Windows Process Loopback 的原生 packet adapter 增加每次擷取獨立的 `LoopbackClock`。裝置位置維持零時，從實際 GetBuffer 回傳的 PCM frame 數計數；必須同時核對前後原生 QPC 間隔符合前一包長度，才交付連續的擷取相對位置。整數 QPC 端點取整容許差僅一個 100 ns tick，小於單一 48 kHz frame；不採用毫秒級模糊容差。

QPC 真實跳躍保留為位置缺口並標記不連續；原生 DATA_DISCONTINUITY 仍原樣保留。時間戳記錯誤、缺失、零值、重複或倒退不以本機收到時間或純 frame 計數偽造連續；恢復時也保留邊界標記。一旦觀測到非零原生裝置計數器，沿用原生位置與既有下游檢查，不再用零位置處理去掩蓋其重設。

此位置只代表已觀測的擷取音訊座標。它不是媒體播放時間、seek、字幕採用或 alignment 證據；不從牆鐘、歌曲長度、模糊分數或播放器狀態推算。現有 QPC 年齡、250 ms 期限、序號／queue 缺口、Session、取消、重採樣、仲裁與多觀測定位規則維持。沒有補靜音、回放舊音訊或放寬 STT 拒收。

狀態固定大小、堆疊配置、每包常數時間，沒有新鎖、配置、I/O、工作執行緒、unsafe 或相依套件。資料仍走原本有界 RAM／helper IPC；不新增內容或時鐘歷史保存。

## 驗證

純邏輯驗證零位置連續、QPC 缺口、原生 glitch、未知／重複／倒退時間、取整界線、非零原生計數器及零長度拒收。原生自有靜音程序同一回歸須由原先失敗轉為通過，並確認資源釋放；另回歸既有 packet acquire/release、擷取隔離與程序樹案例。實際結果另行記錄，不將本機修正直接當成 Chrome 或 Gemini 全面驗收。

```powershell
cargo test --locked --offline win32::loopback_clock::tests --quiet
cargo test --locked --offline win32::soak_tests::native_process_capture_clock_metadata_is_continuous -- --ignored --exact --nocapture
cargo test --locked --offline win32::soak_tests::native_process_capture_active_clock_metadata_is_continuous -- --ignored --exact --nocapture
cargo test --locked --offline win32::tests::native_process_capture_isolates_tones_and_includes_audio_children -- --ignored --exact --nocapture
```

三項原生案例只建立自有程序，分別使用靜音或低振幅合成音；新增 active 案例檢查一千包及九次播放／靜音切換。它們不擷取使用者播放內容、不存音訊、不使用雲端。不要改成執行所有 ignored 測試。
