# Phase 10 工程完成門檻

2026-10-23 使用者要求持續至 Phase 10 結束，除非需要手動處理或指示。沿用既定範圍：正式金鑰／真實服務／使用者媒體留至結案固定遮蔽介面手動驗收；本次以明確 mock、自有合成資料與自有靜音程序驗證工程量測，不以此冒充供應商品質、播放器相容性或 SLA。

- [x] 有界擷取方法耗時、成功交付年齡與 P50／P95 上界，空值、零值、溢位與不一致時間分開處理。
- [x] 同 Session／generation／來源修訂／job／輸出修訂的 STT、翻譯、採用及 native 繪圖提交關聯；不以通知當作畫面可見。
- [x] raw／normalized／STT／翻譯 queue 觀測高水位，角色工作數與真實 provider operation／permit 並行計數，取消後釋放。
- [x] frame 淘汰、修訂替換、final segment 拒收／過期／未完成退休與診斷缺樣分別計數。
- [x] 本程式與精確擷取 helper 的只讀 CPU／private commit／working set／handle 量測，首樣本與錯誤不冒充零。
- [x] Release 三品質／並行／取消各三輪合成管線量測，以及 11 項 Release 亂序回歸。
- [x] Release 自有靜音 native capture 的 idle／十秒穩定消費／慢消費端，以及計時開／關三輪比較；保留量測負擔與噪音限制。
- [x] 原生隱藏視窗、receipt 過期／重複與 Session 隔離測試；30 項 Phase 10 Release 測試通過，mock receipt 明確標記，實際螢幕可見延遲留結案驗收。
- [x] 格式、零警告 Clippy、Debug／Release、相關測試與 Release 後獨立完整回歸：2006 項一般測試通過、零失敗，既有 15 項 ignored 不變。
- [x] 彙整三輪合成與原生實際數據，保留短取樣解析度／開關差分噪音限制；無數據支持的容量／品質優化不實作。已記錄結案手動驗收，並提醒 Phase 11 已具工程啟動條件。

**Phase 10 工程量測與驗證完成，可啟動 Phase 11 — Hardening。** 本次依使用者要求停止於 Phase 10 結束，尚未啟動 Phase 11。完整結果見 [Phase 10](phase-10.md) 及 [ADR 0089](adr/0089-session-numeric-pipeline-performance.md)。本文件不將 mock／自有靜音結果擴張成實際螢幕可見時間、長期穩定性、任意媒體／播放器或雲端效能驗收。產品未新增持久診斷日誌；工程文件只保留數值結果。有效金鑰仍留至結案固定遮蔽介面手動輸入。
