# ADR 0048 — 有界解碼取樣時鐘與原生時間映射

日期：2026-10-13  
狀態：採用於 Phase 08 第十三批的 library 核心；一般 MP3 DSP／ASR 入口仍保留時鐘拒收。

## 問題

第十二批真正 MP3 測試顯示，44.1 kHz 壓縮封包時間獨立取整，不能直接當作連續 PCM frame 時鐘。既有 DSP 要求每個連續 run 由累積取樣數推導時間；放寬精度會影響已驗證的 WAVE，直接重設時間則會失去原生來源時間與缺口。

## 決策

新增獨立 `media::decoded_clock`，不修改原生 reader、DSP、分段或服務協定。`DecodedClockNormalizer` 消費真正 `DecodedAudioBlock`，沿用既有 Session／generation、sequence、format、時間排序門檻。每個 run 以累積 frame 計算 sample clock；單一 block 的 PCM allocation 原樣移交，沒有複製、裁剪、補靜音或重取樣。`MappedDecodedAudioBlock` 將明確命名的 sample-clock block 與原生時間 span 一起交付，不能把正規化時間當作播放證據。

首 run 以第一原生 timestamp 為起點；後續原生缺口大於既有一微秒取整容差時，新的 sample origin 為「前 sample end＋實際原生缺口」。不直接改回下一 native timestamp，避免長 run 的累積量化偏移吞掉小缺口。連續封包的量化差異由各 span 保留，不建立假 run 或提高 WAVE DSP 容差。所有加乘、序號、run 及二十四小時上限均檢查。

每個 span 保存 sample start／end、native start／frame end、來源身分、序號、run、格式及真缺口，也保留 sample run origin 與 source frame 起迄。窗口獨立核對累積 frame 相位與前一 span 的 frame 接續，防止同一 Session 混入另一 normalizer 的不同取整相位；只核對表面相接的微秒端點不足以保證正確。只有 native start 來自原生 timestamp；native frame end 為此起點加上真正 decoded frames 的取樣時間，並非 `IMFSample::GetSampleDuration` 的觀測值。Microsoft 的 [ReadSample 文件](https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsourcereader-readsample) 明定 timestamp 使用 100 ns，STREAMTICK 表示缺口；現有 reader 轉成微秒，映射不憑空恢復未保存的小數精度。

`DecodedClockWindow` 只保留最多 512 spans、sample-time 範圍最多十二秒的 metadata。兩者都是上限；極小封包可能先達到 span 上限，沒有保證任意輸入都能保留十二秒。超限明確 Overloaded，清除 retained evidence 並成為 terminal，不靜默淘汰仍可能被使用的時間資料。呼叫端只能明確 prune 已不需要的完整前綴；保留跨 prune 邊界的原始 span，不捏造裁切端點。窗口保留最後順序狀態，即使所有 retained spans 已清空，也不能重複／倒序 append。

非空半開 cue 區間必須完整落在窗口內同一 run；檢查來源身分與所有涵蓋 spans，拒絕跨缺口、未知 run、缺少或已 prune 的區間及外推。每個 span 內依 native frame duration／累積 sample duration 做整數有理內插，最後取整一次；原生端點精確保留，不累積封包取整誤差。共享端點的 start 使用右側封包，end 使用左側封包，因此原有最多一微秒的 native 相鄰取整差異仍可見。這不是播放對齊分數，也不能保證相鄰轉換結果完全沒有一微秒重疊；後續 cue 合併須驗證並明確拒收不相容時間，不能裁句或猜測。

所有失敗與取消皆為 terminal；窗口清除 metadata，normalizer 清除 run evidence，保留自己的取消 token，不能透過換新 token 恢復。狀態沒有 PCM archive、本文、路徑、序列化或磁碟輸出。操作只在 caller 的有界 CPU 工作內完成，不新增 thread、網路、I/O 或 queue wait。

映射結果 `NativeMediaInterval` 由窗口建立，欄位私有；除起迄時間外，保留完整 Session／generation、來源 run 及涵蓋的第一／最後 decoded block 序號。使用 getters 讀取，不能自行建構或更改時間冒充經核對的結果。來源 block 序號與 run 不等於 ASR chunk 的序號／run，後續接線須明確綁定；時間 metadata 本身仍不構成 Ready、媒體配對或播放定位證據。

## 接線界線

本批完成可運作的時鐘核心及原生 MP3／DSP 證據，沒有將其接入一般 Prepared worker／GUI，也沒有開啟 MP3 上傳。`prepare_local_audio`、私有 producer 及正式 ASR 仍在 PCM／服務要求前回 `UnsupportedAudioClock`，已有字幕優先與 WAVE 流程保持原有行為。

接線還需讓每份 chunk 在產生時持有不可變、有界的映射，包含左右 context。現有最多四份十秒 ASR batch 可能等待四十秒以上，不能等結果回來才查十二秒 rolling window。原生 run 也不必然等於 chunk run：極短 native run 可能沒有輸出 frame。provider 回應先在 sample clock 核對正文／context，再用該 chunk 的映射轉回 native media time，最後合併須驗證共享端點、順序、EOF、來源與完整文件。完成這些之前不能宣稱 MP3 Ready、Prepared 播放或任意使用者檔案相容性。

## 驗證

純 RAM 測試涵蓋長 run 的 44.1 kHz 取整、小真缺口、原有一微秒邊界、來源／序號／格式、區間內插、端點、prune、兩種容量上限、二十四小時／溢位、取消與 terminal 清理。真正原生測試以 Windows 在 RAM 產生的自有兩秒 MP3，涵蓋 32／44.1／48 kHz、mono／stereo；原 block 經映射後交付完全未放寬的 16／24 kHz DSP。核對同一 PCM allocation／checksum／frames、原生端點映射、每 run 的精確輸出 frame 數、缺口、短尾、tone、檔案 pin 與取消後新 Session。

另以十三秒真正 MP3 在兩種 DSP rate 驗證 rolling window 明確回收，長 run 量化偏移保留於 native metadata；EOF 短尾不外推到原生結束時間，已 prune 的初段證據不能再使用。測試專用 encoder 上限調為最大 rate 的十六秒、壓縮 bytes 320 KiB，仍逐小段合成／編碼，不建立整份 PCM buffer；不改變產品容量或加入編碼入口。

合成測試沒有正式金鑰、使用者音訊或雲端要求；來源檔由測試 RAII 管理，產品不建立內容檔。實際通過指令與數字於 [Phase 8 第十三批驗證](../phase-8.md#第十三批驗證) 記錄。金鑰仍於結案在固定遮蔽介面由使用者手動輸入，Phase 09 尚未具備啟動條件。
