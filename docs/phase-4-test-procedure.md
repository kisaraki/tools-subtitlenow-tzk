# Phase 4 結案互動驗收

依使用者安排，只有結案時才要求有效 API 金鑰。先使用 --list-sources 選取專門的測試來源 PID；以本機互動式終端機執行下列命令，金鑰只在隱藏提示手動輸入，僅供當次使用，不儲存到認證管理員或設定。現在沒有執行此測試。

```powershell
cargo run --locked -- --translate-pid 1234 --provider openai --language en --quality standard --interactive-key
cargo run --locked -- --translate-pid 1234 --provider gemini --language ja --quality high --interactive-key
```

將 PID、provider、source language 換成實際測試案例。每個服務都需驗英、日、法、德、韓及 Auto，對照有權使用且可核對的內容；不使用無關私人播放、不寫音訊或逐字稿。逐服務手動輸入金鑰，不把金鑰貼入聊天。切換來源／語言前用 Ctrl+C 停止。

待驗：模型可用性、Tentative／Stable／Final、五語語意及自然臺灣用語、Fast／Standard／High 延遲與上下文效果、名稱／術語連續性、High 校訂回退、來源切換／停止／斷線／429、九分鐘重建及 RAM／handle 穩定性。記錄不含正文的時間數字與錯誤代碼。OpenCC 詞彙測試及 localhost 人工伺服器回覆不等於自然翻譯通過；不能將目前未測的第一個有用字幕延遲宣稱為 <1 秒。
