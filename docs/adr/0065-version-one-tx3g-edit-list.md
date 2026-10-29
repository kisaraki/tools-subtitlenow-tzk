# ADR 0065：文字軌 version 1 edit list

日期：2026-10-16。狀態：Phase 08 第三十二批採用。

## 問題與範圍

部分容器在媒體標頭與實際長度仍位於目前範圍內時，也使用 version 1 elst。前批只支援 v0，無法處理這些字幕。本批只擴充 tx3g 的 elst 欄位寬度；仍限一個正常速度 active 區間，可有一個前置 empty。沿用 [ADR 0064](0064-single-interval-tx3g-text-crop.md) 的交集、完整驗證、RAM 文件、取消及 pin，不新增播放或 Phase 09。

欄位寬度參照 [FFmpeg 7.1 的 mov_read_elst 原始實作](https://www.ffmpeg.org/doxygen/7.1/mov_8c_source.html#l06163)：v1 entry 為 20 bytes，duration／media_time 各 64 bits，rate 仍為 32 bits；v0 entry 保持 12 bytes。只核對格式語意，沒有移植其容錯、配置或演算法，也未安裝／連結 FFmpeg。

## 決策與數值安全

先驗證完整 version／flags word，只接受 v0 或 v1 且 flags 全零；entry count 僅 1 或 2，先核對精確表格長度，再依固定寬度讀取。不得從長度猜版本，不允許截斷、尾資料、未知版本或欄位寬度混用。

duration 以 unsigned 32／64 bits 讀取，media_time 以 signed 32／64 bits 解讀。只有前置 empty 可以是 -1，且 duration>0、rate=1；active 的負值一律拒收，再 checked 轉成 u64。v1 的低 32 bits 即使看似合法，也不截斷高位或接受 0x00000000ffffffff 當作 -1。

沿用 u128 共同有理數交集，media_start 與 duration 必須完整落在原 mdhd 內。delay＋duration 先 checked_add，再核對 tkhd 與 mvhd，最後包含前置空白核對 24 小時。u64 值乘 u32 scale、兩項相加及乘一百萬最多需 117 bits，位於 u128 範圍內；微秒換算仍使用 checked_mul。沒有修改原 sample 時間或累積取整誤差。

**只支援 elst v1，不支援 mdhd／tkhd／mvhd v1。** 正式容器仍要求原有 v0 標頭，總長與媒體範圍不能超過其 u32 表示範圍。高時間尺度下的正 media_time 可高於 i32::MAX，仍可合法位於 v0 mdhd 中；更大的 v1 值必須依真實範圍拒收，不能以本批宣稱無限制長影音相容性。

## 生命週期與驗證

讀取、探索、中文本機處理、所選服務／品質、Ready、固定遮蔽金鑰與清除流程沿用。只擴充數值解碼，不新增配置、worker、queue、鎖、unsafe、相依套件、網路要求或產品寫入。全部文字軌 metadata／edit／byte ranges 都驗證，只有所選本文讀取；被裁掉本文仍驗證，取消或空窗不產生部分 Ready。

測試比對 v0／v1 的完整、前置空白、頭尾裁切、精確邊界與 clear gap；核對分數時間、高於 signed 32 bits 的正 media_start、高位別名拒收、負 sentinel、溢位、24 小時、未知版本／flags、寬度混用、全部截斷、未選軌錯誤及讀取取消。controller／hidden native 用同一流程分別驗證 v0／v1；實跑結果見 [Phase 8](../phase-8.md)。

合成容器包含真實文字 bytes，音訊／視訊 bytes 仍不代表一般 codec／muxer 或使用者影片驗收。多 active edits、A/V edit、文字 ctts／modifier、其他格式、URL／Auto、完整語意、播放定位及真實雲端仍待接續；Phase 09 尚未具備條件。有效金鑰維持結案固定遮蔽介面手動輸入。
