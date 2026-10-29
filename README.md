# Universal AI Subtitle for Windows

原生 UI 外觀更新：淺灰藍底、分色字幕／狀態區、圓角操作按鈕與文字留白，保留既有主頁配置、系統字型及字幕操作。實作範圍與驗證見 [UI 更新紀錄](docs/ui-refresh.md)；下方 Phase 11 結案版本保留為歷史基準。

2026-10-28 Phase 11 已依使用者確認的驗收範圍結案。主頁／狀態視窗與手選語言限制已通過；Chrome 以外播放器、剩餘真實雲端斷線／恢復及混音／串燒／歌詞依明確指示免測，保留未實測限制。交付 E3C6C5CD… 版與既有 2166 項工程證據不變，本次另有原生影片停止／錯誤／中文 sidecar 單案通過。詳見 [結案紀錄](docs/phase-11-closeout.md)。目前 SPEC 的最後階段為 Phase 11，未定義下一階段。

下列為歷史批次紀錄；其中待驗收／未結案文字不作目前狀態。

2026-10-27 新人工回報：使用者對 E3C6C5CD… 版測試選項 2，確認曾出現提示、後續仍繼續更新並恢復。記為本次提示後恢復的實測正例；提示種類、來源語言、確切播放時間、停止／保留與原文字詞完整度未另註明，不推論 Incomplete 特定原因、其他語言或長時間穩定性已通過。本次只更新驗收紀錄，未改程式或重跑前一輪 2166 項測試。Phase 11 未結案。

2026-10-27 單次未完整翻譯隔離版已部署：Live 的單次 Incomplete 以明確提示退休該要求，後續新段可繼續；共同連續第三次仍停止，Prepared 原子性及拒絕／憑證／格式守門保持。五項新增、145 項 Phase 11、翻譯相關 59 項一般測試通過，最後獨立完整回歸 2166 一般成功、零失敗、18 ignored，集合重疊；格式／零警告 Clippy／Debug／Release 通過。來源與執行檔雜湊核對後備份／更新，PID 4420（E3C6C5CD…）待命且回應，未自動開始。實際持續翻譯及原文完整度仍待人工，Phase 11 未結案。見 [ADR 0110](docs/adr/0110-isolate-incomplete-live-translation-requests.md)。以下保留歷史紀錄。

2026-10-27 接句缺漏修正版已部署：OpenAI 接近既有四秒上限時優先使用已觀察短停頓，保留音訊處理、硬上限及真正定稿對應。最後獨立完整回歸 2161 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過；四項新增、140 項 Phase 11 與 65 項串流一般測試為重疊集合。已備份舊版、核對來源與執行檔雜湊並開啟 PID 13500（4F59DEC1…），未自動開始。前一版已回報非常顯著改善，但換段／接句仍掉字；新版真實完整度待驗收，Phase 11 未結案。見 [ADR 0109](docs/adr/0109-prefer-observed-pause-before-openai-turn-limit.md)。以下保留歷史紀錄。

2026-10-27 OpenAI 原文缺漏修正版已部署：保留有限弱音字首並修正突發回覆的消費排程，不擴大佇列或推測字詞。最終獨立完整回歸 2157 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過；17 項 OpenAI、136 項 Phase 11 與 64 項串流一般測試為重疊集合，不累加。已備份舊版並核對更新雜湊，PID 27672（44E7A3CF…）回應且未自動開始。最新人工為 Gemini 小聲「好很多」、OpenAI 英日文基本正常但原文仍掉字；新版真實長句待驗收，Phase 11 未結案。見 [ADR 0108](docs/adr/0108-openai-bounded-prefix-and-consumer-fairness.md)；以下為歷史紀錄。

2026-10-27 新人工正例：使用者對 PID 29572 的正常音量 Chrome 英文／OpenAI／gpt-live-transcribe／英文／Standard 回覆選項 1，確認英文與繁中持續更新、停止正常。只通過這次流程；未提供診斷計數，不據此認定唯一根因、其他語言、逾時復原或效能目標皆通過。Gemini 小聲仍僅稍改善，下一項需其效果不佳當下的完整數值；保留現有程式，Phase 11 未結案。

2026-10-27 OpenAI 提前轉錄對應與安全診斷版已部署：前次人工回報 Gemini 小聲僅稍改善、OpenAI／gpt-live-transcribe 立即格式錯誤，均未全面通過。已重現提交確認前合法轉錄被拒，加入有界暫存與真正 item_id 提交對應，持續送新鮮音訊；另補安全階段／固定代碼及明選 Transcribe 數值進度。最終獨立完整回歸 2150 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過；129 定向與 57 串流測試含十項新增，集合重疊不累加。標準版已備份、更新與雜湊核對，PID 29572（92361A48…）待命且回應；真實 OpenAI 與 Gemini 小聲仍待人工，Phase 11 未結案。見 [ADR 0107](docs/adr/0107-openai-early-transcript-correlation.md)。

2026-10-26 使用者回報 Auto 收尾版首句約五至十秒、持續翻譯正常，但小聲錄製音源在 Auto 與手選語言皆有無字幕情形。新增 Gemini Live 前處理的有界低音量增益：先驗證、上限十六倍及峰值保護，再重採樣／能量端點；不改 Windows 音量或原始擷取，診斷另列套用包數。最終獨立串行完整回歸 2140 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過，119 定向含五項新增、不重複累加。標準版已備份、更新並開啟（PID 5616，5C253A5C…），未自動開始；真實小聲音源仍待人工，增益不等於降噪，Phase 11 未結案。見 [ADR 0106](docs/adr/0106-bounded-live-recognition-input-gain.md)。

2026-10-26 裝置切換測試收到「翻譯正常」正例，但 Auto 開始及長停頓後連英文首句都約三十秒才出現，另有翻譯逾期丟棄回報。已補上 Auto 的本機音訊端點收尾要求，不等首個轉錄、不阻塞後續 PCM、不延長翻譯期限。最終獨立串行完整回歸 2135 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過；114 定向包含六項新增，不重複累加。標準版已備份、更新並開啟（PID 31548，B92E8327…），未自動開始擷取；實際延遲及逾期改善仍待人工，Phase 11 未結案。見 [ADR 0105](docs/adr/0105-auto-client-audio-endpoints.md)。

2026-10-26 最新人工結果：使用者對本機語言備援版的 Chrome 英文來源／Gemini Transcribe Live／Auto／Standard 測試回覆「可用」，記為該組合的基本實測正例。未提供診斷計數，無法確認本次採用服務標記或本機估計；不據此擴大為五語、暫停／停止各步驟或其他服務全面通過。Phase 11 尚有人工驗收與既有媒體間歇逾時追查，未結案；以下保留各批歷史紀錄。

2026-10-26 最新修正改為本機文字語言備援：服務連真正定稿都可能未提供可核對標記，Auto 不再僅依賴該欄位。缺標記時使用 whatlang 全部內建模型後保守篩選五語，逐修訂重判，明示本機估計及不確定略過數，不另作雲端語言判定。最終獨立串行完整回歸 2129 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過；定向 108 成功包含新 11 項，不另累加。標準版已備份、更新並開啟（PID 31984，A8A43962…），未自動開始。實際 Auto 效果與 Phase 11 仍待人工驗收。見 [ADR 0104](docs/adr/0104-local-text-language-fallback.md) 與 [診斷](docs/phase-11-gemini-diagnostics.md)。

2026-10-26 最新 Auto 診斷已有明確數值：915 包音訊成功送出、17 次暫定、0 次定稿／交付。已加入待語言兩秒後、每段一次的服務定稿要求，仍保留五語核對、原八秒期限與持續音訊。新增本機 WebSocket 先重現舊版失敗（8.04 秒）；97 項 Phase 11 定向及最後獨立完整回歸 2118 一般測試成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過。標準版已備份、更新並開啟（PID 4968，05FF7050…），未自動開始；真實 Auto 定稿／語言及字幕效果待重試，Phase 11 未結案。見 [ADR 0103](docs/adr/0103-auto-language-finalization-request.md) 與 [診斷](docs/phase-11-gemini-diagnostics.md)。

2026-10-26 後續：使用者確認同一來源手選英文有字幕、Auto 沒有。已重現並修正未知語言 partial 無限續期的等待，加入獨立八秒語言期限及每秒純數值診斷（音訊收到／送出、服務訊息、partial／final／交付、語言待確認）。93 項 Phase 11 定向及最後獨立完整回歸 2114 一般測試成功、零失敗，18 ignored；格式／零警告 Clippy／Debug／Release 通過。標準版已備份、更新並開啟（PID 19960，7B53CB89…），未自動開始。實際 Auto 無字幕根因待新版計數確認，Phase 11 不結案。見 [ADR 0102](docs/adr/0102-bounded-auto-language-wait-and-numeric-progress.md) 與 [診斷](docs/phase-11-gemini-diagnostics.md)。

2026-10-26 人工回報修正：Gemini Transcribe Live 的 Auto 改用服務自動偵測並在本機維持五語核對，接受一般地區語言標籤、同一句省略標籤的定稿；未回報可核對語言時明示手選提示。主頁「狀態／錯誤（展開）」可開啟可獨立調整寬、高的狀態視窗，關閉不停止翻譯。最終獨立串行完整回歸 2109 一般成功、零失敗、18 ignored；格式、零警告 Clippy、Debug／Release 通過。第一輪漏改的 GPT＋Auto 測試設定已校正並保留失敗紀錄。標準版已備份、更新並開啟待命（PID 27164）；實際 Chrome 英文 Auto 與視窗外觀仍待人工重試，Phase 11 未結案。見 [ADR 0101](docs/adr/0101-gemini-auto-language-metadata-and-status-details.md) 與 [診斷紀錄](docs/phase-11-gemini-diagnostics.md)。

2026-10-26 語言能力前置提示版已部署：使用者確認舊主視窗已關閉後，核對舊版／備份與新候選雜湊，更新標準執行檔至 6677326A6720AF368F60F0444C08B6B95B481EC133B54D382896F68DFB92052F、9,629,696 bytes；新版 --ui 已開啟（PID 29304），程序存活且回應，只待命。沿用前批原生 59／Phase 11 定向 81 一般測試及格式／Clippy／Debug／Release 通過結果，沒有重新執行後再重算。Live Translate／GPT 需手選語言，Auto 請由使用者明確選 Gemini Transcribe Live；新介面與真實 Auto 仍待人工驗收，Phase 11 未結案。

2026-10-25 使用者確認 Live Translate 的暫停／恢復、捲動／回看與停止保留均正常；另回報 Auto 出錯。本批改善既有能力提示：目前程式只有 Gemini Transcribe Live 支援五語 Auto，Live Translate／GPT 的語言第一項改顯示「請手選語言」，未選語言時不能開始，並在主頁顯示明確原因。切換模型不暗選語言或搬移金鑰，直接開始／診斷亦先檢查再取用 key；不新增 Auto 實作或自動換模型。原生視窗 59、Phase 11 定向 81 一般成功（各 1 ignored、有重疊），格式／零警告 Clippy／Debug／Release 通過；候選版完成，待主視窗關閉後更新。前批 2099 項為歷史完整結果，本批不重複宣稱。見 [ADR 0100](docs/adr/0100-live-option-capability-preflight.md) 與 [診斷](docs/phase-11-gemini-diagnostics.md)。

2026-10-25 最新人工結果：使用者先回報 Live Translate 只有第一句，後續明確確認「中文已持續更新，問題已消失」。將目前版本的該次連續中文流程記為通過，不推論初始化等待原因、長時間穩定性或其他語言／服務。標準版仍為 0FBFA316268C7BC67FDBCD710E6E7CFE9E3AC4EB0946C714E7F234F68C85E53C，本次只更新驗收紀錄，沒有修改程式或重跑測試；既有 2099 項通過是前批工程結果。暫停／恢復、捲動／人工回看、停止與其他 Phase 11 條件仍待分項確認，Phase 11 未結案。

2026-10-25 接續人工回報修正：已在本機重現 Live Translate 後續暫定快照被重譯保護擋住，以及底部捲動結束誤停自動跟隨。修正原生完整快照可持續更新、實際回到底部可恢復跟隨，並在版面換行後捲至最新行；搜尋、選取及人工回看保持。連續 partial 不冒充 Final，模型／金鑰／擷取與有界容量不變。格式、零警告 Clippy、Debug／Release 及最後獨立串行完整回歸 2,099 一般成功、零失敗、18 ignored；標準版備份／雜湊核對後更新，新 PID 29612 存活且回應。第一輪原生媒體測試啟動逾時及 186 項 poisoned 連帶失敗、backtrace 與量測均保留：只校正測試原生啟動預算，停止五秒及產品期限不變，不宣稱解碼效能改善。新版正式 Live Translate 與外觀仍待人工驗收，Phase 11 未結案。見 [ADR 0099](docs/adr/0099-native-caption-snapshots-and-bottom-follow.md) 及 [診斷](docs/phase-11-gemini-diagnostics.md)。

