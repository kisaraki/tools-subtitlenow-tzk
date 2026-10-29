# Phase 3 真實辨識驗收

基準：本機 Windows 10 19045。此文件是尚待執行的服務驗收，不代表以下項目已通過。本機協定 fixtures、Windows 憑證測試與人工無效金鑰測試的結果見 [Phase 3](phase-3.md)。

## 設定與啟動

在互動式 PowerShell 執行；金鑰只在隱藏輸入提示中輸入，不放在參數、設定檔或聊天訊息。

依使用者指定，以下有效金鑰測試留至結案才執行。結案時使用 --interactive-key 逐服務手動輸入，只供當次使用，不儲存到認證管理員。下方 --set-stt-key／--check-stt-key 是一般設定的可選途徑，不是結案測試前置條件。

```powershell
cargo run --locked -- --set-stt-key openai
cargo run --locked -- --set-stt-key gemini
cargo run --locked -- --check-stt-key openai
cargo run --locked -- --check-stt-key gemini
cargo run --locked -- --list-sources
```

選取專門播放測試語音的程序 PID，確認同程序的其他內容也包含在擷取範圍。瀏覽器沒有分頁隔離；不要用無關私人內容作為測試來源。將下方 1234 換成來源 PID，每個服務分別測試：

```powershell
cargo run --locked -- --stt-pid 1234 --provider openai --language en --interactive-key
cargo run --locked -- --stt-pid 1234 --provider gemini --language en --interactive-key
```

停止使用 Ctrl+C。更換來源與服務前先停止，再指定新的 PID／provider。文字只在當次終端機與 RAM 顯示，不重導向內容、不儲存錄音。可以人工記錄錯誤代碼與效能數字，不記錄私人逐字稿。

## 待驗矩陣

每個供應商都要測英／日／法／德／韓（en／ja／fr／de／ko），使用可核對且有權使用的語音內容。確認實際文字及語言，不以協定正確代替辨識準確度。

| 項目 | 驗收觀察 | 現況 |
| --- | --- | --- |
| 五語固定選取 | Partial 更新、Final 校正、來源 sequence／時間與段落相符 | 待有效憑證 |
| Auto 五語 | 各語與跨語內容只接受五語回報；不回報或其他語言明確拒絕 | 待有效憑證 |
| Gemini 連續多段 | 自動活動偵測，Final 延後仍持續送新鮮音訊；回覆段落修訂正確，未知擷取時間不偽造關聯 | 本機回歸通過；真實連續辨識仍待人工驗收 |
| 聲音條件 | 安靜、低音量、背景音樂、400 ms 暫停、超過四秒連續語音 | 待有效憑證與調校 |
| 停止／來源退出 | 停止擷取、關閉 socket、join worker；不顯示晚到結果 | 待真實管線 |
| 切換來源 | 新 Session ID；舊內容不進入新 ledger | 待真實管線 |
| 網路中斷／速率限制 | 只重連同一服務；清空舊音訊、有界退避、三次上限 | 待真實管線 |
| 無效金鑰 | 安全錯誤、不洩漏金鑰／供應商正文、無跨服務 fallback | 人工無效金鑰的 WSS 拒絕已驗 |
| 九分鐘重建 | 未完成段落明確捨棄、socket／helper 不殘留、重新建立 Session | 待長時間真實管線 |
| 反壓與效能 | Debug 佇列深度有界、來源年齡不累積；測第一個 Partial／Final 延遲 | 協定慢寫與飽和已有離線測試；實測待憑證 |

紀錄每項測試的供應商、模型、來源類型、語言、測試時間、成功／失敗及不含內容的數值。第一個有用字幕與端到端延遲是工程目標，尚無真實數字；不能從合成 PCM 的協定回覆宣稱達成。
