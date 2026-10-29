# ADR 0025 — 同一時間軸保存來源顯示與 Live 回復狀態

日期：2026-10-10。狀態：Phase 7 第四批採用。

直接把既有字幕當成 ASR／TranslationUpdate 會混淆 utterance、修訂與已提交前綴；另外建立字幕時間軸又會造成重複排程及不同視圖的來源不一致。因此既有來源由 LiveTranslation 所持仲裁器發出內部決策，綁定原有合法定時的 Final ASR 項目，在同一 TranscriptTimeline 保存 ExistingDisplay 及既有 Live 狀態。

binding 須符合 Session／generation、utterance、sequence、revision，完整 ASR 捕捉範圍須由所選 cue 定位涵蓋。相同 cue／capture 範圍不可重複顯示於多個項目。採用不建立新 ASR 事件或猜測時間，保留內容 ID、原文件索引、媒體起點及所選文字來源。

ExistingDisplay 使用仲裁器才能建立的 lease，沿用原始 freshness 與候選獨立取消 token。顯示 accessor 及快照重查 lease；owner 在既有排程輪詢回復失效來源，沒有新 worker 或 timer。兩種 renderer 與 Dual 使用同一份快照，並明確提示即時辨識仍使用雲端；來源顯示不更改 Live 翻譯前文、修訂或穩定前綴。

被取代的翻譯要求須取消，其他 utterance 可繼續處理以填補缺口。回復沿用原剩餘三秒期限及原 Draft／Review 狀態，不重送逾時文字。每次排程新增 job ID 以區隔同來源修訂的舊回覆，更新與時間軸排序共同使用此 ID；否則恢復時會誤收舊回覆，或因新 provider revision 從一開始而拒絕新結果。

新增內容以 64 個時間軸項目與每份有界字串限制，只在 Session RAM，沒有第二條管線或永久快取。這個採用介面不代表定位演算法已完成；正式查詢、文字準備、定位證據與 GUI 候選組合仍待完成，測試不把合成定位當成真實媒體相容性。
