# ADR 0009 — 即時視窗的後端所有權與啟動提示

日期：2026-10-09。狀態：Phase 5 正式指定 PID 視窗命令採用。

Win32 message loop 必須留在 main，雲端與即時 orchestration 不應阻塞它。只把 future 放入 detached thread 會使視窗關閉後仍擷取，或把發出取消誤認為已回收。同時，後端先連線再建立 UI 會使來源提示落後於實際送出。

採單一 UiWorker 擁有固定執行緒及 Tokio runtime。終端機與視窗共用 live.rs 的 run_inner，透過同一份 timeline 及有界 UI bridge 輸出。主視窗保有控制選單，Dual 不建立第二條管線。關閉或視窗初始化失敗取消並 join，Drop 是最後的 RAII 防線；取消的等待涵蓋連線、重試與運行中的工作。

先固定 PID 的程序 handle，UI bridge 發布來源及所選服務的送出提示。UI 完成控制項更新且處於 WS_VISIBLE、非最小化狀態後 ACK，後端再讀取所選認證與連線；這不是使用者批准流程，也不是實際視覺可讀性的證明。停止清空畫面，舊活動 frame 不可恢復文字；終止／錯誤 frame 即使在取消後仍可更新狀態。

隱藏金鑰輸入是明確旗標選項，留在建立 UI／worker 前的 main 互動式終端機。因同步 prompt 無法由 async cancellation 安全中斷，不能把它留在需 join 的背景工作中。輸入不寫入設定或認證管理員，TLS／HTTP 套件內部副本不宣稱全面擦除。
