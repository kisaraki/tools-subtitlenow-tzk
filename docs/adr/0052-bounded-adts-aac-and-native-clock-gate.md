# ADR 0052 — Bounded ADTS AAC and Native Clock Gate

2026-10-14，Phase 08 第十七批建立、第十八批擴大聲道範圍，Windows 19045 基準。

## 決策與原因

新增本機 MPEG-4 AAC-LC／ADTS 的內容門檻與原有 Prepared native actor 接線。初始限 48 kHz 雙聲道，第十八批以直接原生觀測及完整合成／mock Ready 驗證後擴大為 48 kHz 單／雙聲道。沿用取樣／原生時間映射、同一解碼 worker、分段快照與定稿合併，不新增另一條辨識或翻譯管線。已有可靠字幕仍優先；沒有字幕才依明確服務／五語設定辨識，完整成功才有本次 RAM Ready。

內容門檻辨識 44.1／48 kHz、明確單／雙聲道的 AAC-LC 組合，解碼建立階段目前只開放經驗證的 48 kHz 單／雙聲道。44.1 kHz 單聲道合成檔實際出現：第 4 個解碼 block 起點 975235 µs，前段由真實 PCM frames 推導的終點 975239 µs，重疊 4 µs。原有 gate 限一微秒取整差；不能重設時間、補靜音或改掉端點以通過。44.1 kHz 單／雙聲道在 PCM／服務要求前回 UnsupportedAudioClock，保留後續獨立時鐘工作；不是宣稱其 codec 不存在。

## 格式與資源

固定讀取前十二 bytes、第二個相鄰 ADTS header 的七 bytes，以及第二 frame 的最後一 byte，合計最多二十 bytes。核對 MPEG-4 ID、layer、AAC-LC profile、取樣率索引、非 PCE 的單／雙聲道、單一 raw_data_block、七／九 byte header 的有效非空 payload、第二 frame 完整及相同格式；不掃描任意 sync、相信副檔名或讀取整份檔案。CRC 欄位只作 header 長度辨識，不宣稱 checksum 已由門檻驗證。ID3 前置 AAC、MPEG-2 ADTS、其他 profile、HE-AAC、其他率／聲道、ADIF、LOAS/LATM 及 MP4／影片容器尚未開放。

MF 使用同一唯讀 pinned file object 與固定 audio/aac MIME；native type 必須是 AAC，PCM16 協商後再核對真正取樣率／聲道沒有改變。重要 COM getters 都限定同執行緒及 owned interface，取消在 native 呼叫邊界檢查。公開 borrowed DSP 仍只接受直接取樣時鐘；AAC 和 MP3 一樣在該入口前置拒收，正式 Prepared 使用分段持有的原生時間證據。

AAC 每個壓縮 frame 對應 1024 個 PCM frames；合法最密集來源若每次交付一 frame，48 kHz 的十二秒窗口最多需要約 564 筆含邊界的 metadata。共同 row cap 從 512 改為 640，時間仍限十二秒；查詢含 pending 最多 641 rows、完整快照最多 640。這只調整有界 metadata 預算，PCM block 64 KiB、單一 pending block、分段回覆兩份、ASR batch 四份、CPU／native owner 許可及取消規則均維持。超過 row cap 仍明確 Overloaded，未觀測範圍不能外推。

實際 MF 可能合併多個 AAC frame 成為一個解碼 sample，不能把壓縮 packet 數當作 native row 數。十三秒合成檔以真正 decoder 的觀測建立獨立 endpoint oracle；另外以 560 個 1024-frame 的純 RAM spans 驗證最密集情形與十二秒上限，不將合成 span 當作實際播放器相容性證明。

## 測試輸入與生命週期

測試使用 Windows AAC encoder，在可寫／可 Seek 的 SHCreateMemStream／MP4 RAM sink 編碼；再由 Source Reader 讀取 raw AAC packet，包裝 ADTS header。這只存在 cfg(test) 的既有 fixture 模組，不是產品編碼器／匯出功能。沒有安裝元件、使用者錄音、輸出 URL、正式金鑰或雲端要求。

