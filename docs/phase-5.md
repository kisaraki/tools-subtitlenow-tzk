# Phase 5 — UI

2026-10-09 使用者另要求固定的 OpenAI（ChatGPT）／Gemini 金鑰介面，已改為兩列獨立遮蔽輸入與明確安全儲存按鈕。此修正取代下方歷史紀錄中的單一共用欄位／切換即清空行為；現行行為及驗證見 [固定金鑰介面](api-key-ui.md)，正式金鑰與實機驗收安排維持不變。

啟動：2026-10-09，依使用者「繼續製作」接續。有效金鑰與真實服務驗收仍留至結案，以本機互動方式輸入。產品規格依 SPEC 第 51–59、79 節。

## 首批實作

--ui 在主執行緒建立待命 Windows 視窗，不啟動擷取、探索、雲端或金鑰輸入。預設 SideChat，讀取現有 display_mode；選單可切換側邊紀錄、底部字幕及雙視窗。選單切換只變更顯示模型，不啟動任何後端。BottomSubtitle 模式保留控制視窗以便退出穿透／鎖定與切回其他模式。

ui/presentation 從一份 TranscriptTimeline 建立至多 64 項的 RAM 快照。原文、臺灣繁中、來源時間戳與 Tentative／Stable／Final 分別呈現；同一 utterance 的修訂仍是同一項，沒有逐 token 追加泡泡。顯示模式與原文切換共用這一份快照。來源 Session 必須由 owner 明確選取，快照不能自行啟動 Session；舊 Session／generation／版本均拒絕。切換來源或停止清除顯示內容與搜尋。

SideChat 的搜尋不影響底部即時字幕；原文及翻譯均可搜尋，大小寫不敏感。核心搜尋上限 256 UTF-8 bytes，視窗輸入上限 64 UTF-16 單位。支援自動跟隨、手動回看、回到即時字幕；手動捲動關閉自動跟隨，重畫時保留第一個可見行。字幕資料沒有 Serialize、檔案儲存或匯出路徑。

底部視窗置頂、初始定位在控制視窗最近螢幕的工作區域，可拖曳、鎖定位置、啟用滑鼠穿透。原文＋中文可切換，預設中文。採黑底白字與 Microsoft JhengHei UI；字型／字級選擇、描邊、陰影、可調背景透明度、最大行數、指定螢幕與跟隨來源視窗仍待完成，不能以基本文字繪製當成完整規格已完成。

UI thread 於建立視窗前使用 Per-monitor V2 DPI context，處理 WM_DPICHANGED 建議範圍及字型重建。整數縮放的單元測試涵蓋 100／125／150／175／200%；尚未完成各縮放比例與多螢幕的真實畫面矩陣。

Win32 unsafe 集中於 win32/subtitle_window。Box 位址固定，GWLP_USERDATA 只在 callback 期間借用；無 RefCell 借用跨越可重新進入的 Win32 呼叫。UI thread marker 防止移至其他執行緒；Drop 同步銷毀視窗／子控制項／附屬選單，再釋放字型。未附加的選單有 RAII，繪製時恢復 DC 原字型後才 DeleteObject。callback 攔截 unwind 並停止視窗，API 失敗回報固定 SubtitleWindow 錯誤，不輸出私人內容。

Win32 建立與資料指標規則依 [CreateWindowExW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexw)、[SetWindowLongPtrW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowlongptrw)；DPI 策略依 [Microsoft DPI 指南](https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-improvements-for-desktop-applications)。新增既有 windows 0.62 的 Controls／HiDpi／LibraryLoader feature，沒有安裝新的 OS 元件。

## 首批限制與後續

本批尚未把 LiveTranslation 接到視窗，也沒有視窗來源選擇／啟動／停止／供應商／品質控制。--translate-pid 繼續使用既有互動式終端機；--ui 僅待命。正式啟動時必須顯示所選來源與雲端狀態，透過有界快照通知將同一條後端管線供給兩個 renderer，切換模式不能重連。這是下一批 Phase 5 作業。

以 Computer Use 檢查待命視窗時，Windows 回傳了正確標題及 target window，但擷取先後回報 FrameArrived timed out 與 window capture timed out；依技能的單次重試規則停止擷取，未改用自建截圖或注入工具。已回收此次自行啟動且未擷取內容的測試程序。因此本批不能宣稱視覺排版與實際滑鼠穿透操作已驗收。

