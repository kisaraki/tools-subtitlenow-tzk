# ADR 0082：有界的模糊候選時間一致性提示

日期：2026-10-23。狀態：Phase 09 第六批採用。

## 決策與原因

單次文字唯一匹配不能證明播放位置。本批讓既有 `FuzzyProposalSession` 累積已接受的 Final 定位結果，提供獨立 `FuzzyTrackStatus`／sealed `FuzzyTrackProposal`。這只描述近期觀測之間的時間一致性，不產生經驗證錨點、目前位置、偏移估計、seek 判斷或來源採用證據；精確驗證器、來源仲裁、擷取時鐘及 TranscriptTimeline 保持原路徑。

軌跡只保留最多八筆數值 metadata，不保留本文、文件或原生媒體資源。至少三個不同且遞增的 utterance／sequence／cue、相同來源種類、互不重疊且向前的 capture／media 範圍，以及至少一秒的首尾擷取觀測跨度，才回報 Consistent。跨度包含觀測間的空隙，不代表連續播放覆蓋。每筆起訖端點以 signed i128 計算 media 減 capture，整條軌跡所有端點差值的最大值減最小值不得超過 250,000 微秒。這是具名工程容差，不是信心機率、公開 offset 或 drift 校正；不能只比較相鄰兩點而容許累積漂移。

單筆長度差超限回 Rejected(TimingMismatch)。新觀測與既有軌跡順序、來源、範圍或全體差值不相容時，撤回舊軌跡，只以有效的新觀測重新蒐集並回報 Restarted；單次文字候選仍可維持 Proposed，不把不一致藏成文字失配，也不將重新蒐集宣稱為 seek detection。scope 的文件／擷取 Session、generation 與 epoch 不能混用；來源、文件或 epoch 變動時 caller 必須停止並重新建立管理器。

## 修訂、期限與撤回

完全相同的目前 binding／payload 是冪等輸入，不增加觀測、不重設期限。修訂先撤回該 utterance 的舊支持，再排新工作；已記錄的較舊 utterance 收到任何非目前完全重複訊息也保守撤回，即使最新要求規則回 Obsolete，仍不讓潛在修訂留下舊支持。舊訊息不改寫目前的單次候選。新輸入只有真正成功交付且驗證後才加入觀測，錯誤、無唯一匹配或目前要求過期會撤回軌跡。

每點使用原擷取 WindowStamp 的五秒期限，成功交付與後續輸入均不延長。`changed()` 使用目前工作與最早支持到期時間的較早者，舊支持先到期也會喚醒 caller。快照期限是所有貢獻點中最早到期時間，驗證同時檢查來源 availability 與獨立 revision token；任何支持新增、撤回、到期、重建、停止或 owner Drop 都會使舊快照失效。快照沒有 Clone／Debug／Serialize 或任意建立入口，亦無轉成 ExactCueMatch／CandidateEvidence 的路徑。

新 utterance 會照舊取消上一項原始定位工作與單次候選；已獨立接受的 Final metadata 則跟隨文件 availability、原觀測期限及明確修訂撤回，不再依賴被正常取代的 per-job token。這是累積多觀測的必要生命週期差異，不能以此恢復舊單次候選。Ready 停止／替換、driver 取消／Drop 及 owner 停止仍撤回快照。快照只保留兩個取消 token，不增加既有四祖先鏈、不延長文字或媒體 pin 的生命週期。

## 資源與範圍

owner 每次最多掃描八筆 metadata，達上限先移除最舊一筆再加入。沿用原 worker、共享 CPU 名額、三秒工作期限與有界最新輸入／結果；沒有新增 runtime、OS thread、佇列、服務、金鑰取用、I/O、依賴或 unsafe。所有資料僅本次 RAM，沒有音訊、字幕或翻譯保存。

一般 Live／GUI 自動接線、中文與跨 cue 配對、經驗證多觀測錨點、目前媒體位置、暫停／seek／重啟與 offset／drift 規則仍待接續。不能從靜音、持續擷取或牆鐘推算播放前進。Phase 10 尚未具備條件；有效金鑰與真實雲端／私有媒體維持結案固定遮蔽介面手動驗收。

## 驗證

純核心測試涵蓋不同觀測、修訂與重複、三點／一秒／250 ms 邊界、全軌跡累積漂移、反向／重疊、八筆上限、原期限、取消／Drop、錯 epoch、signed 大數運算及快照撤回。實際 worker 管理測試涵蓋舊 job 取消後仍可累積、舊訊息撤回、不匹配／過載／過期、driver abort、重新蒐集與較早支持到期喚醒；真正合成 SRT Ready 核對停止時文字與媒體 pin 可釋放、持有快照也不保留資源，且不增加翻譯服務要求。Recognized Ready 既有測試也核對首筆 metadata 接受。

這些是合成資料及明確 mock 的工程證據，不是實際播放或辨識品質驗收。完整實際命令與結果見 [Phase 9](../phase-9.md)。
