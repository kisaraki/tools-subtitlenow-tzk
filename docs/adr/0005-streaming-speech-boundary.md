# ADR 0005：共用串流辨識邊界與明確滿載復原

日期：2026-10-09。狀態：採用於 Phase 3 第一批核心。第三批雲端管線與重建補充見 [ADR 0006](0006-selected-provider-streaming.md)，真實服務驗收仍待憑證。

## 原因

使用者指定 OpenAI 與 Gemini 可選。兩種服務的採樣率、Partial／Final 格式與 turn 結束機制不同；讓 WASAPI 直接依供應商組封包會破壞既有 realtime 邊界，也會使切換供應商與測試需要改動 native capture。

## 決策

保留既有 48 kHz NormalizedAudio，透過 SpeechRecognizer trait 與獨立 provider 設定模組接入。Provider 自行負責抗混疊重採樣、連線、憑證、事件解析與已提交音訊的時間對應。核心只處理明確能力、五語型別、有界資料、Session／generation、取消與修訂順序。

呼叫端持有工作 future，不由核心 spawn detached task。音訊 try_push 永不等待服務；12 格滿載、250 ms 過期或音訊缺口會明確停止辨識，需重新建立 generation／Session。此首批選擇避免默默拼接缺少音訊，也避免 API 尚未具備 reset／gap 能力時假裝保持串流正確。後續同一 Phase 可加入同供應商明確復原，但須丟棄過時音訊、限制重試並驗證新舊結果隔離。

結果 channel 上限 64 項。滿載會取消，不默默遺失 Final。RAM 修訂暫存按來源順序合併 Partial／Final，不能當成持久逐字稿或 Phase 5 畫面。所有 mocks 限於測試，設定計畫不冒充正式辨識供應商。

## 限制

核心測試不代表真實五語、網路、TLS、重採樣或端到端延遲已驗證。供應商提示候選語言不等於受限 Auto 已成立；正式 capability 需有介接證據。不自動將音訊轉送另一個供應商。
