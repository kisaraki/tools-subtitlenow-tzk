# ADR 0019 — 既有文字查詢只保留目前選擇

日期：2026-10-09。狀態：Phase 6 第六批採用；正式 GUI／內容服務組合尚未啟用。

ExistingTextProvider 與 LRCLIB adapter 已能回傳有界歌詞，但缺少歌曲切換、取消、重複辨識與回覆持有的共同生命週期。本批加入 ExistingTextSession／ExistingTextTask；核心只產生文字候選狀態，沒有 renderer、來源仲裁、翻譯、本地化或播放定位。

Session 只持有目前選擇、最新待處理選擇與最新 publication，Task 同時只有一個 lookup。provider 必須由 caller 明確提供，不選擇預設服務、不跨服務 fallback、不 spawn／detach。每次 lookup 最多三秒，進入與結果採用均檢查 Session／generation／sequence／原查詢五秒 freshness；文字再驗證身分、來源、content ID、時間及大小。

select_confirmed_lyrics 接收 settled resolver evidence，只允許 Music／Confirmed／足夠信心且有歌曲 ID 的候選查詢。Candidate、Transition、Mixed、Unknown、Speech、失敗後的 Unknown 都清除舊選擇；不能把暫時的 Resolving 當作新證據。沒有設定 provider 或確認資料損壞時清除舊文字並回報 typed error，不建構 fake provider。

同一 content ID／source／provider Arc binding 的新視窗，只更新選擇 lease，不取消有用 lookup、不重抓歌詞。caller 應沿用同一 binding；更換 binding 表示明確改變 provider／metadata 設定，因此會重新查詢。Unavailable／Failed 不自行 retry；只有 caller 明確 refresh，或更換選擇才安排新 lookup。服務本身的 cooldown／並行限制仍有效，核心不繞過 LRCLIB 的限制。

選擇 lease 沿用 confirmed freshness 六秒，只由 fresh、單調的新選擇更新。poll 與 next 均能處理 expiry；沒有新證據時取消要求、清除文字並回報 Stale。renew 不修改原 query／歌詞時間，不能冒充新查詢結果、字幕 alignment 或播放起點。這只是目前內容選擇的持有時間，不是永久 cache。

clear、stop、Session Drop、Task future Drop 立即取消 request token。owner 清除 publication，版本門檻拒絕舊回覆；Task 發布時持有短暫 input gate，避免舊回覆在 clear 寫入空槽後又填回文字。舊文字 allocation 在 gate 之外回收。這個短鎖只協調 async／orchestration 選擇與結果，不接觸 capture callback，也不跨 await。epoch 溢位會停止並釋放資料，不能回繞版本。

Live owner 的文字上限更嚴格：1024 cues／128 KiB。先檢查總長度再做字元驗證，限制 async owner 的 CPU 工作；provider 自身也須在 allocation／decode 前有界。通用 ExistingText 契約仍允許更大回覆，但這個 Live owner 不代表所有長片／Prepared 字幕都能直接取用；後續若需要更大資料須另設有界 CPU lane。

只保留 Session RAM，沒有文字歷史、跨歌曲目錄、磁碟 cache、匯出、Debug／Serialize 或憑證讀取。caller 不能額外累積無界 Arc snapshots。Stage failure 不取消 Live STT，不直接修改共用 TranscriptTimeline。

測試包含明確 MockTextProvider 的取消／Drop、50 次選擇飽和、六秒 lease、舊工作階段／非法回覆、200 次跨執行緒 clear／publication 競爭，以及同一 owner 到真正 LRCLIB adapter／本機 HTTP／LRC 的路徑。HTTP 取消以 server EOF 與 permit 回收驗證；所有文字與 metadata 為合成內容，不代表真實 LRCLIB、歌曲識別或端到端同步已驗收。