2026-10-25 本批依使用者要求加入可儲存的 Live 模型選擇：OpenAI `gpt-live-transcribe`、既有 Gemini Transcribe Live、Gemini 3.5 Live Translate。來源／語系／品質移到字幕主頁，系統狀態與錯誤只在主頁；SideChat 顯示最後原文待譯段。Live Translate 明確為「中文串流／僅譯文」，不播放合成語音、不猜測原文配對或播放時間；請明選五語之一並使用 Fast／Standard。GPT 亦須明選語言，三品質實際設定辨識延遲策略；不代表已量測改善。舊服務預設與金鑰不變，設定 schema 1 僅在 RAM 遷移，明確儲存才寫 schema 2。新批自動測試及正式服務人工驗收進度見 [Phase 11 診斷](docs/phase-11-gemini-diagnostics.md) 與 [ADR 0098](docs/adr/0098-live-model-profiles-native-captions-and-main-source-controls.md)，Phase 11 尚未結案。

本批最終格式、零警告 Clippy、Debug／Release 通過；Release 後獨立串行完整回歸 2,096 項一般成功、零失敗、18 ignored。標準版已備份、核對雜湊後更新，新版 --ui 已開啟（PID 30696），只待命，未啟動擷取或正式服務。新模型雲端權限／翻譯品質與主頁外觀仍待使用者固定遮蔽介面驗收，未宣稱舊媒體間歇逾時已修復。

2026-10-25 最新人工回報已確認前版五分鐘以上持續翻譯、停止與保留正常，仍覺得速度慢。本批將服務／金鑰設定獨立、明確儲存所選服務與偏好，主頁常駐開始／停止／清除；設定依目前位置展開，返回還原。修正非人工原生捲動通知誤停自動跟隨，保留手動回看與回到即時字幕。原生視窗 53 項及設定檔往返 1 項通過（原生 1 ignored），格式、零警告 Clippy、Debug／Release 及最後獨立串行完整回歸 2083 一般成功、零失敗、18 ignored；標準版備份／雜湊核對後更新並開啟 PID 24580。沒有量測或宣稱雲端速度改善，Phase 11 仍待新版人工驗收及其他既有條件。見 [ADR 0097](docs/adr/0097-main-actions-service-settings-and-history-follow.md)。

2026-10-25 最新人工回報：手動清除、停止保留及重新開始上捲通過；數分鐘後仍遇到 Gemini 翻譯逾時。本批隔離 Live 單次期限失敗並於狀態框明示，維持三秒期限、不補送過時文字，連續第三次仍交回原有限復原。格式、零警告 Clippy、Debug／Release 及獨立串行完整回歸 2080 項一般測試通過（18 ignored），標準版已更新並開啟（PID 24216）；新版多分鐘人工驗收仍待完成，Phase 11 未結案。見 [ADR 0096](docs/adr/0096-live-translation-deadline-isolation.md)。

2026-10-25 依使用者要求，字幕紀錄不再因停止、錯誤、重連或重新開始而整批清空。固定「清除字幕」按鈕及紀錄選單由使用者操作；清除不停止處理，舊段不因快照補回，下一段正常顯示。最多 64 列 RAM 紀錄往上捲、滿載逐筆淘汰，跨連線內容標示先前字幕，關閉程式釋放；不送回翻譯或定位。UI 86 項、原生視窗 51 項、Live 16 項及 Phase 11 一般 53 項通過（有重疊）；fmt／零警告 Clippy／Debug／Release 通過，最後獨立串行完整回歸 2073 一般成功、零失敗、18 ignored。標準版備份／雜湊核對後更新並開啟 PID 28940，僅待命，沒有擷取或使用正式金鑰。新按鈕與跨連線畫面、其他人工矩陣及前批媒體間歇逾時追查仍待完成，Phase 11 未結案。見 [ADR 0095](docs/adr/0095-user-owned-bounded-subtitle-display-history.md) 與 [診斷紀錄](docs/phase-11-gemini-diagnostics.md)。

Windows 應用程式即時翻譯字幕工具，以 Rust 開發。產品規格以 [SPEC.md](SPEC.md) 為準，開發規則見 [AGENTS.md](AGENTS.md)。

目前階段：**Phase 11 — 本批定向與串行完整回歸通過，待人工驗收及原生媒體間歇逾時追查，尚未結案**。最新完整回歸以單一測試工作執行 2083 項一般測試成功、零失敗，18 項 ignored；fmt／零警告 Clippy／Debug／Release 通過。串行通過不代表第一輪逾時根因已修復。本批未重跑 ignored 或長測；前批自有靜音程序 1800 秒原生長測、DEBUG 300 次原生重啟、故障與隔離短測、本機真正 heartbeat 期限、五分鐘明確 mock 管線及 100 次 Session 切換結果仍分別保留。實體裝置、瀏覽器正反對照及其他服務／語言／真實媒體仍需人工操作；不以合成／mock 結果代替。見 [Phase 11](docs/phase-11.md)、[工程與人工清單](docs/phase-11-gate.md) 及 [手動操作](docs/phase-11-manual.md)。

前一階段 **Phase 10 — Performance 已完成工程量測與驗證**：2006 項一般測試成功、零失敗，15 項 ignored；fmt／零警告 Clippy／Debug／Release 通過。Session 數值診斷、三輪合成管線與原生開／關對照見 [完整報告](docs/phase-10.md)、[工程門檻](docs/phase-10-gate.md)、[ADR 0088](docs/adr/0088-bounded-capture-performance-diagnostics.md) 與 [ADR 0089](docs/adr/0089-session-numeric-pipeline-performance.md)。診斷不保存擷取音訊、字幕或翻譯，mock／靜音數據不代表產品 SLA 或全面產品驗收。

本批人工測試收到 Gemini 啟動即中斷、尚無字幕的回報，真實服務仍未驗收通過。已補安全連線階段／數字代碼，並修正被包在 I/O 內的 rustls 驗證錯誤誤判為一般網路中斷；沒有停用憑證驗證或換服務。追加驗證與獨立新版路徑見 [Gemini 診斷紀錄](docs/phase-11-gemini-diagnostics.md)，不將分類修正當成真實連線已修復。

後續回報已取得初始化階段 WebSocket 1011。本機指定 Gemini 憑證項目的存在已用 Windows 中繼資料確認，沒有顯示金鑰。新批次新增各服務「檢查本機」、儲存後同服務讀回核對及明確的 1011 服務內部錯誤訊息；本機提示與雲端結果分開。二十一項 Phase 11 一般測試、格式／零警告 Clippy／Debug／Release 與 2026 項完整一般回歸通過；標準程式已備份及更新，新版已開啟待命。真實雲端成功仍未確認。

Phase 09 已完成的工程基礎：**Alignment 工程驗證通過**。第十一批已完成 Prepared／Live 驗證整合；完整回歸 1976 項一般測試通過、零失敗，既有 15 項 ignored 不變，fmt／零警告 Clippy／Debug／Release 均通過。該批通過 Phase 10 工程門檻；後續已獲批准啟動效能階段。一般準備視窗可保留真正 RAM Ready 返回主視窗；中文字幕須明確提供同版本五語 SRT／WebVTT 參考軌及版本標識。使用者自行播放並啟動所選應用程式後，至少三筆獨立語音、完整文字歧義及時間模型通過才使用 Prepared 字幕，共用原仲裁器與時間軸。支援最近語音位置、有符號偏移、有限速率漂移及跳轉後重新校時；靜音不推進位置。主視窗「字幕定位」提供暫停／恢復／重設及清除 Ready，失效時繼續 Live AI。沿用單段／相鄰雙段與既有格式／資源限制，不控制播放器或自動播放。設計、操作與驗證見 [Phase 9](docs/phase-9.md)、[ADR 0087](docs/adr/0087-verified-prepared-live-alignment.md) 及 [Phase 10 工程門檻](docs/phase-9-gate.md)。真實媒體與有效金鑰維持結案固定遮蔽介面手動驗收，不代表全面產品驗收已結案。

Phase 8 已完成的準備功能：**Phase 8 — Prepared Mode（第四十三批 HTTPS 字幕準備介面）**。一般 --ui 的準備視窗可探索最多八條 text／tx3g 字幕軌；單軌填入 ID，多軌須從清單明確選一軌，再核對語言。中文／未定語言仍需明確指定，保留手動 ID、外掛 SRT／WebVTT。中文在本機轉為臺灣用語，五語來源字幕共用所選 OpenAI／Gemini 全文摘要／詞彙／翻譯管線，只處理選定軌。無字幕辨識支援本機 WAVE、MPEG-1 MP3、48 kHz AAC-LC ADTS 及有限 44.1／48 kHz AAC-LC M4A／MP4，可含一軌 H.264 Baseline／Main／High；需明確選五語與服務，Auto 尚不支援。內嵌文字支援 version 0／1 的單一正常速度 edit、頭尾區間裁切及前置空白；文字 ctts／modifier、多 active edits 與 A/V edit 仍不支援，所有軌的結構／範圍仍核對，只有選定本文被讀取。探索退出釋放 pin，按準備重新驗證原檔，完整結果才到 RAM Ready，不保存／匯出或自動播放。其他格式／一般 muxer／真實影片、更多遠端格式、語意與雲端品質驗收仍待接續。本批完整工程驗證已通過，已具備開始 Phase 09 對齊核心的工程條件；不代表全面產品驗收已結案。有效金鑰留至結案固定遮蔽介面手動輸入。完整進度見 [Phase 8](docs/phase-8.md)。

第四十三批在一般準備視窗新增明確 HTTPS SRT／WebVTT 模式，於字幕欄填入完整網址，再選格式／語言與需要的翻譯服務。按準備前固定揭露字幕全文連線、媒體唯讀或遠端WAVE標頭用途、中文本機處理或所選AI端點／文字前後文；不回顯私有網址／query。空白或錯誤網址不轉ASR，舊本機模式仍拒收字幕網址；HTTPS模式不探索內嵌軌。既有雙列遮蔽金鑰、只取所選服務、取消／清除與RAM Ready沿用；正式金鑰仍留至結案手動輸入。見 [ADR 0076](docs/adr/0076-explicit-prepared-https-subtitle-ui.md)。

第四十二批提供明確Task入口，將HTTPS SRT／WebVTT接入同一Session的全文處理及RAM Ready。網路、本機外掛與內嵌字幕共用八份上限及語言順位，只讀勝出網址；先持有本機媒體或遠端WAVE標頭，不做ASR。中文本機臺灣本地化，五語使用呼叫者交付的同一翻譯服務；字幕本文配額跨分析／翻譯／completion／Ready保留。一般舊入口仍拒收字幕網址，第四十三批以另一個明確入口接入一般視窗；正式金鑰安排不變。見 [ADR 0075](docs/adr/0075-prepared-https-subtitle-session-ready.md)。

第四十一批新增獨立 HTTPS SRT／WebVTT 原文讀取程式庫，明確格式與語言、單份 GET、八 MiB 原始文件上限、全程序兩份持有者，解析共用原有 CPU lane 與完整字幕預算；取消／Drop 保持配額至實際清理。此 reader 只交付原文，不讀金鑰或套用 Live 時鐘；第四十二批以明確 Task 接入 Session／全文處理，第四十三批再接一般視窗。見 [ADR 0074](docs/adr/0074-bounded-prepared-https-subtitle-reader.md)。

第四十批讓直接 HTTP(S) PCM16 WAVE 可搭配明確選定的本機 SRT／WebVTT：只讀取來源與 WAVE 標頭，優先處理所選文字，不拉取音訊或做 ASR。中文字幕在本機轉為臺灣用語且不取金鑰；五語來源字幕使用所選服務的全文摘要／詞彙／三品質翻譯，音訊 Auto 不影響已選字幕。固定揭露媒體標頭連線與 AI 端點／文字用途，網址不加入 AI 要求或狀態。固定遮蔽金鑰、取消／清除及舊結果隔離保持，結果只在本次 RAM；無法讀取字幕就失敗，不改傳音訊。當批一般視窗仍未支援遠端內嵌／字幕網址；第四十三批已接入明確 HTTPS 字幕。一般網站／其他編碼與真實雲端驗收仍待完成，見 [ADR 0073](docs/adr/0073-remote-wave-local-sidecar-ready.md)。

第三十九批接通一般準備視窗的 HTTP(S) WAVE 無字幕入口：外掛欄真正留空且不選內嵌軌，再明確選五語、服務與品質，按準備後辨識及全文翻譯到 RAM Ready。此音訊分支及固定金鑰安排保持；網址仍不提供字幕軌探索。見 [ADR 0072](docs/adr/0072-explicit-http-wave-prepared-ui.md)。

第三十五批以同一遠端 range owner 執行既有內容格式前置檢查，網路等待釋放 CPU 配額，成功保留原來源與要求序號。尚未接入原生解碼、Ready 或 URL GUI；格式 gate 不代表完整 codec 已驗收。見 [ADR 0068](docs/adr/0068-bounded-remote-content-preflight.md)。

第三十四批新增實際 HTTP(S) 分段讀取 Rust library：強 ETag／If-Match、精確 range／長度、取消與全程序 256 KiB 在途／保留 payload 上限，不保存媒體。本批尚未接入 Prepared 解碼、Ready 或 URL GUI；一般介面仍明確不可用。見 [ADR 0067](docs/adr/0067-bounded-session-http-media-ranges.md)。

第三十三批在原有逐批詞彙分析中加入全文脈絡摘要，限 4096 UTF-8 bytes，所有草稿與校訂共用最後摘要。摘要可能有誤，只作翻譯提示；缺少或無效則明確失敗，不發布部分 Ready。外掛／內嵌來源字幕與 ASR 共用，中文本機與結案手動金鑰不變。見 [ADR 0066](docs/adr/0066-bounded-prepared-document-context.md)。