首批新增五項測試：共用快照與模式／搜尋隔離、舊 Session 與版本拒絕及停止清除、搜尋容量／控制字元、五種 DPI 縮放，以及原生視窗建立／模式切換／置頂／鎖定與穿透 hit-test／銷毀。沒有真實雲端金鑰、實際音訊或人工字幕作為正式辨識輸出。

最終驗證：cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked、cargo build --locked --release 及 Release --help 均通過。一般測試 148 項（123 library、12 設定、4 探索、9 啟動）；十三項既有硬體／手動／外部服務測試仍略過。

保留失敗紀錄：與首次 Release 依賴編譯同時進行的完整測試，曾有四項既有時間敏感測試失敗：synthetic_pcm_stream_reaches_partial_and_final_for_both_providers_and_five_languages、five_language_stt_events_flow_through_http_into_one_timeline、streaming_bursts_are_coalesced_instead_of_overflowing_the_update_queue、stop_drop_and_restart_isolate_sessions_and_join_workers。表現為預期事件未收到或本機 timeout。Release 編譯完成後完整重跑 148 項全部通過；沒有放寬正式佇列／逾期界線、忽略失敗測試或改變測試執行緒數。這支持負載／排程敏感的可能性，不能據此宣稱已排除所有偶發問題；後續 Performance 階段仍需壓力測試。

## 第二批：有界通知與原生整合測試

2026-10-09 依「繼續測試／繼續作業」接續。確認沒有遺留 subtitlenow、cargo 或 rustc 程序。本批未讀取實際金鑰、未連線真實辨識／翻譯服務、未啟動音訊擷取。

ui/bridge 使用一格 watch 通知，只保留最新 UiFrame。Session／generation、來源 PID／所選供應商、固定狀態與 timeline 快照一起發布，讓略過中間通知時也不會遺失來源切換。只有 timeline owner 持有 UiPublisher；publish 拒絕其他 Session 的 timeline。一般更新在複製文字前以 33 ms 節流；完成更新可明確 force，保留最終結果。這是名義更新週期，沒有把它當成已量到的 UI 延遲。

快照使用 Arc，兩個 renderer 與通知共享同一份內容，不為每個視窗複製 64 項歷史。消費端取得 Arc 後立即釋放 watch 借用，再更新原生控制項；不得在繪製時持有通知鎖。UiInbox 丟棄會喚醒取消；後端 owner 仍須觀察取消、回收 requests／capture／worker 並 join，不以發出取消等同於完整資源回收。

重新連線、停止、失敗或 publisher Drop 都發布沒有文字的終止狀態；失敗不會被 Drop 覆寫成普通停止。UI 顯示所選 PID、供應商、處理狀態；停止後不繼續宣稱正在傳送。native run_with_inbox 可消費這條通知，透過 [SetTimer](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-settimer)／WM_TIMER 更新主執行緒的共享 Presentation。WM_DESTROY 取消通知及 [KillTimer](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-killtimer)，已排隊的舊 timer 不攜帶資料指標。

修正了搜尋無結果時錯誤顯示「尚未選擇來源」的問題：現在顯示「找不到符合的字幕」，保留正確的來源與雲端狀態。SideChat 搜尋仍不影響底部即時內容；重連／停止會清除顯示與搜尋。

新增七項測試：1,000 次通知只留下最新 frame 並釋放舊 frame、Final 越過節流、UI 關閉／取消喚醒、publisher Drop 清除內容及保留錯誤、模式切換不重啟後端、單一 LiveTranslation → mailbox → 共用顯示模型，以及背景執行緒 → mailbox → 真正 Win32 控制項。後兩項使用明確 MockTranslator／人工測試 timeline，沒有將人工文字當成正式辨識。原生測試建立隱藏視窗並處理相同 WM_TIMER 路徑，驗證控制項文字、EN_CHANGE、回到即時、重連、拒絕舊 Session、停止清除與關閉取消；不是畫面擷取或真實滑鼠操作驗收。

第二批結束時 --ui 仍為待命介面；run_with_inbox 已提供原生消費入口，但當時正式 app 尚未啟動 live owner、來源選擇／供應商／品質控制，也尚未接上 UI 關閉後的 backend join。這些是第三批的必要整合。首批列出的外觀設定、多螢幕與視覺驗收限制仍適用。金鑰互動驗收維持結案安排。

