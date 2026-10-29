# ADR 0110：隔離單次未完整回傳的 Live 翻譯

日期：2026-10-27。狀態：採用，工程驗證與部署完成，已有單次提示後恢復的人工正例；其餘驗收待補，Phase 11 未結案。

## 問題與證據

同版重新啟動後，使用者補回報「掉字較少」，再確認來源為日文、按停止後字幕紀錄有保留。記為本次相對改善及停止保留正例，沒有量化錯字率或零缺漏證據，版本保持。

部署後第一次回覆選項 2，確認提示後後續繼續更新並恢復。該次提示種類、語言、時間與數值未另提供，不能特定歸因為 Incomplete 路徑，也不據此認定字詞完整度或其他矩陣通過；後續日文補測結果另記於上段，不改已驗證版本。

使用者測試 4F59DEC1… 版後回報自動停止，真正錯誤為 TranslationError::Incomplete，而非辨識佇列滿。最後辨識數值為收到 6489 包／送出音訊訊息 6484 則、合併前綴 3 包、服務訊息 189、暫定 37／定稿 16／交付 53、提交確認前暫存 103。這些是辨識端計數，不證明全部翻譯或原文完整度通過，也不能用包數差推論掉字。來源 PID 16368 與主視窗 PID 13500 不同；使用者已確認關閉主視窗。

合成回歸先失敗：已有一段完整譯文，下一段回傳 Incomplete 就讓整個翻譯 Session 取消，後續段落無法處理；1 failed、0.01 秒。本機證明的是錯誤傳播路徑，不代表已判定真正服務中斷原因。Incomplete 可能來自 response.incomplete、缺少完成事件、空的最終本文等；沒有實際原因欄位，不猜測為額度、付款或輸出上限，也不提高 token／逾時設定。

OpenAI [串流事件規格](https://developers.openai.com/api/reference/resources/responses/streaming-events) 將 incomplete 與 completed 分開。[官方 SDK Response 型別](https://raw.githubusercontent.com/openai/openai-python/main/src/openai/types/responses/response.py) 另列 content_filter 等原因；已知 content_filter 明確分類 Refused，不能當成一般未完成要求跳過。

## 決定

延伸 ADR 0096 的 Live 要求層級隔離，加入 Incomplete；Timeout／Stale 維持原策略。只有 run_live 不因單次這類失敗取消整個 Session；一般 TranslationJob::run／Prepared 仍失敗即取消。憑證、格式、服務拒絕等錯誤與整個 Session 的既有有限復原判定不變，不換服務或重送同一份失敗文字。

以完整 Session／generation、utterance、source revision、sequence、pass、job id 核對目前工作才退休。取消 token、釋放 permit、清除 job id／pending；晚到輸出不能再次採用。新來源修訂或既有字幕已取代的舊工作不算新失敗，也不退休新工作。Incomplete 使用獨立退休入口，不增加 TranslationExpired／FinalSegmentsExpired，避免冒充年齡過期。

沿用同一份有界數值 notice，累計次數與連續次數共同涵蓋 Timeout／Stale／Incomplete；連續第三次仍停止，沒有新重試預算。只有真正 complete 且通過目前來源／job／前綴核對的輸出才重設連續次數，partial 不重設。狀態區分「翻譯未完成」與「翻譯延遲」，顯示未完成定稿、後續處理／已恢復或連續失敗上限；不顯示服務原始 payload、本文或金鑰。

沿用 ADR 0096：既有已顯示的 Tentative 可保留，不能冒充 Final；已完成的歷史譯文保留，未完成內容不得作後續穩定翻譯脈絡。不清除使用者顯示歷史，不建立假定稿。字幕快照與警告獨立，維持單一 TranscriptTimeline；Prepared 原子性、三秒 Live 年齡／傳輸期限、佇列、並行、token 上限、擷取與來源、模型、金鑰、對齊及 capture-only 時鐘不變。

## 驗證與限制

新增五項定向案例：部分輸出後未完成、保留先前定稿與後續有效段落；被新修訂取代的工作與重複退休／非過期計數；真正 localhost HTTP／SSE 未完成後下一要求成功及沒有第三次隱藏重送；內容過濾維持 Refused；UI 警告與快照獨立、恢復及連續失敗上限。既有連續失敗測試擴及 Incomplete，服務拒絕仍立即失敗。

第一輪新測試編譯有 UiStatus 無 Debug 的斷言問題，第二輪有測試 constructor 多傳參數；均修正測試呼叫，不為此擴大產品型別。下一輪為四項成功、一項失敗：測試試圖將 Final 來源再修訂，正確被既有守門拒收；已改為真正 Partial→Final 情境，不放寬來源規則。最終五項新增、145 項 Phase 11、翻譯相關 59 項一般測試通過，最後獨立完整回歸 2166 一般成功、零失敗、18 ignored；格式／零警告 Clippy／Debug／Release 通過。備份與核對來源／執行檔雜湊後更新並開啟 PID 4420 待命，未自動開始；詳見 [診斷紀錄](../phase-11-gemini-diagnostics.md)。集合重疊不累加，不以 localhost 當作真實雲端驗收，實際持續翻譯與原文完整度仍待人工，Phase 11 未結案。
