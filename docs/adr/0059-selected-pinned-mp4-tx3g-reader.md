# ADR 0059：同一 pin 的指定 MP4 tx3g 軌讀取

日期：2026-10-15。狀態：Phase 08 第二十六批採用，GUI／Prepared Ready 尚未接線。

後續第二十七批已完成明確單軌的 Prepared／GUI／Ready 接線，見 [ADR 0060](0060-explicit-embedded-prepared-ready-and-ui.md)。本 reader 的容器限制保持；以下保留當批範圍。

## 決策與原因

前批只有純文字 sample core，無法從實際檔案取得軌道 bytes。新增 provider-neutral `extract_local_mp4_subtitle`，由呼叫者提供本次 Session／generation、track ID 及明確 TextLanguage，讀取完整原文並保留同一媒體 pin。這批完成可執行的檔案讀取介面，不以 stub／dummy 字幕模擬檔內文字，也不新增外部工具、內容快取或 temp media。

共用既有 MP4 top-level loader 與音訊／視訊描述／tables gate，避免複製兩份容器安全檢查。loader 保持 moov 一 MiB／128 boxes／單 mdat／已知 metadata 與原有最後一 byte 可讀性探測；encoded audio／video 不解碼、不整份緩衝。原本音訊 probe 的範圍與效果保持，仍拒收含文字軌的輸入；字幕 API 是獨立讀取入口，不自動開啟 native decoder／ASR。

## 接受的明確子集合

- 一個既有 AAC-LC 音軌、可有一個既有 H.264 視訊軌、一個指定 ID 的 text handler／tx3g 文字軌，最多三軌且 ID 唯一；不是通用 MP4 或一般 muxer 相容性保證。
- 每軌 self-contained data reference；mdat 內正值 sample sizes、排序／非重疊 chunks、完整 stts／stsc／stsz／stco 或 co64。最多三個 borrowed cursor 逐對 merge 拒絕跨軌 alias，沒有每個 sample 的 range Vec。
- 文字單一 tx3g description、保留欄位／reference index、零動態 displayFlags、已知 justification、有效 default box／style header、最多 32 個唯一 font IDs、UTF-8 name 與 default font 核對。未指定 font 可用空 ftab。font size／color 等靜態描述只做結構檢查；不載入字型或重現原外觀，文字輸出仍供產品自身 UI 顯示。UTF-16 font names、未知描述、動態外觀與 sample modifiers 暫不支援。
- 明確文字語言與 packed ISO 639/T 匹配，或檔內為 und 由呼叫者聲明；zho 不區分繁中／簡中／臺灣，不能自動判定 zh-TW。mul／衝突／非法語言拒收。
- 無 edit list／文字 ctts／外部引用／加密／fragmented／未知 handler／額外文字軌。未實作的偏移不以猜測、wall clock 或補靜音填補。

文字 stts 總 ticks 必須等於 mdhd duration，正 duration 與 movie duration 有上限／包含核對；tkhd 接受 rational movie conversion 的 floor／ceil，不用該 rounding 改寫 text start／end。從零 media time 按 stts 原始 ticks 逐份累積，前批 core 仍以 u128 換算絕對端點。每份讀入固定 8194 bytes scratch；八 MiB raw／四 MiB decoded text／8192 cues／16384 samples／24 小時限制保持。清除樣本保留原區間，不延長前 cue。

## 所有權、取消與隱私

既有 open_media 驗證本機路徑、一般檔案與 sharing，worker 只複製同一 readonly handle，不以 path 重開。成功的 ExtractedMp4Subtitle 擁有原文及該 pin，後續 translator 可借用原文；Session 結束時呼叫者必須丟棄 owner。結果不是 Ready、播放 alignment 或來源仲裁證據；區域 content ID 沒有被當作版本 fingerprint。

使用既有 Prepared CPU lane，worker 數量受 available_parallelism 與共用 semaphore 控制；try_acquire 飽和即 Overloaded，沒有第二個 pool 或等候 queue。Drop guard 在 async body 以外建立，未 poll Drop 亦取消；已啟動 blocking closure 持有 CPU 許可到實際退出，abort handle 不假稱取消 kernel I/O。所有迴圈／分塊讀取檢查 token，cancelled result／panic／部分文件不發布。

無新增 unsafe、相依套件、provider／credential lookup、網路要求、音訊／視訊 decoder、產品檔案建立／寫入／temp／cache／export 或第二個時間軸。HTTP(S) 媒體仍 UnsupportedUrl，不以字幕 reader 偷換讀取模式。

## 驗證與後續

合成容器有真正 tx3g bytes／table offsets，音訊與視訊 bytes 明確只為結構 gate fixture，不宣稱 playable AAC／H.264。測試包含 UTF-8／UTF-16、variable／constant sample sizes、stco／co64、多 chunk／空隙／清除時間、兩／三軌、跨軌 alias、external ref／edit／composition／encryption 拒收、description／font／language／duration／truncation、取消與容量。Windows fixture 真正開啟自有 temp 檔，核對唯讀 pin 的 writer／delete 拒絕與釋放；controlled barrier 核對 Future Drop 後 pin／CPU capacity 保持到退出，沒有用 sleep 猜測。

正式 muxer／使用者檔案、更多 handler／codec／樣式／多字幕軌、edit／composition 時間與自動探索仍需個別驗收。下一步須把選軌、語言與優先順序接到 PreparedRequest／GUI，於憑證／服務建立前選定本機中文路徑，並沿用原 pin 到全文翻譯／Ready；此批沒有完成這條 UI 流程。Phase 09 尚未具備條件；有效 API key 維持結案固定遮蔽介面由使用者手動輸入。驗證結果見 [Phase 8](../phase-8.md)。

## 主要格式依據

[ETSI TS 126 245 V10.0.0，5.9／5.13–5.17](https://www.etsi.org/deliver/etsi_ts/126200_126299/126245/10.00.00_60/ts_126245v100000p.pdf)：mdhd 語言、text handler／nmhd、style／box／font record、tx3g description 與 sample 結構。明確拒收動態外觀、edit／composition 與未支援描述是本專案的子集合決策，不代表完整 3GPP decoder。

前批本文與 Unicode 依據見 [ADR 0058](0058-bounded-tx3g-plain-text-samples.md)，既有容器／range 設計見 [ADR 0055](0055-bounded-h264-aac-mp4-prepared-input.md)。
