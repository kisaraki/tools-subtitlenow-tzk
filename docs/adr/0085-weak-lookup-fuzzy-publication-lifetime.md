# ADR 0085：模糊定位借用 lookup 發布並保留原始期限

日期：2026-10-23。狀態：Phase 09 第九批採用。

## 決策與原因

第八批 owned bundle 無法自動遵守既有 lookup lease 的撤回與原始期限。本批新增 `FuzzyCueLocator::from_lookup(scope, &lease)`，接入已存在且驗證成功的 ExistingTextLease；單軌五語與帶同版本來源參考軌的中文共用此入口。不重抓來源、不將原有 Arc 本文複製進新文件，不建立假 query／capture stamp，也不改變來源選擇或採用規則。

resolver 層提供 crate-private WeakExistingTextLease，固定原始 query Arc、兩軌 Weak 與原 availability token。query 是有界 metadata，沒有本文；後續 renew 不修改這份快照。constructor 檢查目前文件身分、來源有效性及語言／參考軌條件；背景工作取得既有 CPU／local 名額後才同時升級兩軌，任一失敗即釋放已取得的另一軌。BorrowedLookupText 只持唯讀 Arc，沒有新的出版物或證據建構入口。

弱快照每次驗證均核對原 availability、query 有效性、原始五秒期限與兩軌是否仍存活。lookup 保留原先的合計 1024 cues／128 KiB 發布預算；不因較大的 owned 配對入口放寬。正式來源原已通過發布契約，CPU job 仍經相同原文／配對核心重新檢查，不依 constructor 的成功免除全文歧義、時間或 DP 驗證。

## 期限與撤回

工作實際截止時間取 offer／dispatch 三秒、capture WindowStamp 五秒及 lookup 原始 query 五秒的最早者，包含排程等待與本文驗證，不在升級或後處理時重設。FuzzyCueProposal 保留弱發布 guard；多觀測每點期限同樣取 capture／lookup 較早者，FuzzyTrackProposal 也保留弱 guard。新鮮 ASR、成功配對、重複輸入與同信心 renew 均不能延長舊快照；新的 lease query 必須建立新 locator／epoch。

FuzzyProposalSession 的 offer／state／poll／changed 核對來源 guard，閒置時也有原發布期限；driver 在沒有 ASR 的情況下到期仍會退出。到期回報 Stale，來源 availability 撤回回報 Cancelled，兩者均不能交付候選。driver 到期時只停止自己的 locator，不取消 lookup／provider 父 scope。工作已開始時，取消／期限仍等待真正清理，輸入與 CPU 名額到實際退出才釋放。

已有 Live owner 的 `lease.retire()`、較低確認信心產生的舊發布撤回、provider guard 取消，會透過既有取消通道喚醒管理器與工作。四祖先上限保持；來源 token 原有祖先也計入，額外 caller 祖先超限仍明確 InvalidData，不新增 relay task 或放寬上限。快照的弱 guard 與期限檢查不增加祖先。

若只釋放全部本文而未發出 retire／provider 取消，弱 guard 的 validate／poll 會立即拒收；Weak 本身沒有 Drop 通知，已等待中的 changed 最遲在原發布期限醒來。本文不會因此被保留。開始中的 job 可持借用到真正清理；工作清理後的結果仍須再次驗證，不能把已釋放來源的結果交付。正常來源撤回仍應使用既有 retire／provider guard 契約。

## 不修改既有 lookup 契約

本批不把 ExistingTextSession::clear／stop 改成取消所有先前建立的獨立 lease；現有 Live owner 仍負責 retire，provider guard 仍由原 metadata 擁有者管理。這保留既有精確定位與消費者的生命週期，避免為了新增模糊路徑而改變全體 lookup 行為。續租共用原本文，舊弱快照保留舊 query；確認信心下降照原規則撤回舊 availability。

定位器／未 poll Future／單次候選／軌跡快照都不保留強本文參考，沒有原生媒體 pin。這是明確 library 橋接，不是自動選擇、呼叫或訂閱新的供應端。一般 Live／GUI 自動接線、Prepared 中文 Ready 參考軌、可靠錨點、目前位置、seek／pause／restart、offset／drift 仍待後續；不改精確驗證器、仲裁、擷取時鐘與 TranscriptTimeline，不將模糊結果轉為採用證據。

## 驗證與範圍

真正 ExistingTextSession 搭配明確 RAM provider fixture，驗證單軌／中文雙軌的原 allocation、零額外 lookup、兩軌弱釋放、原 lease 撤回／父取消、閒置／待處理／已交付管理器、純釋放無 retire 的驗證／poll、來源先於 capture 到期、舊 query 不隨 renew 延長、較低信心撤回、正在清理時的本文／名額、工作原始期限、四祖先與父 scope 隔離及錯文件身分。

沒有新增依賴、unsafe、產品 I/O、內容寫入、服務要求、worker、runtime、OS thread、queue 或音訊保存。測試只用合成文字與 mock lookup，不是真實來源版本、雲端或播放相容性驗收。正式金鑰仍留至結案固定遮蔽介面手動輸入，Phase 10 尚未具備條件。實際命令與結果見 [Phase 9](../phase-9.md)。
