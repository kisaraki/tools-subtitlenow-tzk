# ADR 0028 — Live 候選採用使用有界排程與實際擷取位置

日期：2026-10-10。狀態：Phase 7 第七批採用。

Phase 7 已有文字查詢、準備、完整 cue 驗證及來源仲裁，但 Live 迴圈尚未排程它們。直接在 ASR 或音訊分流中 await 翻譯會阻塞既有管線；用 ASR 起點冒充目前播放位置，又會把已播放完的片段當成當前字幕。因此新增 caller-owned 的 LiveTextPipeline／LiveTextTask，與 STT、Live 翻譯及內容查詢在同一 select 迴圈平行輪詢。

只有明確配置歌詞服務且已選擇翻譯服務的執行才建立文字分支。它借用相同 Translator，沿用其連線、限流、品質與目的地，不建立新 client、不讀額外金鑰或跨供應商備援。一般 GUI／命令的內容設定仍為 None，沒有預設啟用網路字幕或歌詞服務。

輸入使用最新值 watch，最多一個執行中要求及一個替代輸入；結果 mpsc 容量一，進度 watch 只含 ticket 與狀態。最新定稿 ASR 取代舊工作，尚未讀取的舊結果在新選取時清除，完成後仍核對 ticket、出版物 parent、Session 與仲裁版本。排程器不另建 detached task 或每 block 新工作，定位 CPU 沿用先前有界 spawn_blocking。管理物件 clear／stop、來源撤銷與 future Drop 會取消要求，文字 worker 在停止／重連／結束時釋放並回收原 CPU lane。

同一有效 cue 的新確認不重啟執行中工作或延長原期限。成功準備後保留最多一個版本／ASR 綁定／來源／語言／內容 ID 的 metadata watermark；即使出版物撤銷後重選，也不重送同一已定稿片段，停止時釋放。這是 Session 內的重複抑制，不保存譯文、播放歷史或持久快取。正常相鄰來源 cue 保留穩定前文；明確撤銷附帶 reset_context，watch 即使跳過中間的 None 也不能漏掉脈絡重設，sequence watermark 則保留以防重放舊 query。

LivePlaybackClock 僅讀已收到 NormalizedAudio 的 frame_position、frames、sequence 與原 received 時間，保留一份實際游標，不保留樣本或以牆鐘推進位置。frame／sequence 缺口、discontinuity、未知／非法時間、overflow、倒退或過期會使舊定位失效；音訊缺口同時立即撤銷歌詞出版物，不等待下一次 resolver 回覆。觀測須保持原 250 ms 音訊新鮮度，且與仲裁 Session 一致。

候選準備完成時，只有實際游標仍在已驗證 cue 內，才提交到原仲裁與時間軸；未知或已播放完的位置維持 Live AI，不能用 ASR 起點替代游標來通過驗證。核心可能因嚴格完整命中／時間門檻及處理延遲而錯過片段，沒有保證實際媒體必然採用既有文字或滿足延遲目標。字幕 UI 快照仍為同一份 TranscriptTimeline，原生歌詞狀態另提示核對、所選服務準備、採用、無可靠定位與安全的 typed error。

本批串接現有可設定的 Live／GUI 路徑；正式服務配置、中文定位證據及真實媒體驗收仍待完成。Phase 9 的 fuzzy、seek、offset／drift 與持續校正不在本批。正式金鑰與雲端測試維持結案互動輸入，未宣稱實機通過。

後續擴充：Phase 7 第八批的 [ADR 0029](0029-paired-source-reference-evidence.md) 將可選配對版本及參考語言納入同一筆 metadata watermark，補上跨版本倒序拒收與本機中文處理狀態；原有排程容量、期限及實際擷取游標規則不變。
