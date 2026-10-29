# ADR 0078：共用定位 CPU 名額與模糊候選生命週期

日期：2026-10-22。狀態：Phase 09 第二批採用。

## 決策與原因

第一批有界模糊搜尋是同步核心，不能直接在擷取、UI 或 async executor 執行。第二批新增 `FuzzyCueLocator` library 擁有者與 async 入口，使用原精確定位的全程序名額，不另建定位 pool。`subtitle::alignment_work` 持有同一份 Semaphore，精確與模糊定位合計最多 `min(available_parallelism, 2)` 項已允許工作，無等待名額的佇列。每個模糊擁有者另限一項，過載明確回報；其他媒體／resolver 工作的既有容量不變。

精確驗證器僅改為取用同一份名額函式，匹配、原生時間、配對軌、採用證據及取消策略均未放寬。模糊候選仍不能轉成 `CandidateEvidence`、播放位置、offset 或 UI 字幕。

## 所有權與發布邊界

呼叫者把 owned `ExistingText` 移入擁有者，先核對文件身分與有界 cue 數、本文與 metadata bytes；不在建構時掃描完整文字。之後以私有不可變 Arc 借給 worker，不複製全文、不公開文件或新增快取。完整文件、原文語言及 ASR 驗證仍由 CPU 核心負責。換文件／發布／來源／seek 應停止舊擁有者，再以新 scope／epoch 建立新擁有者。

每次 `locate` 返回被動、Send、static Future；未 poll 不排工作。請求子 token 同時連結擁有者與呼叫者取消，沿用原有最多四個扁平祖先的限制，超量明確失敗。只放寬現有 `linked_many` 的 crate 內可見性，未更改其語意；子請求失敗或 Drop 不取消父 scope。

成功後解除該請求的 Drop 取消 guard，但候選保留兩個父 scope 的撤回連結。擁有者 Stop／Drop 或呼叫者取消會使已交付候選的 `validate` 失敗。ASR 最新 revision／sequence／utterance 與 epoch 仍須由使用者端交付 `validate`；此層沒有替 UI 維護最新辨識或自動採用候選。

## 排程、期限與真實清理

poll 時取得單一擁有者及共用名額，使用 Tokio `spawn_blocking`。名額包含在 Tokio blocking pool 等待實際執行的工作，因此這些 pending closures 也受一／二項上限限制，不會排一整份文件的 jobs。三秒總預算從本次首次 poll 計算，與原始擷取視窗五秒有效期限取較早者；同一絕對 deadline 傳到 CPU 核心，worker 開始不得重新計時。

明確取消或逾時先取消請求，abort 尚未開始的工作，await 真正 JoinHandle 清理後才回報原 Cancelled／Stale。已開始的 blocking closure 無法強制終止，靠核心逐 cue／正規化區塊／DP row 的合作檢查離開。Future Drop 無法 await，會同步發出取消並 abort 排隊項；已開始工作保留名額與輸入至真正退出，呼叫者必須維持 runtime 可用直到清理。Stop 釋放擁有者的文件參考，仍存活的 Future／worker 在釋放自身所有權後才回收餘下 RAM。

`AdmittedWork` 將輸入欄位排在兩份 permit 前，包含排隊 closure 被丟棄與 panic unwind，皆先釋放文件／ASR，再歸還名額。成功、typed error、panic 或取消均不交付部分本文；panic 轉為 `WorkerFailed`，沒有 runtime 時為 `RuntimeUnavailable`，滿載為 `Overloaded`。公開入口需 timer-enabled Tokio runtime，不在建構或 Drop 建立 runtime／OS thread。

## 驗證與限制

合成文件與受控 worker gate 驗證成功、未 poll Drop、明確取消、執行中／排隊 Future Drop、Stop／擁有者 Drop、已交付候選撤回、共用及單擁有者滿載、原始期限、panic／錯誤與真實資源釋放；沒有真正網路或使用者媒體。精確定位既有回歸與共享名額指標也保留。

這是獨立背景 library 接線，尚未連到 Live／Prepared controller；Prepared Ready 文件借用與辨識全文 adapter、經驗證的多觀測錨點、媒體目前位置、seek／pause／restart、offset／drift、正式 GUI 及真人媒體驗收仍待接續。不把取消測試或相似度當播放證據。沒有新增依賴、unsafe、服務、保存／匯出、第二份時間軸或讀取正式金鑰。完整結果見 [Phase 9](../phase-9.md)。
