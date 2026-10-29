# ADR 0020 — Live 內容辨識由應用程式組合並公開處理狀態

日期：2026-10-09。狀態：Phase 6 第七批採用；一般啟動尚未配置正式內容服務。

LiveResolver 已有有界音訊分流、取消與缺口隔離，但正式 app 的音訊 drain 仍只提交給 STT。若另外建立獨立 capture 或讓 resolver 操作 renderer，會破壞來源一致性、即時安全與單一時間軸。因此新增 app/content 的 LiveContent，在既有 Live owner 的同一 select loop poll 內容辨識；drain 先由 STT 接收同一 NormalizedAudio，再給明確配置的 resolver 固定大小 RAM 副本。

ContentConfiguration 包含明確的 ContentServiceFactory 與本機／遠端服務提示。factory 每次 capture Session／重連建立新的 identifier，不能在同步 create 中做網路或長 CPU 工作；實際 adapter 仍須自行限制資源並支援取消。應用程式不選擇預設目錄、不讀取額外金鑰、不跨供應商 fallback。run_controlled_with_content 是正式 GUI 的配置入口；一般 --ui／既有命令全部傳入 None，OpenAI／Gemini 的 STT 選項不會隱含啟用其他內容服務。

服務提示採編譯期固定名稱，最多 96 bytes、非空且無控制字元；不能含私密內容。所有實際傳送目的地都必須涵蓋於提示，遠端服務不得標示為本機。GUI source disclosure 與 content disclosure 共用可見 acknowledgement 門檻；增加服務提示會推進版本，舊提示的 acknowledgement 不能讓新服務提前啟動。只有畫面呈現後才讀取 STT 憑證／連線；內容 factory 更晚於新 capture identity 建立時才使用。這是程式的可見狀態門檻，不新增使用者確認步驟。

UiFrame 只新增有界 scalar 狀態：未設定、收集音訊、辨識中、分類／歌曲狀態、型別化失敗、已停止。沒有歌名、歌詞、字幕、指紋、內容 ID 或 provider response。狀態通知共用最新一格 mailbox，依 Session／generation 過濾、同值去重，保留原來的 Arc<TimelineSnapshot>；不重建 STT／翻譯，不改採字幕來源。歌曲確認與電影候選均明確說明字幕仍由即時翻譯提供，避免把識別候選誤呈現成已驗證字幕。

結果取用再次驗證 stamp、來源與單調順序；六秒沒有 fresh 視窗即清除分類狀態，沒有新音訊時也會醒來處理過期。音訊重設回到收集狀態，不以舊視窗的分類延用至下一段。已停止／未設定／初始化失敗的 owner 保持 pending，避免 select 的 ready busy loop。factory 失敗或 panic、provider future panic、CPU 工作失敗只停止內容分支並顯示固定安全訊息，STT／翻譯不因此重連或取消；一般 provider 錯誤仍由原有有界視窗與 adapter 限制處理。

停止、來源切換、同服務重連及 Drop 都 drop request future、取消 token、清除音訊並 join 原有 CPU lane。新 Session 不繼承分類或候選，UI terminal／reconnect 也清除分類狀態與字幕。Debug 只新增內容 audio queue／processed／dropped／reset／worker 數，不含服務 payload。沒有新增執行緒種類、無界佇列、unsafe、網路要求、檔案或內容永久儲存。

驗證使用明確 MockIdentifier／MockSpeech／MockTranslator、真正的既有 FIR／組窗／LiveResolver、UiWorker 與本程式隱藏原生視窗。這些測試可證明組合、提示門檻、取消及 snapshot 隔離；不能代表實際歌曲／電影辨識、正式服務／播放器矩陣或端到端效能已驗收。既有文字 owner、實際目錄、Movie hints、字幕 provider 仍待正式組合；本地化／採用／alignment／來源仲裁留在後續 Phase，沒有直接顯示原始網路文字。
