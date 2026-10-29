# ADR 0011 — 字幕文字與背景分開合成

日期：2026-10-09。狀態：Phase 5 已採用，實機視覺／多 DPI 驗收待結案。

字幕背景透明度不應同時降低文字清晰度。整個視窗設定同一個 Alpha 會淡化文字，色鍵則不能提供連續背景透明度與平滑字緣。因此改用 [UpdateLayeredWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow) 的每像素 Alpha；底部視窗始終保持 WS_EX_LAYERED，穿透只切換 WS_EX_TRANSPARENT，避免混用全視窗透明設定。

共用 Presentation 仍只取唯一 timeline 的最新項目；中文在雙語顯示先呈現，有限行數優先保留翻譯。Appearance 是不含內容／供應商的有界 RAM 偏好；外觀與螢幕指令不進入 backend、不更改 Session、不產生另一條辨識或翻譯管線。

Win32 renderer 建立兩個私有記憶體 DC／32-bit top-down DIB，先以灰階反鋸齒 GDI 字形產生覆蓋率，再合成白字、黑色描邊／陰影與可調黑色背景。輸出使用 premultiplied BGRA。描邊以兩次一維膨脹限制運算量，效果半徑有上限；不讀取桌面像素、不建立檔案映射、不寫圖檔或字幕檔。

表面限制 4096×2048，且最多 4,194,304 像素；配置前驗證尺寸與乘法。兩個 DIB 加兩個覆蓋率暫存的像素記憶體上限約 40 MiB，另有有界文字／GDI 開銷。每次更新後 RAII 回收，沒有跨 Session 的 renderer cache。依 [CreateDIBSection](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-createdibsection) 要求，讀取 DIB 前先 GdiFlush。字型、DC 與 bitmap 恢復選取／銷毀順序明確，不跨 UI 執行緒。這個上限不代表已量測最高字級下的 UI 延遲；效能仍需正式量測。

行數使用所選字型的實際 GDI 高度限制整行，超出內容裁切，DrawText 對長行加省略號。螢幕以裝置名稱維持偏好，不以清單索引跨更新沿用。選定螢幕移除時保留偏好、明確提示並暫置主螢幕；沒有變更音訊來源或供應商。負座標／小工作區域的定位有界，跨螢幕後使用字幕 HWND 的實際 DPI 計算第二次定位。

取捨：目前字型選單為 Windows 常見的三種字型，實際缺少字型／語系字形時仍受 Windows font mapping 影響，需人工確認；外觀與螢幕偏好不自動存檔。完全透明背景時 Windows 的透明區域可能直接讓滑鼠通過，拖曳可用的區域主要是字形／效果。跟隨來源所在螢幕已另見 [ADR 0012](0012-source-window-monitor-follow.md)，混合 DPI／字形／穿透／長文字的可讀性未以單元測試替代驗收。
