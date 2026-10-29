# ADR 0001：以 MTA worker 擁有音訊探索資源

日期：2026-10-05。狀態：採用。範圍：Phase 1。

## 問題

Core Audio 工作階段建立通知要求非 UI 的 MTA 執行緒；僅依賴列舉器可能漏掉新工作階段。視窗、程序與圖示查詢可能比音量讀取慢，不能放入未來的音訊擷取回呼或 UI 訊息迴圈。多個顯示端也不應各自建立探索管線。

## 決策

建立一個可取消的專用 worker，透過 factory 在該執行緒建立 scanner。COM apartment guard 刻意不可 Send／Sync，並最後釋放。原生型別、呼叫與 unsafe 僅在 `win32`，其他模組只依賴 `AudioSessionScanner` 與自有的來源模型。

將建立通知與定期核對結合。通知只以有界通道傳遞 agile COM 參照；worker 管理工作階段集合，定期更新裝置、狀態及音量。過載要明確顯示，並以之後列舉協助恢復。

以 Tokio watch 發布最新快照，防止慢速接收者形成舊畫面佇列。快照包含探索身分、序號、單調時間；UI 不取得原生 handle。停止與 Drop 都 join worker，避免遺留執行緒。

## 取捨

250 ms 輪詢間隔有延遲，短暫峰值不一定會出現在快照中；它只用於來源探索，不是後續音訊擷取的時序。原生呼叫無法中途強制取消，停止需等待目前呼叫返回。先以直接 PID 查詢視窗，不猜測瀏覽器程序樹或分頁，避免建立錯誤的來源關聯。

這個決策不新增擷取管線、AI 供應商、視窗框架、音訊檔案或持久化內容。後續 Phase 可加入音訊擷取與字幕顯示，仍沿用獨立邊界。

依據：[Microsoft IAudioSessionEnumerator](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nn-audiopolicy-iaudiosessionenumerator)。
