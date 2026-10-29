# Phase 11 — Hardening

2026-10-28 Phase 11 已依使用者確認的驗收範圍結案。主頁／狀態視窗與手選語言限制已通過；Chrome 以外播放器、剩餘真實雲端斷線／恢復及混音／串燒／歌詞依明確指示免測，保留未實測限制。交付 E3C6C5CD… 版與既有 2166 項工程證據不變，本次另有原生影片停止／錯誤／中文 sidecar 單案通過。詳見 [結案紀錄](phase-11-closeout.md)。目前 SPEC 的最後階段為 Phase 11，未定義下一階段。

下列為歷史批次紀錄；其中待驗收／未結案文字不作目前狀態。

2026-10-27 新人工回報：使用者對 E3C6C5CD… 版測試選項 2，確認曾出現提示、後續仍繼續更新並恢復。記為本次提示後恢復的實測正例；提示種類、來源語言、確切播放時間、停止／保留與原文字詞完整度未另註明，不推論 Incomplete 特定原因、其他語言或長時間穩定性已通過。本次只更新驗收紀錄，未改程式或重跑前一輪 2166 項測試。Phase 11 未結案。

2026-10-27 單次未完整翻譯隔離版已部署：Live 的單次 Incomplete 以明確提示退休該要求，後續新段可繼續；共同連續第三次仍停止，Prepared 原子性及拒絕／憑證／格式守門保持。五項新增、145 項 Phase 11、翻譯相關 59 項一般測試通過，最後獨立完整回歸 2166 一般成功、零失敗、18 ignored，集合重疊；格式／零警告 Clippy／Debug／Release 通過。來源與執行檔雜湊核對後備份／更新，PID 4420（E3C6C5CD…）待命且回應，未自動開始。實際持續翻譯及原文完整度仍待人工，Phase 11 未結案。見 [ADR 0110](adr/0110-isolate-incomplete-live-translation-requests.md)。以下保留歷史紀錄。

2026-10-27 接句缺漏修正版已部署：OpenAI 接近既有四秒上限時優先使用已觀察短停頓，保留音訊處理、硬上限及真正定稿對應。最後獨立完整回歸 2161 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過；四項新增、140 項 Phase 11 與 65 項串流一般測試為重疊集合。已備份舊版、核對來源與執行檔雜湊並開啟 PID 13500（4F59DEC1…），未自動開始。前一版已回報非常顯著改善，但換段／接句仍掉字；新版真實完整度待驗收，Phase 11 未結案。見 [ADR 0109](adr/0109-prefer-observed-pause-before-openai-turn-limit.md)。以下保留歷史紀錄。

2026-10-27 OpenAI 有界字首／消費排程版完成驗證與部署：最終獨立串行回歸 2130 library＋27 整合＝2157 一般成功、零失敗、18 ignored（library 793.84 秒）；格式、零警告 Clippy、Debug、Release 通過。17 項 OpenAI、136 項 Phase 11 與 64 項串流一般測試包含七項新增，集合重疊。備份與雜湊核對後開啟 PID 27672（44E7A3CF…），未自動開始。最新人工為 Gemini 小聲「好很多」、OpenAI 英日文基本正常但原文仍掉字，新版實際長句待驗收；Phase 11 未結案。見 [ADR 0108](adr/0108-openai-bounded-prefix-and-consumer-fairness.md)。以下保留歷史，不以舊 PID 作目前啟動指示。

2026-10-27 新人工正例：使用者對 PID 29572 的正常音量 Chrome 英文／OpenAI／gpt-live-transcribe／英文／Standard 回覆選項 1，確認英文與繁中持續更新、停止正常。只通過這次流程；未提供診斷計數，不據此認定唯一根因、其他語言、逾時復原或效能目標皆通過。Gemini 小聲仍僅稍改善，下一項需其效果不佳當下的完整數值；保留現有程式，Phase 11 未結案。

2026-10-27 OpenAI 提前 delta 的有界提交對應與安全診斷已部署。前次回報 Gemini 小聲稍改善仍不足、OpenAI gpt-live-transcribe 立即 InvalidData，仍待新版人工驗收，不把已重現的提前回覆缺口當作唯一原因。129 項 Phase 11 定向、57 項串流模組一般測試通過，含十項新增；最終獨立串行完整回歸 2123 library＋27 整合＝2150 一般成功、零失敗、18 ignored（library 855.10 秒），格式／零警告 Clippy／Debug／Release 通過。標準版備份／雜湊核對後更新並開啟 PID 29572（92361A48…），未自動開始。原有限復原、模型／金鑰、實際音訊上下文及時鐘界線保持；Gemini 小聲與其他人工矩陣仍未完成，Phase 11 不結案。見 [ADR 0107](adr/0107-openai-early-transcript-correlation.md)。

2026-10-26 PID 31548 的 Auto 人工回覆選項 1、約五至十秒；未分別量測首次／恢復，不宣稱達成一秒工程目標。小聲錄製音源在 Auto 及明選語言皆未出字幕，已重現固定能量門檻對合成小聲訊號不啟動。Gemini Live 專用、前處理 worker 內的有界增益已完成；原始擷取／時鐘／Windows 音量／Prepared／OpenAI 不改。119 定向含五項新增成功，最終獨立串行完整回歸 2113 library＋27 整合＝2140 一般成功、零失敗、18 ignored（library 809.66 秒），格式／零警告 Clippy／Debug／Release 通過。標準版已備份、更新及雜湊核對，PID 5616（5C253A5C…）待命且回應；真實小聲實測與其他人工矩陣仍待完成，Phase 11 未結案。見 [ADR 0106](adr/0106-bounded-live-recognition-input-gain.md)。

2026-10-26 本次裝置切換、重新開始及停止測試回覆正常；Auto 的英文首句（含長停頓後）約三十秒延遲與翻譯逾期仍未通過。已重現預處理端點未送服務收尾的缺口，補上 Auto 專用收尾並保持語言／真正定稿／媒體時間界線。六項新增納入 114 項定向成功；最終獨立串行完整回歸 2108 library＋27 整合＝2135 一般成功、零失敗、18 ignored（library 832.46 秒），格式／零警告 Clippy／Debug／Release 通過。標準版已備份、雜湊核對並開啟 PID 31548（B92E8327…），未自動開始；真實首句及逾期改善與其他人工矩陣仍待驗收，Phase 11 未結案。見 [ADR 0105](adr/0105-auto-client-audio-endpoints.md)。

