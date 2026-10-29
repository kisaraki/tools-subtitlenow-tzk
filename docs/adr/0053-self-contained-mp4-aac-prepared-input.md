# ADR 0053 — Self-contained MP4 AAC Prepared Input

2026-10-14，Phase 08 第十九批建立、第二十批擴大取樣率；第二十一批標籤政策另見 ADR 0054，Windows 19045 基準。

## 決策

新增本機 M4A／MP4 的第一個明確子集合：非 fragmented、單一音軌、AAC-LC；初始 48 kHz 單／雙聲道，第二十批經原生量測與完整 Ready 驗證後擴大為 44.1／48 kHz 單／雙聲道。使用原有唯讀 file object、MF Source Reader、native actor、不可變時間快照、ASR、全文詞彙與翻譯管線，成功後交付本次 RAM Ready。已有外掛字幕仍優先，沒有自動播放。

容器格式不能只靠副檔名或 `ftyp` 確認。MP4 可以包含其他軌道、資料引用及擴充描述；本批在建立 MF reader 前先確認有限、可核對的檔內結構，避免原生 handler 接觸不受本次選檔約束的內容。不自行寫完整 MP4 decoder，也不假裝所有 M4A／MP4 都已支援。

## 容器邊界

`local_mp4_format` 只讀相同 pinned file 的 metadata，以 seek 跳過音訊；最多 128 個頂層 boxes、128 個同層子 boxes、128 bytes 的 ftyp、1 MiB 的 moov。moov 每次最多讀 8 KiB 並檢查取消。sample table 即使使用固定 sample size 也最多一百萬份樣本，逐筆檢查取消；超限明確拒收。沒有整份媒體 RAM buffer 或產品暫存檔。

第一個 box 必須是 ftyp，major brand 限 isom／mp41／mp42／M4A，至少一個相容 brand 也須在這個集合。頂層只允許一份 moov、一份非空 mdat、free／skip，以及經核對的 Windows writer version UUID。該 UUID 只接受指定 identifier、最多 64 bytes 與長度一致的數字／句點版本字串；其他 UUID 拒收。頂層 extended size 使用 checked u64；open-ended size、越界、截斷、未知 boxes、fragmented moof／mfra／mvex 均拒收。

moov 必須只有一份 trak，handler 為 soun；stsd 必須只有一個 version-0 mp4a，data_reference_index 為 1。dref 必須只有一個無 payload、flags=1 的 self-contained url entry。ES descriptor 不得含 URL、dependency 或 OCR，必須是 object type 0x40、audio stream type、兩 bytes 的 AAC-LC AudioSpecificConfig，明確單／雙聲道、44.1／48 kHz、1024 frames，且與 sample entry 一致。HE-AAC／SBR／PS、PCE、加密 entry、其他音訊、影片、多音軌、內嵌字幕及 edit list 不在此子集合。

核對 stts 的樣本總數與 stsz 一致、stsc 各 run 有序且使用 description 1、stco／co64 只能擇一。每份 sample 的非零 byte size、每個 chunk 的完整範圍都必須位於同一 mdat payload；chunk 不可倒退、重疊、超過樣本數或指向 metadata／其他檔案。允許 moov 在 mdat 前或後，允許固定 sample size、多 chunk／run 及 64-bit offsets。這是內容門檻，不宣稱壓縮 payload 已解碼或字幕已配對。

第十九／第二十批的可選 udta 只接受 native writer 的空 meta／ilst、空 Xtra，因此常見標籤／封面會被拒收。第二十一批依 [ADR 0054](0054-bounded-mp4-descriptive-tags.md) 開放有界的已知單值文字、JPEG／PNG 封面與少數數值欄位；未知子結構、超量／多值及一般 metadata 變體仍拒收。音訊檔內引用與時間規則沿用，不是普遍 MP4 相容性宣告。

## 原生時間與資源

通過門檻後設定固定 audio/mp4 MIME，沿用同一唯讀 IStream，不交付 URL 或 filename 給 MF。native type 必須為 Audio／AAC，固定 14-byte HEAACWAVEINFO tail＋ASC 在 bounded stack buffer 核對，PCM16 協商後率／聲道須保持一致。第十九批保守拒收 44.1 kHz MP4；第二十批完成下列獨立觀測及 Ready 驗證後擴大。ADTS 44.1 kHz 的既有觀測不能用來聲稱 MP4 必然有相同重疊，兩者門檻分別決定。

48 kHz 的真正合成 mono／stereo 以原有一微秒 gate 驗證；使用既有映射還原 native endpoint，不裁切音訊、補靜音、重設時間或放寬精度。公開 borrowed DSP 仍拒收 packet clock，正式 Prepared 使用同一有界 actor。PCM block 64 KiB、native sample 1 MiB、metadata 640 rows／十二秒、單一 pending、回覆兩份及 ASR batch 四份保持。

COM／MF 與 getter 留在原執行緒。停止、錯誤、舊 Session 撤回及 Ready 持有原檔 pin 都沿用既有 owner；取消於 native 呼叫返回後合作處理，不能承諾同步原生 API 的固定取消時間。沒有新 OS worker、無上限 queue、跨服務 fallback 或額外憑證讀取。

