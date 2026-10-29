# ADR 0069：有界 HTTP PCM16 WAVE 解碼

日期：2026-10-16。狀態：Phase 08 第三十六批採用。

## 決策與範圍

以已通過前置檢查的 ProbedRemoteAudio 為輸入，新增 RemoteWaveDecoder，非同步拉取並實際解出未壓縮 PCM16 WAVE。輸出沿用 DecodedAudioBlock／DecodedAudioGate；本機 Windows Media Foundation、原有壓縮音訊解碼與 Prepared 管線保持原樣。

現有本機 MF IStream 可由原生內部 MTA 執行緒回呼，完整 HTTP／COM 接線還須管理跨執行緒 CPU 配額、取消及原生資源退出。本批先提供不需壓縮 codec 的 PCM16 WAVE 路徑，沒有把原生同步 MediaDecoder 假裝成可在 executor 上等待網路，也沒有重寫 MP3／AAC decoder。這是獨立 async library；一般 Prepared／GUI 仍拒收 URL，沒有 ASR、Ready、自動播放或永久內容輸出。

## 容器與音訊

格式依據為 Microsoft 的 [RIFF](https://learn.microsoft.com/en-us/windows/win32/xaudio2/resource-interchange-file-format--riff-) 與 [WAVEFORMATEX](https://learn.microsoft.com/en-us/windows/win32/api/mmreg/ns-mmreg-waveformatex)。下列限制是本批的明確子集合，不是所有合法 WAVE 都必須遵守的規則。

只接受 little-endian RIFF／WAVE、單一十六-byte fmt、WAVE_FORMAT_PCM、16 bits、單／雙聲道，取樣率沿用 DecodedAudioFormat 的 8–192 kHz 上限。核對 block alignment、byte rate、資料完整框格及 RIFF total；不從 URL、副檔名或 MIME 推斷。多聲道需要聲道配置，本批不接收 WAVEFORMATEXTENSIBLE、float、8／24／32-bit、壓縮格式、RF64 或 RIFX。

fmt 必須在唯一非空 data 之前，完整 RIFF 最多128 chunks。只額外接受總計最多一 MiB 的 JUNK，跳過其本文；奇數長度須有本子集合要求的零 padding。未知 LIST／其他 chunks、重複或遺漏 fmt／data、截斷、資料長度不符或殘留 bytes 都在返回第一塊 PCM 前拒收。沒有靠略過未知音訊清單、裁掉壞資料或補靜音來通過。

每塊最多64 KiB、完整框格對齊。時間由累積 frames／sample rate 一次整數取整產生，不加總已取整 duration，不用 wall clock；沿用一微秒捨入容差與二十四小時上限。HTTP／PCM 的 sequence 各自保持原有語意，全部綁定同一 identity／generation。DecodedAudioGate 拒收錯序、錯身分或非預期缺口。data 完整交付後才有 EOF，EOF 重複拉取不發網路要求，取消仍優先。

在第一塊 PCM 前，依 data 大小與完整框格區塊數核對剩餘 HTTP sequence 預算，必須包含已花在 probe／標頭的要求；不能處理到大檔案末尾才發現超過65536份限制。以只提供44-byte實體標頭、虛擬總長的 sparse loopback fixture 核對二十四小時與四 GiB 要求預算邊界，不分配或宣稱實際讀完 GiB 音訊。

## 容量與取消

保留原 HTTP owner、URL、client、強 ETag、長度與已前進的要求序號；每個標頭／data range 都沿用 If-Match，不重新開啟來源。標頭 range 用完立即釋放，不累積配額。版本變更與網路錯誤保留 typed error，任何錯誤或 Future Drop 都退休 scope，先前交付 PCM 的新存取也失效。

除了 ADR 0067 的四份 HTTP range buffers，另有全程序四份在途／下游保留 PCM 許可，至多256 KiB PCM payload。RemoteWaveBlock 不提供 Clone／Debug／Serialize／into_inner，借用 DecodedAudioBlock 也先檢查取消；decoder Drop 不釋放仍被下游保留的 PCM 配額，只有 block Drop 才釋放。過載在新 I/O 前明確失敗，不排隊或用切換來源繞過配額。呼叫端仍須遵守既有禁止複製／保留音訊歷史的契約，並在消費邊界核對取消與 Session；Rust 已借出的 slice 不會因 token 而物理撤銷。

網路等待不持 CPU 許可。收到一塊 range 後，以共用 Prepared CPU lane 的 try_acquire 取得許可，才配置及轉換 PCM；每4096 samples 檢查取消並合作 yield。這是小量有界 endian conversion，不是長時間 codec 或另建 CPU pool；沒有每 block 工作執行緒、背景預讀、無界佇列或來源音訊檔案。CPU 過載、取消及 Future Drop 會釋放當次 wire／PCM／CPU 許可，不交付部分 block。

## 驗證與後續

以實際 TCP loopback 和自有合成 PCM WAVE 驗證取樣值、單／雙聲道、時間與尾段、完整容器拒收、seek-free data 拉取、來源版本、CPU／PCM 配額及取消。測試不擷取使用者音訊、不使用正式金鑰；實際結果見 [Phase 8](../phase-8.md)。不以合成檔案或 library 成功宣稱一般網站、壓縮格式、雲端辨識品質或播放器同步已驗收。

後續仍在 Phase 08：將遠端解碼與既有分段／ASR／Ready 生命週期接線，加入 URL GUI 目的地揭露，另處理壓縮格式的原生 HTTP／COM owner。五語 Auto、真實媒體／語意／雲端及播放定位仍待接續；Phase 09 尚未具備條件，金鑰維持結案固定遮蔽介面手動輸入。
