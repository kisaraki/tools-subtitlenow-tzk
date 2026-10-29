# ADR 0080：最新定位候選的有界狀態管理

日期：2026-10-22。狀態：Phase 09 第四批採用。

## 決策與原因

定位 worker 與 Ready 弱參考解決單次工作的容量及文字生命週期，但個別呼叫者仍容易保留舊 ASR 候選、替每次更新建 Future，或在背景延遲後重設期限。本批新增 `session::fuzzy_proposals::{FuzzyProposalSession, FuzzyProposalTask}`，統一管理最新接受的 Final ASR 與對應候選，不改精確驗證、仲裁或時間軸。

建構時移入一份 locator，返回 owner 與 caller 必須持有／poll 的 Task，沒有內建 spawn／runtime／thread。owned 及 Prepared Ready locator 共用此入口；文件身分、實際擷取身分、epoch 由既有 locator 提供，不建立假 capture stamp。來源、文件發布或 seek epoch 改變時，caller 必須停止並建立新的 owner。

## 輸入與替換

只接受同擷取身分、有完整範圍的五語 Final ASR；ASR sequence 不得晚於視窗最後一塊 sequence。保留原有全文、Unicode、唯一性、時間及 DP 契約，不把臺灣譯文當原文。

新 utterance 的 sequence 與 utterance 必須同時遞增，revision 可重新開始；同 utterance 固定 sequence，只接受較高 revision。倒序要求明確 Obsolete；窗口 sequence／觀測時間也不得倒退。錯誤輸入不取代目前已接受要求；唯一例外是同完整 binding 偷改本文、語言、時間或 kind，必須立即撤回原要求並回 InvalidData，包括新本文超量。這些規則管理最新接受的 Final ASR，不是新的 STT ledger 或 Partial 管線。

同 binding／payload 即使帶新窗口仍為重複，回 false、不建立 ticket、不重試、不延長期限。失敗、過載或未匹配後，也需下一筆有效 revision／utterance；沒有自動重試。保留一份最多 8192 UTF-8 bytes 的最新 ASR 快照作去重及水位，直到替換／Stop／來源撤回；錯誤與過期後不保留歷史清單。

## 有界工作與期限

輸入與結果各用一格 watch，最多執行一項定位工作；連續更新直接覆蓋待處理快照。新輸入同步取消舊 request，Task 等舊 worker 真正退出後才處理最新替換；舊結果、舊錯誤須通過 ticket、scope、完整 ASR binding 及取消檢查，才可改變目前狀態。結果沒有本文或文件 owner。

期限從 offer 成功時計算，取三秒及原擷取視窗五秒的較早值，將同一絕對 deadline 交給 worker，包含 Task 尚未被 poll、舊工作清理與 blocking pool 等待；不重新起算。直接 locator 的既有公開介面仍維持首次 poll 起算契約。名額滿載明確 Overloaded，不新增 CPU pool 或等待名額；全程序容量與每 locator 一項限制沿用。

## 狀態、取消與清理

狀態為 Waiting、Locating、Proposed、Unmatched、Failed、Cancelled、Stopped。只有目前且仍有效的 Proposed 可以借用 metadata；不對外 Clone／Serialize 內容。poll 最多消費最新結果，changed 可與 caller 的新 ASR 等待一起 select；丟棄等待 Future 不取消 owner。

Stop／owner Drop 取消 request 並清除最新輸入、結果及本文；caller await Task 才表示開始的 CPU 工作完成清理。Task Drop 沿用 locator／worker Future 的合作取消規則，不能強制終止已開始的 blocking 工作，runtime 仍須維持到實際回收。Ready 撤回及 locator Drop 透過只讀 availability token 喚醒 idle／pending／已交付 owner；此 token 不加入 job 祖先鏈，因此仍遵守原有四祖先限制。子 request、owner 或單一 locator 取消均不取消 Prepared Ready 或其他 locator。

## 限制與驗證

以合成資料驗證連續修訂合併、舊輸出隔離、原期限、去重、錯誤／過載、真正 blocking pool 排隊與清理、等待取消安全性、driver／owner Drop、Ready 撤回及父 scope 隔離。實際檢查結果見 [Phase 9](../phase-9.md)。

本批沒有 Live 自動接線、一般視窗操作、Prepared recognized adapter、中文配對、多觀測可靠錨點、播放位置、seek／pause／restart、offset／drift 或採用。相似度及兩段時間差不構成上述證據；Prepared Ready 不自動播放。未讀金鑰、傳送私有內容或新增永久內容／依賴／遙測／unsafe。Phase 10 尚未具備啟動條件；正式金鑰維持結案固定遮蔽介面手動輸入。