## 證據與限制

測試 encoder 沿用 cfg(test) 的 Windows AAC／MP4 RAM sink，只新增傳回真正 finalized MP4 的選項；合成 fixture 檔由 RAII 移除，正式產品不新增音訊保存／匯出。純 parser fixture 只驗證門檻，不當作可播放 AAC。

驗證包括容器 truncation／overflow／外部引用／codec／table 反例、byte mutation 不 panic、短讀／I/O／取消，以及真正 mono／stereo native tone、PCM／時間／pin。十三秒真正合成輸入透過兩個所選服務的明確 mock、16／24 kHz 前處理，核對原生 endpoint、全文詞彙、Draft／Review、Ready／取消／失敗／中文字幕優先及資源釋放；原生 endpoint 的 oracle 只讀 decoder timestamp／frames，不使用產品 map。

一般使用者 M4A、其他 encoder、影片／多音軌、非空 metadata、edit list、URL、Auto、完整語意與播放／實際定位仍待驗收或實作。合成／mock 成功不代表真實雲端品質；有效金鑰維持結案固定遮蔽介面手動輸入。Phase 08 尚未結案，Phase 09 尚未具備條件。

## 第二十批：44.1 kHz 原生時間證據

測試直接使用純 RAM 的真正 MP4 Source Reader，查詢 ReadSample timestamp、GetSampleTime、GetSampleDuration 及 PCM byte length。不呼叫產品 decoder、normalizer 或 map；不複製／保留整份 PCM。率／聲道、frame alignment、native sample 上限、錯誤／格式變更 flags、正時間及真正 EOF 都有核對，最多讀取 1024 次。每種率／聲道另使用約一秒、二秒加 137 frames、十三秒加 311 frames 三種長度，覆蓋非整除的尾段。

Windows 19045 的實際 mono／stereo 數值相同，以下每列代表兩種聲道，native overlap 單位為 100 ns：

| 率 | 輸入 frames | native samples | 解碼 frames | 最後 frame 端點（µs） | 最大 native duration 重疊 | 最大 frame 端點重疊（µs） |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 44.1 kHz | 44100 | 44 | 45056 | 1021677 | 1 | 0 |
| 44.1 kHz | 88337 | 87 | 89088 | 2020135 | 1 | 0 |
| 44.1 kHz | 573611 | 561 | 574464 | 13026393 | 1 | 0 |
| 48 kHz | 48000 | 47 | 48128 | 1002666 | 0 | 0 |
| 48 kHz | 96137 | 94 | 96256 | 2005332 | 0 | 0 |
| 48 kHz | 624311 | 610 | 624640 | 13013332 | 0 | 0 |

讀取 timestamp 與 sample time 必須相同；正式測試對兩種率都要求 native duration／frame 端點重疊不超過既有一微秒，不要求其他 Windows 版本重現表中固定數字。編碼器 padding 所產生的 frames 不會被裁切或捏造端點；Ready 以真正 decoder 觀測核對，而不以輸入 frame 數當作音訊時間。

44.1 kHz 的正式 decoder 測試另驗證 64 KiB block、DecodedAudioGate、tone／RMS、EOF、唯讀 pin 與原 bytes；十三秒加尾段經兩個所選服務的明確 mock、16／24 kHz、詞彙／Draft／Review 到完整 Ready，使用現有獨立 NativeOracle 核對每個 cue endpoint。停止／服務錯誤／中文 sidecar 優先覆蓋兩種率，外部引用反例仍無任何 speech／詞彙／翻譯要求。公開 borrowed DSP 繼續在 PCM 前拒收全部 compressed input。

正式程式只將 aac_native_format 的 48 kHz 限制限定於 ADTS，MP4 可接受原先已由 ASC 核對的 44.1／48 kHz；GUI 分別顯示兩者範圍。原有一微秒精度與 pipeline 容量皆沿用，沒有音訊修正、native timestamp 改寫、duration 替換或額外保存。容器門檻的 stts 正值及樣本總數一致性只是 metadata 檢查，不能單獨當作時鐘品質證據。

## 官方資料

[Microsoft MPEG-4 File Source](https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-source) 說明 audio/mp4 MIME、mp4a 可對應不同音訊 subtype，以及外部軌道／stream 支援的限制。因此產品另做自己的單音軌與檔內引用門檻，不把 MF 支援表視為本產品的驗收。

[Microsoft AAC Media Types](https://learn.microsoft.com/en-us/windows/win32/medfound/aac-media-types) 說明 MFAudioFormat_AAC 的 USER_DATA 包含 HEAACWAVEINFO 尾端及 AudioSpecificConfig。這次只接受可核對的短 LC 描述，未知 extension 不交給默認解碼器猜測。

後續第二十二批允許可選的 H.264 Baseline 視訊軌，核對兩軌引用與跨軌 byte overlap，見 [ADR 0055](0055-bounded-h264-aac-mp4-prepared-input.md)；初始單音軌門檻屬本文件的歷史階段。
