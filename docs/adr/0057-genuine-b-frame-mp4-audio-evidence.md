# ADR 0057 — Genuine B-frame MP4 Audio Evidence

2026-10-14，Phase 08 第二十四批，Windows `10.0.19045.0`。

## 原因與範圍

前批已接受嚴格 ctts 描述，但測試 encoder 使用零 B frames，不能據此宣稱真正影格重排檔案可準備音訊。本批補上真正 Main／High B-frame 合成 MP4 的壓縮證據、原生 AAC 時間與完整 Prepared 生命週期。正式容器門檻、decoder、時鐘映射、ASR、詞彙／翻譯、RAM Ready 與 UI 不變，沒有放寬一微秒檢查或新增播放。

這是本機合成案例的驗證，不是任意使用者影片、其他 muxer、播放器／seek、雲端品質或所有 B-frame 編碼設定的相容性驗收。

## 真正原生 B-frame fixture

cfg(test) 的固定 NV12／PCM encoder 新增 B-frame 模式。Sink Writer 已建立型別、尚未 BeginWriting 時，取得此視訊軌的原生 H.264 MFT；保存輸入／輸出型別、清除型別、透過 ICodecAPI 設定 maximum B-picture count=2，再按 output→input 還原原有型別。所有 COM interfaces、數值 VARIANT、buffer 與 writer 在同一 MTA，借用值跨同步呼叫保持有效；失敗沿用 fixture 的 RAII cleanup，不留下產品功能或跨執行緒 MFT。

啟用既有 windows 0.62.2 的 Win32_System_Ole binding feature，供測試的 VARIANT／ICodecAPI 使用；沒有新增套件、版本、下載或系統元件安裝。產品仍不包含 cfg(test) 的 encoder／媒體寫入入口。

設定值只代表上限，不保證實際 B-picture 數量。因此另選 FIRST_VIDEO 的原生壓縮 H.264，核對尺寸、每份 sample 非空、影格數與真正 EOF；借用鎖定的 buffer，掃描 Annex B 的三／四 byte start code，只解析 slice 的 first_mb_in_slice／slice_type。固定 128×96 只有 48 macroblocks，prefix 最多五個零；type 1 或 6 且 NAL type=1 才作為 B-slice 證據。IDR、I／P 的兩組 slice_type、SPS 或無 start code 的 bytes 不當成 B frame。它是有界 fixture probe，不是產品視訊 decoder，也不驗證完整 bitstream 可播放性。

同時要求 raw compressed sample 的 presentation timestamps 出現倒序。只有 B slice 與時間重排都實際出現才完成 fixture；不修改 NAL、ctts、mdat 或時間製造成功。每份影片限十六秒／320 KiB，raw reads 限 64，唯讀借用當前 native buffer，沒有完整解碼影像或額外視訊 cache。日誌只報合成影格／B-frame／倒序的數量，不輸出壓縮 payload。

## 音訊與 Prepared 邊界

每個案例仍通過檔內引用、sample／chunk／cross-track byte 範圍、一 MiB moov／64 KiB avcC、High 8-bit／4:2:0、版本／count 一致性門檻。正式路徑 deselect 所有 streams 後只選 FIRST_AUDIO，沒有把視訊的倒序或 ctts offset 混入音訊。

Main／High、44.1／48 kHz、單／雙聲道各以四秒、六秒加 137 frames、十三秒加 311 frames 直接查詢 IMF AAC timestamp／duration／PCM frames，共 24 種 native 組合。tone／RMS、64 KiB block、gate、EOF、writer／delete pin、取消及公開 borrowed clock 拒收亦有測試。PCM 不裁 padding、補靜音或重設時間。

Main 44.1 kHz mono 與 High 48 kHz stereo 的十三秒加尾段、文字／PNG 描述，用兩個所選服務 mock／16／24 kHz 經 ASR、全文詞彙、Standard Draft／Review 到 RAM Ready，cue endpoint 由獨立 native oracle 核對，metadata 不送到服務。兩個其他率／聲道組合核對 Network 錯誤、pending 取消、token、中文字幕優先及清理；無效 video ctts／reference 在所有 speech／詞彙／翻譯要求前拒收。

純 slice helper 測試只驗證證據分類、prefix 上限與截斷；畸形的 fixture 必須失敗，不能變成假成功。所有來源為自有合成資料，測試用暫存輸入沿用原有 RAII；沒有有效金鑰、私人影音、正式雲端、產品音訊／字幕保存、匯出、額外 worker 或 provider fallback。

## 仍待接續

最終程式碼實際通過格式、所有 target／feature 的 Clippy `-D warnings`、1433 項一般測試及 Debug／Release 建置，既有 15 項 ignored 不變。7 項 B-frame 聚焦與 2 項純證據測試通過；24 種直接原生音訊時間的最大 duration 重疊為 0.1 微秒，frame-derived 無重疊。測試模組依 lint 移至檔案末尾後，最終全套仍通過，沒有新增 lint suppression。完整指令與原生／Ready 證據見 [第二十四批驗證](../phase-8.md#第二十四批驗證)。

一般影片／其他 muxer、B-frame 設定／GOP／尺寸／fps 變體、edit list、fragmented、多音軌、內嵌字幕、URL、五語 Auto、全文語意及播放／實際定位仍未完成。Phase 08 尚未結案，Phase 09 尚未具備條件；金鑰留至結案固定遮蔽介面手動輸入。

## 主要來源

[Microsoft H.264 Video Encoder](https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder) 說明 B-picture count 的上限值、需於 output type 前設定，以及 Baseline 強制零 B frames，因此本批只用 Main／High 的原生測試 encoder。

[Microsoft GetTransformForStream](https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsinkwriterex-gettransformforstream) 說明取得指定 stream 的 MFT、零起算的 index 與 caller 的 release 責任；fixture 透過 Rust COM RAII 持有／釋放。

[FFmpeg 專案的 H.264 picture type table](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/h264data.c) 可交叉核對 slice_type 的 P／B／I 分類。本產品不呼叫、安裝或整合 FFmpeg，證據 probe 自行以兩個 bounded UE 欄位實作。