第三十二批擴充文字 elst version 1，保留 64 位元時間與正確 signed media_time；mdhd／tkhd／mvhd 仍限原有 version 0，範圍與容量上限保持。見 [ADR 0065](docs/adr/0065-version-one-tx3g-edit-list.md)。

第三十一批能解讀文字軌的單一 edit 區間：重疊的字幕保留完整原文並調整時間，外側 cue 不進入詞彙分析／翻譯；只含空窗則明確失敗，不產生空白 Ready。整條選定軌仍先完整驗證，裁切不會繞過損壞或容量上限。這不是媒體剪輯／播放功能，操作與固定金鑰安排不變，見 [ADR 0064](docs/adr/0064-single-interval-tx3g-text-crop.md)。

第三十批先解析文字軌的有限 version 0 edit list：完整正常速度字幕可帶前置空白時間，讓 Ready 保留容器宣告的呈現起點；不同時間尺度以有理數絕對端點一次取整。當批無 edit 的舊時間規則保持，裁切尚未支援；當批 version 1 尚未支援；變速／重播及音訊／視訊 edit 仍拒收。這是靜態文件準備，沒有新增播放或 Phase 09 動態對齊。操作及固定金鑰安排不變，見 [ADR 0063](docs/adr/0063-full-track-tx3g-edit-delay.md)。

使用方式（沿用第二十九批）：填本機媒體完整路徑，按「探索 MP4 字幕軌」。若有多軌，從字幕欄右側清單選一軌，會填入 ID 及五語提示；中文／未定仍須明確選擇語言，再核對品質與所選翻譯服務後按「準備字幕」。未選軌不能準備，也不會改傳音訊。探索清空金鑰但不讀取、不用雲端、不準備或播放；來源變更／重探／取消清除舊清單，手動修改 ID 取消清單選取。只解碼所選文字，未選軌本文未驗收，不宣稱一般 MP4 相容性。設計見 [ADR 0062](docs/adr/0062-bounded-mp4-multiple-subtitle-tracks.md)。

第二十八批先提供單軌探索，沿用 bounded worker／取消及原檔 pin，第二十九批擴大為明確多軌清單；當批記錄見 [ADR 0061](docs/adr/0061-bounded-local-mp4-subtitle-discovery.md)。

第二十七批先把指定 tx3g 軌接到一般 Prepared 視窗與同一個 Session／全文處理／Ready，當時需已知非零軌 ID；探索在第二十八批接上。中文不取用金鑰；來源字幕只將原文與脈絡送往所選服務，固定遮蔽金鑰欄維持既有規則。內嵌模式空白／非法 ID 拒收，讀取錯誤不改送音訊；取消／清除釋放 RAM 及同一原檔 pin，完成後不播放。見 [ADR 0060](docs/adr/0060-explicit-embedded-prepared-ready-and-ui.md)。

第二十六批提供 Rust 呼叫介面 `extract_local_mp4_subtitle`：從同一個唯讀本機 pin 讀取明確指定的單一 tx3g 軌，驗證 description／fonts、各軌 sample tables／檔內引用／mdat 範圍與跨軌 byte overlap，沿用前批純文字核心。限定一個既有 AAC-LC 音軌、可有一個既有 H.264 視訊軌及一個文字軌，沒有自動選軌／語言；edit list、文字 ctts、動態顯示／modifier、外部引用與多文字軌明確拒收。當批未接 GUI／Ready，後續接線見第二十七批；合成容器／檔案測試不代表一般 muxer 或使用者影片已驗收。決策與限制見 [ADR 0059](docs/adr/0059-selected-pinned-mp4-tx3g-reader.md)。

第二十五批新增有界 MP4 `tx3g` 純文字樣本核心：嚴格 UTF-8／UTF-16、空字串清除、原始軌道時間與 Session／序號核對；任一錯誤取消整份文件，不交付部分字幕。輸出沿用 ExistingText，可供既有臺灣用語／全文翻譯階段使用。當批未接入容器／GUI；一般 --ui 仍不讀取內嵌字幕，未宣稱真實 MP4 相容性或 Ready／播放驗收。樣式／卡拉 OK／連結等 modifier 暫時明確拒收。設計與限制見 [ADR 0058](docs/adr/0058-bounded-tx3g-plain-text-samples.md) 及 [Phase 8](docs/phase-8.md)。

第二十四批補齊真正含 B-frame 的 Main／High 本機合成 MP4 驗證：實際 B slice 與影格時間重排均有證據，音訊以原有管線到 RAM Ready。正式 parser／decoder／時鐘不變，沒有產品視訊解碼／encoder、音訊保存或播放；其他 muxer／設定／一般影片與雲端仍待驗收。設計與證據見 [ADR 0057](docs/adr/0057-genuine-b-frame-mp4-audio-evidence.md)。

第二十三批開放 H.264 Main／High 的有限 MP4 描述，High 直接核對 SPS 的 8-bit／4:2:0，並接受嚴格版本與樣本數的視訊 ctts。維持只解碼 AAC 音訊、同一管線與 RAM Ready，其他 profile／codec、真正 B frames／一般影片／muxer 及播放仍待驗收。設計見 [ADR 0056](docs/adr/0056-ordinary-h264-profiles-and-composition-tables.md)。

第二十二批新增有限的本機 H.264 Baseline＋AAC-LC MP4 影片輸入，只取音訊進入同一 Prepared 管線。每軌檔內引用與 sample／chunk 完整範圍及跨軌 overlap 都核對；moov 一 MiB、avcC 64 KiB、尺寸 8192 與原有時鐘／取消／pin 限制保持。視訊不解碼、渲染、上傳或保存；其他 profile／codec、多音軌與未知容器仍拒收。設計與證據見 [ADR 0055](docs/adr/0055-bounded-h264-aac-mp4-prepared-input.md)。

第二十一批開放 M4A／MP4 的已知 UTF-8 文字標籤、JPEG／PNG 封面及少數數值欄位，這些描述資料只做有界檢查，不送到辨識／翻譯或保存。每筆文字最多 64 KiB、封面 512 KiB、ilst 合計 768 KiB，整份 moov 仍限一 MiB。真正合成檔加標籤前後的每個 PCM sample／native timestamp 完全相同；Prepared Ready 與取消流程沿用。未知、重複／多值、超量標籤與其他未支援容器仍拒收。限制見 [ADR 0054](docs/adr/0054-bounded-mp4-descriptive-tags.md)。

第二十批完成 AAC-LC M4A／MP4 44.1 kHz 單／雙聲道的原生時間與 Prepared Ready 驗證，擴大前批的 48 kHz 範圍。真正 RAM 編碼的十二種率／聲道／長度組合直接查詢 IMF timestamp／duration／PCM frames；44.1 kHz 本機最大 native duration 重疊只有 0.1 微秒，frame-derived 端點沒有重疊。原有一微秒 gate、時間映射與有界容量保持；ADTS 44.1 kHz 的限制仍保留，影片、封面等容器限制當時沿用。詳見 [ADR 0053](docs/adr/0053-self-contained-mp4-aac-prepared-input.md)。

第十九批先核對單一音軌、檔內資料引用、AAC-LC 描述，以及每個音訊 chunk／sample 的完整 byte 範圍，再交給同一 pinned file 的 MF reader。門檻最多讀取一 MiB moov，跳過音訊 payload；外部引用與未知結構明確拒收。初始 48 kHz mono／stereo 經原有一微秒 gate、映射、ASR／詞彙／翻譯到 Ready，沒有放寬時鐘或新增媒體保存。限制與決策見 [ADR 0053](docs/adr/0053-self-contained-mp4-aac-prepared-input.md)。

第十八批將 AAC-LC／ADTS 支援擴大為 48 kHz 單／雙聲道。直接查詢 IMF sample time／duration 的十三秒合成測試另確認：44.1 kHz 的原生 duration 仍有最多 4.3 微秒重疊，不能用它取代 PCM 推導端點解決問題；44.1 kHz 維持前置拒收。48 kHz 單聲道沿用同一 actor、時間快照及 ASR／詞彙／翻譯到 Ready，原有一微秒 gate 與 bounded 容量不變。合成／mock 驗證不代表使用者檔案、雲端品質或播放同步。詳見 [ADR 0052](docs/adr/0052-bounded-adts-aac-and-native-clock-gate.md)。

第十七批新增 AAC-LC／ADTS 的二十 bytes 內容門檻，沿用 native actor、時間快照與全文準備到 Ready；初始開放 48 kHz 雙聲道。44.1 kHz 單聲道合成檔觀測到四微秒的時間重疊，原有一微秒檢查保持，其餘組合當時先回 UnsupportedAudioClock。共同 metadata 最多 640 rows／十二秒，以涵蓋合法的密集 1024-frame 情形；PCM、worker、回覆與 ASR 容量維持。真正 MF 可合併多個封包，壓縮 packet 數不等於 native row 數。合成資料／mock 與獨立 decoder endpoint oracle 驗證本批，真實 AAC 檔案及雲端品質仍待結案。詳見 [ADR 0052](docs/adr/0052-bounded-adts-aac-and-native-clock-gate.md)。

第十六批在任何 Draft 前完成所有原文窗口的專名／術語分析，再供整份 Draft／Review 使用。候選必須在原文中逐字存在，UTF-8 範圍由本機推導；生成譯名在有界 CPU lane 本地化，手動詞彙優先、衝突或超過 64 筆明確失敗。OpenAI／Gemini 分析沿用同一所選 client、速率與要求許可，中文不連雲端。介面揭露專名／詞彙整理用途；有效金鑰及真實譯名品質留至結案。詳見 [ADR 0051](docs/adr/0051-grounded-prepared-session-glossary.md)。

第十五批在分段取得不可變快照後才提交新 block 的時間證據，依真正保留的左脈絡與未交付 DSP 起點限制窗口。保留完整跨界原始 span，已回收前綴不能再查詢；十二秒可用範圍、512 rows、單一 pending block 與每批兩份分段維持有界。真缺口先排出舊尾段，即使新 run 沒有輸出也不延留舊正文；WAVE 精度與字幕優先保持原有方式。設計見 [ADR 0050](docs/adr/0050-native-prepared-clock-handoff.md)；第十四批快照與定稿合併設計見 [ADR 0049](docs/adr/0049-chunk-owned-clock-evidence-and-native-finals.md)。

第十三批的 `media::decoded_clock` 以累積取樣數建立 DSP／ASR 用時鐘，另保留每個原生起點與真正 PCM frames 推導的媒體端點。真缺口、Session、序號與格式都有核對；最多 512 spans／十二秒的 rolling metadata 明確 prune，不外推或跨缺口。PCM allocation 原樣移交，不補靜音或放寬既有 WAVE 精度。真正合成 MP3 經映射驗證 16／24 kHz DSP；第十四／十五批已將映射綁定分段、結果合併及正式 producer；合成／mock Ready 不代表雲端品質或播放同步。設計見 [ADR 0048](docs/adr/0048-bounded-decoded-sample-to-native-clock-map.md)。

第十二批的 `decode_local_audio` library 支援以同一唯讀檔案 object 解碼 MPEG-1 MP3。內容門檻只讀最多 31 bytes，核對有界 ID3 與兩個相鄰 frame，不依副檔名、不掃描任意 sync，也不接受 MPEG-2／2.5、free format、AAC、MP4 或播放清單。本機合成 MP3 已揭露 decoder delay 與 100 ns frame 時間取整會影響原有 PCM 時鐘；當時前處理／ASR 先回報 `UnsupportedAudioClock`；第十五批 ASR 改用完整映射，公開 borrowed DSP 仍拒收壓縮封包時鐘，沒有放寬精度或重設時間。MP3 搭配已有字幕仍可準備。完整時間映射已接入；實際 VBR／使用者檔案相容性待驗收，設計見 [ADR 0047](docs/adr/0047-bounded-mp3-native-decoding-and-clock-gate.md)。

