# ADR 0013 — 來源選項與活動資料分開更新

日期：2026-10-09。狀態：Phase 5 已採用，視覺與實機矩陣尚待驗收。

峰值每輪探索會變化；若把活動變化當作選項變更，每 250 ms 重建清單會打斷鍵盤／滑鼠選擇，也可能重設回看位置。來源消失或 PID 重用時，舊清單索引不能套用到新的程序。

SourceChoice 保留來源身分、有限標籤、Arc 圖示與 SourceActivity。身分仍由 Session ID／PID／程序建立時間構成；same_option 只比較身分與標籤，活動／圖示變更不重建 COMBOBOX。來源清單展開或 backend busy 時保留原映射；繪製與活動區以最新快照重新匹配同一身分，找不到時明確顯示資料不可用，不用另一個來源代替。

圖示只接受至多 32×32、完整長度且符合 premultiplied BGRA 的資料。原生 source combo 採 CBS_OWNERDRAWFIXED／CBS_HASSTRINGS，繪製圖示與標籤，同時保留原生鍵盤及 accessibility 文字。DC 與 bitmap 限於 UI thread，借用的繪製 DC 以 SaveDC／RestoreDC 回復字型、顏色與裁切；AlphaBlend 使用既有有界 DIB RAII。資料未知時不使用假的程式圖示。

SourceActivity 將有效峰值量化成千分比、音量量化成百分比，以便 immutable snapshot 比較；訊號門檻沿用既有 SIGNAL_THRESHOLD，計算前拒絕 NaN／Infinity／超出 0..1 的資料。串流執行狀態與有訊號分別呈現。未知峰值隱藏量表且顯示未知，不捏造零讀值。活動取自所選音訊 Session，不能宣稱包含程序樹全部音訊或保證喇叭有聲。

一般控制器更新只在 busy／notice／選取改變時重寫 SideChat；圖示／峰值更新僅更新來源控制項，避免打斷搜尋、回看與文字。新增狀態區與量表的排列有 DPI 幾何測試，未使用像素測試替代實際可讀性確認。

SideDock 是 Left／Right／Floating 的 renderer 偏好，預設 Right。停靠只是調整自己的 Win32 視窗，在所在螢幕工作區域取 460 DIP 寬及完整高度；DPI／螢幕變更與拖移結束後重新定位。沒有 AppBar、其他程式 resize、hooks 或後端指令，不重啟唯一的 timeline 管線。偏好在 RAM，未修改設定 schema。桌面空間不會被保留；自行調整大小請切至自由移動。