2026-10-26 最新人工結果：使用者對本機語言備援版的 Chrome 英文來源／Gemini Transcribe Live／Auto／Standard 測試回覆「可用」，記為該組合的基本實測正例。未提供診斷計數，無法確認本次採用服務標記或本機估計；不據此擴大為五語、暫停／停止各步驟或其他服務全面通過。Phase 11 尚有人工驗收與既有媒體間歇逾時追查，未結案；以下保留各批歷史紀錄。

2026-10-26 最新實測已有真正 final 但仍無可核對語言（音訊 348、暫定 5、定稿 1、交付 0、收尾 1）。已加入缺標記時的本機文字語言備援，顯示估計／略過，只放行五語，不寫成服務 context，不另作雲端語言判定。最終獨立完整回歸 2102 library＋27 整合＝2129 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過。標準版已備份、更新並啟動 PID 31984（A8A43962…），只待命；108 項定向與新 11 項均包含於完整集合。實際 Auto 功能與其他人工矩陣未勾選，Phase 11 未結案。見 [ADR 0104](adr/0104-local-text-language-fallback.md) 與 [診斷](phase-11-gemini-diagnostics.md)。

2026-10-26 最新 Auto 實測：收到／送出音訊 915、服務訊息 18、暫定 17、定稿／交付 0，最後 LanguageNotReported。已加入待語言兩秒、每段限一次的 audioStreamEnd，等待真正定稿及五語資訊，不延長總期限或暫停音訊。本機 WebSocket 舊版重現失敗 8.04 秒；定向 97 與最後完整回歸 2091 library＋27 整合＝2118 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過。標準版已備份、更新並啟動 PID 4968（05FF7050…），只待命；實際服務及 Phase 11 仍未驗收。見 [ADR 0103](adr/0103-auto-language-finalization-request.md) 與 [診斷](phase-11-gemini-diagnostics.md)。

2026-10-26 後續同來源比較：手選英文有字幕，Auto 沒有。九秒本機回歸先失敗，再修正 unknown partial 無限續期，新增獨立八秒語言期限與同 Session 純數值進度。定向 93 成功、零失敗、1 ignored；最後獨立串行完整回歸 2087 library＋27 整合＝2114 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過。已備份及更新標準版，新 PID 19960 待命（7B53CB89…）。實際 Auto 根因待新版計數確認，狀態視窗外觀人工項目亦未取得結果，Phase 11 未結案。見 [ADR 0102](adr/0102-bounded-auto-language-wait-and-numeric-progress.md) 與 [診斷](phase-11-gemini-diagnostics.md)。

2026-10-26 英文 Auto 前版實測失敗；已修正 Gemini 自動偵測設定、地區語言標籤及同一句的語言 metadata，新增可獨立調整寬、高的狀態視窗。第二輪獨立完整回歸 2082 library＋27 整合＝2109 一般成功、零失敗、18 ignored，格式／零警告 Clippy／Debug／Release 通過。第一輪 2081 成功、1 個既有 GPT＋Auto fixture 失敗已記錄並校正明選英文，未放寬正式閘門。標準版已備份及更新為 CE00C131…，新版 PID 27164 待命；Chrome 英文 Auto 與視窗人工結果待重試，Native 明選英文正例不當作 Auto 證據，Phase 11 維持未結案。見 [ADR 0101](adr/0101-gemini-auto-language-metadata-and-status-details.md) 與 [診斷](phase-11-gemini-diagnostics.md)。

2026-10-26 語言能力提示版完成標準部署：使用者回覆主視窗已關閉，重新確認主程式及擷取輔助程序皆已退出，舊標準版／備份 0FBFA316… 一致，新候選及更新後標準版 SHA256 6677326A6720AF368F60F0444C08B6B95B481EC133B54D382896F68DFB92052F 一致，9,629,696 bytes。--ui 新 PID 29304 存活且回應，只待命，未開始擷取或讀取金鑰。未改新碼或重跑已通過的定向／品質檢查。下一步由使用者確認 Live Translate／GPT 的「請手選語言」與前置阻擋，若要測 Auto，明確改選 Gemini Transcribe Live；新選項與實際 Auto 仍待回報，不將前版 Native 正例當成 Auto 通過，Phase 11 未結案。

2026-10-25 最新回報：Live Translate 本次暫停／恢復、最新一行、人工回看／回底部及停止保留全部正常，記為單次人工正例。Auto 錯誤源於既有整合不支援該組合但 UI 未前置提示；本批修正語言第一項、開始停用與明確狀態，啟動／診斷均在取 key 前檢查，不改五語限制、服務、偏好或金鑰。原生視窗 59、Phase 11 定向 80 library＋1 config 一般成功（各 1 ignored、有重疊），格式／零警告 Clippy／Debug／Release 通過。候選版及可還原備份已完成，標準版仍待使用者關閉主視窗後更新；前批完整 2099 結果不當作本批重跑。見 [ADR 0100](adr/0100-live-option-capability-preflight.md) 與 [診斷](phase-11-gemini-diagnostics.md)，Phase 11 其他人工矩陣及真實 Auto 仍待驗收。

2026-10-25 最新人工澄清：使用者在先報第一句後停止之後，明確選擇「中文已持續更新，問題已消失」。只將目前 Gemini 3.5 Live Translate 的連續中文顯示記為單次人工正例；沒有取得原始服務回應或確切等待時間，不歸因於模型、完成事件或復原。標準版雜湊仍為 0FBFA316268C7BC67FDBCD710E6E7CFE9E3AC4EB0946C714E7F234F68C85E53C；檢查到 PID 29612 與 14560 均為同一路徑及版本、可回應，未擅自停止或切換來源。此次只更新文件，未修改程式、未重建或重跑舊 2099 項測試。暫停、捲動／回看、停止、OpenAI 及其他既有人工條件仍待分項驗收，Phase 11 未結案。

