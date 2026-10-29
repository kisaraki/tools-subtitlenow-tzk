# ADR 0040 — Prepared 媒體時間與有界音訊前處理

日期：2026-10-12  
狀態：採用於 Phase 08 第五批；Prepared ASR、GUI 音訊接線及播放尚未提供。

## 問題

本機 WAVE 解碼保留 PCM16 的原生取樣率、聲道與媒體時間，但後續語音辨識需要明確的單聲道服務格式。現有 Live 的 NormalizedAudio／StreamingResampler／PreprocessWorker 固定從 48 kHz 音訊開始，使用 AudioStamp、Instant 接收時間與 MAX_AUDIO_AGE 檢查；把離線音訊裝成新的 capture stamp 會混淆媒體時間與即時期限，也不能處理 44.1 kHz 等非整數取樣比例。

一般 decode_local_audio 的 callback 只能借用當前 PCM block，沒有 EOF 完成步驟。Centered FIR 需要有限 lookahead，輸出二十毫秒分片也可能留下不足一整塊的尾段；把 state 搬到外部共享 PCM 容器，會增加生命週期、取消與容量管理的風險。

## 決策

新增獨立的 PreparedAudioProcessor 與 prepare_local_audio library 入口。輸入是既有 DecodedAudioBlock；輸出 PreparedPcmBlock 保留完整 Session／generation、checked 序號、媒體時間及明確 discontinuity，固定為 PCM16 單聲道，取樣率只能明確選擇 16 kHz 或 24 kHz。每次 consumer 借用最多二十毫秒，也會交付較短尾段；PCM 不具 Debug、Clone 或持久化能力。

一至八聲道使用 i64 累加後等權平均，保留平均值的小數直到最終 PCM16 量化。不推論 speaker layout，不宣稱這是具 surround channel mask 的聲道配置策略。相同輸入與輸出取樣率直接量化，跳過 FIR 與係數表配置。

不同取樣率採 257 taps、256 phases 的 centered Blackman windowed-sinc FIR，每個 phase 正規化以保留 DC。截止頻率是原生取樣率下的 `0.45 * min(1, output_rate / input_rate)`，預留有限過渡帶。係數表固定 526,336 bytes，ring history 固定 2,056 bytes，lookahead 為 128 個 native frames；不依媒體長度或 rational denominator 增加容量。Phase table 取整只近似 DSP 取樣位置，誤差小於一個 native frame 的 1/256；輸出位置與數量仍以 checked 整數 rational frame 計算。

降採樣須降低截止頻率，理想無限長 sinc 需加 window 才能成為有限 FIR；設計原理由 Julius O. Smith III 的 [Windowed Sinc Interpolation](https://dsprelated.com/freebooks/pasp/Windowed_Sinc_Interpolation.html) 支持。257 taps、256 phases、128-frame lookahead 與 0.45 截止比例是本專案的固定工程政策；測試頻譜門檻不是該章節提供的品質保證。

每個連續區段保留實際起點與累積 native／output frames。Native block 時間只能與該 clock 相差最多一微秒，不以逐 block 重設起點掩蓋累積漂移。DecodedAudioGate 仍拒絕舊身分、序號、格式、向後或重疊時間；前進缺口會先完成前區段的 FIR 與尾塊，再重新建立 filter／clock，下一個輸出明確標示 discontinuity。不跨 gap 混合音訊，也不補合成靜音。

完成區段時，首尾 sample extension 只參與 filter 邊界計算。輸出總數精確為 `floor(native_frames * output_rate / input_rate)`，尾端可能比原區段少於一個 target frame；不交付區段外的額外 output，也不將 FIR lookahead 加到媒體時間。完全不足一個 target frame 的音訊回報明確 InsufficientAudio，不偽造 PCM 或辨識成功。

Decode worker 加入私有 EOF-aware consumer。它只容許明確有界的 FIR／pending output state；原有 decode_local_audio 的公開借用 callback 契約保持不變。Native decoder、下混音、FIR、EOF flush 與資源銷毀都在同一個 blocking worker，持有同一份 process-wide CPU 許可。此 lane 依 available_parallelism 決定，最多兩項工作，過載立即拒收，不建立音訊佇列或每 block 工作。

未 poll 的 Future Drop、執行中 Drop、外部停止與 consumer 錯誤皆取消 scope。已開始 worker 保留許可到真正退出；native 同步呼叫須返回檢查點後才能合作取消。Processor 與 DSP 在輸入、初始化、處理、flush 及 callback 前後檢查取消；失敗、取消或完成後不可繼續使用同一 processor。只有完整有效 EOF 與 flush 成功才返回 PreparedAudioSummary，失敗不交付部分摘要。

## 結果與限制

新入口只從明確指定的本機 RIFF/WAVE 產生短、RAM-only 的 mono PCM block 與不含音訊的摘要。沒有新增 crate、unsafe、元件安裝、產品暫存檔、音訊匯出或內容快取。Consumer 只能做短而有界的同步 CPU 工作，不得保留 PCM 複本、連網、寫檔、操作 UI 或等待佇列空位。

此摘要不是 ASR、字幕 Ready、翻譯、媒體配對或播放定位證據。沒有 GUI 音訊接線、服務連線、音訊上傳、MP3／AAC／影片容器或 URL 讀取；現有外掛字幕優先路徑不額外解碼。Live 的時間／期限與重採樣行為保持獨立。

有限 filter 的 passband、stopband、upsample image 與邊界行為須以測試核對，不由理想 sinc 名稱推定品質。測試涵蓋代表與非整數比例取樣率、分片一致、DC／極值、floor 尾段、128-frame lookahead、固定容量、時間／gap／漂移、取消與真正的本機 WAVE worker；實際指令、數字與尚未驗收項目另記於 [Phase 8](../phase-8.md)。Phase 09 尚未具備啟動條件，真實 OpenAI／Gemini 金鑰仍於結案由使用者在固定遮蔽介面手動輸入。
