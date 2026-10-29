# ADR 0017 — 明確設定、Session-only 的歌詞查詢

日期：2026-10-09。狀態：Phase 6 第四批採用；正式 GUI／resolver 組合尚未啟用此服務。

ExistingTextProvider 已有統一歌詞／字幕契約，但尚無真正的 HTTP adapter。本批實作 LRCLIB 的唯讀歌詞查詢，接收已確認歌曲的名稱與演出者；音訊、指紋、ASR 對白、來源程式名稱與憑證都不進入查詢。這是明確建立 LrcLibService 後才可呼叫的介面，不能把 STT 的 OpenAI／Gemini 選擇當成 LRCLIB 授權；現行 app／GUI 不建立它，也沒有跨服務 fallback。

固定 HTTPS endpoint、驗證 TLS、禁止 redirects，避免 metadata 被轉送到另一個 host。已確認歌曲由 LyricsTrack.from_resolution 檢查 Music／Confirmed／歌曲信心與合法 ID／名稱／演出者。已知專輯及整首時長可以提供，未知則省略；不能以 capture window 時長代用。回覆再比對歌名／演出者及已提供的專輯／時長，只容許大小寫與空白差異，不剝除 Live／版本名稱等可能影響身份的文字。

同一 LrcLibService 的各歌曲 binding 共用一個 request permit 與冷卻狀態。並行或冷卻期間的查詢直接回報 RateLimited，不累積等待佇列；一般最小間隔 500 ms，每次最多三秒。429／503 的 Retry-After 支援秒數與 HTTP 日期，不縮短較長的服務限制；缺少欄位採六十秒保守冷卻，無法解析／溢位則停止此 Service 的後續查詢至 owner 回收。沒有自動 retry、沒有背景工作或自動換供應商。

response 限制 128 KiB，檢查 Content-Type／Content-Length，串流接收時也檢查上限。LRC 解析限 1024 個標記／cues、展開文字 128 KiB、每 cue 8192 bytes，以限制 chorus 重複展開造成的記憶體與 CPU 工作。這是比通用 ExistingText 更嚴格的 adapter 子集，超限回報錯誤；沒有宣稱 decode 的效能 SLA。

只支援行級 LRC：分鐘／秒與一至三位小數、多個 chorus 時間標記、常見 metadata tag、零 offset。非零 offset／enhanced word timing／非法標記不會被悄悄剝除或改用 plain lyrics。空白時間標記保留為句尾邊界；相同起點可有多個 cue，依下一個不同標記或回覆的整首時長推導 duration。這些仍是歌曲相對時間，不能當成目前 playback offset。只有未提供 syncedLyrics 時，才回傳明確未定時的 plain cues；404／無歌詞／純演奏回報無候選，矛盾或格式損壞回報 typed error。

LRCLIB 資料沒有語言欄位，因此 caller 必須提供已知 TextLanguage，adapter 不猜測語言，也不自行宣稱文字已本地化。回覆原始來源文字及時間，Phase 7 才負責本地化、定位、來源仲裁及單一 timeline 呈現；本批不操作 renderer、匯出、寫檔或保存 lookup 結果。每次結果包含原查詢 Session／generation／content ID，取用前重新檢查取消與 freshness，連 404／無文字的遲到結果也會被拒收。

契約依據：[LRCLIB 官方 API](https://lrclib.net/docs)；HTTP 日期解析：[httpdate 1.0.3](https://docs.rs/httpdate/1.0.3/httpdate/fn.parse_http_date.html)。測試使用合成歌名／歌詞及 localhost response，取消／Drop／逾時的 server EOF 證明本機傳輸回收；不能推論正式服務涵蓋率、metadata 精準度、歌詞授權或端到端同步已驗收。