2026-10-25 後續人工回報：Live Translate 無可見字幕但仍顯示正確模型／正在翻譯，大量字幕最後行不可見。本機先重現兩個缺陷，再修正原生完整快照的 partial 更新及捲動結束／底部跟隨；普通 ASR 重譯保護、真正 Final、Session 隔離、手動清除與容量不變。格式、零警告 Clippy、Debug／Release 與最後獨立串行完整回歸 2072 library＋27 整合＝2099 一般成功、零失敗、18 ignored（library 1213.16 秒）。第一輪 187 失敗與單案 backtrace／量測不隱藏，只校正既有原生媒體測試的啟動預算，停止五秒及產品期限保持，不宣稱產品媒體效能改善。12 檔 UTF-8／7 份文件本機連結檢查通過。標準版備份／雜湊核對後更新至 0FBFA316268C7BC67FDBCD710E6E7CFE9E3AC4EB0946C714E7F234F68C85E53C、9,627,648 bytes，PID 29612 存活且回應，只待命，未擷取或測正式服務；新版仍待人工，Phase 11 未結案，見 [ADR 0099](adr/0099-native-caption-snapshots-and-bottom-follow.md) 及 [診斷](phase-11-gemini-diagnostics.md)。

2026-10-25 最新批次：使用者要求新模型，並回報前版最後一段未顯示。已接入明確 Live 模型偏好、GPT 三品質 delay 與 Gemini Live Translate 有界純譯文輸入；來源／語系／品質及三操作常駐主頁，狀態框只在主頁，開始或診斷先回主頁再通過可見披露閘門。SideChat 重新顯示最新待譯段，BottomSubtitle 保持同段配對／最近中文。新模式無假 ASR、無猜測時間、無語音播放／內容落盤；Auto／High 等能力限制明示，舊預設不自動替換。模型與人工步驟見 [ADR 0098](adr/0098-live-model-profiles-native-captions-and-main-source-controls.md) 及 [手動操作](phase-11-manual.md)。新批實際驗證結果見 [診斷](phase-11-gemini-diagnostics.md)，正式服務與 Phase 11 均未宣告結案。

本批最終工程檢查完成：格式、零警告 Clippy、Debug／獨立 Release 通過；最後 Release 後串行完整回歸 2069 library＋27 整合＝2096 一般成功、零失敗、18 ignored（library 780.44 秒），未重跑 ignored／長測。34 個本批檔案 UTF-8 檢查通過。標準版備份後更新至 SHA256 A0B2441EA056670F4D366491AE29390DB51125EB329CAEFF7BD26700D01FDD2F、9,625,088 bytes；PID 30696 存活且回應，只待命。下一步為使用者以已存 Gemini 金鑰及所選來源明確驗收 Live Translate；OpenAI 亦須另以固定遮蔽 UI 測試。未代按開始、未讀取正式憑證、未以本機成功冒充雲端或外觀通過。

2026-10-25 前批多分鐘 Chrome／英文／Gemini／Standard 已收到持續翻譯、停止與紀錄保留的人工通過，使用者仍覺得慢並要求介面調整。本批服務／金鑰獨立頁及顯式偏好儲存、主頁開始／停止／清除、原位展開還原、真正人工操作才停用自動跟隨已實作。53 項原生視窗（1 ignored）與設定檔 1 項通過；格式、零警告 Clippy、Debug／Release 通過；Release 後獨立串行完整回歸 2057 library＋26 整合＝2083 一般成功、零失敗、18 ignored，library 760.96 秒。標準版備份／核對雜湊後更新並開啟 PID 24580（存活、有主視窗且回應），本批實際介面仍待人工驗收。遠端延遲沒有新量測，其他真實服務／媒體與既有原生間歇逾時待辦保持，Phase 11 未結案。見 [ADR 0097](adr/0097-main-actions-service-settings-and-history-follow.md)。

2026-10-25 最新人工驗收：手動清除／停止保留／重新開始上捲正常，但數分鐘後仍為翻譯要求逾時。已重現單份 Timeout 終止 Live 的路徑，改為依完整 job 身分捨棄該次未完成要求、警告與累計獨立於字幕，後續新段繼續；連續三次期限失敗仍採既有共用有限復原。原三秒期限、過期合併槽的接收期限及修訂水位、完成核對與 Prepared 失敗契約保持。格式、零警告 Clippy、Debug／獨立 Release 通過；Release 後獨立串行完整回歸 2054 library＋26 整合＝2080 一般成功、零失敗、18 ignored，library 734.54 秒。標準版備份／雜湊核對後更新並開啟 PID 24216（存活、有主視窗且回應），正式服務多分鐘穩定性仍待驗收，不宣告 Phase 11 結案。見 [ADR 0096](adr/0096-live-translation-deadline-isolation.md)。

2026-10-25 使用者回報前版基本可用，但要求不再自動整批清除字幕，另設清除按鈕。依此更新顯示政策：本次開啟期間最多 64 列 RAM 紀錄跨後端停止／錯誤／重連保留，新段上捲；舊身分或已撤回來源明示先前字幕，不再送回處理／定位。固定清除按鈕及選單只清顯示，保留身分／水位防回填，不停止辨識，搜尋／手動選取跨連線保持。UI 86 項、原生視窗 51 項（1 ignored）、Live 16 項、最後 Phase 11 一般 53 項（1 ignored）通過，有重疊；舊清除預期與短視窗失敗及修正均見診斷。格式、零警告 Clippy／Debug／Release 通過；最後 Release 後獨立串行完整回歸 2047 library＋26 整合＝2073 一般成功、零失敗、18 ignored，library 816.59 秒。標準版備份／雜湊核對後部署，新 PID 28940 存活、有主視窗且回應。舊程式已關閉，尚未驗收正式服務的新按鈕與跨連線畫面，Phase 11 未結案。見 [ADR 0095](adr/0095-user-owned-bounded-subtitle-display-history.md)。

2026-10-25 本批接續 PID 27032 的暫停回報：使用者狀態仍「正在翻譯」，不能把消失直接當成重連。修正新尾段無中文時取空字串的顯示缺口，兩視窗維持同快照最近有中文的段落，手動原文歷史／搜尋不刪除；另補 Gemini 音訊中斷排空後一次 audioStreamEnd，讓真正 Final 可在無新音訊時到達、已收到文字繼續翻譯，恢復使用同一連線／Session。六項新增回歸含原生 pending 尾段及選取／捲動保護；最後 Phase 11 一般 48 項、完整原生視窗 49 項通過（各 1 既有 ignored）、串流 26 項通過（2 既有 ignored）、UI mailbox 30 項與 Presentation 4 項通過，有重疊。格式、零警告 Clippy（52.66 秒）、Debug／獨立 Release 通過；Release 後獨立串行完整回歸 2042 library＋26 整合＝2068 一般成功、零失敗、18 ignored，library 1035.87 秒。標準版備份與雜湊核對後部署，新 PID 22588 存活、有主視窗且回應。使用者已回覆關閉舊視窗，未自動擷取或使用正式金鑰，暫停及長句仍待新版人工驗收，Phase 11 未結案。見 [ADR 0094](adr/0094-gemini-idle-input-drain-and-caption-retention.md) 與診斷紀錄。