第十一批將正式 Prepared 辨識接入原有背景 worker。準備視窗新增獨立的音訊來源語言欄，字幕格式／語言只用於非空外掛字幕；空白字元、無效或損壞字幕不會觸發音訊上傳。固定目的地列在啟動前揭露所選服務將收到短片段音訊、辨識原文及翻譯脈絡；來源字幕只傳文字，中文仍本機本地化。所選遮蔽金鑰只解析一次，辨識與翻譯使用同一服務，取用後清空雙列與 undo；留空才由背景工作讀取該服務安全憑證。取消／關閉等待真正 worker 清理，完整成功才有本次 Ready，不新增保存、匯出或播放。使用方式與限制見 [固定金鑰介面](docs/api-key-ui.md#prepared-本次辨識與翻譯) 及 [ADR 0046](docs/adr/0046-explicit-prepared-asr-gui.md)；真實雲端辨識品質尚未驗收。

第六批新增與 Live 期限獨立的 provider-neutral ASR 契約：10 秒正文、左右各最多 500 ms 真實音訊 overlap、最多 11 秒的 owned chunk；分段 ready 最多兩份，辨識每批最多四份，按明確服務能力限制一至四個並行要求，每個要求六十秒。原生定稿時間必須完整落於唯一正文，跨界句明確拒收而不裁句；倒序結果還原後交付獨立的 PreparedTranscript，保留辨識來源，不冒充既有字幕或播放定位證據。全文限 8192 cues／四 MiB，前文限八筆／六十四 KiB；取消、失敗及 Future Drop 清除 RAM。第七批新增本機 WAVE 的私有拉取 actor 與 recognize_local_media library，透過 owned DSP 移交分段；命令容量一、回覆最多兩份、辨識批次最多四份。原生 owner 與 CPU 許可各依 available_parallelism 最多兩份，服務等待釋放 CPU 且停止新的解碼拉取；COM 建立、使用與銷毀固定同執行緒，EOF 等待真正退出。取消及 Future Drop 保持容量到實際清理；舊公開 decode／preprocess callback 契約保留。正式 ASR adapter、Ready 組合與 GUI ASR 仍待接續。GUI 無字幕仍回報 ASR 未接入，可靠外掛字幕不額外解碼。MP3／AAC／影片容器、內嵌字幕、URL 實際讀取、自動專名／詞彙建立及播放定位仍待接續，Phase 09 尚未具備啟動條件。完整進度、驗證及限制見 [Phase 8](docs/phase-8.md)、[ADR 0040](docs/adr/0040-prepared-audio-clock-and-bounded-preprocessing.md)、[ADR 0041](docs/adr/0041-bounded-prepared-recognition-and-transcript-provenance.md) 與 [ADR 0042](docs/adr/0042-demand-driven-native-prepared-asr.md)。沒有新增 crate、unsafe、音訊落盤或服務連線；有效金鑰及真實雲端驗收仍留至結案固定遮蔽介面手動輸入。

第八批提供 `translate_recognized_transcript`：caller 明確交付 owned ASR transcript、同 Session／generation 詞彙表與翻譯服務，與外掛來源字幕共用借用原文的全文翻譯引擎。Fast／Standard／High 使用一／三／八句脈絡，Fast 一次草稿，Standard／High 完成全文草稿後做一致性校訂；每要求三十秒，沿用 client 共用的並行／速率限制。網路等待不持 CPU 許可，最後在共用有界 worker 本地化臺灣用語並核對原始身分、時間、順序、句數、每句 8192 bytes 與全文四 MiB。成功回傳保留原始 ASR 的 `PreparedTranslatedTranscript`，譯文使用 boxed owner；取消、失敗及未 poll／執行中 Future Drop 不回傳部分本文。沒有假 `ExistingSubtitle`、整份原文複製、Debug／Clone／Serialize、媒體 pin、Ready 或播放定位證據。正式 ASR adapter、自動全文脈絡／glossary 分析及同一媒體 handle 的 Ready 組合仍待完成；一般 GUI 行為與結案金鑰安排不變。設計見 [ADR 0043](docs/adr/0043-recognized-transcript-translation-provenance.md)，本批實際驗證另記於 [Phase 8](docs/phase-8.md)，不將 library 或 mock 結果當作真實雲端品質驗收。

第九批新增 `PreparedTask::run_with_recognizer`，在沒有可用外掛字幕時，將同一唯讀媒體 pin 經辨識、全文翻譯與臺灣本地化交付 Session Ready。媒體只開啟一次，native IStream 使用同一檔案物件的複製 handle；真正 EOF 銷毀 native 資源並回收 worker 後，原 pin 仍保持至 Ready 清除。`PreparedDocument::original()` 明確區分既有字幕及辨識原文，`cue_count()`／譯文共用，辨識來源保留原生時間、五語及品質，不冒充出版物或播放定位。既有字幕優先且不觸碰 recognizer；舊 glossary 在 ASR 前拒收，取消／失敗／舊 completion 不交付文件。這是 caller 明確配置服務的 Session library，沒有正式 ASR adapter 或 GUI 音訊上傳入口；一般 GUI 無字幕仍回報不可用，Phase 09 尚未具備條件，結案金鑰安排不變。設計見 [ADR 0044](docs/adr/0044-pinned-recognized-prepared-ready.md)，驗證另記於 [Phase 8](docs/phase-8.md)。

第十批提供 `HttpPreparedRecognizer`：OpenAI whisper-1 的 verbose segment 時間與 Gemini gemini-3.5-transcribe 的 Interactions VERBATIM word_info 都使用真正 API 欄位。caller 明確選服務、金鑰及五語來源；Auto 在上傳前拒收，Gemini 語言提示不冒充偵測。短期 PCM 在 RAM 包裝 WAVE，Gemini 關閉保存／背景／串流且不用 Files API；不加入來源路徑。CPU worker 處理編碼／解析，HTTP 最多兩個要求並依固定 CPU 容量調整，六十秒總預算、有界重試、Retry-After、完整 JSON 容量與取消均受控。子範圍清理保留真正的 typed error，worker lease 至實際退出。OpenAI 僅保留最後 1024 bytes 原文提示；Gemini 時間戳模式不送前文／ASR 詞彙。未標註本文、跨正文、未知輸出類型或時間均保守拒收，不裁句或捏造時間。一般 GUI ASR 與真實雲端仍待完成；設計及官方資料見 [ADR 0045](docs/adr/0045-bounded-prepared-speech-http-providers.md)。

Phase 7 已完成的核心與配置：已補齊 Phase 6 中斷的歌詞查詢管理，並加入可靠電影候選的字幕管理、正式 HTTPS factory 與獨立服務提示；必須明確配置內容／文字服務及已知語言才可查詢，預設仍未設定服務。Phase 7 提供中文臺灣用語本地化、五語字幕視窗翻譯、最終 ASR 的保守完整 cue 驗證、中文配對來源參考軌驗證、HTTPS SRT／WebVTT 純文字讀取、依定位證據仲裁，以及同一 TranscriptTimeline 內的字幕採用與 Live AI 補洞介面。可設定的 Live／GUI 將字幕及歌詞交付同一候選 task，採用位置來自實際擷取游標；明確啟用時可將一筆有完整原生時間的當前 Final ASR 與已設定媒體線索交付電影目錄，視窗／Session／取消核對不符就拒收；Windows 來源可使用核對 PID／建立身分的唯一直接所屬標題，兩秒原始觀測期限與有界可停止工作保護 metadata，多視窗或未知資訊不推論；字幕／歌詞已共用一份 Session metadata 管理，核對完整身分與發布水位，相同資料不重抓，更新／撤回會取消進行中及已完成的舊文字；歌詞來源補上 Session 隔離，兩類 owner 依 metadata 版本重新綁定。一般 GUI 已提供「字幕來源（本次）」：明確片名、HTTPS SRT／WebVTT 及五語來源設定，字幕下載後在本機以所選程序標題及獨立完整對白核對，與字幕管理共用一份 Session RAM document；只有原有精確定位／翻譯／仲裁通過才顯示。正式全球電影目錄、中文真實配對資料與媒體驗收仍待完成，預設 GUI 仍使用 Live AI；外部歌曲辨識服務依使用者選擇暫緩。Phase 7 第十四批共 711 項一般測試、格式、Clippy、Debug／Release 建置通過，並確認 Phase 8 啟動條件；使用者已於 2026-10-11 批准啟動。進度見 [Phase 7](docs/phase-7.md) 與 [Phase 6](docs/phase-6.md)。實際歌曲／電影目錄、分類與字幕服務、播放器／DPI／效能及真實雲端驗收仍待完成，沒有因進入下一階段而列為通過。有效金鑰與真實服務測試維持結案時手動輸入，固定 OpenAI／Gemini 介面不變。Phase 2 的 19045 核心功能、300 次重啟與 30 分鐘擷取已驗證，完整應用程式矩陣仍待人工。

第五批加入電影／影集 metadata 交叉核對核心：片名線索必須與目錄對白參考同時相符，已知年份／語言／片長也須一致，保留原信心與多個模糊候選。只完成 provider-neutral 比對與要求生命週期；實際 MovieCatalog、當前視窗／ASR hints 分流和 GUI 接線仍待完成，測試目錄是明確 mock。

第六批加入已確認歌曲到有界文字查詢的 owner：同一歌曲 binding 不重抓，切換／Transition／停止／過期會取消並清除，只有最新選擇與結果，支援明確 refresh。已建立到 LRCLIB 本機 HTTP 的完整查詢路徑；正式 GUI 尚未建立此 owner，也不自動採用原始文字或啟動內容服務。

第七批將 LiveResolver 接入正式 Live drain／select／停止／重連流程，新增明確 ContentConfiguration／factory 的 GUI 組合入口及有界狀態。視窗呈現服務處理目的地後才允許啟動，內容分支失敗不停止 STT／翻譯；只顯示分類／歌曲狀態，不顯示 metadata 或改採字幕來源。一般 --ui／既有命令仍傳入 None，不讀取額外金鑰或啟動內容服務。

第八批修正內容辨識的取消與結果生命週期：重設／停止同步清除共享結果，未讀與已讀 Ready／Resolving 都依原始時間過期，舊發布不能填回空槽，新有效發布也不會被延遲清除擦掉。新增同步 stop、取消與 deadline 喚醒，並避免重發較舊時間戳或重複終態；仍不修改字幕來源或啟用額外服務。

第九批補齊 LiveLyrics 與 LRCLIB 配置：正式 Live 會共同輪詢內容辨識與歌詞查詢，GUI 先呈現兩個服務的處理提示才允許啟動。同一歌曲 metadata 不重抓；切換／缺口／失敗／停止清除候選，查詢失敗不取消 STT。只有查詢狀態進入 UI，原始歌詞沒有直接顯示。Phase 7 本地化目前是獨立 library 入口，尚未從 GUI 自動呼叫或採用結果。

2026-10-10 Phase 7 第二批加入明確選取一至四個來源字幕片段的翻譯入口，沿用既有 Translator 與 OpenAI／Gemini adapter。Fast／Standard／High 分別使用最多一／三／八句前文，High 另做校訂；完整視窗成功後才提交前文。共用 HTTP client 最多兩個執行中要求，取消、過期、失敗或遲到結果均不交付候選。這是獨立 library 入口，尚不會從 GUI 自動選取、翻譯或採用字幕；正式金鑰測試仍留至結案。驗證紀錄見 [Phase 7](docs/phase-7.md#最終驗證)。

本批新增 17 項測試；完整 384 項一般測試、格式、Clippy 與 Debug／Release 建置通過。既有 14 項略過項目不變，沒有把本機 HTTP 合成測試列為真實雲端驗收。

Phase 7 第三批加入 Session 所有的 SubtitleSourceArbiter。最多四份候選，依內容／定位信心、新鮮度、可用性及實際翻譯品質篩選，再按原生 zh-TW、其他繁中、簡中、來源翻譯排序。只採用證據明確涵蓋的單一 cue；缺口回傳 LiveAi 決策，同等優先度保持目前來源。取消／過期釋放候選，失效版本拒收舊結果。這是獨立仲裁入口，正式 GUI 尚未啟用字幕來源切換。

第三批新增 17 項測試，保留裁切字幕的原文件索引與媒體時間；完整 401 項一般測試、格式、Clippy 與 Debug／Release 建置通過。既有 14 項略過不變，正式金鑰與實際定位／播放器驗收仍待結案；詳細驗證見 [Phase 7](docs/phase-7.md#最終驗證)。

第四批將仲裁接入 LiveTranslation 與原有時間軸。只有同工作階段、明確定時且已定稿的 ASR 片段，能綁定涵蓋其完整捕捉範圍的候選；同一 cue 不會新增重複字幕項目。顯示來源保留文件索引與媒體時間，Live AI 修訂／前綴仍留在同一項目作回復。採用時取消被取代的翻譯要求，缺口／取消／過期恢復原有譯文或在原剩餘預算內重新排程。三種顯示模式共用同一份來源快照，狀態會提示使用既有文字且即時辨識仍用所選雲端；正式 GUI 的定位與候選產生組合仍待完成。

第四批新增 14 項離線測試；完整 415 項一般測試、格式、Clippy 與 Debug／Release 建置通過，既有 14 項略過不變。實際定位、GUI 候選組合及實機驗收仍待接續，正式金鑰安排不變。

第五批加入 LiveContent／LiveLyrics 的工作階段文字介面及 TextPreparationSession，串起實際查詢結果、本地化／來源視窗翻譯、仲裁與時間軸採用。中文共用單一 CPU lane，來源字幕沿用所選翻譯 client；撤銷查詢或停止準備會使準備中及已採用候選失效，淘汰單一字幕不影響共用查詢。只有新的明確確認結果能產生新的選取時間戳，舊候選期限不隨輪詢延長；辨識信心下降會撤銷舊候選，呼叫端不能提高確認信心。正式定位證據、GUI 候選排程與實機驗收仍待完成。

第五批新增 12 項整合測試；完整 427 項一般測試、格式、Clippy 與 Debug／Release 建置通過，既有 14 項略過不變。仍以合成服務及定位證據測試，正式金鑰保持結案手動輸入。

第六批加入可由最終 ASR 計算的完整 cue 定位驗證：五語一致、全文唯一完整命中、實際時間長度相符且完整 cue 位於捕捉視窗才交付證據。僅放寬 ASCII 大小寫及空白，重複歌詞、短句、Partial、未知時間或中文候選維持 Live AI。全文 CPU 掃描 process-wide 最多二個要求，取消／Drop 回收並保留原期限；命中後僅準備該 cue，提交再核對 ASR 綁定與仲裁版本，沿用單一時間軸。新增 17 項離線測試；正式 GUI 排程、中文定位證據與真實媒體驗收仍待完成。進階 fuzzy／seek／drift 留於 Phase 9，金鑰維持結案手動輸入。

第六批完整 444 項一般測試、格式、Clippy 與 Debug／Release 建置通過，既有 14 項略過不變；這些合成輸入的結果不代表真實媒體已完成同步驗收。

第七批將最新最終 ASR、查詢出版物、完整 cue 驗證與準備接入可設定的 Live／GUI select 迴圈。最多一個執行中要求、一個最新替代輸入與一個未讀結果；新的選取取消舊工作，確認更新不延長期限或重送已準備片段。採用使用實際擷取 frame 游標，未知、缺口或已播放完的位置維持 Live AI；歌詞狀態列提示核對、所選服務準備、採用與 typed error。新增 16 項離線測試，全部 460 項一般測試通過，既有 14 項略過不變；實機同步未驗收，金鑰仍留至結案手動輸入。第七批結束時中文定位證據及正式來源服務配置仍有實作缺口。

第七批最終格式、Clippy、Debug／Release 建置均通過，完整驗證紀錄見 [Phase 7](docs/phase-7.md#最終驗證)。

第八批加入同版本來源參考軌：provider 在同一出版物聲明兩軌版本，最終 ASR 須在完整來源軌唯一命中，再以唯一相同起點及長度對應中文 cue，不比對 Live 中文譯文或假設相同索引。兩軌共用原期限與撤銷訊號，合計仍限 1024 cues／128 KiB。中文準備只做本機本地化，UI 提示處理臺灣用語；提交拒絕跨出版物替換，新版本／參考語言／有無配對切換可替換工作，同版本續期維持去重。新增 31 項離線測試，正式配對 provider 尚未配置，LRCLIB 仍為單軌資料。

第八批完整 491 項一般測試、格式、Clippy 與 Debug／Release 建置通過，既有 14 項略過不變；跨版本倒序與正式排程拒收亦驗證不會取消較新工作。所有配對服務及捕捉位置均為合成資料，實機同步未驗收。

目前仍不建議啟動 Phase 8：正式來源服務與真實配對資料配置尚有實作缺口。後續每批會檢查下一階段啟動條件並提醒，金鑰及真實雲端驗收維持結案手動輸入。第八批最終驗證見 [Phase 7](docs/phase-7.md#最終驗證)。

第九批加入明確配置的 HTTPS SRT／WebVTT 字幕 adapter，取得主軌及可選的同版本來源參考軌後原子交付。只接受有界 UTF-8 純文字子集、明確時間及完整回應，保留重複／重疊讓定位器判定歧義；不猜測語言或版本，不產生辨識信心。拒絕不安全網址、重新導向、非法字元編碼及超量資料，取消／future Drop 回收網路及有界 CPU 工作，兩軌共用原期限與總預算。新增 35 項解析及真實 localhost HTTP／時間軸整合測試；所有內容與服務皆為合成資料。

本批提供正式字幕 transport，Live GUI 的 ExistingSubtitle 來源管理、內容服務與真實 metadata／配對配置仍待完成；一般 GUI 不自動啟用新的來源，也未啟動 Prepared Mode。下一階段判定與最終驗證見 [Phase 7](docs/phase-7.md#最終驗證)。

第九批完整 526 項一般測試、格式、Clippy 與 Debug／Release 建置通過，既有 14 項略過不變。真實 HTTPS 來源與播放器同步尚未驗收；目前仍不建議 Phase 8，待正式來源配置及 Live GUI 管理完成後再檢查啟動條件並提醒。

2026-10-11 第十批加入 LiveSubtitles 與可設定 GUI 字幕管理。可靠電影候選須具有對白、遠端 metadata 及片名／檔名線索，實際信心達既有 850 門檻且與第二名差至少 100 才查詢；未知、混合、弱或模糊結果會立即撤銷舊字幕。相同電影不反覆下載，新的可靠確認才更新選取，信心下降、缺口、故障、過期及停止同步撤回。正式 HTTPS factory 核對目前 Session 與電影 ID，來源 metadata 的語言／版本不能猜測。

字幕與歌詞共用一個候選工作及原有時間軸，GUI 在建立服務前先呈現字幕讀取目的地，各來源狀態獨立投射且不替換共用快照。新增 33 項管理／factory／Live／UI 整合測試；正式內容服務、metadata 提供者與可用配置仍待完成，預設 --ui 未啟用新來源。最終驗證及下一階段判定見 [Phase 7](docs/phase-7.md#最終驗證)。

第十批全套 559 項一般測試、格式、Clippy 與 Debug／Release 建置通過，既有 14 項略過不變。可設定 GUI 的字幕管理已完成接線；目前仍不建議 Phase 8，待正式服務與來源 metadata 配置補齊後再評估並提醒，金鑰及真實服務維持結案手動測試。

--ui 可選擇來源、OpenAI／Gemini、五語／Auto、品質並開始／停止／重新啟動，兩種視窗共用同一條辨識／翻譯管線。已提供來源圖示／活動、左右停靠、字型／字級、描邊、陰影、背景不透明度、行數與主螢幕／指定螢幕／跟隨來源所在螢幕。預設內容辨識狀態顯示尚未設定服務；明確指定本次字幕來源後，可靠候選還須通過定位與仲裁才可顯示。

「字幕來源（本次）」可填入已知影片的片名、HTTPS 字幕網址、SRT／WebVTT 格式及英文／日文／法文／德文／韓文之一；套用不連線，明確開始後才下載。網址遮蔽，唯讀主機欄可查看完整目的地；設定只在 RAM，停止、自然結束、切換來源或關閉清除。只下載指定文字，不傳送音訊或辨識本文至字幕網站；辨識及翻譯仍使用所選 OpenAI／Gemini。需所選程序直接所屬的唯一可靠標題包含該片名，且來源字幕與當前 Final ASR 有唯一完整台詞與時間命中。跨句、重複台詞、標點不符、多視窗或無法核對標題時沿用 Live AI，不猜測分頁／程序關係。支援嚴格純文字字幕，最多 1024 段／128 KiB 本文，不能任意載入超限的整部影片字幕；不保存或匯出。

--stt-pid 顯示 ASR 文字，--translate-pid 顯示終端機翻譯，--translate-window-pid 啟動指定 PID 的字幕視窗。--ui 開啟互動視窗及唯讀音訊探索，不自動擷取／連雲端；明確選取來源及服務、按開始後才啟動。運行時鎖定選項，停止並回收後可在同一視窗切換來源或服務、重新開始。停止後清除字幕；關閉時取消並等待後端回收。音訊、前文、翻譯及詞彙只在 RAM、不寫檔。

停止、重新開始與重連清除字幕時，同步清空搜尋欄。手動回看時，字幕更新及最舊歷史淘汰會保留仍存在的字幕項目位置與反白；正在看的項目已被淘汰時，回到最早仍保留的字幕並取消失效選取。「回到即時」會捲至最新內容。主視窗最小化時隱藏底部字幕、恢復時重新顯示所選模式；字幕更新不自行啟用或還原主視窗，也不重啟後端。這些原生合成測試已通過，實際滑鼠／鍵盤與播放器操作仍待驗收。

「字幕外觀」選單提供微軟正黑體 UI／微軟正黑體／新細明體、16–72 DIP 的八種字級、描邊／陰影、0／25／50／70／100% 背景不透明度及 1–6 行。字級是 96 DPI 的邏輯像素高度，不是點數。預設為正黑體 UI、28 DIP、描邊與陰影開啟、70% 黑色背景、最多三行；背景透明不會同時淡化白色字幕。中文＋原文時先顯示中文，合計受行數限制；超出內容裁切於完整行，長行由原生文字排版加省略號，完整內容仍在本次 SideChat 歷史。「字幕螢幕」可選主螢幕或實際裝置，並重新整理；已選螢幕移除時明確提示暫置主螢幕。這些設定僅保留於視窗 RAM，切換不重啟後端。

「跟隨來源所在螢幕」每 500 ms 查詢已選來源的可見、非最小化視窗，優先同程序的前景視窗，其次較大的視窗；螢幕工作區域變更後才重新定位字幕。先以 PID／程序建立時間驗證，無法驗證、沒有視窗或最小化時保留位置並顯示原因。只查所選 PID 的頂層視窗，不猜測瀏覽器音訊子程序與父視窗的關係；此情況可自行選指定螢幕。跟隨選項與開始／停止各自獨立，不啟動擷取或雲端。

--ui 的來源清單直接顯示探索取得的圖示、程式名稱／PID／標題；下方顯示所選音訊工作階段的串流狀態、訊號、峰值量表、音量與靜音。未知峰值顯示未知並隱藏量表；未知圖示保留空白且明確顯示無法取得。活動更新不重建選項、不重寫 SideChat 歷史；清單展開或辨識執行中仍按 Session ID／PID／程序建立時間比對最新資料，來源消失不沿用另一個程序的圖示或峰值。這些量測只代表所選工作階段，不保證程序與子程序的全部音訊或喇叭實際可聽見。

「側邊位置」提供靠左、靠右、自由移動，預設靠右。停靠使用控制視窗所在螢幕的工作區域，寬度 460 DIP、填滿工作區域高度；小螢幕會限制寬度。拖移至另一螢幕結束後會按原側別重新定位；自由移動則可自行調整位置與大小。停靠與音訊／雲端 Session 獨立，不使用 AppBar、不保留系統桌面空間，偏好僅在 RAM。

**開發與擷取驗收基準為本機 Windows 10 22H2（19045.6456）。** 已移除 20348 組建門檻，改由實際 API 結果判定；本機已通過非靜音、不同程序隔離、既有／新增子程序、根程序退出、停止／重啟與三批共 300 次重啟驗證。先前「19045 不支援此 API」的結論已更正，當時只是本程式的版本檢查提前拒絕。

[Microsoft 參數文件](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_activation_params) 列出的最低支援組建為 20348，但 [OBS 官方指南](https://obsproject.com/kb/application-audio-capture-guide) 支援 Windows 10 2004 以上的應用程式音訊擷取。本專案沿用 Process Loopback；每個工作階段使用獨立背景程序，避免本機原生元件反覆啟動後殘留 Timer，停止時由 Job Object 與程序退出回收。詳見 [19045 設計](docs/windows-10-capture-plan.md) 與 [ADR 0004](docs/adr/0004-session-owned-capture-process.md)。本機自建來源測試不能外推到所有 Windows 版本、裝置或應用程式；來源失效仍不改抓 System Mix。

## 開發與執行

環境需求：Windows x86_64、Rust stable MSVC 工具鏈，以及 Visual Studio C++ Build Tools／Windows SDK。`rust-toolchain.toml` 指定 stable、rustfmt 與 Clippy；`Cargo.lock` 固定應用程式相依套件版本，應一併納入版本控制。

在專案根目錄執行：

```powershell
cargo run --locked
cargo run --locked -- --list-sources
cargo run --locked -- --watch-sources
cargo run --locked -- --capture-pid 1234
cargo run --locked -- --set-stt-key openai
cargo run --locked -- --check-stt-key openai
cargo run --locked -- --stt-pid 1234 --provider openai --language en
cargo run --locked -- --stt-pid 1234 --provider gemini --language auto
cargo run --locked -- --translate-pid 1234 --provider openai --language en --quality standard --interactive-key
cargo run --locked -- --translate-pid 1234 --provider gemini --language ja --quality high --interactive-key
```

未指定參數時只讀取設定，輸出 `Idle`、`phase: 6`、`content_resolver_configured: false`、設定偏好與未擷取／未使用雲端的 JSON 日誌後結束。`--list-sources` 額外列出來源，`--watch-sources` 持續輸出最新快照，按 Ctrl+C 停止。`--help` 顯示操作說明。非 Windows 平台執行時會明確回報不支援。

在支援擷取的系統上，先列出來源，再將 `1234` 換成清單中的獨立程序 PID。`--capture-pid` 明確擷取該程序與其子程序在所有播放裝置上的音訊；不保證瀏覽器個別分頁隔離。啟動前及執行中顯示 PID，按 Ctrl+C 停止。切換來源請先停止，再執行新的擷取命令。指定程序結束、裝置或服務失效時停止並回報錯誤，不會自動選取其他程序。

`--set-stt-key openai|gemini` 以隱藏輸入將 API 金鑰寫入此 Windows 使用者的認證管理員；再次執行會替換所選服務的金鑰。不要把金鑰放在命令列、一般設定或聊天訊息。`--check-stt-key` 只檢查讀取與格式，不連線、不證明服務端權限。ChatGPT／Gemini 網頁登入不會自動提供這些 API 憑證。

結案測試使用 `--stt-pid 1234 --provider openai --language en --interactive-key`（Gemini 可換成 gemini）。這個旗標在啟動時以隱藏提示要求手動輸入，金鑰僅供本次使用、不寫入認證管理員，停止時清除本程式持有的金鑰。未指定這個旗標時仍讀取既有安全憑證。開發期間不會自動要求輸入或啟動真實服務測試。

翻譯模式使用 --translate-pid 與相同的來源／供應商／語言選項；--quality 接受 fast、standard、high，省略時使用設定的 translation_quality。音訊辨識與文字翻譯都交給同一個服務，不跨供應商 fallback。使用者安排的有效金鑰測試留至結案，以上命令不會由開發流程自動執行。

指定來源的視窗驗收在結案時執行 --translate-window-pid，使用相同 provider／language／quality 與 --interactive-key 旗標；隱藏提示在開啟視窗前於本機終端機輸入，不儲存。視窗來源提示完成後才讀取既有認證／連線。操作步驟見 [Phase 5 測試程序](docs/phase-5-test-procedure.md)。

--ui 主視窗固定提供 OpenAI（ChatGPT）與 Gemini 兩個獨立的遮蔽 API 金鑰欄位，在結案時手動輸入。切換服務不混用或移動輸入；開始只取用所選服務的欄位，隨即清空兩組輸入與復原紀錄。未按「儲存」時只供本次啟動，不寫入設定／認證管理員；留空時讀取所選服務的既有 Windows 認證。每個欄位有自己的「儲存」按鈕，明確按下才寫入或取代該服務的 Windows 安全憑證，清空該欄位而保留另一組輸入。儲存不需選來源、不擷取、不連線，也不代表服務端權限已通過。操作與限制見 [固定金鑰介面](docs/api-key-ui.md)。

`--stt-pid` 必須在互動式終端機執行，拒絕將逐字稿重導向到檔案；不提供匯出功能。供應商與語言旗標可覆寫設定，省略供應商且設定為 null 時回報錯誤。語言只接受 auto、en、ja、fr、de、ko。只有 Gemini Transcribe Live 支援 Auto：未知語言 Partial 暫存至受支援的語言回報，再釋放；Final 未回報語言或包含五語以外的語言時停止。GPT 須明選五語之一。未知來源時間保留空值，不捏造時間。GUI 的可選 Live 模型不改變此命令列的供應商預設模型。

STT 使用抗混疊重採樣。OpenAI 沿用 400 ms 靜音／4 秒上限的本機斷句及 committed acknowledgement／item_id。Gemini 改用連續 PCM（含靜音）及服務端自動活動偵測，不等待前段 Final 才送下一段音訊；回覆段落順序不是擷取區塊關聯，無精確關係時 start／duration 保持未知，只供即時字幕／翻譯，不作媒體定位。所有高頻佇列仍有界，250 ms 音訊年齡、最多三次復原、同來源／同服務、九分鐘 Session 輪替及停止回收保持。完整策略與限制見 ADR 0091，真實服務品質仍待人工驗收。

來源資料輸出至 stdout，診斷日誌輸出至 stderr。探索所有啟用中的播放裝置，不啟用麥克風或音訊擷取。每輪掃描後等待 250 ms 再更新；建立通知補足新工作階段，過期工作階段與移除的裝置會從後續快照消失。

串流「執行中」與「有訊號」分別顯示，峰值未知時不假設為零；峰值不代表喇叭一定可聽見聲音。系統音效及跨程序工作階段會明確標示。缺少視窗或圖示時顯示「無法取得」，不虛構資料。圖示以 32×32 BGRA 像素保留於 RAM，命令列顯示取得狀態，--ui 的來源清單呈現實際圖示；指定 PID 的診斷視窗不另開完整來源探索面板。

詳細設計、限制及驗證方式見 [Phase 1 說明](docs/phase-1.md)及 [Phase 2 說明](docs/phase-2.md)。

## 設定

Windows 設定位置為 `%LOCALAPPDATA%\SubtitleNow\config.json`。檔案不存在時使用記憶體內的預設值，啟動不會建立資料夾或寫入設定。

需要調整時，可自行建立該路徑的 UTF-8 JSON 檔案（不含 BOM）：

```json
{
  "schema_version": 2,
  "source_language": "Auto",
  "translation_quality": "Standard",
  "display_mode": "SideChat"
}
```

| 欄位 | 可用值 | 預設 |
| --- | --- | --- |
| `schema_version` | `2`，必填；可讀舊 `1` | `2` |
| `source_language` | `Auto`、`English`、`Japanese`、`French`、`German`、`Korean` | `Auto` |
| `translation_quality` | `Fast`、`Standard`、`High` | `Standard` |
| `display_mode` | `SideChat`、`BottomSubtitle`、`Dual` | `SideChat` |
| `speech_provider` | `OpenAi`、`Gemini`、`null`，可省略 | `null`（未選擇） |
| `live_speech_model` | `GptLiveTranscribe`、`GeminiTranscribeLive`、`GeminiLiveTranslate`、`null`；須與供應商一致 | `null`（沿用供應商預設） |

speech_provider 保存供應商偏好，Live 模型另存 live_speech_model；明確按開始或執行雲端命令才啟動。金鑰不能放入設定，未選擇的可選欄位儲存時省略。schema 1 在 RAM 遷移至 2，模型保持空值，不自動寫回或切換原供應商。若需回到只支援 schema 1 的舊執行檔，先備份偏好，再手動移除 live_speech_model 並將版本改為 1；不影響 Windows 認證管理員金鑰。

翻譯目標固定為臺灣繁體中文 `zh-TW`，不能透過設定覆寫。省略偏好欄位時使用預設值；不支援的版本、未知欄位、錯誤選項、損壞內容與超過 16 KiB 的檔案都會回報錯誤，保留原檔並以非零結束碼離開。

設定僅容納上述偏好，不接受 API 金鑰、音訊、逐字稿、字幕、歌詞或翻譯輸出。`AppConfig::save` 會先驗證、寫入同資料夾的暫存設定檔，成功後才替換目標檔；一般失敗會清理暫存檔。服務頁的「儲存服務與偏好」明確儲存模型／服務、來源語言與品質，保留其他偏好，不自動儲存；固定金鑰介面的各列儲存仍使用 Windows 認證管理員。

## 日誌與錯誤

日誌使用 `tracing`，以 JSON 寫至 stderr。預設 `INFO`；可透過 `SUBTITLENOW_LOG` 指定 `ERROR`、`WARN`、`INFO`、`DEBUG` 或 `TRACE`（不分大小寫）：

```powershell
$env:SUBTITLENOW_LOG = 'DEBUG'
cargo run --locked
Remove-Item Env:SUBTITLENOW_LOG
```

啟動錯誤包含固定錯誤代碼與臺灣繁中訊息，不輸出原始設定、路徑或錯誤來源鏈。日誌尚未初始化時，錯誤以純文字寫至 stderr。程式不建立日誌檔、不傳送遙測；一般命令的雲端只用於明確啟動的 STT／翻譯，可選內容服務須由組合入口明確配置及顯示處理目的地。文字顯示於互動式終端機或原生視窗，不進入 tracing 日誌；視窗模式不向 stdout 輸出內容。Debug 顯示擷取、STT、預處理與翻譯佇列深度、翻譯要求數、丟棄 frame 與重連次數；已配置內容分支另顯示音訊 queue／processed／dropped／reset／worker 數，不含內容資料。

## 模組邊界

| 模組 | 責任與目前範圍 |
| --- | --- |
| `app` | 啟動組合、設定、指定來源辨識、可選內容 factory／LiveContent 與同服務重連；狀態日誌包含 Idle／StartingLive／LiveRunning／Recovering |
| `domain` | 固定語言、翻譯品質與顯示模式的共用值型別 |
| `error`、`logging` | 型別化錯誤、安全診斷訊息與日誌初始化 |
| `audio` | 探索、擷取生命週期、有界 ring、PCM 正規化、抗混疊重採樣與統計 |
| `win32` | Core Audio、Process Loopback、COM／handle、程序／視窗／圖示與本程式憑證 |
| `ai` | STT WSS、翻譯 HTTPS／SSE、三種品質、字詞轉換、有界並行與安全錯誤 |
| `resolver` | 有界音訊組窗、Chromaprint、歌曲投票、文字選擇／查詢生命週期、歌詞／字幕契約、LRCLIB adapter 與電影 metadata 比對；正式服務組合待接線 |
| `media` | 私密本機／HTTP(S) 輸入型別、唯讀檔案 handle、有界字幕讀取／優先選取、WAVE／MPEG-1 MP3 原生解碼、內容門檻及有界時鐘映射核心；WAVE 支援 16／24 kHz 前處理與有界 ASR 拉取，MP3 原生 actor 的快照移交、其他容器及 URL 尚待接續 |
| `ui` | 命令列來源列表、單一時間軸快照、內容處理狀態／目的地提示與共用顯示層；原生視窗在 win32 |
| `subtitle` | 共用 TranscriptTimeline、來源／翻譯修訂、Stable 前綴及 Final 保護 |
| `session` | LiveTranslation 持有翻譯與單一時間軸；LiveResolver 持有內容分流；PreparedSession 管理外掛字幕或同一媒體 pin 的辨識／全文翻譯、RAM Ready、取消與舊結果隔離，正式 ASR client、GUI ASR 與播放接線待完成 |

尚未實作的預留模組只有責任說明。採單一 Cargo package，透過 library 與 binary 分離可測試邏輯和程式入口。原生資源封裝於 `win32`；探索與擷取各由自己的 MTA worker 負責，正規化在獨立 worker 執行。Tokio 管理狀態、消費端計時與 Ctrl+C。擷取父程序只固定所選 PID；最新 Core Audio 來源確認在背景程序、原生 client 啟動前完成。背景程序內有擷取、正規化與 pipe 傳送三個固定角色，父程序有一個接收 worker，不再重複正規化。每個工作階段最多一個背景程序。

## 驗證與 CI

驗收指令：

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

測試涵蓋預設值、支援選項的設定往返、版本拒絕、大小上限、損壞設定、讀寫失敗、設定暫存檔清理、啟動結束碼與敏感輸入不進入日誌。測試使用隔離的暫存資料夾及子程序環境，不需要 API 金鑰或雲端服務，也不修改使用者的實際設定。

2026-10-05 本機 Windows MSVC 驗收：上述四項指令全部通過，共 18 項測試（2 項日誌單元測試、11 項設定整合測試、5 項啟動整合測試）。

上述 18 項為 Phase 0 的驗收紀錄；Phase 1 新增慢速消費端、停止／重新啟動、錯誤復原、來源識別、音量未知值、終端機控制字元與圖示測試。實機工作階段生命週期測試需要播放裝置，預設略過，可另外執行：

```powershell
cargo test --locked native_session_appears_updates_and_disappears -- --ignored --nocapture
```

此測試建立兩個獨立程序，只播放記憶體中的靜音緩衝，驗證新增、閒置與程序結束後移除，不錄製或寫入音訊。

[Windows CI](.github/workflows/ci.yml) 會在 GitHub push、pull request 或手動觸發時執行格式、Clippy、測試與建置檢查，Cargo 相依解析使用 `--locked`。目前工作目錄尚未初始化 Git，也未連接 GitHub；遠端 CI 尚未執行。

Phase 2 最終一般測試 71 項通過，12 項需明確環境或長時間的測試預設略過；另有十項短時間實機／資源驗證全數通過，包含三批共 300 次重啟。30 分鐘連續擷取亦通過。完整結果、失敗與工具限制見 [Phase 2](docs/phase-2.md)，重現方法見 [測試操作](docs/phase-2-test-procedure.md)。

2026-10-09 Phase 3 一般測試 113 項通過，格式／Clippy／Debug 與 Release 建置通過；另已執行無效金鑰的外部 WSS 拒絕測試。兩個供應商 × 五語的本機協定 fixtures 不代表真實辨識準確度。本機 API 金鑰均未設定，剩餘服務驗收見 [Phase 3 測試操作](docs/phase-3-test-procedure.md)。

2026-10-09 Phase 4 第一批：一般測試 127 項通過，新增 14 項翻譯核心測試，格式／Clippy／Debug 與 Release 建置通過。本批沒有向使用者索取金鑰或執行雲端測試。實際翻譯 adapter 與 STT 接線仍待完成，見 [Phase 4](docs/phase-4.md)。

2026-10-09 Phase 4 第二批已接入兩種翻譯 adapter 與 LiveTranslation。一般測試 141 項通過，新增 14 項；格式／Clippy／Debug 與 Release 建置通過。五語事件 → 本機 HTTP fixture → 共用時間軸均已測試，沒有向真正供應商送出內容。自然度、準確度與延遲仍留至結案測試。

2026-10-09 Phase 4 第三批補上固定前綴提交的狀態保護，避免在要求執行或結果未套用時改寫前綴而用 Session 失敗。一般測試 143 項通過，新增兩項回歸測試；格式／Clippy／Debug 與 Release 建置通過。真實服務驗收仍依安排留至結案。

2026-10-09 Phase 5 首批：--ui 待命原生視窗與共用顯示模型已加入。最終一般測試 148 項通過，格式／Clippy／Debug 與 Release 建置通過；即時視窗後端接線仍待完成。Computer Use 畫面擷取逾時，以及編譯負載下四項既有測試的暫時失敗與完整重跑結果均記錄於 [Phase 5](docs/phase-5.md)，不宣稱視覺或真實服務驗收通過。

2026-10-09 Phase 5 第二批：新增一格有界通知與原生視窗消費入口，使用 Arc 共用 timeline 快照，測試單一翻譯流程及背景執行緒到原生控制項。一般測試 155 項通過，新增七項；格式／Clippy／Debug 與 Release 建置通過。--ui 仍待命，正式 app 的來源控制與 live owner 啟動／回收仍待整合；沒有進行真實服務或視覺驗收。

2026-10-09 Phase 5 第三批：--translate-window-pid 正式連接既有辨識／翻譯管線與視窗；新增來源提示啟動限制、停止選單與後端 RAII／join。新增十項測試，一般測試 165 項通過；格式／Clippy／Debug 與 Release 建置及 Release --help 通過。有效憑證、真實音訊／雲端與視覺驗收仍留至結案，沒有把 mock 的取消／回收結果當成所有實機情境已通過。

2026-10-09 Phase 5 第四批：--ui 加入來源、供應商、語言、品質、遮蔽金鑰及開始／停止／重新啟動。加入程序建立時間比對及來源消失不改選；一般測試 171 項通過，格式／Clippy／Debug 與 Release 建置通過。實際 accessibility 檢查確認待命按鈕與預設值；畫面擷取逾時／存取被拒，仍不宣稱視覺或雲端驗收通過。

2026-10-09 Phase 5 第五批：加入字型／字級、描邊、陰影、獨立背景不透明度、最大行數、主螢幕／指定螢幕與移除提示。新增十項像素／原生視窗／幾何測試，一般測試 181 項通過；格式／Clippy／Debug 與 Release 建置通過。五種 DPI 的 GDI／幾何驗證不代表實機畫面通過；跟隨來源視窗、完整來源資訊與視覺／真實服務驗收仍待完成，金鑰安排不變。

2026-10-09 Phase 5 第六批：加入跟隨來源所在螢幕、程序身分原子通知、節流查詢及明確失敗提示。新增六項測試，包括自有原生視窗的實際 monitor 查詢；一般測試 187 項、格式／Clippy／Debug 與 Release 建置通過。未測完整應用程式／混合 DPI 視覺矩陣，未啟動真實雲端或讀取正式金鑰。

2026-10-09 Phase 5 第七批：來源清單顯示圖示，新增串流／訊號／峰值／音量／靜音與未知資料提示，SideChat 預設靠右並可靠左或自由移動。新增七項模型／原生像素／活動與停靠測試，一般測試 194 項、格式／Clippy／Debug 與 Release 建置通過。實機 DPI、視覺、應用程式矩陣與真實服務驗收尚待完成，未將合成圖示／峰值／字幕當成實際擷取。

2026-10-09 Phase 5 第八批：待命 accessibility 檢查發現搜尋欄名稱誤用活動文字，已新增獨立搜尋標籤；字型量測重現並修正供應商提示裁切。新增三項一般測試，197 項、格式／Clippy／Debug 與 Release 建置通過；隔離程序 60 次 GUI 循環，三批 GDI 3／USER 2 均未成長。畫面擷取仍逾時／存取被拒，不宣稱實機視覺通過。實作與驗收狀態見 [驗收表](docs/phase-5-acceptance.md)，金鑰安排不變。

2026-10-09 Phase 5 第九批：修正停止／重新開始／重連的搜尋殘留、字幕更新／歷史淘汰的回看與選取，以及回到即時和主視窗最小化／恢復。新增九項原生合成測試，206 項、格式／Clippy／Debug 與 Release 建置通過；隔離 GUI 資源循環也通過。沒有真實音訊／雲端／正式金鑰或新的畫面預覽；實機與結案驗收仍待完成。

2026-10-09 Phase 6 首批：依使用者批准啟動，加入有界滑動音訊視窗、候選投票／hysteresis、可取消的單一 resolver 要求，以及指紋／歌曲／影集／既有文字契約。新增十四項離線測試；220 項、格式／Clippy／Debug 與 Release 建置通過。正式服務、指紋演算法與 GUI 分流尚待接線，啟動狀態明確回報尚未設定內容服務；沒有把合成候選當成實際歌曲辨識，正式金鑰安排不變。

2026-10-09 Phase 6 第二批：加入真正的 Chromaprint 本機計算、可取消／join 的固定 CPU worker、單一 pending／completed 欄位與目錄 adapter 接口。新增十一項測試，231 項、格式／Clippy／Debug 與 Release 建置通過；50 次壓力提交與 20 次 stop／submit race 通過。實際歌曲目錄、分類／Movie／文字服務與 GUI 分流仍待接線，測試目錄只用明確 mock；沒有正式金鑰或額外雲端傳送。

2026-10-09 Phase 6 第三批：加入獨立有界音訊組窗 worker、LiveResolver owner／分流入口，以及缺口 epoch、舊查詢取消與未讀結果隔離。新增十三項離線測試，244 項、格式／Clippy／Debug 與 Release 建置通過；合成 SpeechSession Partial／Final 可與慢 resolver 並行，50 block 壓力與 20 次 stop／submit race 通過。正式 GUI 組合及實際服務仍待接線，content_resolver_configured 仍為 false，金鑰／真實服務驗收安排不變。

2026-10-09 Phase 6 第四批：加入明確設定的 LRCLIB 唯讀歌詞 adapter、Confirmed metadata 比對、有界 LRC 解析及同 Session 跨歌曲共用的並行／冷卻限制。新增十四項本機 HTTP／解析測試，258 項、格式／Clippy／Debug 與 Release 建置通過；取消、future Drop 與逾時均確認 socket／permit 回收。沒有啟用正式 GUI 查詢、真實 LRCLIB 或正式金鑰；文字本地化／定位／仲裁留至 Phase 7，Phase 6 尚未結案。

2026-10-09 Phase 6 第五批：加入電影／影集 metadata 比對核心，交叉核對片名線索與對白，並檢查已知年份／語言／片長；保留來源信心與模糊候選。新增十六項合成 metadata／MockCatalog 測試，274 項、格式／Clippy／Debug 與 Release 建置通過；取消、future Drop、三秒 timeout 與舊 Session 拒收通過。實際目錄 adapter、hints 分流及 GUI 組合仍待完成；沒有正式金鑰／雲端、實際電影或 Phase 7 驗收。

2026-10-09 使用者指定固定金鑰介面：主視窗加入 OpenAI（ChatGPT）／Gemini 的獨立遮蔽欄位與明確 Windows 安全儲存按鈕，維持本次使用預設、provider 隔離、開始／取用清空及執行中鎖定。新增六項原生 mock 測試，修正字型量測發現的標籤寬度問題；280 項、格式／Clippy／Debug 與 Release 建置通過。沒有讀取／覆寫正式憑證或啟動雲端，結案互動安排不變，未啟動新 Phase。

2026-10-09 Phase 6 第六批：加入已確認歌曲到有界文字查詢的生命週期，同一 binding 去重、最新選擇／回覆、過期清除及取消／Drop 回收。新增十七項測試，含 200 次 clear／publication 競爭與兩項真正 LRCLIB adapter 的本機 HTTP 路徑；297 項、格式／Clippy／Debug 與 Release 建置通過。正式 GUI／內容服務仍待組合，沒有正式金鑰或額外雲端要求，Phase 6 尚未結案、Phase 7 未啟動。

2026-10-09 Phase 6 第七批：將可選 LiveResolver 接入正式 Live drain／select／停止／重連，加入明確 factory 配置、可見服務提示與有界 GUI 內容狀態，保留共用字幕快照。新增十三項測試；310 項一般測試、格式／Clippy／Debug 與 Release 建置通過。初始化／provider panic、20 次新 Session 回收、慢查詢與 STT／翻譯平行、舊更新及未公開服務拒收、原生顯示／停止已驗證。一般啟動仍未配置內容服務；實際 provider、既有文字組合及真實驗收待完成，金鑰安排不變。

2026-10-12 Phase 8 第九批：同一媒體 pin 的 Session ASR Ready 接線完成，新增 20 項一般測試。345 項 Prepared 相關測試與全套 1138 項一般測試通過，既有 15 項 ignored 不變；格式、Clippy、Debug／Release 建置均通過。正式 ASR client、GUI ASR 及播放定位尚未完成，合成 WAVE／mock 結果不代表真實辨識品質驗收。詳細紀錄見 [Phase 8](docs/phase-8.md#第九批驗證)。

2026-10-12 Phase 8 第十批：OpenAI／Gemini Prepared 辨識 HTTP library 完成，新增 35 項一般測試。53 項辨識相關測試及全套 1173 項一般測試通過，既有 15 項 ignored 不變；格式、Clippy、Debug／Release 建置均通過。本機 HTTP 與合成 WAVE 驗證協定、原生時間、取消、真正 worker 回收及同一媒體 pin 的 Ready，不代表有效金鑰或雲端品質驗收。限五語 Auto、GUI ASR 及播放定位仍待接續，Phase 09 尚未具備啟動條件。詳細紀錄見 [Phase 8](docs/phase-8.md#第十批驗證)。

2026-10-13 Phase 8 第十一批：一般 Prepared GUI 接入 OpenAI／Gemini 辨識與翻譯，新增 27 項一般測試。407 項 Prepared 相關測試與全套 1200 項一般測試通過，既有 15 項 ignored 不變；格式、Clippy、Debug／Release 建置均通過。本機 WAVE／mock 與原生隱藏視窗驗證用途揭露、獨立音訊語言、所選金鑰一次解析、取消、Ready 及 pin 回收；真實服務、Auto、多格式與播放定位仍待接續。Phase 09 尚未具備啟動條件。詳細紀錄見 [Phase 8](docs/phase-8.md#第十一批驗證)。

2026-10-13 Phase 8 第十二批：原生 MPEG-1 MP3 解碼與內容／時鐘門檻完成，新增 24 項測試。16 項門檻、6 項 native MP3、409 項 Prepared 與全套 1224 項一般測試通過，既有 15 項 ignored 不變；格式、Clippy、Debug／Release 建置均通過。MP3 的原生時間映射至 Prepared sample clock 尚未完成，前處理／ASR 明確拒收、零音訊交付／服務要求；已有字幕仍優先，沒有放寬時間精度或部分成功。Phase 09 尚未具備條件。詳細紀錄見 [Phase 8](docs/phase-8.md#第十二批驗證)。

2026-10-13 Phase 8 第十三批：有界解碼取樣／原生時間映射核心完成，新增 32 項一般測試。最終全套 1256 項一般測試通過，既有 15 項 ignored 不變；格式、Clippy、Debug／Release 建置均通過。核對長 run 44.1 kHz 取整、真缺口、容量、prune、取消、來源／frame 相位，真正兩秒及十三秒 MP3 經 16／24 kHz DSP 並保持原檔 pin／bytes。一般 MP3 ASR 尚須綁定 chunk／context 映射與結果合併，GUI 仍明確拒收；Phase 09 尚未具備條件。詳細紀錄見 [Phase 8](docs/phase-8.md#第十三批驗證)。

2026-10-13 Phase 8 第十四批：分段持有的不可變映射與原生辨識定稿合併完成，新增 46 項一般測試。最終全套 1302 項一般測試通過，既有 15 項 ignored 不變；格式、Clippy、Debug／Release 建置均通過。核對延遲／倒序、脈絡、重綁、EOF、零輸出 run、來源相位、較早已提交 scope 撤回及八個 guards 上限；真正十三秒 MP3 在兩種 DSP rate 經明確 mock 還原原生時間。一般 MP3 actor／GUI 仍待快照移交接線，沒有開啟音訊上傳，Phase 09 尚未具備條件。詳細紀錄見 [Phase 8](docs/phase-8.md#第十四批驗證)。

## 產品方向

- Windows / Rust
- Process / Application Audio Capture
- 多執行緒、非同步、低延遲
- 英／日／法／德／韓 → 臺灣繁體中文
- 即時翻譯
- 現成字幕／歌詞識別與同步
- 預先翻譯模式
- SideChat / BottomSubtitle / Dual
- SideChat 為預設顯示模式
- 不保存原始音訊
- 不建立永久字幕／翻譯／歌詞 Cache
- 網路取得內容僅保留於目前 Session

2026-10-09 Phase 6 第八批：修正 ResolverSession 的取消／未讀結果過期／共享 metadata 回收，加入 input→output 發布 gate、同步 stop 與 next 喚醒，避免舊結果回填、新結果被擦掉、重發舊時間及重複終態。新增二十項回歸測試，含三組各 200 次跨執行緒競爭；內容辨識相關 105 項與全部 330 項一般測試、格式／Clippy／Debug／Release 建置通過。沒有啟用正式內容服務或使用正式金鑰，實際內容／播放器與 Phase 6 結案驗收仍待完成。

## Phase 08 第二十四批驗證

2026-10-14 真正含 B-frame 的 Main／High 合成 MP4 經實際 B slice、presentation timestamp 倒序及 EOF 核對，沿用只取 AAC 音訊的原有 Prepared 管線。新增 9 項測試；最終 1433 項一般測試、格式、無警告 Clippy、Debug／Release 建置通過，既有 15 項 ignored 不變。7 項 B-frame 聚焦及 2 項純證據測試通過；24 種直接 AAC 原生時間、一微秒 gate、tone／pin／取消、兩個所選服務 mock／16／24 kHz／完整 Ready 與錯誤／中文字幕優先及零要求反例均有核對。這是本機固定合成案例，其他 muxer／設定／一般影片、播放同步與真實雲端仍待驗收。完整證據見 [第二十四批驗證](docs/phase-8.md#第二十四批驗證) 與 [ADR 0057](docs/adr/0057-genuine-b-frame-mp4-audio-evidence.md)。Phase 09 尚未具備條件；金鑰維持結案固定介面手動輸入。

## Phase 08 第二十三批驗證

2026-10-14 有限影音 MP4 擴大為 H.264 Baseline／Main／High，High 核對 8-bit／4:2:0，視訊組合時間表只做有界結構檢查。新增 13 項測試；最終 1424 項一般測試、格式、無警告 Clippy、Debug／Release 建置通過，既有 15 項 ignored 不變。MP4 聚焦 60 項通過，Main／High 共 24 種直接原生時間組合、一微秒 gate、tone／pin／取消，以及明確 mock Ready／錯誤／中文字幕優先及零服務要求反例均核對。一般影片、真實 B frames／其他 muxer、雲端品質與播放仍待驗收，詳見 [第二十三批驗證](docs/phase-8.md#第二十三批驗證) 與 [ADR 0056](docs/adr/0056-ordinary-h264-profiles-and-composition-tables.md)。Phase 09 尚未具備條件；金鑰維持結案固定介面手動輸入。

## Phase 08 第二十二批驗證

2026-10-14 有限的 H.264 Baseline＋AAC-LC 本機 MP4 只取音訊到 RAM Ready。新增 15 項測試，最終 1411 項一般測試、格式、無警告 Clippy、Debug／Release 建置通過，既有 15 項 ignored 不變；MP4 聚焦 47 項通過。真正合成影音核對十二種 native timing、四種率／聲道、兩個所選服務 mock／16／24 kHz、完整 Ready 與取消／錯誤／中文字幕優先及資源清理；無效視訊軌不發服務要求。一般影片、其他 profile／codec／muxer、使用者檔案、雲端與播放仍待驗收，詳見 [第二十二批驗證](docs/phase-8.md#第二十二批驗證) 與 [ADR 0055](docs/adr/0055-bounded-h264-aac-mp4-prepared-input.md)。Phase 09 尚未具備條件；金鑰維持結案固定介面手動輸入。

## Phase 08 第二十一批驗證

2026-10-14 支援已知有界 M4A／MP4 文字、JPEG／PNG 封面及少數數值欄位，不輸出或傳送描述資料。新增 13 項測試，最終 1396 項一般測試、格式、無警告 Clippy、Debug／Release 建置通過，既有 15 項 ignored 不變。真正合成資料已核對全部 PCM／native timestamp 與原檔相同，並經 mock 完成 Ready／取消／清理及服務要求內容檢查。一般標籤變體、真實檔案、雲端品質與播放仍待驗收，詳見 [第二十一批驗證](docs/phase-8.md#第二十一批驗證) 與 [ADR 0054](docs/adr/0054-bounded-mp4-descriptive-tags.md)。Phase 09 尚未具備條件，金鑰仍留至結案固定介面手動輸入。

## Phase 08 第二十批驗證

2026-10-14 有限的 AAC-LC M4A／MP4 擴大到 44.1／48 kHz 單／雙聲道，ADTS 仍限 48 kHz。新增 3 項測試，最終 1383 項一般測試、格式、無警告 Clippy、Debug／Release 建置通過，既有 15 項 ignored 不變。真正合成資料與明確 mock 核對十二種 native timing、原有 gate、16／24 kHz、兩個所選服務、Ready／取消／錯誤／中文優先及資源回收。真實檔案、雲端品質及播放仍待驗收，詳見 [第二十批驗證](docs/phase-8.md#第二十批驗證) 與 [ADR 0053](docs/adr/0053-self-contained-mp4-aac-prepared-input.md)。Phase 09 尚未具備條件，金鑰仍留至結案固定介面手動輸入。

## Phase 08 第十九批驗證

2026-10-14 有限的本機單音軌 AAC-LC M4A／MP4 接入 Prepared Ready，限 48 kHz 單／雙聲道。新增 15 項測試，最終 1380 項一般測試、格式、無警告 Clippy、Debug／Release 建置通過，既有 15 項 ignored 不變。合成檔及明確 mock 已核對容器檔內引用／容量、native 時間、兩個服務與兩種 DSP rate、取消／停止／錯誤／字幕優先及資源回收；真實檔案、雲端品質與播放同步仍待驗收。影片、多音軌、封面等未支援結構仍拒收，詳見 [第十九批驗證](docs/phase-8.md#第十九批驗證) 與 [ADR 0053](docs/adr/0053-self-contained-mp4-aac-prepared-input.md)。Phase 09 尚未具備條件，金鑰仍留至結案固定介面手動輸入。

## Phase 08 第十七批驗證

2026-10-14 AAC-LC／ADTS 48 kHz 雙聲道接入完整 Prepared 到 RAM Ready；其他辨識得到的 AAC 時鐘組合在 PCM／服務要求前拒收。新增 11 項測試，最終 1362 項一般測試、格式、無警告 Clippy、Debug／Release 建置通過，既有 15 項 ignored 不變。真正十三秒合成檔及所選服務 mock 已核對兩種 DSP rate、原生時間、詞彙／翻譯、pin／停止／失敗；不代表真實檔案或雲端品質已驗收。詳見 [第十七批驗證](docs/phase-8.md#第十七批驗證) 與 [ADR 0052](docs/adr/0052-bounded-adts-aac-and-native-clock-gate.md)。Phase 09 尚未具備條件，金鑰仍留至結案固定介面手動輸入。

## Phase 08 第十六批驗證

2026-10-14 全文專名／詞彙分析已接入 Prepared 來源字幕及 ASR 到 Ready 的正式 GUI 路徑。新增 22 項測試；最終全套 1351 項一般測試通過，既有 15 項 ignored 不變，格式、無警告 Clippy、Debug／Release 建置均通過。取消、Session／原文證據、最大全文遍歷、容量、衝突與同服務 client／要求限制已驗證；真實金鑰、雲端接受程度與五語譯名品質仍留至結案固定介面手動測試。Phase 09 尚未具備條件。詳見 [第十六批驗證](docs/phase-8.md#第十六批驗證) 與 [ADR 0051](docs/adr/0051-grounded-prepared-session-glossary.md)。

## Phase 08 第十五批驗證

2026-10-14 原生 producer 映射移交完成。格式、無警告 Clippy、全套 1329 項一般測試（另既有 15 項 ignored）與 Debug／Release 建置通過；producer／MP3 聚焦分別 22／14 項。一般 Prepared／GUI 支援內容核對通過的本機 MPEG-1 MP3，合成資料與明確 mock 已完整驗證 RAM Ready、原生時間、pin／失敗／取消；真實 VBR／媒體、雲端品質及播放仍待驗收。詳見 [Phase 8 第十五批驗證](docs/phase-8.md#第十五批驗證) 與 [ADR 0050](docs/adr/0050-native-prepared-clock-handoff.md)。Phase 09 尚未具備啟動條件；金鑰維持結案固定介面手動輸入。

### Phase 8 第十八批驗證

2026-10-14 AAC-LC／ADTS 擴大至 48 kHz 單／雙聲道，原有一微秒時間 gate 保持；44.1 kHz 直接原生 duration 仍觀測到重疊，維持前置拒收。新增 3 項測試，最終 1365 項一般測試、格式、無警告 Clippy、Debug／Release 建置通過，既有 15 項 ignored 不變。合成／mock 已核對兩個服務與兩種 DSP rate、原生 cue 時間、詞彙／翻譯、停止／失敗／字幕優先及 pin 釋放；真實檔案、雲端及播放同步仍待驗收。詳見 [第十八批驗證](docs/phase-8.md#第十八批驗證) 與 [ADR 0052](docs/adr/0052-bounded-adts-aac-and-native-clock-gate.md)。Phase 09 尚未具備條件，金鑰仍留至結案固定介面手動輸入。

2026-10-16 Phase 8 第三十七批：遠端 PCM16 WAVE 接入既有 DSP／分段／有界辨識與原文合併，完整來源保留到結果 Drop，新增13項一般測試。全套1632項一般測試通過、15項 ignored不變；格式、Clippy、Debug／Release 建置均通過。遠端全文分析／翻譯／Ready 與 URL GUI 尚未接線，Phase 09 尚未具備條件；金鑰維持結案固定遮蔽介面手動輸入。詳見 [Phase 8](docs/phase-8.md#第三十七批驗證)。

2026-10-16 Phase 8 第三十八批：明確遠端 PCM16 WAVE 程式庫入口接通全文摘要／詞彙、三品質翻譯、臺灣本地化與本次 RAM Ready，原來源保留跨服務等待、completion與Ready。本機／遠端共用全文處理，選定文字的 URL 組合先拒收，不改傳音訊。新增13項測試；全套1645項一般測試通過、15項 ignored不變，格式／Clippy／Debug／Release均通過。一般 URL GUI、可靠字幕組合及真實雲端／媒體仍待接續，Phase 09尚未具備條件；金鑰維持結案固定介面手動輸入。見 [Phase 8](docs/phase-8.md#第三十八批驗證)。

2026-10-17 Phase 8 第三十九批：一般準備視窗接通直接 HTTP(S) PCM16 WAVE，沿用所選服務、三品質與 RAM Ready。網址用途／AI端點揭露、固定遮蔽金鑰取用與清除、取消及舊Session隔離已驗證；URL加選定字幕先拒收，保留使用者選擇。淨增18項測試；全套1663項一般測試通過、15項 ignored不變，格式／Clippy／Debug／Release均通過。測試使用合成音訊、本機HTTP及明確AI mock，正式金鑰留至結案手動輸入。可靠字幕組合、其他遠端格式、五語Auto及真實媒體／雲端／播放驗收仍待接續；Phase 09尚未具備條件。見 [第三十九批驗證](docs/phase-8.md#第三十九批驗證) 與 [ADR 0072](docs/adr/0072-explicit-http-wave-prepared-ui.md)。

2026-10-18 Phase 8 第四十批：HTTP(S) PCM16 WAVE可搭配明確本機SRT／WebVTT，只讀來源標頭而不做ASR；中文本機本地化免金鑰，五語來源字幕走同一所選服務的全文摘要／詞彙／三品質翻譯。新增29項測試；最終單獨完整回歸1692項一般測試通過、15項ignored不變，格式／Clippy／Debug／Release均通過。首輪既有B-frame案例逾時及共享測試鎖連帶失敗已如實記錄，未放寬限制；該案例及最終全套重跑均通過。測試仍限合成資料、loopback、明確mock及隱藏視窗，正式金鑰留至結案固定介面手動輸入。遠端內嵌／字幕網址、其他編碼、五語音訊Auto、真實媒體／雲端／播放仍待接續，Phase09尚未具備條件。見 [第四十批驗證](docs/phase-8.md#第四十批驗證) 與 [ADR 0073](docs/adr/0073-remote-wave-local-sidecar-ready.md)。

2026-10-18 Phase 8 第四十一批：獨立HTTPS SRT／WebVTT原文讀取及完整文件解析library完成，單份GET、八MiB原始body、全程序兩份要求／本文／結果持有者，解析共用CPU lane及8192 cues／四MiB本文限制。新增20項測試；全套1712項一般測試通過、15項ignored不變，格式／Clippy／Debug／Release均通過，建置與完整回歸依序執行。公開入口、取消／Drop、配額、錯誤與原文身分／時間均以合成loopback驗證，沒有正式金鑰、使用者媒體或真實雲端。尚未接入Session／Ready／一般視窗，字幕網址在GUI仍拒收；金鑰維持結案固定介面手動輸入，Phase09尚未具備條件。見 [第四十一批驗證](docs/phase-8.md#第四十一批驗證) 與 [ADR 0074](docs/adr/0074-bounded-prepared-https-subtitle-reader.md)。

2026-10-18 Phase 8 第四十二批：明確HTTPS字幕Task接入同Session全文處理與RAM Ready，共用八份來源預算／語言順位，只讀勝出網址，媒體與字幕lease保留跨分析／翻譯／completion／Ready。中文本地化免服務，五語沿用同一翻譯器的全文摘要／詞彙／三品質，不做ASR。新增19項測試；最終1731項一般測試通過、15項ignored不變，格式／零警告Clippy／Debug／Release均通過。首輪八MiB容量fixture五秒逾時、原案單獨通過及改用有限空白行padding的原因均記錄；容量、期限與正式parser未變，定向與最終完整回歸通過。一般controller／GUI仍拒收字幕網址，待輸入與用途揭露接線；真實雲端／媒體／播放未驗收，正式金鑰維持結案固定遮蔽介面手動輸入。Phase09尚未具備條件，符合時會提醒。見 [第四十二批驗證](docs/phase-8.md#第四十二批驗證) 與 [ADR 0075](docs/adr/0075-prepared-https-subtitle-session-ready.md)。

2026-10-18 Phase 8 第四十三批：一般準備視窗新增明確HTTPS SRT／WebVTT模式與用途揭露，透過新的backend入口共用既有worker、全文分析／三品質翻譯、取消及RAM Ready。中文免金鑰，來源字幕只取所選服務的固定遮蔽欄，舊模式仍拒收字幕網址；空白或錯誤不轉ASR。新增22項測試；最終1753項一般測試通過、15項ignored不變，格式／零警告Clippy／Debug／Release均通過。43項HTTPS及64項原生回歸通過；首輪大字型窄視窗文字覆蓋按鈕36px，僅縮短等價用途文案後，原138次量測矩陣全過，未放寬布局斷言。測試仍限合成媒體、loopback、明確mock與隱藏視窗；實際金鑰留至結案固定介面手動輸入。依SPEC第82節核心目標核對，已具備啟動Phase09對齊核心實作的工程條件，尚未啟動；不代表Phase08全面產品驗收結案，Auto、更多格式／相容性及真實媒體／雲端／真人DPI仍另列待辦。見 [第四十三批驗證](docs/phase-8.md#第四十三批驗證)、[下一階段工程條件](docs/phase-8.md#下一階段工程條件) 與 [ADR 0076](docs/adr/0076-explicit-prepared-https-subtitle-ui.md)。
