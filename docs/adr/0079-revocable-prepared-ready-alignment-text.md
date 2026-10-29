# ADR 0079：Prepared Ready 到定位工作的可撤回文字借用

日期：2026-10-22。狀態：Phase 09 第三批採用。

## 決策與原因

第二批定位擁有者接受 owned ExistingText，但 Prepared Ready 原文只能借用；直接複製全文會增加本次 RAM，直接把整份 Ready 包成 Arc 又會把媒體 pin／遠端 decoder 的生命週期帶進背景工作。本批把 Prepared 文件內部的文字 body 與媒體資源分離，提供明確 Session／controller library 入口，不自動開始 capture、服務、播放或來源採用。

`PreparedDocument` 仍只公開借用 getter，沒有 Clone／Debug／Serialize／公開 Arc。私有 `PreparedTextOwner` 持有原文、譯文與可能的 HTTPS 本文 quota lease；body 在 lease 前析構，確保文字真正釋放後才歸還配額。媒體 owner 留在原 Ready document，不在文字 owner 或定位工作中。

## 明確入口及來源限制

`PreparedSession::fuzzy_locator(capture_identity, epoch)` 及 `PreparedController` 同名委派入口只在目前 completion 已被 apply 為 Ready 時可用。Idle／Preparing／未 apply／失敗／停止狀態回 NotReady；不從 completion 預借文字，不自行 reap 或建服務。文件 Session 取自目前 Ready，capture Session／generation 與非零 epoch 由 caller 明確交付。

只接受 Ready 的五語 `PreparedOriginal::Existing`，在 CPU 上借用真正原文；不把臺灣譯文、中文或 Prepared recognized transcript 包裝成既有字幕。不支援的來源明確 UnsupportedEvidence。這些結果仍只是 FuzzyCueProposal，不是內容身分、同發行版本、Current position、offset、CandidateEvidence 或自動採用證據。

## 弱參考與真實清理

locator 與尚未 poll 的 Future 只持有私有 Weak 文字參考及 Prepared Session 發布 token。真正取得單擁有者／共用定位 CPU 名額後，才升級文字 Arc，供一次 job 借用。查找仍掃描整份有界文件，不複製本文、不只挑單句，不建立假擷取戳記。原文及譯文共用 Ready 的原有 allocation。

Stop／start 新要求／Session Drop 同步取消舊發布並移除 Ready，因此 locator、未 poll Future、已交付 proposal 都不能讓舊 Ready 文字長駐或重現。沒有執行中工作時，保留 locator／proposal 不會保留文字或媒體；已有工作只能持文字至合作取消／清理，媒體 pin 可先釋放。每次請求同時連結 locator、caller 及 Ready 發布三個 scope，仍遵守原有最多四個扁平祖先；超限明確拒收，子取消不能取消 Ready 或其他 locator。

共用定位名額、每擁有者一項、三秒總預算與原擷取視窗五秒期限、逐 cue／normalization／DP row 取消、輸入先於 permit 清理、錯誤與 panic 路徑均沿用第二批。沒有新 worker pool、runtime、OS thread、第二份時間軸、HTTP 要求、AI 要求或永久內容。

## 驗證及後續

合成來源經真正本機 SRT／WebVTT、mock Translator 與 Prepared Ready 流程，核對五語、原生媒體時間、來源與 capture 兩份身分、原文字串 allocation 位址、弱參考強度、未 poll／Stop／替換／Drop、工作仍執行時釋放媒體 pin、舊候選撤回與父 scope 隔離。既有 HTTPS 本文 quota 測試另保留私有文字 owner，驗證 Ready 清除後至文字真正釋放前仍占配額。既有真實合成 ASR 測試確認 recognized Ready 沒有變成 Existing 證據。

這是 Ready 的背景定位 library 接線；一般視窗尚無播放／定位操作，Live 音訊控制器未自動使用此入口，沒有開始播放。中文配對、Prepared recognized adapter、多觀測可靠錨點、目前位置、seek／pause／restart 與 offset／drift 仍待後續；Phase 10 尚未具備條件。有效金鑰與真實雲端驗收留到結案固定遮蔽介面手動輸入。實際驗證見 [Phase 9](../phase-9.md)。