2026-10-25 最新人工回報：PID 5192 的 Chrome／英文／Gemini／Standard 持續播放已有中文、停止正常；這一項流程通過，快速長句仍有同段譯文替換過快的問題。接續保留舊暫定中文直到新修訂完整輸出，並依指定調整右側自由移動、半螢幕高／三分之一寬、系統字體及可切換的窄字幕／設定區。內容辨識提示澄清為獨立歌曲／影片服務；不自動啟用它。42 項 Phase 11、48 項原生視窗、22 項翻譯核心、16 項 Live、3 項內容提示通過（有重疊），fmt／零警告 Clippy／Debug／Release 通過。第一輪完整回歸影片 worker 五秒期限失敗，186 項因共用鎖 poisoned 連帶失敗；單獨重跑一敗一成功，未改期限或媒體程式。最後 Release 後獨立串行完整回歸 2036 library＋26 整合＝2062 一般測試成功、零失敗、18 ignored，library 756.02 秒；原間歇逾時根因仍待追查，不因串行通過視為已修復。標準版備份、新舊雜湊核對及部署完成，PID 27032 存活、主視窗存在且回應，未自動擷取或讀取正式金鑰。新外觀、長句及其他人工條件仍待驗收，Phase 11 未結案。見 [ADR 0093](adr/0093-readable-draft-replacement-and-compact-side-defaults.md) 與 [完整診斷](phase-11-gemini-diagnostics.md)。

2026-10-25 接續人工回報：Gemini 已顯示正在翻譯，但要暫停 YouTube 才有中文，尚未通過即時翻譯驗收。本機重現每 40 ms 的相容 Partial 更新取消 240 ms 翻譯要求的排程缺陷；Live 入口改為有界合併最新更新，先讓已啟動的相容翻譯串流完成，Final／回退／停止仍立即取消，原期限與精確修訂核對不變。16 項 Live、22 項核心與 38 項 Phase 11 一般定向通過（有重疊）；格式、零警告 Clippy、Debug 與獨立 Release 通過。Release 後獨立完整回歸 2032 library＋26 整合＝2058 一般測試成功、零失敗、18 既有 ignored，library 560.21 秒；未重跑 ignored 或長測。舊版已關閉並備份，核對新舊雜湊後更新標準版，新 PID 5192 存活、有主視窗且回應。沒有自動擷取、使用正式金鑰或驗證真實延遲；待使用者保持 Chrome 播放驗收中文及停止，Phase 11 未結案。見 [ADR 0092](adr/0092-live-partial-translation-coalescing.md) 與 [診斷紀錄](phase-11-gemini-diagnostics.md)。

2026-10-25 連續轉錄與分框批次完成工程驗證及部署。Gemini 連續送出音訊、由服務斷句，不等前段 Final 而積壓 PCM；無精確擷取關係時時間保持 None，不作媒體定位。系統狀態／錯誤與字幕分框，保留原搜尋／捲動／Session 清理，短視窗可縮小狀態區。47 項原生視窗、34 項 Phase 11、23 項串流一般定向通過（有重疊），格式／零警告 Clippy／Debug／Release 通過；最後 Release 後獨立完整回歸 2028 library＋26 整合＝2054 一般測試成功、零失敗、18 ignored，library 577.55 秒。第一輪三項舊狀態框假設失敗已保留於診斷紀錄並修正；未重跑 ignored 或長測。標準版已核對備份與雜湊、更新並啟動 PID 3248，程序回應、主視窗存在。唯讀畫面擷取兩次逾時，尚無目視或真實中文驗收；沒有代按開始或讀取金鑰。設計見 ADR 0091，Phase 11 未結案。

2026-10-24 使用者回報數值版仍出現「辨識佇列已滿」，英文原文被錯誤後通用「等待字幕」取代。本批重現最終轉錄到達卻因缺少 turnComplete 卡住的缺陷，移除多餘屏障，保留最終結果關聯；新增具名佇列容量及真實狀態空畫面。三十三項 Phase 11 定向、Windows 控制項重連／錯誤／停止回歸、格式／零警告 Clippy／Debug／Release 通過；Release 後獨立完整回歸 2026 library＋26 整合＝2052 一般測試通過，零失敗、18 ignored，library 504.98 秒。本批沒有重跑 ignored 或原生長測。標準版已備份、核對雜湊、更新並開啟 PID 23016，程序存活且回應；不自動擷取或使用正式金鑰。實際 Chrome／Gemini／臺灣繁中仍待使用者驗收，Phase 11 未結案。

