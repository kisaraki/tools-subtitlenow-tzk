# ADR 0056 — Ordinary H.264 Profiles and Composition Tables

2026-10-14，Phase 08 第二十三批，Windows `10.0.19045.0`。

## 決策與原因

前批只收 H.264 Baseline 的影音 MP4。本批擴大為 Baseline（66）、Main（77）、High（100），保持單一 avc1 視訊軌、單一 AAC-LC 音軌及原本的本機 Prepared 路徑。產品仍只選 FIRST_AUDIO，不解碼、渲染、上傳或保存視訊；沒有新增播放器、媒體 encoder 或另一條辨識管線。

High 的原生 muxer 描述可能沒有尾端色彩／位元深度 extension，因此不只檢查 profile 名稱。每個 High SPS 先核對 profile／compatibility／level 與 avcC 相同，再用借用的 bit reader 核對 SPS ID ≤31、chroma_format_idc=1（4:2:0）、兩種 bit_depth_minus8=0（8-bit）。bit reader 不配置 RBSP 副本，遇兩個零後的 emulation-prevention byte 才跳過，核對下一 byte 範圍；Exp-Golomb prefix 最多五個零，每個欄位有固定工作上限。這只解析格式 header，沒有驗證完整 SPS／PPS、參考影格或壓縮位元流可播放性。

avcC 的 reserved bits、非零 level 與 SPS／PPS 描述一致性維持檢查。High 若帶 extension，只收完整四 bytes 的 4:2:0、8-bit、零 SPS extension 形式；Baseline／Main 不收這個尾段。High10、High422、High444、Extended、scalable／其他 profiles、非 4:2:0、其他深度、非空 SPS extension、加密／未知 atom 仍明確拒收。avcC 64 KiB、moov 一 MiB、尺寸 8192 與原本的軌道引用／chunk／跨軌 overlap 上限保持。

## 組合時間表邊界

視訊 stbl 允許 ctts version 0 或 1，flags 必須零、完整記錄大小、正值 count、總數與 stsz 一致；每個 row 檢查取消，數量仍限一百萬且受 moov 一 MiB 限制。version 0 offset 為 unsigned、version 1 為 signed 的 32-bit 欄位；產品不計算視訊 presentation time，因此不使用 offset 算音訊或改寫時間。音訊仍只接受 version 0，其他 sample tables 仍要求 full-box version／flags 零，不因新增 table_rows 跳過原本驗證。

ctts 的接受只代表容器描述可交給同一原生來源，不是實際 B-frame／seek／播放驗收。測試 encoder 沿用 Windows 原生預設的零 B frames；目前沒有真實 B-frame 或其他 muxer 相容性證據。一般使用者檔案仍可能在原生 reader 或既有一微秒時間 gate 明確失敗，不偷偷修改時鐘或換服務。

## 測試與隱私

cfg(test) 新增 typed profile 選擇，交付真正 Windows H.264 encoder，核對輸出 avcC profile，沿用獨立 FIRST_VIDEO 原生型別／尺寸及非空壓縮 samples 數量／EOF。所有影格、PCM 及媒體內容為自有合成資料，編碼與來源檢查在 RAM；測試暫存輸入只為驗證 product 唯讀 pin，由既有 RAII 清理。沒有有效金鑰、使用者影音或正式雲端要求。

描述純測試分開核對三種 profiles／SPS mismatch、可選 High extension、色彩／深度／SPS ID、bounded prefix、emulation prevention、截斷／mutation／取消；影音表格則核對 signed／unsigned offsets、版本／flags／大小／count，以及引用／extent 不可被 ctts 掩蓋。純測試的 codec payload 只測結構，不作為真正解碼證據。

原生測試覆蓋 Main／High 各四種率／聲道的音訊 tone／gate／pin／取消，及各十二種率／聲道／長度的直接 IMF 時間觀測。Prepared 的 Main 44.1 kHz mono 與 High 48 kHz stereo 使用兩個所選服務 mock、16／24 kHz、完整詞彙／Draft／Review 到 Ready；另含標籤與 PNG，服務要求不得有描述內容。停止／Network 錯誤／中文字幕優先，以及不支援視訊軌的零 speech／詞彙／翻譯要求另有整合測試。

## 未完成範圍

最終程式碼實際通過 `cargo fmt --check`、所有 target／feature 的 Clippy `-D warnings`、1424 項一般測試及 Debug／Release 建置，既有 15 項 ignored 不變；沒有新增 lint suppression。MP4 聚焦 60 項通過，Main／High 共 24 種直接原生時間組合的最大 native overlap 為 0.1 微秒、frame-derived 無重疊。固定切塊測試 helper 依 Clippy 改為 `as_chunks` 後，最終全套仍通過。完整指令與證據見 [第二十三批驗證](../phase-8.md#第二十三批驗證)。

仍是有限的 avc1／AAC-LC MP4 子集合；其他 codec／profile、edit list、fragmented、多音軌、內嵌字幕、一般影片／真實 B frames／muxer 相容性、URL、五語 Auto、完整語意及播放／定位待接續。Phase 08 尚未結案，Phase 09 尚未具備條件。金鑰維持結案固定遮蔽介面手動輸入。

## 主要來源

[Microsoft H.264 Video Encoder](https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder) 定義三種 profile、輸出屬性與零 B frames 預設；此處只用作測試 encoder，沒有安裝元件或產品編碼。

[FFmpeg 專案的 AVC helper](https://github.com/FFmpeg/FFmpeg/blob/master/libavformat/avc.c) 可核對 High extension、SPS 格式欄位與 Exp-Golomb 編碼形狀。這是格式交叉檢查；本產品自行實作只讀四欄位的 bounded safe Rust reader，沒有安裝、呼叫或整合 FFmpeg。

[GPAC 專案的 ISO container reader](https://github.com/gpac/gpac/blob/master/src/isomedia/box_code_base.c) 可核對 ctts 的記錄大小與 signed／unsigned offset 分支；本產品只驗證表格，不引入該工具或其視訊時間計算。

後續第二十四批的 genuine B-frame 合成影片證據、原生音訊時間與 Ready 生命週期見 [ADR 0057](0057-genuine-b-frame-mp4-audio-evidence.md)；本文件保留前批零 B frames 的歷史測試範圍。
