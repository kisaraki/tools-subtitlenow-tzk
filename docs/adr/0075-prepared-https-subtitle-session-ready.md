# ADR 0075：HTTPS 字幕接入 Prepared Session 與 RAM Ready

日期：2026-10-18。狀態：Phase 08 第四十二批採用。

## 決策與原因

`PreparedRequest::with_remote_subtitle` 明確加入 HTTPS SRT／WebVTT 候選，與本機外掛及內嵌軌共用八份總預算、語言順位與同順位歧義。規劃只讀 metadata，不連線、不讀檔或取金鑰。來源語言只根據宣告，不依 URL、檔名、MIME 或模型猜測。

新增明確 `PreparedTask::run_with_https_subtitles`，由呼叫者在揭露用途後使用。舊 task 入口與一般控制器仍拒收任何 HTTPS 字幕候選，包含較低順位候選；歧義先回報，拒收在 I/O、服務或金鑰取用前完成。這讓原有本機／音訊準備呼叫不會因增加候選而開始連線至新的字幕目的地。本批尚未開放一般視窗的字幕網址輸入。

## 來源選取與處理

新入口需要至少一份明確 HTTPS 候選，保留共同順位；來源字幕缺 translator、舊 Session glossary 或遠端媒體帶任何內嵌選取，都在對應 preflight 失敗。只有勝出的網址會被讀取；本機外掛或本機內嵌軌勝出時，先丟棄未選網址，再沿用既有選定文字讀取流程。錯誤或找不到選定字幕不改為 ASR。

勝出 HTTPS 字幕時，先持有實際本機媒體唯讀 pin，或驗證同一遠端 PCM16 WAVE 的標頭。接著使用上一批的單份 GET／有界全文解析入口，沿用實際 Session／generation／sequence；不建立音訊解碼、DSP 或辨識要求。遠端媒體仍只支援既有 WAVE 子集合，不新增內嵌網路軌或一般網站支援。

中文字幕共用本機臺灣用語本地化，不需要服務或金鑰。五語來源字幕共用既有全文摘要／詞彙分析、Fast／Standard／High、翻譯一致性校訂及最後本地化；只使用呼叫者交付的同一服務，不另建 client 或選供應商。URL、signed query 和媒體路徑不加入 AI 本文／脈絡或狀態。

## 持有者、容量與取消

遠端結果以 crate-private 移交拆出原文與 lease，移動文字而非複製整份文件。lease 保留連結的子取消 scope，並持有全程序兩份要求／本文配額中的一份許可；媒體與 lease 跨分析、翻譯、尚未 apply 的 completion 和 Ready 持續存活。未選候選不抓取，也不先釋放許可後將其本文留在 Session。

載入結構與 Ready 的欄位釋放順序讓原文／譯文先釋放、lease 後釋放；中途失敗也保留完整持有結構到清理，避免綁定拆開後因區域變數逆序銷毀而先歸還配額。網路與服務等待不持 CPU，解析與本地化共用原有 CPU lane。停止／Future Drop 退休本 Session，已開始 worker 保持許可至真正退出，晚回或舊 completion 不能發布新 Ready。

`MediaError::RemoteSubtitle` 保留獨立的平坦 transport failure，與 reader 公開錯誤分層，避免遞迴錯誤型別或把 404／429／redirect 蓋成取消。解析錯誤保留原有字幕原因；媒體、字幕及 Session 使用連結的子 scope，子持有者清理不向上取消父 scope 而蓋掉 typed cause。

## 驗證與限制

測試使用本機合成媒體、literal loopback 字幕／WAVE、明確 AI mock 及同步 barrier，核對共同順位／八份預算、中文／五語／品質、原文／時間／Existing 身分、臺灣用語、持有者配額、錯誤、取消、未 apply completion 與舊 Session 隔離。正式傳輸仍只接受 HTTPS 字幕，測試 HTTP 例外沒有設定入口。

Ready 表示本次完整文件準備，不授予媒體版本、內容識別、來源仲裁或播放定位證據，也不自動播放。沒有內容落盤、cache、temp、export、telemetry、第二份時間軸、正式金鑰或真實雲端驗收。一般視窗／控制器用途揭露與網址選取接線仍待後續，金鑰維持結案固定遮蔽介面手動輸入；Phase 09 尚未具備條件。實際驗證見 [Phase 8](../phase-8.md)。