2026-10-24 零位置修正後的 Chrome 人工結果仍為 Receive／DiscontinuityFlag，曾短暫出現英文原文後消失；不能標記辨識／翻譯通過。本批自有合成音源的一千包及九次播放／靜音切換原生回歸通過，未重現剩餘故障；新增失敗時數值摘要與序號／位置缺口優先分類，保留原拒收及復原預算。十四項語音邊界、三十項 Phase 11 一般定向測試、fmt／零警告 Clippy／Debug／Release 已通過；Release 後獨立完整回歸 2023 library＋26 整合＝2049 項一般測試成功、零失敗、18 ignored，496.48 秒。原靜音時鐘回歸另行通過（1.28 秒），未重跑舊三十分鐘／三百次長測。標準版已核對備份與新舊雜湊、更新並開啟 PID 30752，程序存活且回應；不自動擷取或呼叫正式服務。詳見 [診斷紀錄](phase-11-gemini-diagnostics.md#零位置修正後仍中斷數值連續性診斷2026-10-24)，Phase 11 未結案。

2026-10-24 零位置時鐘批次最終結果：格式／零警告 Clippy／Debug／Release、六項純時鐘、六項 packet 與十七項有界擷取定向回歸通過；Release 後獨立完整回歸 2022 library＋26 整合＝2048 一般測試通過、零失敗、17 ignored，library 514.57 秒。兩項自有來源原生案例已另外明確執行通過，零位置誤判的 99 個後續缺口降為零，音源隔離／程序樹保持。確認舊程式已關閉後，標準執行檔備份及新雜湊核對完成，新版 --ui 已啟動（PID 1532）。沒有取用正式金鑰或擷取 Chrome；實際 Chrome／Gemini 字幕與 Phase 11 仍待人工驗收。

2026-10-24 零位置時鐘修正：新版安全訊息定位至 Receive／DiscontinuityFlag；原生自有來源重現 QPC 前進而裝置位置持續為零，造成 100 包中的 99 個後續假缺口。新增 Process Loopback 專用、由實際 frame 數與原生 QPC 共同驗證的擷取相對時鐘；原生同一回歸修正後通過，後續旗標／位置缺口均為零，既有音源隔離／程序樹也通過。完整工程檢查與部署待追加；不改 STT 缺口保護或把擷取時間當媒體位置，Chrome 與 Phase 11 仍待實際驗收。見 [ADR 0090](adr/0090-process-loopback-zero-position-clock.md)。

2026-10-24 音訊階段診斷批次工程驗證完成：格式／零警告 Clippy／Debug／Release 及 Release 後獨立完整回歸通過，2016 library＋26 整合＝2042 一般測試，零失敗、16 ignored（library 535.57 秒）。新 Release 與舊版備份已核對雜湊；使用者回報已關閉後，再確認無同名程序，標準程式已更新並重新啟動（PID 3088），雜湊與獨立 Release 相同。新版增加具體錯誤階段與原因，不放寬期限或有限復原預算，不代表 Chrome 根因或實際辨識／翻譯已修復；Phase 11 維持未結案。

2026-10-24 使用者補充：實際選擇 Chrome 播放英文內容，並非誤選測試來源。音訊過期／不連續仍需定位；本批新增安全的音訊階段與原因診斷，保持原期限、擷取／定位界線及共用有限復原預算。工程驗證與更新後人工結果待追加，不能宣稱 Chrome 已修復或 Phase 11 結案。

2026-10-24 接續實際辨識回報：來源 PID 9772（目前查得 Chrome）發生音訊過期或不連續，與安排的獨立英文 TTS PID 不同，待確認實際來源及重測。英文無音訊初始化通過保持；實際辨識／翻譯未通過。本次 18 項相關本機回歸成功，沒有修改程式或放寬保護；已重開一般程式與獨立測試視窗供使用者明確操作。詳見 [診斷紀錄](phase-11-gemini-diagnostics.md#初始化成功後的音訊不連續回報2026-10-24)。

2026-10-24 最新人工結果：使用者明確選英文，回報 Gemini 固定模型資訊可讀且列有串流方法、串流初始化成功；未擷取或傳送音訊。本機金鑰、模型查詢與英文初始化已分別確認，實際辨識／翻譯及其他語言仍待驗收，Phase 11 未結案。當目標準版 SHA-256 為 `352917B69A23BADB9D92106B93536D910F75F68095BA207FA7D11682DAEA6B78`，最近重新啟動 PID 27532。此前不同設定的無效資料結果仍保留為歷史；不能將成功單獨歸因於預付、語言或 Binary JSON 修正。本次僅更新人工紀錄，沒有程式變更或新一輪自動測試。

Binary JSON 批次已完成工程驗證：fmt／零警告 Clippy／Debug／Release、串流模組 20 項一般測試及最終完整回歸通過；完整一般測試 2038 項、零失敗、16 ignored（library 531.73 秒）。標準執行檔已備份及以雜湊核對後更新，新版 --ui 待命 PID 21104。下一步仍由使用者選回原語言、留空使用已存 Gemini 金鑰並按無音訊診斷；真實根因／初始化／辨識／翻譯尚未確認，不宣稱 Phase 11 結案。

2026-10-24 預付後的新回報：模型資訊可讀，初始化變為 SetupReceive／InvalidData，不含前次 1011。已發現並修正 Gemini 接收端一律拒收 Binary JSON 的缺陷，沿用原驗證與限制；尚未觀察此次真實原始回覆，不宣稱已確認根因或服務已通過。當前批次工程檢查與新版資訊見 [診斷紀錄](phase-11-gemini-diagnostics.md#預付後-invaliddata-與-gemini-binary-json-接收修正)，Phase 11 維持未結案。

2026-10-24 最新澄清：使用者已自行完成預付，因此瀏覽器觀察到的 Prepay／正值餘額是帳戶變更後的新狀態，不能回推先前 1011 發生時也已有餘額或排除計費原因。代理未操作付款、方案或上限，不保存私人識別碼／金額。先維持原失敗嘗試的語言與其他設定，用既有無音訊診斷重試一次，暫緩先前改 English 的建議，以免同時改動兩個條件。初始化及 Phase 11 仍待實際結果，未改程式或模型。

2026-10-24 帳戶補充：使用者回報本模型有可用額度，但頁面要求一個或多個計費帳戶切換 Prepay 以避免服務中斷。已對照官方遷移文件，仍待核對該帳戶是否支付本專案、截止日期及實際可用狀態，不能直接宣稱 1011 根因已確認。付款／帳戶遷移須使用者手動決定與處理；未修改程式、金鑰或計費設定，Phase 11 維持未結案。

來源語言補充：使用者回覆日文／法文／德文／韓文群組，確切語言未指定。下一步請使用者查看同一 Google 專案的 `gemini-3.5-transcribe-live` 實際額度；代理無法從公開頁取得登入後的私人額度，尚未確認是額度、權限、服務端或設定交互問題。

最新人工結果：2026-10-24 無音訊診斷的固定模型資訊可讀且列有串流方法，但初始化仍於 SetupReceive 收到 WebSocket 1011。只將模型查詢新增標記通過，實際辨識／翻譯及 Phase 11 維持未結案；等待確認本次來源語言，必要時再由使用者確認專案模型額度。本次僅更新人工記錄與官方設定核對，不重建或宣稱已修復雲端服務。

2026-10-24 使用者確認 Gemini 本機已存且可讀，但接續實際初始化仍以 1011 失敗。已接續實作固定模型資訊與串流初始化分開回報的明確無音訊診斷，不重存金鑰、不改模型或服務、不開始音訊擷取；實際 Gemini 及整個 Phase 11 仍未結案。完整狀態與驗證見 [診斷紀錄](phase-11-gemini-diagnostics.md)。

無音訊診斷批次最終 fmt／零警告 Clippy／Debug／Release、十項新增測試與十一項固定金鑰 UI 定向測試通過；Release 後獨立完整回歸 2010 library＋26 整合＝2036 項一般測試成功、零失敗、16 ignored，library 514.33 秒。舊版已備份，標準執行檔更新且雜湊核對相同，新版 --ui 已開啟待命（PID 29668）。本批沒有執行真實金鑰診斷或擷取／時鐘改動，下一步由使用者明確按「Gemini 診斷（連線、不傳音訊）」回報兩階段結果。

2026-10-23 使用者批准啟動，要求持續至本階段結束，除非需要手動排除或指示。起點為 Phase 10：2006 項一般測試通過，15 項 ignored；fmt／零警告 Clippy／Debug／Release 均通過。Windows 19045 基準與既有架構保持，不讀正式金鑰、不擷取使用者播放內容、不修改系統預設音訊裝置或服務。

**狀態：自動工程驗證完成，停於必須人工操作的驗收，尚未結案。** 未完成的播放器／裝置／服務條件不因 mock、歷史測試或程序樹 fixture 通過而勾選。

## 範圍與方法

| SPEC 項目 | 工程驗證 | 仍需分開處理 |
| --- | --- | --- |
| 長時間 Live | 既有自有靜音程序的 1800 秒 native 擷取；新增 300 秒合成 Live 管線與歷史容量／資源檢查 | 真正服務、媒體與使用者端長時間播放 |
| 多次切換 Source | 原生 300 次重啟；A／B／子程序切換；新增 100 次合成 Session 切換 | 任意播放器／真實來源切換矩陣 |
| Network disconnect／reconnect | 兩個明確本機協定 mock，各 16 次 TCP／WebSocket 中斷；共用有限復原預算 | 真正供應商與使用者網路條件 |
| AI timeout | 本機 silent peer 的真正 heartbeat 期限；既有送出、翻譯及 resolver 逾時回歸 | 真實服務逾時與帳號限流 |
| Process exit | 自有來源退出與 helper 終止，原生 stop／新 Session／來源存活檢查 | 使用者實際播放程式 |
| Audio device change | 真正 packet adapter 的 acquire／release device-invalidated 具型別故障注入；雙端點自有串流 | 實體拔插、預設裝置變更或服務中斷，不能用錯誤注入冒充 |
| Prepared cancellation | 本機／遠端工作、舊 completion、分析與翻譯取消、Ready 釋放回歸 | 真實服務／媒體 |
| Temp cleanup | 產品媒體路徑目前不建立暫存內容；合成 fixture 驗證取消／失敗／Ready 清除後僅剩 caller 輸入，TempDir 能實際刪除 | 不新增沒有實際用途的暫存 cleaner 或 cache |
| Mixed audio／Medley | 自有合成音隔離；120 次明確 mock Mixed 邊界，要求新歌曲重新投票；既有歧義／新鮮度回歸 | 真實混音／串燒的辨識準確度與歌詞服務 |
| Browser multi-process audio | 既有根／既有子／新增子／根退出的原生程序樹 fixture | 真正 Chrome／Edge 播放正例與暫停對照；fixture 不能代替相容性驗收 |

實機測試僅操作本次建立的靜音或低音量合成測試音來源，沒有錄製／保存音訊、字幕或私有本文。原生長測與編譯／其他重負載檢查分開，Cargo 串行，最終完整回歸於 Release 後獨立執行。正式金鑰仍只由使用者在固定遮蔽介面手動輸入。

## 復原判定

將原 Live 迴圈重複的錯誤分類／重試預算抽為內部 `Recovery`。保持原有最多三次、500／1000／2000 ms 加 0～250 ms 有界 jitter、健康運行三十秒才重設；短時間 SessionExpired 也必須耗用預算，健康連線的預期輪替保持單獨路徑。語音與翻譯交替失敗不能各自取得新預算。

原迴圈仍先停止舊 socket、擷取、翻譯、定位與內容工作，再判定重連；退避仍受原停止訊號取消。選定 PID／建立時間與 provider 不由此模組變更。憑證、TLS、來源退出、裝置失效與無效回應維持具型別終止，不偷偷重試另一來源／服務。未新增 provider API、網路要求、執行緒、queue、內容保存或全域鎖。

來源身分檢查保留父程序的 owned query／synchronize handle，跨 helper 啟動及整個 Session，並重複核對來源存活。審查時核對 [Microsoft 的程序 ID 生命週期說明](https://devblogs.microsoft.com/oldnewthing/20110107-00/?p=11803)：程序物件在仍有 handle 時保留，其 ID 不會交給另一個物件；不因猜測 PID 重用而改成新來源或新增 IPC 入口。這不取代真實程序退出測試。

新增本機 close frame 測試核對原有分類：正常關閉／傳輸中斷、權限、限流與協定錯誤；私有 close reason 不進入使用者錯誤。長 heartbeat 測試只有明確指定才執行，不使用真實端點或憑證。

新增的重複實際本機斷線測試首次發現 `ResetWithoutClosingHandshake` 被通用 Protocol 分支誤分為 InvalidData，導致真正傳輸斷線不重連。已只將此具名 EOF／未完成關閉交握案例改為 Network，沿用同來源／服務的三次有限重試；其他 malformed opcode／close sequence 仍為終止型 InvalidData。沒有放寬測試斷言或將所有協定錯誤改成重試。修正後十二項新增一般測試在 Debug（0.34 秒）及 Release（0.48 秒）全部通過；長 heartbeat 額外明確執行，完整回歸另行記錄。

## 重現命令

```powershell
cargo test --release --locked --offline phase11 -- --nocapture
cargo test --release --locked --offline ai::streaming::tests::phase11_silent_local_peers_hit_real_heartbeat_deadlines_and_release_workers -- --ignored --exact --nocapture
cargo run --release --locked --offline --example hardening_probe
```

`hardening_probe` 固定運行三百秒，使用真正 CaptureSession 有界 PCM／正規化、SpeechSession、LiveTranslation、時間軸及 UI bridge；辨識／翻譯與繪圖 receipt 明確 mock，不是 native 擷取、真正供應商或實際螢幕可見時間。十秒暖機後每十五秒採程序數值，private commit 成長最多八 MiB、handle 最多兩個暫時差值作回歸警報，不是普遍產品記憶體上限。歷史必須實際達到 64 筆並輪替，至少 128 筆 Final 輸入；另執行一百次新 Session、輪流使用三品質，逐次確認停止／清空／操作與 worker 歸零。不提供縮短時長後宣稱長測成功的參數。

探測來源的第一輪未提供時間資訊，觸發既有每塊 discontinuity／STT 拒收，立即失敗且未產生長測成功報告。已只修正開發 fixture：提供明示的合成 sample clock 並在輸出標記，不以它作 native clock、媒體位置或定位證據；產品對未知／不連續音訊的檢查保持。修正後完整五分鐘另行執行，不將第一次失敗改記為通過。

原生測試使用精確篩選，不執行全部 ignored，避免誤啟動真正 TLS／無效金鑰測試或需要外部操作的 manual fixture：

```powershell
cargo test --release --locked --offline win32::soak_tests::native_process_capture_thirty_minute_soak -- --ignored --exact --nocapture
cargo test --release --locked --offline win32::soak_tests::native_process_capture_restart_batches_do_not_accumulate_handles -- --ignored --exact --nocapture
cargo test --release --locked --offline win32::soak_tests::native_process_capture_helper_crash_requires_new_session -- --ignored --exact --nocapture
cargo test --release --locked --offline win32::tests::native_process_capture_stops_and_detects_source_exit -- --ignored --exact --nocapture
cargo test --release --locked --offline win32::tests::native_process_capture_isolates_tones_and_includes_audio_children -- --ignored --exact --nocapture
cargo test --release --locked --offline win32::tests::native_process_capture_spans_two_render_endpoints -- --ignored --exact --nocapture
```

雙端點測試需要兩個已啟用的本機播放端點，只操作 fixture 串流；不自行啟用／停用裝置。所有容量、年齡、handle 與 worker 門檻沿用，失敗應查明原因，不能放寬到通過。

## 驗證紀錄

### 原生三十分鐘長測

本機重新確認 Windows 10 Education 19045.6456、八個邏輯處理器，rustc 1.98.1（48a229cea，2026-09-01）。指定的 Release native soak 測試正常退出，測試耗時 1800.33 秒，擷取區間 1800 秒、暖機後 29 次取樣。該測試使用當批起點的 Release binary；本批後續調整應用層復原判定、STT 傳輸錯誤分類並新增明確測試／probe，沒有修改擷取、IPC、正規化或 capture 資源路徑。最終新版本另跑相關原生短測與完整回歸，不將 capture 長測宣稱為真正雲端 Live／GUI 長測。

| 指標 | 暖機基準 → 最終／最大 |
| --- | --- |
| 父程序 private bytes | 2,211,840 → 2,269,184 |
| helper private bytes | 5,095,424 → 4,980,736 |
| 父程序 handle／thread | 233／7 → 233／4 |
| helper handle／thread | 313／10 → 309／5 |
| 交付 frames | 86,353,920；不由此推論所有 native frames 均未淘汰 |
| queue 觀測最大 | 2 格，上限 12 |
| 呼叫起點最大年齡 | 151.1207 ms，產品原門檻 250 ms 保持 |
| 回傳後最大觀測年齡 | 151.1232 ms，另列排程差異 |
| 分鐘區間單核心 CPU 最大 | 父程序 1.878%、helper 2.110%；各自最大，不相加成同時峰值 |
| 停止 | 36.0013 ms；normalized queue 清空、無殘留交付、helper handle 已 signaled |

既有 private growth 八 MiB、handle growth 四個、thread growth 兩個警報均通過，沒有提高門檻。以上為自有靜音程序與 capture／IPC／正規化的本機觀測；沒有量測整個 OS 音訊服務或真正服務、影片、UI 可見時間與語意品質，也不聲稱已直接量測所有 COM 參照數。

### 最終新版本原生短測

十二項新一般測試、零警告 Clippy、Debug／Release 及三個 Release example 建置通過。三批重啟測試額外強制 DEBUG 到數值限定的 sink，不寫日誌檔；每個 Session 都斷言 helper query probe 與 timings 真正存在，然後以原本兩個暫時 handle 容許值核對釋放，沒有只測 default INFO 路徑。

| 精確指定的 Release 原生短測 | 實際結果 |
| --- | --- |
| DEBUG 300 次重啟，另有三次暖機 | baseline／100／200／300 次皆 224 handles；46.00 秒 |
| helper crash | WorkerExited、來源仍存活、重新啟動 Session 隔離；0.39 秒 |
| 來源退出／三次正常啟停 | 通過；0.81 秒 |
| A／B 單獨及同時／交換來源／子程序／根退出 | 目標 0.031250、非目標 0.000000、靜音 peak 0.000031；2.51 秒 |
| 同一來源兩個播放端點、動態新增／移除第一串流 | 兩個目標與移動後剩餘目標皆 0.031250，非目標 0.000000；1.59 秒 |

上述沒有更改系統音量、預設裝置或服務，沒有放寬振幅／handle／freshness 門檻，也不是 Chrome／Edge、實體拔插或真正音樂辨識實證。

另外精確執行新增的本機長 heartbeat ignored 測試，兩種協定 mock 分別在 30,014 ms（OpenAI 路由）與 30,005 ms（Gemini 路由）回報具型別 Timeout，無殘留 transcript，原預處理 worker 已 join，整項 60.03 秒通過。這不是向真正供應商連線或以真實金鑰製造逾時。

### 五分鐘明確 mock 管線與 Session 切換

修正 fixture 連續時鐘後，Release probe 正常退出。長測實際 300,019 ms，採二十筆資源樣本，192 筆 Final ASR 輸入，歷史約一百秒達到 64 筆並持續輪替。交付合成捕捉計數 9,209,280 frames，淘汰／過期皆零；停止未完成 Final 為零、provider operation／permit／queue current 及捕捉 worker 全歸零，最後 UI bridge 的 Session／snapshot 均清空。

| 指標 | 第一筆暖機後 → 最後一筆／最大 |
| --- | --- |
| private commit | 1,449,984 → 1,527,808 bytes；最大 1,527,808 |
| working set | 4,993,024 → 5,144,576 bytes |
| handles | 二十筆全部 78；停止後切換基準 76 |
| 分段單核心 CPU 觀測最大 | 4167 ppm，即 0.4167%；首筆空值，短區間零樣本不等於零成本 |
| 歷史容量 | 峰值 64，超量舊項目正常退休 |

其後依序完成一百個新 Session，輪流三品質；每個至少有一筆真正套用至時間軸的 Final 譯文才結束，逐次取消／停止／清空並檢查所有權。每二十次的 handle 都為 76，與切換基準相同；private commit 基準 913,408 bytes，二十次後 843,776、四十至一百次皆 851,968，沒有提高原八 MiB／兩 handle 警報。切換合計 Final ASR 輸入 183，不是完成翻譯數；短 Session 關閉時允許取消後續未完成工作。

這是 std timer 驅動的合成 sample clock，實際排程用 nominal frame 時長與牆鐘不同。不能用 frames／sample rate 推論 native 媒體播放時間，也不能把 mock receipt 當成原生畫面或真實服務效能。首輪 fixture 拒收與上述修正後完整結果分開保留，沒有降低連續性、容量、年齡或資源門檻。

### 最終完整回歸與審查

全部 Release 建置、指定原生／heartbeat 測試及五分鐘 probe 結束後，獨立執行 `cargo test --locked --offline --quiet`：1991 項 library 通過、零失敗、16 項 ignored，耗時 534.47 秒；另 12 項設定、4 項來源探索及 10 項啟動測試全數通過，合計 2017 項一般測試。新增十二項一般測試並整合移除一項舊重試測試，較 Phase 10 淨增十一項；新增一項需實際等待的 ignored heartbeat，已精確指定執行通過。共七項指定 ignored 原生／heartbeat 案例已另行通過，不表示其餘 ignored 或真實服務測試也已執行。

`cargo fmt --check`、`cargo clippy --locked --offline --all-targets --all-features -- -D warnings`、Debug／Release 主程式及三個 Release example 建置全部通過。最後 probe fixture 時鐘修正後，格式、Clippy 與 Release probe 亦重新通過。後續僅同步本紀錄與操作文件。

審查確認本批正式程式變更限於內部共用復原策略整理與具名 WebSocket EOF 分類修正；沒有新增 unsafe、相依、內容持久化、金鑰讀取、provider fallback、capture callback I/O、無界 queue 或執行緒。原有擷取／IPC／正規化、單一時間軸、精確／模糊採用、Session 與定位時鐘界線保持。停止、配額釋放、舊 Session 隔離、Prepared 清理及裝置故障路徑均包含於通過的相關測試；實體與服務驗收仍須人工。門檻見 [Phase 11 工程與人工驗收清單](phase-11-gate.md)。

## 人工邊界

歷史 Chrome 播放正例低於固定振幅門檻，原因尚未釐清，見 [Phase 2 應用程式條件](phase-2.md#應用程式與外部條件)。獨立設定檔啟動曾遭自動核准政策阻擋，本批未繞過或重試該歷史命令。Chrome／Edge 安裝存在不代表相容性通過；程序樹 mock 也不是瀏覽器實證。

自動檢查已完成，目前需使用者建立可明確確認全部音訊都是測試內容的瀏覽器來源，完成播放／暫停正反對照，以及實體音訊裝置操作。結案真實服務時，從固定 OpenAI／Gemini 金鑰欄位手動輸入後選定來源，核對揭露再啟動；不在聊天或一般檔案輸入金鑰。本文件不提前宣布 Phase 11 或全面產品驗收完成，也不虛構 Phase 12。

具體步驟見 [手動驗收操作](phase-11-manual.md)。開發用 Windows TTS 合成語音來源已通過 PowerShell 語法檢查；本機 Windows PowerShell 5.1 的 System.Speech 可載入並列出兩個已啟用英文 voice。此查詢沒有發聲、開啟視窗或讀金鑰，不等於語音／GUI／雲端端到端測試已通過。

交接前已另外啟動 Release `--ui` 與合成語音待命視窗；兩程序存活、有主視窗 handle 且 Responding。未自動按 Start、輸入金鑰、啟動擷取／供應商或操作使用者播放。操作文件的編輯器開啟要求回報 queued，不把它當成使用者已閱讀。此時依使用者允許的人工例外停止自動作業，保留人工清單未勾選。

## 人工回報後的 Gemini 診斷批次

使用者提供「按開始後立即中斷、尚未出現字幕」的真實失敗畫面。前述自動工程結果不取代這次服務驗收。已追加安全連線階段與 HTTP／WebSocket 數字代碼，以及 rustls 錯誤被 I/O 包裝後的分類修正；保留真實根因未確認的狀態。追加變更、官方文件查核、本機測試及新版路徑見 [Gemini 安全診斷](phase-11-gemini-diagnostics.md)。

追加批次格式、零警告 Clippy、Debug／獨立 Release、十七項 Phase 11 一般測試及 Release 後完整回歸通過。新完整回歸為 1996 library＋26 整合＝2022 項一般測試、零失敗、16 項 ignored，library 524.36 秒；前批長測不重複冒充。已在舊程序關閉後備份並更新標準執行檔，雜湊核對相符，開啟新待命視窗；真實 Gemini 重試仍由使用者固定介面輸入金鑰，未收到新結果前維持未結案。

後續已收到初始化 1011 與儲存後欄位空白的回報。本批只查指定 WinCred 項目中繼資料確認存在，補上各服務「檢查本機」、寫入後讀回核對與 1011 ServiceInternal 訊息，不把本機存在當成雲端權限。二十一項 Phase 11 一般測試、格式／零警告 Clippy／Debug／Release 通過；最新完整回歸 2000 library＋26 整合＝2026 一般測試、零失敗、16 ignored，library 528.49 秒。標準程式已備份並更新，新版待命視窗已啟動，使用者檢查可讀性與真實服務結果仍待回報；完整步驟、雜湊及範圍見同一份診斷紀錄，Phase 11 不提前結案。

2026-10-24 使用者已回報新版 Gemini「本機已儲存且可讀取」，本機儲存／讀取人工驗收通過。實際 Gemini 初始化、字幕與翻譯及 1011 是否解除仍待驗收；金鑰欄可留空沿用已存項目，不需反覆重新儲存。本次僅更新驗收記錄，沒有變更程式或重跑先前已通過的建置／測試。
