# Phase 2 實機測試操作與驗收界線

更新：2026-10-08。限定 Windows 10 19045 的 Phase 2；STT、翻譯與字幕 UI 不在本次測試範圍。結果見 [Phase 2](phase-2.md)。

## 一般與短時間原生測試

一般檢查不需要音訊裝置：

    cargo fmt --check
    cargo clippy --locked --all-targets --all-features -- -D warnings
    cargo test --locked
    cargo build --locked

十項短時間原生／資源測試逐一執行，需要本機播放裝置；雙端點測試需要兩個已啟用的播放端點。測試會播放約 -30 dBFS 的 RAM 合成音，不改變整體音量、不保存 WAV／PCM：

    cargo test --locked --lib -- --ignored --skip thirty_minute --skip manual_application --nocapture --test-threads=1

這組包含程序隔離、來源退出、100 次與三批共 300 次重啟、探索生命週期、雙端點、慢消費端與背景程序異常。不能把有封包當成隔離成功。

## 30 分鐘連續擷取

    cargo test --locked --lib native_process_capture_thirty_minute_soak -- --ignored --nocapture

測試使用自有獨立程序輸出 RAM 靜音，在 10 秒暖機後，每 60 秒取樣父程序與擷取背景程序的 private bytes、handle、thread 及累計 CPU 時間。CPU 百分比以單一核心為 100%，不是整台電腦的百分比。來源與程序 handle 均由測試保留；只查詢資源數據，不擷取記憶體傾印。

迴圈檢查 Session、sequence、有限樣本、靜音容許量、五秒內持續有新 PCM，以及佇列不超過 12 格。取出前已超過 250 ms 的資料不得交付；取出後執行緒可能被 Windows 暫停，因此「呼叫起點的音訊年齡」與「回傳後觀測年齡」分開記錄，不把後者當成硬即時保證。

暖機後的資源回歸警戒值：private bytes 增加不超過 8 MiB、handle 不超過 4 個、thread 不超過 2 個。這些是此測試的具名回歸門檻，不是所有 Windows 元件、應用程式或完整 working set 的產品上限。停止後須清空佇列，並以持有的同步 handle 確認自有背景程序已退出。

Windows 會鎖定正在執行的測試 EXE。若測試中需要修改或重新建置其他測試，加入 --target-dir target/phase2-verification；不要重寫或終止正在長時間驗證的 EXE。該目錄是已忽略的建置產物，不是音訊快取。

## 應用程式測試來源

    node tests/manual/audio-source-server.mjs

此開發工具只監聽 127.0.0.1:19405，最多八個連線。HTML 以 Web Audio 在 RAM 產生測試音；播放器用的 /tone.wav 回應也在 RAM 產生，回傳 Cache-Control: no-store，不建立音訊檔。工具不屬於正式產品後端，測試結束須停止。

瀏覽器測試頁為 http://127.0.0.1:19405/?hz=1730，亦可選 440、880 或 1320。1730 Hz 對應 100 ms 分析視窗的整數頻率格，但仍須先量測既有內容的暫停基準，不能假設沒有干擾。播放器可開啟 http://127.0.0.1:19405/tone.wav?hz=1320；這是三十秒 RAM 合成音，讓人工操作與擷取探針有時間重疊，需在播放中進行擷取，不能把已播放完畢算成程式無法擷取。

先以 --list-sources 確認當下實際音訊程序 PID。瀏覽器音訊 helper 可能被多個分頁共用；若包含既有內容，必須由使用者明確同意測試範圍，或先停止其他內容。不能宣稱單一分頁隔離。選取 PID 仍須通過目前原生探索的獨立程序驗證，沒有偷偷改選其他 PID 的回退。

PowerShell 範例：

    $env:SUBTITLENOW_TEST_SOURCE_PID = '當下確認的 PID'
    $env:SUBTITLENOW_TEST_EXPECTED_HZ = '1730'
    cargo test --locked --lib manual_application_source_tone_is_verified -- --ignored --nocapture

測試顯示所選 PID、範圍與預期頻率；在 100 ms 視窗中辨識頻率，只輸出振幅摘要。沒有輸出或保存原始音訊。測試 PID 應隨每次播放重新確認，不照抄驗收紀錄中的歷史 PID。

暫停測試音後，以 SUBTITLENOW_TEST_TONE_ABSENT=1 做頻率消失對照；獨立安靜來源也可將預期頻率設為 0 檢查靜音。若既有內容也含有相同頻率，單次頻率正例不能證明測試音的開關，應記錄條件不成立，不能提高閾值掩蓋問題。未提供 PID／頻率時，此手動測試明確失敗，不假裝通過或偷偷選取來源；因此不能無條件執行所有 ignored 測試。

## 需要人工或其他環境的項目

逐項記錄應用程式版本、實際 PID／程序樹、播放／暫停／結束、排除干擾及當次結果。VLC、PotPlayer、Chrome、Edge、Spotify、Media Player 與遊戲各自驗收，單一來源不能代表同列所有程式。

本機雙端點測試只操作測試程序自己的串流，沒有變更系統預設裝置；它不等於實體拔除裝置或重啟 Windows Audio 服務。系統服務測試會影響其他程式，不能在連續擷取測試中自動操作。實體拔除、服務中斷、Windows 11 及未提供的應用程式，需安排人工或另一台環境後另行記錄。

人工裝置／服務驗收操作：

1. 結束其他硬體測試，選定僅播放合成音的來源，記錄 OS 修訂號、播放端點與來源 PID。執行 $env:SUBTITLENOW_LOG='DEBUG'，再執行 .\target\debug\subtitlenow.exe --capture-pid 實際PID；每秒的 captured_frames 與 queue depth 僅為數值診斷，不記錄音訊內容。
2. 確認 Running 與持續新 PCM，再由操作者拔除所選實體裝置，或在獨立測試時段中斷 Windows Audio 服務。每輪只做一種變動，記錄變動時間及實際操作，不由測試腳本停止系統服務。
3. 記錄是否仍有新 PCM、明確錯誤／停止原因與停止耗時；靜音封包不能證明來源已恢復。若回報停止，確認沒有自有 capture helper／worker 遺留，並清空舊 Session 資料。
4. 操作者復原裝置／服務後重新探索，重新選取當下 PID，以新 Session 啟動。須重新取得合成音正例，並確認舊 Session 結果不進入新 Session；失敗時保留錯誤摘要，不自動切換來源。

上述是待執行程序，並非已通過的測試結果。

UI 工具失效、核准審查拒絕啟動命令、程式未安裝、測試條件不足與程式本身錯誤必須分別記錄。不要把操作受阻寫成相容性失敗，也不要把只有收到 PCM 寫成完整相容性通過。