第二批最終驗證：cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked 與 cargo build --locked --release 均通過。完整測試先完成，再執行 Release 建置，任一命令失敗即停止。一般測試 155 項（130 library、12 設定、4 探索、9 啟動），十三項既有硬體／手動／外部服務測試略過；本批未出現首批的四項時間敏感失敗。這仍不等同於排除負載下的偶發問題。

## 第三批：正式即時視窗與後端所有權

新增 --translate-window-pid，支援與終端機翻譯相同的 provider、language、quality、interactive-key 選項。來源必須明確指定，供應商可由旗標或既有設定選擇；沒有供應商時停止，沒有 dummy／另一個服務的自動替代。--ui 仍只開待命視窗；新命令才明確啟動雲端與指定來源。

正式 app 透過 ui_owner 啟動一個固定的後端執行緒，Tokio runtime、STT、翻譯及擷取 orchestration 留在此執行緒，Win32 訊息迴圈留在 main。live.rs 的 run_inner 是兩種輸出共用的正式流程；GUI 使用同一個 SpeechSession、LiveTranslation 及 TranscriptTimeline，沒有另一套辨識或翻譯。一般修訂有界通知，已穩定／完成結果強制發布；被節流的暫定尾端由 33 ms dirty timer 補送，不在待命時重複複製整份歷史。視窗模式不把逐字稿寫至 stdout 或日誌。

啟動前保留所選程序 handle，避免等待金鑰或建立視窗期間悄悄換到重用 PID。先發布包含 PID／供應商／音訊及文字送出提示的準備狀態。後端等待主視窗 ACK，再讀取所選 Windows 認證及建立 TLS 連線。ACK 表示控制項已更新、視窗有 WS_VISIBLE 且沒有最小化；[IsWindowVisible](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-iswindowvisible) 不保證視窗未被遮住，也不代表使用者已閱讀或真實視覺驗收。UI 先關閉／初始化失敗時不開啟啟動限制，也不讀取正式憑證或開始連線。

--interactive-key 明確選用時，在 main 建立視窗與 worker 前先於互動式終端機隱藏輸入所選 OpenAI／Gemini 金鑰。這避免背景 rpassword 阻塞而使關閉視窗無法 join；提示不自動啟動，也不將金鑰放入參數、設定或認證管理員。一次 RAM 金鑰可供本次同服務重連，停止後回收。正式操作留至結案，見 [Phase 5 操作程序](phase-5-test-procedure.md)。

工作階段選單提供停止辨識與翻譯。停止立即清除顯示、顯示「正在停止」並拒絕未讀取的舊活動 frame；backend 取消 STT／翻譯、停止擷取、回收 runtime 後發布不含文字的終止 frame。終止通知可在取消後送出，避免實際失敗被取消旗標隱藏；錯誤只顯示固定狀態。UiWorker 以 RAII 持有 JoinHandle；關閉、初始化失敗及 owner Drop 均取消並 join，不留下 detached worker。選單停止後 worker 可先退出，JoinHandle 由仍開啟的視窗 owner 持有至關閉後 join。重新啟動／更換來源目前須關閉並重下命令。

新增十項測試：worker Drop／視窗失敗後的取消與 join、worker panic 的固定錯誤、來源提示 ACK 與新版本、提示完成前關閉、原生隱藏窗／停止後拒絕舊 frame、原生停止選單與 worker 的整合（provider 啟動計數為零）、取消後保留終止失敗、命令列選項及啟動失敗不讀取正式憑證／不擷取。Source／requests／worker 資源使用明確 mocks；未用人工文字宣稱正式辨識。

第三批最終驗證：格式、Clippy（all-targets／all-features，-D warnings）、完整 cargo test --locked、Debug 與 Release 建置均通過。一般測試 165 項（139 library、12 設定、4 探索、10 啟動），十三項既有硬體／手動／外部服務測試略過。本批沒有手動輸入提示、正式金鑰讀取、真實雲端或實際音訊擷取；沒有重跑畫面擷取工具。修正迭代中的 Clippy 格式警告後通過，沒有用 allow 隱藏警告。

第三批結束時仍待完成視窗內控制、可調外觀、多螢幕、DPI 實機及視覺驗收。正式路徑接線不等同於有效帳號、五語準確度或端到端延遲驗收通過。

## 第四批：視窗內選擇、啟動與重新啟動

