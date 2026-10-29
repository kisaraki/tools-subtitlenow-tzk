# ADR 0064：單一正常速度 tx3g 區間裁切

日期：2026-10-16。狀態：Phase 08 第三十一批採用。

後續：[ADR 0065](0065-version-one-tx3g-edit-list.md) 擴充 elst v1 的欄位寬度；本文件保留當批決策，其餘區間與生命週期規則沿用。

## 問題與決策

前批只接受完整字幕軌，合法的正向 media_time 或較短 active duration 仍拒收。本批把相同 version 0／零 flags／一筆 active edit（可有前置 empty edit）擴大成單一正常速度區間，解析容器明確宣告的頭尾裁切。不是使用者剪輯功能、不改寫原檔、不建立播放／動態對齊或 Phase 09。

active media_time 必須為非負 signed 32-bit，duration>0、rate=1；整個區間必須精確位於 mdhd 媒體長度內。前置 empty 仍為正 duration、media_time=-1、rate=1。tkhd 等於兩段總長、mvhd 涵蓋；重複、變速、dwell、未知版本、A/V edit／文字 ctts／modifier 保持拒收。無 edit 的舊 header floor／ceil 與媒體時間規則保持。

格式語意依 [Apple edit list 範例](https://developer.apple.com/documentation/quicktime-file-format/playing_with_edit_lists) 與 [edit list table](https://developer.apple.com/documentation/quicktime-file-format/edit_list_atom/edit_list_table)：edit duration 使用 movie timescale，media_time 使用 media timescale；原始 sample 時間仍依 [stts](https://developer.apple.com/documentation/quicktime-file-format/time-to-sample_atom)。以下交集與一次取整算法是本機實作決策，不宣稱其他播放器或一般 muxer 已驗收。

## 交集與呈現時間

以共同分母 movie_scale × text_scale 保留有理數。media_start × movie_scale 是區間起點，active_duration × text_scale 是區間長度；各樣本以 media ticks × movie_scale 取得同單位端點。採半開區間交集，完全位於外側的 cue 移除，邊界上沒有重複字幕。

交集起訖各自減去 media_start，再加前置 empty 的 movie 時間，以 checked u128 一次 floor 到微秒。不同尺度下 active end 可位於分數媒體 tick，不以 floor／ceil 延長來源或依逐段 duration 累積誤差。只有時間區間被截短；任何仍有重疊的文字保留完整原文，不切字元或猜測句子中哪個字對應邊界。小於一微秒而無正呈現長度仍明確失敗。

## 完整驗證與有界記憶體

讀取器先以原始 media timebase 送全部選定樣本到既有 Tx3gTrack，連裁切範圍外的本文也驗證 UTF、長度、sample／wire／text／cue 預算、原序號與 clear samples。不能用裁切繞過損壞文字或容量上限。保留原有四 MiB 本文、8192 cues 與16384 samples，不改為只計算可見區間。

另存最多 8192 筆純數值時間對照，不複製原文；來源 document 完整成功後以 retain 原地過濾／改時間，再做呈現文件驗證。原 text Vec 是唯一本文所有者，不配置第二份全文／時間軸；時間對照返回前釋放。只有此最終可見文件交給 glossary／翻譯，裁切範圍外的原文不會送往服務。若區間只覆蓋 clear gap 或沒有可見 cue，探索可顯示結構選項，但準備明確 InvalidSubtitle，不產生空白 Ready 或偷偷 ASR。

全部文字軌仍驗證 metadata／edit／跨軌 byte ranges，只有選定軌讀本文。既有 CPU lane、唯讀 pin、Session／取消／完整 completion 保持；filter 每 cue 檢查取消、驗證後再核對取消。沒有新增 task、pool、queue、lock、unsafe、相依套件、產品檔案／temp／cache／export。

## 驗證與限制

測試覆蓋頭尾同時截在 cue 中間、整段裁掉／精確邊界、保留 clear gap、只有空窗的錯誤、分數 active end 與 media_start、全檔截斷、讀取被裁掉本文時取消、被裁掉本文損壞仍拒收、多軌獨立範圍與未選軌無效 edit。controller 驗證三種中文不建服務、兩服務／三品質 mock 僅收到剩餘原文及正確時間，空窗在任何分析／翻譯要求前失敗。hidden native 沿用明確選軌到 Ready／清除與空窗失敗流程；完整結果見 [Phase 8](../phase-8.md)。

fixture 仍是自有合成容器／實際文字 bytes，音訊／視訊 bytes 不作真實 codec 或一般 muxer 證據；沒有正式金鑰或真實雲端。v1／多 active edits／A/V edit／composition／modifier、更多格式、URL／Auto、完整語意、實際影片與播放定位仍待接續。Phase 09 尚未具備條件；金鑰仍留至結案固定遮蔽介面手動輸入。
