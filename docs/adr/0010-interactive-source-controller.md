# ADR 0010 — 互動來源控制與新 Session

日期：2026-10-09。狀態：Phase 5 互動控制採用。

來源、服務與品質必須由使用者明確選取，清單更新不能把已選程式換成其他 PID，停止前也不能啟動第二條管線。只保存 PID 不足以防止探索後的程序重用；把全部來源字串每 33 ms 重建會浪費 UI 資源。

沿用固定的 DiscoveryMonitor 與單格 Arc 快照，UI poll 不建立 COM 物件。來源描述用 Arc 共用，只有變更時替換；顯示映射以 Session ID、PID、程序建立時間比對。開始時驗證當前程序 handle 的建立時間，持有至停止，後續 helper 的最新來源確認維持原策略。

UiController 掌管唯一 UiWorker。運行期間鎖定選項，停止先取消，確認執行緒已完成才 join／解鎖；重新啟動使用新的 inbox／token／Session。Win32 callback 不等網路或活動 worker join，模式切換僅變動顯示模型。

金鑰是遮蔽的 RAM-only 輸入，不是偏好設定。開始取用與服務切換後清除原生欄位及復原紀錄；本程式的字串副本 zeroize，不宣稱擦除 Win32／HTTP／TLS 所有內部副本。留空使用所選服務的安全認證儲存，不跨供應商替代。
