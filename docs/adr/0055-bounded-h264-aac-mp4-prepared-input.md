# ADR 0055 — Bounded H.264 / AAC MP4 Prepared Input

2026-10-14，Phase 08 第二十二批，Windows 19045 基準。

## 決策與原因

前批只收單音軌 M4A／MP4，即使影片中的 AAC 音訊可辨識也會拒收。本批接受一個 AAC-LC 音軌加零或一個 H.264 Baseline 視訊軌，仍只準備音訊，不提供影片播放器或視訊辨識。這是有界本機影片子集合，不代表一般 MP4 相容性。

moov 只有 trak 可以重複，最多兩個；必須正好一個 soun，另一個只能 vide，track ID 非零且不得重複。每個軌道都核對自己的 self-contained url data reference、sample description、stts／stsc／stsz／stco 或 co64。兩軌各自的 chunk 必須遞增且完整落於唯一 mdat，再合併借用的兩個 chunk cursor，拒絕任何跨軌 byte overlap／alias。不配置每個 sample 的 range 陣列，整份 moov 仍最多一 MiB、每軌最多一百萬 samples／table rows；每個 chunk／sample／run 都檢查取消。

視訊限單一 avc1、Baseline profile 66。VisualSampleEntry 要求檔內 reference 1、尺寸 1–8192、frame count 1、24-bit depth、固定 reserved 欄位；extension 只接受 avcC、固定 btrt 和正值 pasp。avcC 最多 64 KiB，核對 version、profile、NAL 長度 framing、非空 SPS／PPS 的型別與界限，SPS 的 profile／compatibility／level 必須與 header 相符，拒絕尾端未知 extension。這是描述結構檢查，沒有解析完整 SPS、驗證壓縮位元流或解碼視訊；不能把門檻成功當成影片可播放。

視訊允許有序、不重複且落於 sample count 的 stss；ctts 視訊重排、Main／High、加密 encv、其他視訊 codec、edit list、fragmented、多音軌、無音軌或額外 metadata 軌仍拒收。原生 muxer 在影音檔的 AAC 軌會產生 version 0 ctts，本批驗證其完整記錄大小、正值 counts 與 stsz 數量一致；不以該表替代真正的 IMF 時間或修改 PCM。

## 音訊與隱私邊界

正式解碼器沿用原先全部 stream deselect，再只選 FIRST_AUDIO；不協商視訊解碼器或 renderer。AAC native subtype／14-byte USER_DATA／LC ASC、44.1／48 kHz、單／雙聲道與協商後 PCM 均再次核對。原有一微秒時間 gate、64 KiB block、一 MiB pending sample、時鐘快照、拉取 actor、ASR 排程、詞彙／翻譯與 RAM Ready 沿用。

視訊 byte 只由容器 metadata 核對位置，沒有產品視訊 buffer、解碼、上傳、保存或快取。來源媒體保留原同一唯讀 pin；描述標籤／封面仍只檢查，沒有送到服務。取消與真正 worker／native 退出、Session 隔離、中文字幕優先保持原契約。金鑰與實際雲端驗收維持結案固定遮蔽介面手動輸入。

## 測試設計

cfg(test) 使用 Windows 原生 sink writer，在 RAM 編碼固定 128×96 NV12 影格與 440 Hz 合成 PCM，形成真正 H.264 Baseline＋AAC；沒有使用者媒體、產品 encoder／匯出 API、外部下載或新增元件。測試另選 FIRST_VIDEO 的原生 H.264 型別、尺寸並讀完所有非空壓縮 samples，核對影格數與 EOF；產品路徑則只讀音訊。

純門檻 fixtures 的 fake codec payload 僅測結構，不用來證明解碼或 Ready。反例涵蓋重複／缺少／額外軌道、外部引用、保護與 unsupported profile、SPS／PPS／尺寸、交錯 chunk、跨軌 aliases、mdat 越界、ctts／stss 數量、截斷／mutation／取消。原生時間、tone、EOF、pin、停止與 Prepared 整合以真正 native fixture 驗證，服務仍是明確 mock。

## 未完成範圍

本機最終驗證：`cargo fmt --check`、所有 target／feature 的 Clippy `-D warnings`、1411 項一般測試及 Debug／Release 建置全部通過，既有 15 項 ignored 不變。MP4 聚焦 47 項通過；原生十二種影音時間組合 native duration 最大重疊 0.1 微秒、frame-derived 無重疊。四種率／聲道經兩個服務 mock、16／24 kHz 與完整 Ready／停止／錯誤／字幕優先生命週期；這不是正式雲端或播放驗收。完整指令與證據見 [Phase 8 第二十二批驗證](../phase-8.md#第二十二批驗證)。

一般影片／真實檔案、其他 profiles／codec／muxer、內嵌字幕、URL 媒體讀取、限五語 Auto、完整語意脈絡及播放／實際定位仍待接續。Phase 08 尚未結案，Phase 09 尚未具備啟動條件。

## 主要來源

[Microsoft H.264 Video Encoder](https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder) 列出 NV12 input、H.264 output、必要 media attributes，以及 Baseline 不使用 B frames。本次只有測試 encoder，沒有因此新增產品影片編碼。

[Microsoft MPEG-4 File Source](https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-source) 說明 avc1／mp4a、音訊與視訊 stream、檔內 mdat 和外部軌道的限制。產品自行做較窄門檻，不把系統支援表當成相容性驗收。

[Apple Video sample description](https://developer.apple.com/documentation/quicktime-file-format/video_sample_description) 列出 data reference、尺寸、frame count、compressor name 與 depth 等固定欄位；原生合成檔的 header 另經實測核對。

後續第二十三批的 Main／High、High 格式欄位與視訊 ctts 描述門檻見 [ADR 0056](0056-ordinary-h264-profiles-and-composition-tables.md)；本文件保留第二十二批只收 Baseline 的歷史決策與驗證。