此機的 HGLOBAL stream 對 MF 回報 readable／seekable，MP4 sink 要求 writable；AAC fixture 因而使用明確可寫的記憶體串流。Sink Writer 使用零起算 stream index，與 MP4 sink 的 stream identifier 分開。COM／MF、native buffer Lock／Unlock 與 RAM stream 依原有 RAII 在同執行緒釋放，合成檔案另由測試 TempDir 清理。

正式流程沒有新增產品音訊寫入、temp、cache、subtitle／glossary 持久化、跨供應商 fallback 或自動播放。Ready 原檔 pin、停止／錯誤／關閉與舊 Session 撤回沿用原 owner。

## 第十八批的直接原生時間證據

在測試內以純 RAM 真正 Source Reader 解碼十三秒合成 AAC，分別查詢 ReadSample timestamp、IMFSample::GetSampleTime、GetSampleDuration 及總 PCM bytes。兩個 timestamp 必須一致、PCM 長度有界且 frame-aligned、duration 為正、格式符合輸入，最多讀取 1024 次並要求真正 EOF。只累積 frame／時間統計，不複製或保存整份 PCM；COM／MF／reader／sample 都在相同執行緒使用及釋放。正式 decoder 不新增 duration 修正或測試 bypass。

本機 Windows 19045 的實際觀測如下，ticks 單位為 100 ns：

| 取樣率／聲道 | native samples | PCM frames | 最大相鄰原生 duration 重疊 | 最大 duration 與 frame 推導差 |
| --- | ---: | ---: | ---: | ---: |
| 44.1 kHz／單聲道 | 41 | 573440 | 43 ticks | 10 ticks |
| 44.1 kHz／雙聲道 | 28 | 573440 | 43 ticks | 15 ticks |
| 48 kHz／單聲道 | 28 | 624640 | 0 ticks | 7 ticks |
| 48 kHz／雙聲道 | 28 | 624640 | 0 ticks | 11 ticks |

直接採 native duration 並未消除 44.1 kHz 重疊，故不能假設 GetSampleDuration 是時鐘修復。48 kHz 單聲道另通過原有 PCM/frame gate、兩秒 tone／EOF／pin 及十三秒全 Prepared 到 Ready 的獨立 endpoint oracle，因而只擴大此組合。測試只對開放的 48 kHz 要求 native duration 重疊不超過原有一微秒；不要求其他 Windows 版本必須重現 44.1 kHz 的特定錯誤數字，也不宣稱所有編碼器或長度已驗收。

GUI 提示改為 48 kHz 單／雙聲道。原有十二秒／640-row、native sample 一 MiB、PCM block 64 KiB、worker／ASR、Session、取消、中文優先、RAM Ready、固定金鑰與無自動播放規則保持。48 kHz 單聲道的停止／mock 服務錯誤與中文字幕優先也經同一 controller 驗證，44.1 kHz 前置拒收仍沒有 PCM／ASR／詞彙／翻譯要求。

## 官方資料與限制

[Microsoft AAC Encoder](https://learn.microsoft.com/en-us/windows/win32/medfound/aac-encoder) 說明 LC 編碼、44100／48000 輸入與每個 AAC frame 的 PCM 數量；[AAC Decoder](https://learn.microsoft.com/en-us/windows/win32/medfound/aac-decoder) 說明 ADTS／AAC subtype 與單一 raw block 要求。

[MFCreateMPEG4MediaSink](https://learn.microsoft.com/en-us/windows/win32/api/mfidl/nf-mfidl-mfcreatempeg4mediasink) 說明可寫／可 Seek 的輸出 stream 及 audio-only sink；[SHCreateMemStream](https://learn.microsoft.com/en-us/windows/win32/api/shlwapi/nf-shlwapi-shcreatememstream) 說明記憶體 IStream 的配置／釋放介面。

[GetSampleTime](https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfsample-getsampletime) 回傳 presentation time，[GetSampleDuration](https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfsample-getsampleduration) 回傳 duration，兩者使用 100 ns 單位；duration 可以不存在或為零，官方介面本身不保證本專案所需的跨 sample 精度。

合成／mock 驗證不代表真實 AAC 檔案、其他編碼器、受保護媒體、播放器同步或雲端品質已通過。44.1 kHz 時鐘、其他 AAC 包裝／容器、Auto、URL、全文語意與播放／定位仍待接續。Phase 08 尚未結案，Phase 09 尚未具備條件；金鑰仍留至結案固定遮蔽介面手動輸入。
