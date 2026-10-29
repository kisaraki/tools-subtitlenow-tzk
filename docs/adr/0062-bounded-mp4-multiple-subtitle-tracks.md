# ADR 0062：有界 MP4 多字幕軌與明確選擇

日期：2026-10-16。狀態：Phase 08 第二十九批採用。

後續：第三十批對文字軌開放有限的完整軌道／前置空白 edit list，見 [ADR 0063](0063-full-track-tx3g-edit-delay.md)；下列無 edit 範圍記錄當批行為。

## 問題與決策

前批只能探索單一 text／tx3g，含多語字幕的容器一律拒收。本批把既有格式範圍擴大至最多八條 tx3g 字幕軌，仍要求一條既有 AAC-LC 音軌、可有一條既有 H.264 視訊軌，總計最多十軌。八條與既有 Prepared 選擇預算共用常數；這是探索候選數，準備要求依然只有一份明確 Mp4SubtitleSelection，不同內嵌軌不會同時翻譯或互相混合。

Mp4SubtitleTracks 使用固定八格陣列，結果只含 ID／語言提示、無路徑／本文／序列化。新的 plural discovery API 返回完整受支援清單，原 singular API 在多軌時回 AmbiguousSubtitle，避免既有呼叫者默默拿到第一軌。容器順序只影響清單排列，不代表品質或語言順位；zho 不證明繁簡／臺灣地區，使用者必須宣告，und 同理。

## 容器與資源邊界

抽取與探索共用同一次 pinned metadata 驗證。全部軌都核對唯一非零 ID、handler／codec、檔內引用、描述／fonts、原始時間與 sample tables；任兩軌 byte ranges 都必須不重疊，包含未選文字之間、文字與音訊／視訊。任一軌的結構不受支援或無效就拒收整份要求，不略過後宣稱完整探索成功。其他語言／handler、加密、外部引用、edit list／文字 ctts／modifier 等門檻維持。

moov 一 MiB、children 上限、共享 CPU lane、取消及每文字軌 16384 samples／八 MiB wire、四 MiB decoded text／8192 cues／24 小時規則保持；沒有同時配置八份本文。探索不讀完整 samples，仍保留共用 loader 最後一 byte mdat 可讀性探測。準備只解碼所選文字軌；未選軌的本文並未驗收或上傳。結構成功不是本文有效、版本或 alignment 證據。

每次探索仍一個 owner／唯一 identity／一個 terminal reply。UI 存放固定大小 catalog，改語言／品質／服務不丟失同一來源的選擇清單；更換媒體、重新探索、取消／清除／關閉會退休清單，錯誤不返回部分清單。開始準備仍重新開檔／完整驗證所有 metadata 及選定本文，以同一 Session pin／既有翻譯管線到 RAM Ready，不新增 decoder、pool、時間軸或雲端 fallback。

## 一般介面

媒體路徑旁按「探索 MP4 字幕軌」。一軌沿用前批自動填入 ID；多軌先選內嵌模式、清空 ID／語言，要求從右側下拉清單明確選擇。下拉清單只顯示 ID 與已知語言提示，沒有檔案自帶的任意標題。選軌填 ID 與五語提示，中文／未定仍須明確指定；選回提示列會清空 ID，不能變成 ASR。

保留手動 ID。修改 ID／字幕路徑時取消 combo 中的選取，避免兩者互相矛盾；來源路徑變更會清除探索 ID／語言與清單。既有 Ready 鎖定、目的地揭露與固定雙列遮蔽金鑰規則保持，探索清空金鑰但不讀取、不建 provider；介面文字補上探索會清空的說明。取消先停兩種 backend，再處理可能失敗的 Win32 清單清除，不因 UI cleanup 錯誤留下長任務。

新增 combo 沿用既有 subtitle 行，固定動作列及 scroll／DPI 架構不變。新 unsafe 只操作同一 UI 執行緒的自有 combo，以 scalar index／同步複製的有界 label，附 SAFETY；沒有自有 pointer payload、callback buffer 或 COM 逸出。

## 驗證與限制

合成容器以八種明確文字語言／各軌不同本文驗證順序與指定抽取、原始時間、九軌超限、重複 ID、未選 tables 錯誤與跨文字 byte alias、截斷／取消，並證明未選損壞本文不被解碼。controller 用同一所選服務 mock 與中文不建服務分支驗證 Ready／pin；hidden native 測試完整走多軌探索→選第二條中文→真實本機 Ready→清除，另核對八選項／手動 ID／來源變更／失敗重探。原有單軌、取消、CPU 飽和、舊結果及 DPI bounds 回歸保持，詳見 [Phase 8](../phase-8.md)。

codec bytes 仍是結構 fixture，真實 muxer／使用者影片與視覺操作／雲端品質未驗收。本批不擴大其他格式、音軌數、語言、播放或自動最佳字幕選用；原 MP4 音訊 ASR gate 仍拒收文字軌。沒有產品持久化、錄製、匯出、暫存媒體或下載；有效金鑰維持結案固定介面手動輸入，Phase 09 尚未具備條件。