--ui 現在建立互動控制面板及既有的唯讀 DiscoveryMonitor。提供來源、OpenAI／Gemini、Auto／英文／日文／法文／德文／韓文、快速／標準／高品質、遮蔽金鑰欄位及開始／停止按鈕。來源不自動選取；沒有設定服務時維持空選項。停止並完成 worker join 後才解鎖選項，可在同一視窗重新開始。命令列指定 PID 路徑仍支援原本旗標。

來源清單以音訊 Session ID、PID 及程序建立時間維持選取；列出可獨立驗證的 SingleProcess，排除系統音效、跨程序、自身、未知程序身分。未知或不完整來源有明確訊息，不改抓整體混音。新增 RAM-only process_created；在探索 worker 使用 [GetProcessTimes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes) 查詢並固定於 Session metadata，開始時以新開啟的程序 handle 比對，防止掃描後 PID 重用。此小型前置檢查沒有改 WASAPI、helper hello 協定或來源確認策略。

DiscoveryMonitor.poll 立即取得最新 Arc，含未讀取的初始化失敗，關閉後不重複使用舊快照。控制器只將變更的來源描述替換為新 Arc，UI timer 不複製全部來源字串。下拉清單展開時暫緩重建，保留已呈現項目的映射；開始仍需與控制器最新來源一致。來源消失／建立時間不符清除選取，不跳到其他程式。此來源代表程序及子程序全部音訊，不代表單一分頁或播放裝置。

