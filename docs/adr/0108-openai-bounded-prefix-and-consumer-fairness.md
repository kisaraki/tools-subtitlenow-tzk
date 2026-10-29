# ADR 0108：OpenAI 有界字首音訊與轉錄消費排程

日期：2026-10-27。狀態：採用於 Phase 11 修正，真實長句完整度待人工驗收。

## 問題與證據

使用者回報 Gemini 小聲音源「好很多」，OpenAI 長句仍掉字、翻譯不完整，曾因佇列滿停止。重新測試後英日文基本正常、佇列正常，但原文本身仍有缺漏。沒有具名佇列或數值，不能把本機重現原因直接認定為該次唯一根因。

兩項測試先失敗：活動旗標啟動前的弱音訊未送出，預期 1920 bytes 實收 960；同一 inline owner 的 localhost WebSocket 一次送出 80 筆 delta 時，TranscriptOutput 的六十四項佇列先滿，消費端尚無機會執行。後者為排程飢餓，不是容量不足的證據。

## 決定

OpenAI 只保留目前已由使用者啟動之 Live Session、尚未傳送的 inactive PCM 尾端，最多八十毫秒且不超過既有十二包上限。超量淘汰最舊包；原始 received 時戳不更新，超過 MAX_AUDIO_AGE 減 SEND_TIMEOUT（目前 150 ms）的包移除，定時輪詢亦清理。啟動語音時，與當前區塊合併為單一 append，避免額外網路往返；已送語音不重播。這是活動門檻前的有限字首保留，不是開啟翻譯前的背景擷取。

字首與當前包的格式、Session 必須一致。提交上下文採最舊實際音訊的 sequence／start，duration 包含實際傳送 frames；最舊 start 未知時保持 None，不借用後包已知位置。取消／停止釋放 Session RAM，不保存 PCM、字幕或翻譯。原四秒區段、四百毫秒靜音、增益、音訊年齡限制、金鑰、模型及真正提交／Final 對應不改。ExactCueVerifier 與 capture-only LivePlaybackClock 不改。

OpenAI 每處理一則服務訊息後採 cooperative yield，讓 inline owner 有機會取出轉錄。維持六十四項事件佇列、不刪 Final、不阻塞擷取；真正慢消費或其他超量仍回報具名錯誤。Gemini 不採此額外讓步或字首策略。

診斷新增「前綴音訊」實際合併成功包數。「送出音訊訊息」是 append／音訊訊息數，不必等於輸入包數；合併本身不應解讀成掉包。不顯示本文、原始 JSON 或機密。

## 驗證與限制

新增七項測試：弱 PCM 只送一次與真實上下文、跨句字首、時間／項目上限、逾期清理及未知時間、錯 Session／格式與 Gemini 不變、真正 localhost PCM span／數值，以及八十筆服務 delta 的 inline 消費。17 項 OpenAI 定向、136 項 Phase 11 與 64 項串流一般測試通過；最終獨立完整回歸 2157 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過。已備份、雜湊核對並更新標準版，PID 27672 待命且回應；真實長句待人工。集合重疊，不累加；數值與部署紀錄見 [診斷紀錄](../phase-11-gemini-diagnostics.md)。

沒有推測或補造原文字詞，亦不宣稱消除模型辨識錯誤、固定收尾邊界影響或所有佇列中止。真實英日文長句完整度仍須固定介面人工測試；不以合成資料冒充雲端驗收，Phase 11 未結案。
