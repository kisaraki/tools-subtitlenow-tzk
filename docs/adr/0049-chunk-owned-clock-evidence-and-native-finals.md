# ADR 0049 — 分段持有的時間映射與原生辨識定稿

日期：2026-10-13  
狀態：採用於 Phase 08 第十四批的 library；一般 MP3 producer／GUI 尚未開放。

## 問題

第十三批的十二秒 rolling window 無法等待最多四份十秒辨識要求全部完成。窗口回收後若才查映射，會失去來源時間；把正規化取樣時間直接寫入 PreparedTranscript，則會冒充原生媒體時間。原生 run 也可能因為過短而沒有輸出 frame，不能假設 source run 等於 chunk run。

## 決策

`DecodedClockWindow::snapshot` 依完整 audio／左右音訊脈絡範圍移交不可變 `DecodedClockSnapshot`，從涵蓋的 spans 推導來源 run。保留原始完整 span，不裁切其端點或重算 frame 相位；另外記錄精確可用區間，不能取用區間外的資料。最多 512 spans／十二秒，沒有 Clone、Debug、Serialize、PCM、本文或路徑。窗口 prune、Drop 或後續資料錯誤不會破壞已移交的有效 metadata；真正準備取消仍撤回快照。

快照保存原始 window 與建立當下的準備取消 scope；只複製既有 token，不建立 child、relay task 或 Drop 後取消 parent。不能綁定 provider request child：目前要求 guard 在正常返回後也會取消 child，否則成功結果會被錯判取消。窗口與快照使用相同的半開區間／有理內插函式，完整驗證來源、區間、run、原生正長度及容量。

`PreparedSpeechChunk::with_clock_map` 消費快照，核對完整身分及精確 audio 起迄與正文 coverage，拒絕已綁定後覆寫。原有 PCM allocation 原樣保留，stamp／provider 回應仍使用辨識取樣時間。只有內部 Arc 共享不可變 metadata，merger 不持 PCM；服務 adapter 不將 source map 傳入雲端 payload。既有 `new` 的直接時間模式保持可用，同一份 mapped 文件不能混用兩種模式。

merger register／complete 必須使用同一個 map owner，不能用內容相同的另一份快照重綁。相鄰映射核對 format、source run origin、完整共用 source rows；無共用 rows 時必須具備相鄰來源 sequence／frame 及原生時間。比對來源 row 的交集，允許合法擴充左脈絡；來源 metadata 的音訊結束時間不能倒退，以免丟掉已提供的未擁有右脈絡。正文及 chunk sequence 的前進仍由原契約核對。

同一 chunk run 必須對應同一 source run；新 chunk run 的 source run 必須嚴格前進，可以略過沒有輸出 frame 的原生 run。不能要求兩種 run ID 相等或同時加一。原有 sample 正文連續性、新 run 真缺口與最終 `audio_end == body_end` 規則保持；EOF 短正文可擁有前段右脈絡，audio end 可以相同，甚至重用相同的完整音訊脈絡範圍。DSP 合法不足一個 target frame 的短尾不延伸到最後 raw source span 的端點。

complete 先核對回應身分、語言、sample-clock 時間與唯一正文；排除脈絡或跨界句的規則保持原有行為。按原序還原後，逐 cue 使用所屬不可變 map 轉成原生媒體時間，核對正長度、原生正文邊界及整份順序。任何 cue 重疊，包含一微秒，明確拒收，不裁句、移動時間或放寬精度。轉換後才有 PreparedTranscript；不代表媒體配對、Ready 或播放定位證據。

## 取消與容量

空白／靜音回應、倒序暫存結果、已提交本文及最後文件交付都核對準備取消。merger 保留每個不同 scope 的小型 token，較早 creation scope 在提交後取消也會撤回整份文件，優先於 NoSpeech；不只檢查最後一段。context 在撤回後回傳空值，下一個 register／complete 清除全部本文與 retained guards；finish 消費 owner，不交付部分結果。

scope guards 上限為兩個 scope × 四份 batch，即八個不同 scope；依既有 `same_scope` 去重，正常原生 producer 重用 Session scope 不增加數量。超過上限明確 Overloaded，不累積整部媒體的取消樹或忘記已提交來源的撤回規則。只保留 tokens，不保留所有舊 maps。最多四份 pending slots 與最後一份已註冊 map；Slot 與 request 共享同一份資料，額外記憶體由既有 batch、窗口及快照上限約束。沒有新增 worker、queue、mutex、網路要求或磁碟內容。

## 驗證與接線界線

純 RAM 測試核對精確脈絡、半開端點、窗口回收後快照、四份延遲／倒序要求、來源／frame 相位、重綁及混用拒收、原生一微秒重疊、EOF 同端點、左脈絡擴充、零輸出 source run、空結果及較早 scope 取消、去重與容量。真正自有十三秒 MP3 經 native decoder、未放寬精度的 16／24 kHz DSP、分段、快照及明確 mock ASR；窗口與 decoder 在辨識前銷毀，以原始 native timestamp／frames 的獨立寬整數公式核對最終時間與同一檔案 pin。

本批完成分段及辨識合併的 library 契約，不修改原生 producer factory、`prepare_local_audio`、正式 MP3 gate 或 GUI。產品無字幕 MP3 仍在 PCM／服務要求前回 UnsupportedAudioClock，已有字幕仍優先；WAVE 及固定金鑰介面保持原有用途。後續需將快照移交接入有界 native actor，處理大 block、長缺口與窗口邊界，再決定正式開放 MP3。合成／mock 測試不能代替真正辨識品質、VBR、使用者媒體或播放驗收。

實際指令與數字見 [Phase 8 第十四批驗證](../phase-8.md#第十四批驗證)。沒有新增 crate、unsafe、音訊／本文保存、內容快取或匯出；有效金鑰維持結案固定遮蔽介面由使用者手動輸入。Phase 08 尚未結案，Phase 09 尚未具備啟動條件。
