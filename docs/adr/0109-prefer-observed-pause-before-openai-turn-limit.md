# ADR 0109：OpenAI 接近區段上限時優先使用已觀察停頓

日期：2026-10-27。狀態：採用，工程驗證與部署完成，真實長句仍待人工驗收；Phase 11。

## 問題與證據

使用者測試 ADR 0108 字首／消費排程版後回報「非常顯著改善，但還是有掉字」，進一步確認集中在換段、接句或重新開口附近。這是改善的定性證據，不代表所有語言、實際字錯率或整體完整度通過。

現有能量端點在四百毫秒低能量後或連續活動四秒時切段。合成測試先重現：已收到 3.6 秒有聲音訊，再有 100 ms 低能量停頓，仍未要求收尾；接下來繼續有聲音時才在四秒上限切段。初始定向測試 1 failed（0.01 秒），只證明切段行為，不宣稱已重現模型遺漏某個真實字詞。

OpenAI 官方 [Realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription) 要求 gpt-live-transcribe 由客戶端偵測收尾並 commit。保留該模型與既有 API、品質 delay、明選語言及真正 item_id／Final 對應。

## 決定

增加獨立的 EndpointPolicy。只有 gpt-live-transcribe 選 PreferPauseNearLimit；Gemini 維持 FixedLimit，不把供應商判斷放入音訊處理演算法。

在既有四秒活動區段的最後四百毫秒（累計達 3.6 秒）內，若已觀察連續一百毫秒低能量，可在當前包之後要求 commit。這兩個窗口分別沿用既有正常靜音期限及最短活動區段；不增加等待、不回溯、不暫存等候未來音訊。早期的正常停頓仍需四百毫秒，短暫能量下探不立即切段；沒有合適停頓仍於四秒硬上限收尾。既有最多八十毫秒未送字首策略獨立保持。

worker 仍回傳每個輸入包的原有重採樣結果，不因 policy 重寫樣本、received、sequence 或 capture 位置。串流端沿用非活動包篩選及最多八十毫秒的新鮮未送字首；不代表所有靜音都會上傳，區段 span 只含實際送出資料。前處理仍使用既有單一有界 worker，取消與 drop 清理不變；無新佇列、執行緒、網路請求種類或持久內容。ExactCueVerifier、LivePlaybackClock、翻譯期限、來源選擇、金鑰與 Gemini 增益不改。

## 驗證與限制

四項新增測試涵蓋：接近上限時的短停頓、新句重新開始、純靜音／早期正常停頓／短能量下探／連續硬上限與重設；兩種 policy 經真正 worker 的全部樣本與原始時戳一致；localhost WebSocket 的兩次真正 commit、相同 item_id 定稿、後續音訊與實際 span。

四項新增、140 項 Phase 11 與 65 項串流一般測試通過，集合重疊。最後獨立完整回歸 2161 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過；標準版備份與雜湊核對後更新，PID 13500 待命且回應，未自動開始。詳細結果見 [診斷紀錄](../phase-11-gemini-diagnostics.md)。能量停頓不是語意或詞界保證，持續快速無停頓仍可能在硬上限切段；沒有補造或拼接模型遺漏文字，不宣稱消除所有辨識錯誤。新版真實長句仍需人工驗收。