原生控制項使用 [COMBOBOX](https://learn.microsoft.com/en-us/windows/win32/controls/combo-boxes) 與 [ES_PASSWORD](https://learn.microsoft.com/en-us/windows/win32/controls/edit-control-styles)。金鑰欄位從啟動取用後清空，服務切換也清空，復原紀錄另外清除；本程式的 UTF-16／String 副本用 Zeroizing，ApiKey 與傳輸庫仍沿用既有安全處理。欄位不序列化、不存檔、不寫認證管理員；Win32／TLS 內部副本不宣稱全面擦除。留空才依所選服務讀取既有認證。每次啟動是新 worker／UiInbox／Session；先前輸入不沿用。

UI 主執行緒只處理輸入、短程序 handle 驗證及產生工作，COM 探索與雲端仍在各自的既有 worker。運行時鎖定來源、供應商、語言、品質、金鑰與開始；停止立即取消且不在 Win32 callback 等待 join，timer 確認 worker 結束後才 join／解鎖。關閉仍有 RAII join 防線。模式切換不改控制器或重啟後端。

新增六項測試，涵蓋來源過濾／容量及控制字元、原生選取與設定傳遞／金鑰清除／運行鎖定／停止再啟動、相同 PID 身分改變及來源移除、Discovery 初始化失敗的 poll，以及操作按鈕／搜尋位置與金鑰復原紀錄。既有取消前驗證測試追加真實的當前程序建立時間查詢與不符拒絕。MockController 不連線、不擷取；不是雲端或實際播放相容性驗收。

Computer Use 的 accessibility 檢查成功，實際 --ui 顯示來源未選、供應商未選、Auto／Standard，開始及停止按鈕停用，未啟動雲端。沒有按實際來源的開始按鈕或輸入正式金鑰。畫面擷取先報 FrameArrived timed out，重新選取及啟用視窗時報 GetCursorPos failed／存取被拒 0x80070005，依技能停止重試；不宣稱視覺排版通過。已回收此次自行啟動的待命預覽程序。

第四批最終驗證：cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked 與 cargo build --locked --release 均通過；一般測試 171 項（145 library、12 設定、4 探索、10 啟動），新增六項，十三項既有硬體／手動／外部服務測試仍略過。沒有正式金鑰或真實雲端測試、實際音訊擷取。Accessibility 檢查與原生幾何測試不等同於完整視覺／DPI 驗收。

第四批結束時仍待可調字型／字級、描邊、陰影、背景透明度、最大行數、指定及跟隨螢幕、DPI 實機與真實服務驗收；來源、服務、語言、品質及停止後再次啟動已能在 --ui 操作。設定變更目前保留於視窗 RAM，不自動寫入偏好檔。

## 第五批：字幕外觀與指定螢幕

新增「字幕外觀」選單：微軟正黑體 UI／微軟正黑體／新細明體、16／20／24／28／36／48／64／72 DIP、描邊、陰影、0／25／50／70／100% 背景不透明度、1–6 行與還原。預設 28 DIP、描邊／陰影開啟、70% 黑色背景、三行。字級為邏輯像素高度；雙語顯示改為中文在前，有限行數優先保留翻譯，原文與中文合計計算。所選字型的 GDI 實際高度限制整行，完整內容仍由同一份 SideChat 歷史回看。字型缺少或字形不足仍受 Windows font mapping 影響，不能以「有像素」宣稱全部字形正確。

字幕視窗改用每像素 Alpha，始終保留 WS_EX_LAYERED；滑鼠穿透只切換 WS_EX_TRANSPARENT。背景透明度與文字分開合成，背景 0% 時白字仍清楚，半透明字緣使用 premultiplied BGRA。灰階 GDI 字形輸入兩次有界一維描邊膨脹及固定偏移陰影，再合成黑色背景；沒有讀取桌面、檔案映射或影像檔。原生更新使用 [UpdateLayeredWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow)，詳見 [ADR 0011](adr/0011-subtitle-alpha-surface.md)。

每個表面上限 4096×2048／4,194,304 像素，建立前驗證乘法；兩個 DIB 與覆蓋率暫存最多約 40 MiB 像素記憶體，更新後回收、不建立 renderer cache。GdiFlush 後才讀 DIB，所有 font／bitmap／DC 在同一 UI 執行緒 RAII 恢復及回收。邏輯字型／外觀不含 provider 或 timeline；外觀、螢幕及模式切換均維持原 Session 與單一後端。未量測最高外觀設定的延遲，不把固定容量當成效能通過。

「字幕螢幕」提供主螢幕、目前列舉的實際裝置及重新整理，上限 32 台。偏好以裝置名稱維持，重建清單不把舊索引套用至另一台。WM_DISPLAYCHANGE 重新整理；指定裝置移除時保留偏好，明確提示暫置主螢幕，不改音訊來源或供應商。位置支援負座標，表面與底部邊距限制在工作區域內；跨螢幕定位第二次使用字幕 HWND 的實際 DPI。小工作區域不足時只顯示可容納的完整行；完全透明背景的透明區域可能由 Windows 直接穿透，拖曳需點字形／效果。

新增十項測試：透明背景／不透明白字與半透明字緣、描邊及陰影 Alpha、拒絕錯誤尺寸／偏好、原生字型與表面建立、五種 DPI 的實際 GDI 行數裁切、負座標／小區域／五種 DPI 定位、本機原生螢幕清單、外觀及螢幕命令保持 Session／內容且穿透關閉仍保留 layered 樣式、明確的螢幕移除提示，以及背景／效果／字型家族變更保留手動拖曳位置。字級或行數改變才重新計算底部定位。測試使用合成字形／mock timeline／隱藏的自有視窗，不擷取其他程式畫面或音訊、不連正式服務。

第五批完整 cargo test --locked 通過 181 項（155 library、12 設定、4 探索、10 啟動），十三項既有硬體／手動／外部服務測試略過。cargo fmt --check、Clippy（locked／all-targets／all-features、-D warnings）、Debug 與 Release 建置通過；初次編譯的 unused import 已刪除，未隱藏警告。沒有正式金鑰讀取、真實雲端、實際音訊擷取或新的 Computer Use 預覽；原生像素／幾何測試不能替代實機多螢幕、視覺排版或端到端效能驗收。

第五批結束時仍待跟隨來源視窗、來源圖示／音訊活動的完整 UI 呈現、SideChat 左右側停靠及實機 DPI／視覺／真實服務驗收。外觀和螢幕選擇只留在視窗 RAM，不自動寫偏好檔；沒有啟動 Phase 6。

## 第六批：跟隨來源所在螢幕

接續使用者「繼續作業」，新增與主螢幕／指定螢幕互斥的 MonitorPreference::FollowSource。「字幕螢幕 → 跟隨來源所在螢幕」每 500 ms 查詢選定來源的可見、非最小化、非 tool 頂層視窗，優先同 PID 的前景視窗，其次最大面積。工作區域改變後才移動字幕；同區域的後續樣本不覆寫拖曳位置。UI 跟隨不啟動擷取或服務，也不重啟後端。

正式 GUI owner 將已持有 ProcessTarget 的建立時間帶入同一 UiFrame，與來源／Session／狀態／timeline 原子發布。同 PID 重連保留此身分；切換 PID 清除。Idle 可從已選 SourceChoice 查詢，未選供應商也能選跟隨。查詢先開 query／synchronize handle、比對建立時間與零等待存活，列舉前後驗證 HWND 所屬程序；程序控制代碼在本次查詢 RAII 關閉。MAX_WINDOWS 既有上限仍適用；沒有 hooks、遠端訊息、私人控制項、額外 thread／queue。詳見 [ADR 0012](adr/0012-source-window-monitor-follow.md)。

無來源、身分失效、無可見視窗、查詢失敗有固定提示，保留字幕位置／內容／Session；可恢復或改選主螢幕／指定螢幕。跟隨僅關聯所選 PID，不猜測 Chrome／Edge 音訊子程序的父視窗；沒有頂層視窗時請手選螢幕。跟隨的是來源所在螢幕工作區域，不宣稱貼住播放器下緣或支援 exclusive fullscreen。

新增六項測試：前景與面積排序／拒絕空範圍、真實程序建立時間不符時提前拒絕、以自有無啟用的原生測試視窗驗證實際 PID → HWND → 螢幕工作區域、建立時間隨重連保持／不同 PID 清除、跟隨失敗／恢復及相同區域保留拖曳位置並保持 Session，以及 500 ms 節流／未驗證活動來源。原生 fixture 是本機合成測試視窗，沒有音訊、服務或其他程式輸入操作；它驗證查詢路徑，不能替代應用程式／雙螢幕／混合 DPI 的視覺矩陣。

第六批 cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked 及 cargo build --locked --release 通過。完整一般測試 187 項（161 library、12 設定、4 探索、10 啟動），十三項既有硬體／手動／外部服務測試略過。沒有正式金鑰、真實雲端或實際音訊擷取；結案互動安排不變。

第六批結束時，來源圖示／音訊活動的完整 UI 呈現、SideChat 左右側停靠及實機 DPI／視覺／真實服務驗收仍待完成。外觀和螢幕偏好仍只在 RAM；不以離線測試宣稱實際翻譯品質或效能通過。

## 第七批：來源圖示／活動與 SideChat 左右停靠

--ui 的來源 COMBOBOX 改為固定高度 owner-draw，使用探索 worker 已取得的 Arc<ApplicationIcon> 與原有程式名稱／PID／標題。CBS_HASSTRINGS 保留原生 accessibility 文字與鍵盤選擇。圖示須在 32×32 上限內、像素長度正確且符合 premultiplied BGRA；缺失／無效不虛構替代圖示，活動區顯示「圖示無法取得」。原生繪製借用的 DC 以 SaveDC／RestoreDC 恢復字型、顏色、裁切，AlphaBlend 的私有 DIB 在同一執行緒 RAII 回收。

新增所選音訊工作階段的活動區與峰值量表，分別呈現串流執行中／閒置、有訊號／無訊號／未知、峰值、音量與靜音。NaN／Infinity／超出 0..1 的量測視為未知；有效峰值以千分比保留，音量以百分比顯示。未知峰值隱藏量表，不顯示假的零值。這是所選音訊 Session 的資料，不能推論涵蓋程序與子程序全部音訊、喇叭一定有聲或即時辨識是否正常。

選項描述與活動更新分開。身分仍按 Session ID／PID／程序建立時間匹配，same_option 只比較身分與標籤；峰值／圖示更新不重建下拉清單。清單展開及 backend busy 時保留原索引，繪製／活動從最新快照找相同身分，找不到時顯示來源已消失或資料不可用，不採用重用 PID 的新資料。控制器只在 busy／notice／選取變更時重寫 SideChat，活動更新不打斷回看。只選來源、尚未選服務時也顯示正確的已選 PID，開始仍停用。

SideChat 預設靠右，新增「側邊位置 → 靠左／靠右／自由移動」。靠左／右使用控制視窗所在螢幕的工作區域、460 DIP 寬與完整高度，小螢幕限制寬度；DPI／螢幕更新及拖移結束後依側別重新定位。自由移動保留手動位置及大小。沒有 AppBar、系統空間保留、其他應用程式 resize、hooks 或後端重啟；原 timeline／Session 不變。來源活動面板只在 --ui，固定 PID 診斷視窗不另開探索面板；位置選單兩條 GUI 路徑都適用。詳見 [ADR 0013](adr/0013-source-presentation-and-side-docking.md)。

新增七項測試：量測／串流狀態分離與非法讀值、圖示尺寸／Alpha／Arc 共用與選項身分、原生 AlphaBlend 像素／不變的外圍／無效圖示拒絕、五種 DPI 與負座標的小區域左右定位、來源活動更新保留索引／歷史及未知峰值隱藏量表／重用程序身分不採用、透過真正 WM_DRAWITEM 繪製圖示至自有測試 DIB 並保留可存取標籤，以及原生左右／自由停靠保持 Session 和面板幾何間距。測試全部使用明確 mock 來源／timeline 或自有 DIB；沒有將合成圖示、字幕或峰值當成正式擷取結果。

第七批 cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked、cargo build --locked 及 cargo build --locked --release 通過；一般測試 194 項（168 library、12 設定、4 探索、10 啟動），十三項既有硬體／手動／外部服務測試略過。迭代中修正 windows binding 的 flags 型別與 Clippy 的固定 chunks 警告，未放寬 lint 或略過失敗測試。沒有正式金鑰讀取、真實雲端、實際音訊擷取或新的 Computer Use 預覽。

Phase 5 的來源／服務控制、共用視窗、字幕外觀、螢幕選擇／跟隨、來源圖示／活動與左右停靠已接線，尚未完成實機 DPI／視覺、應用程式完整矩陣及真實服務／效能驗收，因此不標示 Phase 5 已結案。圖示仍是探索產生的 32×32，在高 DPI 放大可能較柔和；文字或窄螢幕的可讀性須人工檢查。視窗偏好仍只留 RAM，金鑰與雲端驗收維持結案手動輸入安排，未啟動 Phase 6。

## 第八批：搜尋標籤、提示裁切與原生資源檢查

使用 computer-use 技能檢查自行啟動的 Release --ui 待命視窗。Accessibility 成功：來源／服務未選、Auto／Standard、開始／停止停用，未知活動量表隱藏；未按開始或輸入金鑰。發現搜尋 edit 的原生名稱誤用活動說明「請先選擇來源；尚未取得音訊活動資料」。因此在搜尋 edit 前新增獨立、可見且不接受 Tab 的 STATIC「搜尋本次字幕」，給予穩定標籤，並保留空搜尋值／原有搜尋事件與有界內容。原生 sibling 順序及標籤／edit 間距測試通過；修正後的輔助工具名稱仍待實機確認。

字型量測另外重現服務提示裁切：原「請選擇雲端服務」寬 126 像素，預設選取區域寬 109 像素。縮短為「選擇服務」，保留字級、OpenAI／Gemini 選項與未選服務狀態；開始仍須明確選取。測試取得真正 COMBOBOXINFO／已套用字型／實際控制項文字，再以 GetTextExtentPoint32W 量測，修正後通過；不把此尺寸檢查當成其他 DPI／所有長文字的視覺通過。

新增隔離子程序 GUI 資源測試，避免其他平行測試改變計數。三次暖機後執行三批、每批 20 次隱藏視窗建立／圖示繪製／Dual／穿透切換／字型更換／螢幕選單重建／銷毀；query-only GetGuiResources 檢查本程序 GDI／USER 物件。三批均維持暖機後 GDI 3、USER 2，沒有累積。測試界線容許最多兩個差額，本次實測差額為零；不是完整 backend／捕捉或長時間播放的資源驗收。子程序具 60 秒上限與 kill／wait RAII，合成來源／圖示僅在 cfg(test)，不擷取、不讀憑證、不連服務。

新增三項一般測試：搜尋標籤與順序／幾何、預設服務提示尺寸、隔離 GUI 資源 parent。內部 isolated_ui_resource_fixture 標成 child-only ignored，已由正常 parent 在隔離程序實際執行並通過；不是未跑卻算通過。cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked 通過 197 項（171 library、12 設定、4 探索、10 啟動），cargo build --locked 與 cargo build --locked --release 也通過。主測試列 14 ignored，含十三項既有硬體／手動／外部服務項目及一個另由 parent 執行的 child fixture。新增提示尺寸測試最初失敗的 126／109 證據保留於此，沒有放寬尺寸要求或隱藏失敗。

畫面擷取仍報 FrameArrived timed out，重新選取視窗／嘗試啟用時報 GetCursorPos failed／存取被拒 0x80070005。依 computer-use guidance「Refresh the app/window selection and retry once」完成一次重試後停止；沒有另用其他工具繞過畫面擷取。已確認 PID 與路徑後回收本次自行啟動的待命預覽，不把硬停止當成正常關閉／join 的人工驗收。沒有正式金鑰或真實雲端／實際音訊。最新 [驗收表](phase-5-acceptance.md) 將已實作、已測與待實機分開記錄；Phase 5 仍未結案。

## 第九批：搜尋同步、回看定位與最小化

原生合成測試先重現六項失敗：停止搜尋未立即清空、待命搜尋在開始後殘留、None → None 終止狀態未清空搜尋、字幕更新把選取重設為 (0, 0)、淘汰最舊字幕使回看位置偏移，以及最小化主視窗後 refresh 再次顯示底部字幕。現在停止、開始、Session 切換與無 Session 的清除通知同步清空搜尋控制項與 undo，通知 guard 避免中途重入更新。開始先持有新 inbox，再執行控制項清除，錯誤路徑仍保留可取消的通知入口。

SideChat 新增當前控制項的 RAM 投影與 UTF-16 項目範圍。保留同一 Session／generation、sequence／utterance 的回看及選取；狀態文字變更、尾端新增及最舊項目淘汰後，按保留項目重新計算字元／顯示行位置，不將舊行號當作相同字幕。已淘汰／篩掉的回看錨點移至最早保留項目，失效選取取消；搜尋變更重設定位。同一文字不再反覆 SetWindowText；投影沿用 64 筆與每欄 8192-byte 的 timeline 界線，更新期間最多暫存舊／新兩份、替換後釋放舊份，停止／Session 切換立即清除，沒有跨啟動或磁碟內容快取。

另外修正真正的 auto-follow／回到即時捲動。原 EM_SETSEL(-1, -1) 只取消選取，並不移至末尾；改以實際 UTF-16 長度移動游標後，以 WM_VSCROLL／SB_BOTTOM 返回底部。只改游標再 EM_SCROLLCARET 的第一次修補，在隱藏測試視窗仍未捲動，保留該失敗並補上真正底部捲動，沒有移除返回底部斷言。API 語意見 [Microsoft EM_SETSEL](https://learn.microsoft.com/en-us/windows/win32/controls/em-setsel)、[EM_GETSEL](https://learn.microsoft.com/en-us/windows/win32/controls/em-getsel)、[EM_LINESCROLL](https://learn.microsoft.com/en-us/windows/win32/controls/em-linescroll) 及 [WM_VSCROLL](https://learn.microsoft.com/en-us/windows/win32/controls/wm-vscroll)。

refresh 只在可見狀態改變時操作視窗，使用不啟用的 ShowWindow；最小化主視窗時隱藏底部字幕，恢復由 WM_SIZE 同步所選模式，不呼叫 controller start／stop。最小化的零 client 尺寸不重新排版控制項，避免背景字幕將歷史壓成 1 px 而破壞回看。啟動時仍保留明確顯示主視窗，來源提示須主視窗可見且非最小化才放行。API 語意見 [Microsoft ShowWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindow)。

新增九項一般原生測試：上述六類案例、被淘汰錨點／新 Session、超過 65,535 字元的完整 DWORD 選取與狀態文字偏移，以及最小化來源提示的啟動門檻；回看測試含 emoji／UTF-16 偏移、未改文字的選取保持、返回底部與最小化期間持續更新後恢復。測試只操作自有 mock 視窗／合成文字；最小化案例短暫以不啟用方式顯示，不操控其他應用程式或擷取畫面，也沒有音訊／雲端／正式金鑰。active-window 查核不能證明所有真實前景／鍵盤情境通過。

第九批 cargo fmt --check、cargo clippy --locked --all-targets --all-features -- -D warnings、cargo test --locked 通過；一般測試 206 項（180 library、12 設定、4 探索、10 啟動），主測試 14 ignored 的組成仍為十三項既有硬體／手動／外部服務與一個由正常 parent 實際執行的資源 fixture。隔離 60 次 GUI 資源循環仍在完整測試中通過既有界線，本批未另讀取逐批原始計數。Debug／Release 建置通過。實機 DPI／視覺、真實鍵盤／滑鼠／工作列、應用程式完整矩陣與雲端品質／效能仍待驗收；金鑰留至結案互動輸入，Phase 5 尚未結案，未啟動 Phase 6。
