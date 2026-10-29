# ADR 0107：OpenAI 提前轉錄的有界提交對應與安全診斷

日期：2026-10-26 開始，2026-10-27 完成工程驗證與部署。狀態：採用，Phase 11 OpenAI 錯誤修正；正常音量英文已有單次人工正例，其他矩陣仍待驗收。

## 問題與證據

使用者回報 Gemini 小聲音源僅稍微改善、仍不理想；切換 OpenAI／gpt-live-transcribe 後立即顯示 InvalidData，未提供當時語言或進度計數。兩項皆不列人工通過。

依 OpenAI Docs 的 [Realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription)，指定模型會於音訊提交前提供增量文字，最終回覆以 item_id 與提交對應；跨段完成順序不保證。既有實作只在 input_audio_buffer.committed 註冊對應，因此合法提早 delta 會被視為未知 item 而失敗。本機測試已重現；這不代表已證實本次實際錯誤僅有此原因。

## 決定

保留 gpt-live-transcribe、既有安全端點與初始化格式，沒有更換模型或移轉供應商。傳送 PCM 不等待轉錄；未知 item 的 delta／completed 只在確有活躍或待提交音訊時暫存，不用回覆先後猜測來源。

暫存最多沿用 EVENT_QUEUE_ITEMS（六十四項，與十二包音訊佇列不同）；同 item 的追加 delta 合併為一個暫定本文，另外保留服務真正 completed，兩份本文各受原 MAX_TRANSCRIPT_BYTES（8 KiB）約束，不截斷。只保留轉錄必要欄位及經核對的原語言欄位，略過其他頂層 payload；語言欄位另受原始 wire 訊息 64 KiB 上限約束。超量、格式／語言不符、final 後追加皆明確拒收。首份回覆起算八秒待對應期限，後續文字不得續期，逾時沿原有限復原。

只有真正提交確認才能用原 pending 音訊上下文註冊相同 item_id，並依暫定→真正 final 交付該 item。其他 item 不釋出；期間 event_queue 保持無未對應文字。重播合併內容不重複增加實際服務訊息序號。既有捕捉區段時間只在提交上下文內保留，不把文字抵達時間當成播放位置或服務詞級時間。ExactCueVerifier 與 LivePlaybackClock 不改。

安全診斷增加 OpenAI 串流接收／事件處理／等待提交對應階段，以及固定允許的服務錯誤代碼；未知 code、服務 message／param、金鑰與本文不顯示或寫入紀錄。新的包裝維持 root_cause 及原有限復原判定。Transcribe 明選語言也每秒顯示純數值進度，列「提交確認前暫存」實際訊息數；階段標籤改為通用「辨識階段」，不誤稱 Auto。Native Live Translate 的進度呈現維持原樣。

## 驗證與限制

十項新測試涵蓋有界合併、無關欄位不留存、錯誤輸入與語言、上限與不續期、反向抵達的兩個 item 按真正 ack 對應、final 不提前交付、無音訊的 unsolicited 拒收。真正 localhost WebSocket 在提交前回兩個 delta，驗證二十六包新鮮 PCM 持續送出、ack 後 partial／final 正確交付與取消；另一例在沒有音訊時回服務錯誤，驗證安全階段／代碼與零音訊傳送。

本方案為來源對應安全保留原四秒區段收尾：提早文字可等到真正提交確認才顯示，不宣稱首字延遲達一秒目標。所有緩衝為 Session RAM，停止／重啟釋放，不新增相依、音訊保存、內容快取、授權或雲端自動測試。真正 OpenAI 權限、初始化及字幕仍需固定遮蔽介面手動驗收；Gemini 小聲問題仍未結案。

最終 129 項 Phase 11 定向及 57 項串流模組一般測試通過，含十項新增；獨立完整回歸 2150 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過。已備份、雜湊核對並更新標準版與開啟待命，完整失敗／修正、數據及部署資料見 [診斷紀錄](../phase-11-gemini-diagnostics.md)。

2026-10-27 使用者回覆正常音量 Chrome 英文／OpenAI gpt-live-transcribe／英文／Standard 的原文與繁中持續更新、停止正常。未提供進度計數，不據此證明唯一根因、其他語言或延遲目標。Gemini 小聲不足仍維持未結案。
