# ADR 0012 — 以已驗證程序身分跟隨來源螢幕

日期：2026-10-09。狀態：Phase 5 已採用，混合 DPI／實際應用程式視覺矩陣待驗收。

視窗只能用於字幕位置，不能當作音訊擷取身分。PID 可能重用，音訊子程序也可能沒有頂層視窗；任意猜測父程序或選另一個可見程式會讓 UI 顯示錯誤的關聯。

正式 GUI owner 將已保留 ProcessTarget 的建立時間放入原子 UiFrame。共用 pipeline 在同 PID 重連時保留身分，改 PID 時清除；新視窗啟動仍由新 publisher／inbox 隔離。Idle 控制面板可使用所選 SourceChoice 的已驗證建立時間，不必選好服務或啟動雲端。

跟隨選項使用明確的 MonitorPreference::FollowSource，與 Primary／Specific 互斥。UI 每 500 ms 開 query／synchronize handle，比對建立時間及零等待存活狀態，透過 [EnumWindows](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumwindows) 列舉最多既有 MAX_WINDOWS 上限內的頂層視窗。不發送遠端訊息、不讀私人控制項、不用 hooks、不改 capture。只考慮該 PID 的可見／非最小化／非 tool window；同 PID 前景視窗優先，其次最大面積。取得螢幕工作區域前後重新驗證視窗所屬 PID，避免消失／重用的 HWND 被誤採。

只在工作區域變更或使用者明確重置尺寸／選項時重新定位。相同區域的下次樣本不覆寫拖曳位置。無來源、身分失效、無可見視窗或查詢失敗各有固定提示，保留位置／Session／字幕；恢復時清除提示。選擇主螢幕或指定螢幕可退出跟隨。不讀取 API 金鑰、不變更服務、不另外辨識。所有 process handle 在查詢結束 RAII 關閉，沒有新增工作執行緒或佇列。

限制：跟隨的是來源所在螢幕工作區域，沒有承諾貼住播放器視窗底緣。只處理所選 PID；Chrome／Edge 等音訊子程序沒有可見視窗時，明確提示並允許手選螢幕，不自行推定父子視窗關係。前景改變或視窗移動至同一螢幕不會每輪移動字幕。Exclusive fullscreen 不保證可見，未加入侵入式 hook。500 ms 是名義查詢週期，不是已驗證的移動延遲 SLA。
