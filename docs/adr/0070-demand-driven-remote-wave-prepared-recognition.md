# ADR 0070：隨需拉取的遠端 WAVE Prepared 辨識

日期：2026-10-16。狀態：Phase 08 第三十七批採用。

## 決策

新增 `recognize_remote_wave` 程式庫入口，接受已通過原有內容／PCM16 容器檢查的 `RemoteWaveDecoder`、來源語言、16／24 kHz 與呼叫端明確選定的 `PreparedSpeechRecognizer`。沿用同一 `PreparedAudioProcessor`、`PreparedSegmentBuilder`、`PreparedRecognitionSession` 及定稿合併，沒有第二套重採樣、分段或辨識演算法。服務能力先驗證，失敗不再讀取媒體或發出辨識要求。

本批接通來源辨識，不是全文翻譯、PreparedSession／Ready、URL GUI、網站解析或播放器。一般準備視窗仍明確拒收 URL。既有來源字幕優先規劃、本機 MF 路徑與固定遮蔽金鑰介面不變；正式金鑰仍留至結案由使用者手動輸入。

## CPU、背壓與容量

遠端 HTTP 非同步讀取不持有 CPU 許可。每次取得一份最多64 KiB PCM 後，將一份 DSP／分段狀態與該 block 移至 Tokio blocking pool；取得既有共用 CPU lane 的 try_acquire 許可後才排程，不另開 OS thread／CPU pool、不排隊等待配額。每個來源最多一份工作，完成後才開始下一份讀取；DSP 使用既有同步取消檢查。WAVE 不涉及需要同一執行緒釋放的 COM，故不保留專用 idle thread。這與本機原生 decoder actor 的執行緒需求不同。

每次 pull 最多回傳兩份分段，正文十秒、左右各最多半秒脈絡，單段最多十一秒。辨識端只允許零至兩份待處理分段繼續拉取；三或四份先完成辨識批次，再讀後續媒體。服務等待不持 CPU、HTTP range 或原始 PCM block，不預讀整份媒體。辨識的有界並行、逾時、脈絡、身分與亂序合併沿用既有契約。

原來兩個遠端 owner、四份 wire buffers、四份原始 PCM、二十四小時／四 GiB／HTTP sequence 預算保持；分段及 DSP 的工作記憶體是另外的既有有界預算，不把256 KiB原始 PCM 宣稱為整條 Prepared 管線總記憶體上限。沒有磁碟、temp、音訊／文字快取或匯出。

## 取消與來源生命週期

公開 future 在建構時即持有取消 guard，未 poll Drop 也退休來源。任何網路／DSP／服務失敗不交付部分 transcript；來源變更等 typed error 不被連帶取消蓋掉。已開始的 blocking job 不會因 JoinHandle abort 而瞬間消失：它持有 CPU 及原始 PCM 許可直到真正退出；被取消的排隊工作也持有 captured 許可到 runtime 回收，替換來源不能繞過配額。一般取消等待當次 job 真正退出，future Drop 則取消 scope 並 abort 尚未開始的 job。

只有完整 data EOF、DSP 尾段與所有辨識批次成功，才回傳 `RecognizedRemoteWave`。它持有同一 EOF decoder／HTTP owner 與 transcript，原始 client／URL／強 ETag／要求序號未重開。最後服務等待與結果保留期間 owner 仍占配額；結果 Drop 退休 scope。文字只有檢查取消後的借用，不提供 Clone／Debug／Serialize 或取走 transcript 的入口。消費端仍須核對目前 Session／generation；已借出的 Rust slice 無法被 token 物理撤銷。

這證明本次讀取的來源一致性，不是內容識別、字幕版本、播放定位或 Ready 證據，也不會在 EOF 後反覆讀取遠端來源。

## 驗證與後續

以自有合成 PCM、真實 loopback HTTP 與明確辨識 mock 驗證分段、取樣值、尾段、背壓、脈絡、亂序、來源變更、服務失敗、過載、取消／Drop 及許可回收。結果記錄於 [Phase 8](../phase-8.md)。不使用正式金鑰、不擷取使用者音訊，不以 mock 文字宣稱雲端品質已驗收。

後續仍在 Phase 08：遠端全文分析／翻譯／RAM Ready、URL GUI 與用途揭露、壓縮格式及真實媒體／雲端／語意／播放驗收。Phase 09 尚未具備條件。
