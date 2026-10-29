# ADR 0086：明確對齊控制器與 epoch 生命週期

- 日期：2026-10-23
- 狀態：採用，Phase 09 第十批
- 範圍：library 對齊工作生命週期；播放器操作、實際擷取與一般 GUI 尚未接入

## 原因

既有 FuzzyProposalSession 固定綁定文件、擷取 Session 及 epoch。呼叫端在暫停、收到跳轉通知或更換擷取來源時，必須撤回舊提示，等待真正清理後再建新工作。僅更改旗標、丟棄 Future 或等待 driver 被 abort，不能證明開始中的 blocking 工作已釋放輸入。重用 epoch 也可能讓相同文件／擷取身分的舊結果混入新回合。

## 決策

新增 AlignmentController，從 Prepared Ready 或既有 ExistingTextLease 明確建立。建構保持 Paused，不建立 runtime、工作、擷取或服務要求。內部只保留弱文件綁定、原發布 guard 及原有工作許可；不持媒體 pin 或強本文。Prepared 中文 Ready 尚無來源參考軌，仍明確 UnsupportedEvidence。

- `resume().await` 使用呼叫端目前的 Tokio runtime 啟動一個既有 FuzzyProposalTask。健康執行中的 resume 冪等；從暫停恢復、重設或明確恢復異常工作時，配置新的非零 epoch。序號由控制器共用的程序內 AtomicU64 單調配置，checked overflow 明確失敗，不能回繞重用。這項唯一性適用於此控制器配置的 epoch。
- `pause().await` 先停止舊 owner，立即撤回候選／觀測快照與輸入，再等待 driver 及同一 local 許可。該許可由 AdmittedWork 在輸入和 CPU 許可之後釋放，因此即使 driver 被 abort，也必須等真正輸入清理結束。
- `reset(capture).await` 先驗證非零 Session／generation，再完成同一清理，重新綁定 capture 並恢復新 epoch。文件身分保持獨立。相同 capture 的 reset 仍產生新 epoch；它是呼叫端觀測到不連續後的明確通知，不偵測、推測或操作 seek。
- `stop().await` 為終止操作，撤回並等待清理後放掉來源綁定，可重複等待；後續 resume／pause／reset 拒收。停止對齊不取消來源的父 provider 或 Prepared Session。

三個生命週期 Future 一旦被 poll，就先封鎖新輸入並撤回舊提示。等待期間 Future 被 Drop 時，owner、JoinHandle、弱來源與已取得的 join 結果仍留在控制器。再次操作先完成原清理；已完成的 JoinHandle 不重複 poll。reset 在清理前中斷取消時保持原 capture，且維持 Paused；中斷 stop 保持終止狀態，可再次 stop 等待。

控制器的 Paused／Stopped 狀態表示輸入已封鎖，不單獨證明資源清理完成；必須等待操作成功返回。Controller Drop 是緊急取消：停止 owner、abort driver，已開始 CPU 工作仍合作退出並保留許可至實際清理，不宣稱同步 join。呼叫端應維持原 runtime 存活並等待 stop 完成，以確認清理。

來源取消／本文消失／原 query 到期都不續用舊候選。暫停、resume、新 Final 或 reset 不延長 lookup 原五秒期限；新出版物須新控制器。單軌／中文參考軌沿用原四祖先及文字／CPU／時間上限，沒有 relay task、額外來源要求或第二條 STT／翻譯管線。

## 界線與驗證

沿用可撤回的 lexical／時間一致性提示，不能轉為 ExactCueMatch、CandidateEvidence、字幕採用、可靠錨點、offset 或目前播放位置。ExactCueVerifier、source arbiter、capture-only LivePlaybackClock 與 TranscriptTimeline 不改接。這項 API 不暫停／跳轉播放器，也不啟動擷取，Prepared Ready 不自動播放。

測試使用真正 RAM lookup 發布、合成外掛字幕的 Ready 及明確 translator mock。涵蓋重複恢復、新 epoch／capture 隔離、舊快照撤回、父來源隔離、期限、弱本文／媒體釋放、runtime 缺少、序號用盡、driver abort 與生命週期 Future 中斷／重新等待。測試用 local 許可模擬仍在清理的輸入，驗證清理屏障及 join 結果保留；不將這個 gate 當成成功定位或正式 worker。實際命令結果記錄於 [Phase 9](../phase-9.md)。

無新依賴、unsafe、永久內容、產品網路／磁碟 I/O 或雲端測試。一般視窗、實際擷取控制、可靠錨點與媒體時間規則仍待後續；Phase 10 尚未具備啟動條件。金鑰仍留至結案固定遮蔽介面手動輸入。
