# Phase 11：Gemini 啟動即中斷的安全診斷

## 2026-10-28：依驗收範圍結案

使用者對 PID 10852 的主頁／設定／可調狀態視窗與兩服務手選語言限制，各回覆選項 1。對 Chrome 以外播放器回覆「這段免測」；後續明確指定實際雲端服務斷線／恢復與真實混音／串燒／歌詞「兩類皆列本次免測，保留限制並結案」。依此完成 Phase 11 驗收，免測不當成通過，既有失敗與限制保留。交付版本、工程與人工證據見 [結案紀錄](phase-11-closeout.md)。

核對先前影片 worker 等待問題的既有紀錄，確認前批曾定位並校正原生啟動的測試預算；本次未改碼，精確重跑該停止／錯誤／中文 sidecar 案例 1 成功、零失敗、14.69 秒。失敗啟動 3.6768／3.3818 秒、Pending 啟動 3.4536／3.2966 秒、取消清理 1.3217／1.2687 毫秒，保留原取消／sidecar 五秒限制。不以單案成功宣稱所有媒體情境或產品效能改善。執行檔 E3C6C5CD… 雜湊未變；既有 2166 項全套工程紀錄沒有重跑或重複累加，未讀金鑰、擷取內容或啟動服務。以下為歷史診斷。

## 2026-10-28：OpenAI 法文／德文／韓文基本流程通過

使用者對 Chrome／OpenAI gpt-live-transcribe／Standard、手選對應來源語言的法文、德文、韓文三項各回覆選項 1。逐語約一分鐘、暫停十秒再恢復、最後停止的安排，皆回報原文與繁中持續更新、暫停恢復及停止保留正常。分別記錄基本流程正例，未提供量化完整度、延遲或診斷計數。與既有英文、日文及 Gemini Auto 紀錄合併檢視，兩種辨識服務的五語已有基本正例，各次版本與限制仍分別保留。

標準版 SHA256 仍為 E3C6C5CD0A267980DC6B898742986CE91AFB384C41A7DE78678A39C20C157462。原程式已關閉，已重開同版 PID 10852／Handle 920016／Responding true。啟動後 WaitForInputIdle 曾回報無法等待；隨後獨立查詢確認該 PID 有主視窗且回應，未重複啟動。工具未讀金鑰、改服務或開始擷取。接續主頁配置、手選語言前置阻擋與其他播放器可用性確認，待人工回覆。本批只更新文件，不改程式或重跑既有 2166 項測試，Phase 11 未結案。

## 2026-10-28：Chrome 法文／德文／韓文 Auto 基本流程通過

使用者對三項獨立問題各回覆選項 1。條件為 Chrome／Gemini Transcribe Live／Auto／Standard，逐語約一分鐘、暫停十秒再恢復，停止後核對紀錄保留；法文、德文、韓文皆確認原文與繁中持續更新及上述流程正常。只記三個基本流程正例，沒有診斷計數、服務／本機語言依據、量化完整度或延遲資料，不推論零缺漏、其他來源或所有噪音情境。

標準版 SHA256 仍為 E3C6C5CD0A267980DC6B898742986CE91AFB384C41A7DE78678A39C20C157462。原程式已關閉，已重開同版 PID 4252／Handle 1510170／Responding true；工具未讀金鑰、改服務或開始擷取。接續同音源 Chrome／OpenAI · gpt-live-transcribe／Standard，法文、德文、韓文各輪手選對應來源語言，結果待回覆。本批僅更新驗收文件，不改程式或重跑既有 2166 項測試，Phase 11 未結案。

## 2026-10-28：法文／德文／韓文 Chrome 批次準備

使用者對三語音源可用性選項回覆 1（三種都有），來源欄自由文字回覆 chrome，依此先安排 Chrome，不強制改為其他播放器，也不推論其他播放器未安裝。沿用同版現有 PID 21124／Handle 724342／Responding true／Reused true，標準版 SHA256 仍為 E3C6C5CD0A267980DC6B898742986CE91AFB384C41A7DE78678A39C20C157462。工具未切換服務或開始擷取、未讀金鑰或媒體內容。

交付三個分開回覆的案例：Gemini Transcribe Live／自動偵測／Standard，各語播放約一分鐘、暫停十秒再恢復、停止確認紀錄保留；換語言前停止並重新核對 Chrome 來源，再建立新工作階段。按開始將所選程序音訊送至 Gemini，由使用者明確操作。異常只收完整狀態、Auto／語言依據與數值，不收字幕本文或金鑰。三語目前均待結果，音源可用不等於辨識通過；本次只更新文件，沒有改程式、安裝或重跑既有測試，Phase 11 未結案。

## 2026-10-28：補記 Live Translate 十分鐘輪替驗收

已收到使用者對同一日文來源／Gemini 3.5 Live Translate／日文／Standard 至少十分鐘安排的回覆選項 1：觀察到輪替後恢復、字幕紀錄保留。這是人工觀察正例，沒有精確重連延遲、計數或原始服務訊息；不據此唯一判定輪替觸發原因、無縫或零缺漏，也不推論更長時間／其他來源。前次日文清除與 Transcribe Auto 正例保持，不需重跑。

標準版 SHA256 仍為 E3C6C5CD0A267980DC6B898742986CE91AFB384C41A7DE78678A39C20C157462。原程式已關閉，已由 --ui 重開同版 PID 21124／Handle 724342／Responding true，只確認視窗存在與程序回應，工具未改服務或開始擷取／雲端。查詢本機 SAPI 及 OneCore 離線語音 metadata，均僅有 en-US、ja-JP、zh-TW，沒有 fr-FR、de-DE、ko-KR；只查詢語音語系，未播放、產生或儲存音訊，未安裝元件。接續先確認使用者可用的其餘三語音源及 Edge／VLC／PotPlayer／Media Player／Spotify 等來源，再交付實測步驟；本次僅更新文件，沒有改程式或重跑既有測試，Phase 11 未結案。

## 2026-10-27：Gemini Transcribe Live 日文 Auto 驗收

使用者依同一自有日文來源／Gemini Transcribe Live／Auto／Standard 的安排回覆選項 1：日文原文與繁中持續更新、暫停十秒恢復及停止後紀錄保留。只記這次流程正例，未提供數值、實際語言標記或本機估計計數，不推論採用哪一種依據、效能分位數或其他三語通過。現有 PID 24244／Handle 35980838／Responding true、E3C6C5CD… 版保持，沒有重啟或重建。下一項為使用者手動選 Live Translate／日文／Standard 連續至少十分鐘，觀察跨九分鐘連線期限後是否恢復及紀錄保留；尚待回覆，不預先勾選。九分鐘為本機 CONNECTION_LIFETIME，正常穩定執行後的 SessionExpired 走 RecoveryAction::Rotate，不是實測的供應商服務限制或無縫保證。本次只更新文件，沒有工具操作雲端或重跑既有測試。

## 2026-10-27：Gemini Live Translate 日文手動清除驗收

使用者依同一自有來源／日文／Standard／Gemini 3.5 Live Translate（中文串流）的安排回覆選項 1：手動清除後持續更新、舊段不回補、下一段繼續，停止後紀錄保留。記為該次操作正例，沒有確切時長、效能數值或所有語言證據，不當作長測。標準版 SHA256 仍為 E3C6C5CD0A267980DC6B898742986CE91AFB384C41A7DE78678A39C20C157462，沿用 PID 24244／Handle 35980838／Responding true；不重啟或重建，不以工具更改服務或開始擷取。接續由使用者手動選 Gemini Transcribe Live／Auto／Standard 測同一日文來源，觀察原文、繁中、暫停恢復及停止保留；這項尚待回覆。本次只更新驗收文件，未重跑既有測試，Phase 11 未結案。

## 2026-10-27：單次翻譯未完整回傳造成整條 Live 停止

最新補充：使用者對來源語言回覆選項 2（日文），對停止後字幕保留回覆選項 1（有保留），併同先前「掉字較少」記錄為本次日文補測的相對改善及停止保留正例。不倒推前一次恢復提示的原因或語言，不計算錯字率。現有 PID 24244／Handle 35980838／Responding true，標準版 SHA256 仍為 E3C6C5CD0A267980DC6B898742986CE91AFB384C41A7DE78678A39C20C157462，沒有重啟或重建。接續 Gemini Live Translate 手動清除案例只交付使用者操作，不自動切換服務或送出音訊。以下為各次歷史觀測。

後續補測：使用者要求重新啟動同版後，已由原執行檔開啟 PID 24244；隨後回報「掉字較少」。記為相對改善，沒有提供來源語言或停止保留結果，也沒有量化字錯率或保證完全不掉字。再次核對標準版 SHA256 仍為 E3C6C5CD0A267980DC6B898742986CE91AFB384C41A7DE78678A39C20C157462。本次只更新驗收紀錄，保留版本、不重跑既有測試；接續只補確認語言及停止後字幕是否保留，不要求重跑長時間測試。以下程序狀態與 PID 為各次歷史觀測。

部署後人工回覆選項 2：有提示，後續繼續更新並恢復。只記本次恢復正例；未提供提示種類、來源語言、數值、確切播放時間或停止／保留與字詞完整度，不據此判定是 Incomplete 或 Timeout／Stale 路徑、不計算延遲或錯字率，也不勾選所有語言或長期穩定性。已核對標準版 SHA256 仍為 E3C6C5CD0A267980DC6B898742986CE91AFB384C41A7DE78678A39C20C157462，目前無 SubtitleNow 程序；本次僅更新驗收文件，不修改程式或重跑既有測試。接續只補確認語言、剩餘缺漏及停止保留，不要求重跑前次安排的五分鐘流程。

完成紀錄：格式通過；Clippy 全 target／feature 零警告（119 秒）、Debug（96 秒）、隔離 target 的 Release（122 秒）通過。Release 後最後獨立串行完整回歸 cargo test --locked --offline --quiet -- --test-threads=1：2139 library＋27 整合＝2166 一般成功、零失敗、18 ignored；library 942.35 秒，整合各 1.75／0.00／2.62 秒。五項新增、145 項 Phase 11 與翻譯相關 59 項為重疊集合；沒有重跑 ignored 原生長測。Cargo.toml／Cargo.lock 與本批快照雜湊一致，沒有新相依。

確認主視窗已關閉且無舊程序，核對六個 Rust 檔案與驗證前來源雜湊相同。備份原標準版至 target/phase11-incomplete-translation-82f810f111394b6eb4e48e729460027b/subtitlenow.before.exe，SHA256 4F59DEC1651A2470B90C4348393467C0DA062D2B01D134D47C5AC446853BC1C4；由 target/phase11-diagnostics/release/subtitlenow.exe 更新 target/release/subtitlenow.exe。候選與安裝版皆 SHA256 E3C6C5CD0A267980DC6B898742986CE91AFB384C41A7DE78678A39C20C157462、9,973,760 bytes。--ui 啟動 PID 4420，MainWindowHandle 11209646、Responding true；只確認視窗與程序回應，不冒充外觀或真正服務驗收。未以工具讀取真實金鑰、擷取或送出真實音訊，新版持續播放與完整度仍待人工，Phase 11 未結案。

以下為本批過程；其中待續記的工程／部署項目由上列完成紀錄更新，人工仍待驗收。

使用者測試短停頓版（主視窗 PID 13500）回報自動停止，狀態來源 PID 16368／OpenAI／gpt-live-transcribe。最後收到 6489 包、送出音訊訊息 6484 則、前綴 3 包、服務訊息 189、暫定 37／定稿 16／交付 53、提交確認前暫存 103；語言收尾、本機音訊收尾、增益、本機語言及略過計數皆零，服務／估計語言依據尚未確認。真正末端錯誤為翻譯 Incomplete；這不是佇列滿證據，不用此回報認定新切段的字詞完整度通過。使用者已關閉視窗。

本機先重現單次 Incomplete 讓整個翻譯 Session 取消（1 failed、0.01 秒）。加入 Live 要求隔離與非過期退休、共同連續三次上限、明確 UI 提示，Prepared 仍原子失敗；已知 content_filter 明確 Refused。五項新增定向通過（1.18 秒），145 項 Phase 11 一般成功／1 ignored（72.78 秒）、翻譯相關 59 項一般成功（4.73 秒）、格式通過；集合重疊。中途兩輪測試型別呼叫編譯錯誤及一次無效 Final 修訂的 fixture 失敗（4 passed／1 failed）均已修正，不放寬正式來源守門。其餘品質檢查與部署續記，Phase 11 未結案。見 [ADR 0110](adr/0110-isolate-incomplete-live-translation-requests.md)。

## 2026-10-27：OpenAI 接近上限時的停頓收尾

完成紀錄：四項新增定向（8.53 秒）、140 項 Phase 11 一般成功／1 ignored（71.28 秒）、65 項串流一般成功／2 ignored（41.47 秒）通過，集合重疊。格式通過，Clippy 全 target／feature 零警告（57.29 秒）、Debug（60 秒）、隔離 target 的 Release（93 秒）通過。Release 後最後獨立串行 cargo test --locked --offline --quiet -- --test-threads=1：2134 library＋27 整合＝2161 一般成功、零失敗、18 ignored；library 983.37 秒，整合各 1.00／0.00／1.79 秒。沒有重跑 ignored 原生長測；Cargo.toml／Cargo.lock 與本批快照雜湊相同，沒有新相依。

確認 PID 26540 已關閉，核對三個修改的 Rust 檔案與驗證前雜湊相同。備份舊標準版至 target/phase11-pause-boundary-5fccda82a9f347a4ad2bda4510717913/subtitlenow.before.exe，SHA256 44E7A3CF4F869DB6C007B263516AC16B48EAAE4C184D459BECD9B78352995A82。由 target/phase11-diagnostics/release/subtitlenow.exe 更新 target/release/subtitlenow.exe，候選與安裝版皆為 SHA256 4F59DEC1651A2470B90C4348393467C0DA062D2B01D134D47C5AC446853BC1C4、9,973,248 bytes。--ui 啟動 PID 13500，MainWindowHandle 20450516、Responding true；這只證明視窗存在與程序回應。未讀取真實金鑰、擷取或傳送真實音訊；實際長句完整度仍待使用者明確開始。Phase 11 未結案。

以下為本批修正過程；其中待續記的工程／部署項目由上列完成紀錄更新，人工仍待驗收。

使用者接續測試 44E7A3CF… 版回報「非常顯著改善，但還是有掉字」，選項 1 確認集中於換段、接句或重新開口附近；不將此記為完整度通過，也沒有分語言數值。已依要求關閉 PID 26540。

合成測試先紅：3.6 秒有聲加 100 ms 低能量仍不收尾，預期 end=true、實際 false；1 failed、0.01 秒。實作 OpenAI 專用 PreferPauseNearLimit，在最後既有 400 ms 窗口選已觀察 100 ms 停頓；四秒硬上限、早期 400 ms 停頓、八十毫秒字首、模型與品質不變。四項新增定向通過（8.53 秒），含兩種 policy 真正 worker 的逐樣本／時戳一致，以及 localhost 的 185／220 包兩次 commit、真正 Final 對應與 3.7 秒／0.7 秒 span；沒有虛構掉字的詞彙重現或雲端成功。完整檢查與部署待續記，Phase 11 未結案。見 [ADR 0109](adr/0109-prefer-observed-pause-before-openai-turn-limit.md)。

## 2026-10-27：OpenAI 原文缺漏、有限字首與消費排程

完成紀錄：格式通過；Clippy 全 target／feature 零警告（28.24 秒）、Debug（39.51 秒）、隔離 target 的 Release（69 秒）通過。串流模組 64 一般成功、2 ignored（34.65 秒）。Release 後最後獨立串行完整回歸 2130 library＋27 整合＝2157 一般成功、零失敗、18 ignored；library 793.84 秒，整合各 0.50／0.00／1.54 秒。既有 ignored／原生長測未重跑，不把定向集合重複相加。Cargo.toml／Cargo.lock 與本批快照雜湊相同，沒有新相依。

確認舊程式已關閉後，先備份 target/phase11-openai-prefix-3ba055154f7e4c90ae9f36357d02b2c9/subtitlenow.before.exe（SHA256 92361A48F336C4CD05D7333C6AC34A41A09C4CD8F9F9478A4BB42257479DCCEE），再由 target/phase11-diagnostics/release/subtitlenow.exe 更新標準 target/release/subtitlenow.exe。候選與安裝版皆 SHA256 44E7A3CF4F869DB6C007B263516AC16B48EAAE4C184D459BECD9B78352995A82、9,972,736 bytes。以 --ui 開啟 PID 27672，MainWindowHandle 54006994、Responding true；僅確認視窗存在及程序回應，未以此冒充 UI 外觀或實際雲端驗收。未擷取／傳送音訊或讀取真實金鑰；新版英日文長句實測待使用者按開始，Phase 11 未結案。

以下保留本批修正過程；其中「待續記」的工程檢查及部署已由上列完成紀錄更新，人工仍待驗收。

最新人工回報 Gemini 小聲「好很多」；OpenAI 長句仍有掉字與翻譯不完整，曾佇列滿停止。重開 PID 32920 後使用者回報英日文基本正常、佇列正常，但仍掉字；追問選項 1 確認缺漏在原文本身。未取得先前滿載佇列名稱，不判定唯一根因。

先紅後綠：合成弱 PCM 字首測試原實收 960 bytes、預期 1920（1 failed）；localhost 單批 80 delta 原得到 TranscriptOutput／64 QueueFull（1 failed）。加入八十毫秒／最多十二包／原時戳逾期清理，合併第一個 append，保留實際來源 span；OpenAI 每個回覆後 cooperative yield，維持原佇列及真正 Final。新增七項測試含真實 localhost PCM 與 inline 消費，17 項 OpenAI 一般成功（2.49 秒），136 項 Phase 11 一般成功、1 ignored（52.37 秒），兩集合重疊。完整品質檢查、部署與人工長句驗收待續記，Phase 11 未結案。見 [ADR 0108](adr/0108-openai-bounded-prefix-and-consumer-fairness.md)。

以下為先前各批紀錄。

## 2026-10-26–27：OpenAI 提前回覆與安全錯誤階段

2026-10-27 部署後人工回覆選項 1：PID 29572、正常音量自有 Chrome 英文來源、OpenAI／gpt-live-transcribe／英文／Standard 的英文與繁中持續更新、停止正常。只記本次端到端正例，沒有診斷計數，不據此認定提早回覆為唯一根因或擴大其他語言／逾時／效能通過。Gemini 小聲仍未通過，已請使用者改 Gemini Transcribe Live／明選英文，以原小聲來源及相同音量重現二十秒，在效果不佳時回報完整狀態與增益／音訊／轉錄計數；不再要求 OpenAI 重複同一已通過測試，不重啟程式。

使用者回覆 Gemini 小聲源稍有改善但仍不足；OpenAI／gpt-live-transcribe 立即顯示資料格式或工作階段無效，追問回覆選項 1（立即、尚無字幕），沒有提供當時語言或數值。保留失敗，不能只憑提示判定為金鑰、權限或唯一協定原因。

使用 OpenAI Docs 技能，實際讀取指定模型及 [Realtime transcription 官方文件](https://developers.openai.com/api/docs/guides/realtime-transcription)，核對既有端點／設定並確認可有提交前 delta。新增測試先重現 Err(InvalidData)（1 failed、0.01 秒）；之後實作有界暫存及真正 ack 對應。第一輪編譯漏補 EarlyTranscripts 佇列列舉，已補正，不列為行為驗證；十項新測試最終成功（0.84 秒），包含兩個真正 localhost WebSocket 的提前回覆／零音訊服務錯誤。

未知 item 每項保留合併暫定與真正 Final，本文章節上限原樣、項數沿用事件上限六十四、首份回覆八秒且不續期；待實際提交確認後才釋出相符 item，其他不猜測。明選 Transcribe 亦每秒數值進度，固定安全服務碼、接收／事件處理／提交對應階段不留原始服務 message／param；root_cause 復原預算保持，模型仍為 gpt-live-transcribe。工程檢查與部署結果列於下段，實際 OpenAI 尚待人工；本批不調高 Gemini 增益，不宣告小聲問題修復。見 [ADR 0107](adr/0107-openai-early-transcript-correlation.md)。

第一輪 Phase 11 定向 128 成功、1 失敗、1 ignored（51.20 秒）：新階段包裝額外包住已含安全關閉碼的既有 RemoteStatus，破壞原精確型別斷言。已縮小為只包裝普通 InvalidData／固定 RequestRejected，不改原關閉碼測試或復原預算。最終 Phase 11 定向 129 成功、零失敗、1 ignored（51.76 秒）；整個串流模組 57 成功、零失敗、2 ignored（32.85 秒）。集合重疊不累加。格式與零警告 Clippy（49.03 秒）、Debug（64 秒）及獨立 Release（74 秒）通過。

Release 後獨立執行 `cargo test --locked --offline --quiet -- --test-threads=1`：2123 library 成功、零失敗、18 ignored（855.10 秒）；整合 13＋4＋10＝27 成功（0.51／0.00／1.38 秒），總計 2150 一般成功。未重跑 ignored／原生長測，定向與完整集合重疊不累加，不以 mock 或公開模型文件當作帳號權限／真正字幕通過。

2026-10-27 部署前確認無 SubtitleNow 程序；舊標準及備份 `target/phase11-openai-early-640fa90510ec43a7bfa9f8f9484cfa4f/subtitlenow.before.exe` 均為 5C253A5C…；候選與更新後標準 `target/release/subtitlenow.exe` 均為 SHA256 92361A48F336C4CD05D7333C6AC34A41A09C4CD8F9F9478A4BB42257479DCCEE、9,981,440 bytes。新版 --ui 已開啟，PID 29572、MainWindowHandle=148378334、Responding=True；只核對程序中繼資料，未讀取正式金鑰或代按開始。真正 OpenAI 與 Gemini 小聲結果仍待人工，Phase 11 未結案。

## 2026-10-26：小聲錄製音源的辨識前處理增益

使用者對 PID 31548 的 Auto 首句測試回覆選項 1、約五至十秒，未分別提供開始及長停頓後的秒數；另回報小聲錄製音源（如街邊）有時不啟動翻譯，追問回覆 1、3：Auto 原文／繁中皆無，明選語言也發生。未取得當下計數或實際音量，不將所有情境歸因於本機門檻，也不將約五至十秒寫成效能達標。

新增完整 worker 測試，以振幅 0.0005、500 Hz 合成音訊重現原樣訊號無法啟動端點（1 failed，0.01 秒）。最初測試誤呼叫私有 Cancellation 建構子而編譯失敗，已改用既有 SpeechSession 取得取消，不更動可見性；該編譯錯誤不列為行為證據。四項低音量核心測試在加入增益後通過（0.01 秒）。

初次 Phase 11 回歸 118 成功、1 失敗、1 ignored（52.53 秒）；失敗為新 WebSocket fixture 把明選語言但沒有服務語言標記的 Final 預期成帶 English。已保留既有 None 契約並修正 fixture，Auto 仍必須取得合法服務標記或本機判定；不因增益假造語言。Clippy 初次另要求測試中的固定兩-byte 分組改用 as_chunks，已修正，沒有壓制警告。

Gemini Live（Transcribe Auto／明選及 Live Translate）在既有 worker 內先驗證與有限增益，再重採樣及能量端點；Original 保持原有波形，沒有新 worker／佇列／相依或音訊保存。新增「低音量增益」前處理包數；實際送出振幅由 localhost 解碼 PCM 檢查，不只改旗標。修正 fixture 後 Phase 11 最終定向 119 成功、零失敗、1 ignored（49.48 秒），含五項新增；格式與零警告 Clippy（45.99 秒）、Debug（45.85 秒）與獨立 Release（99 秒）通過。見 [ADR 0106](adr/0106-bounded-live-recognition-input-gain.md)。

Release 後獨立執行 `cargo test --locked --offline --quiet -- --test-threads=1`：2113 library 成功、零失敗、18 ignored（809.66 秒）；整合 13＋4＋10＝27 成功（0.90／0.00／1.32 秒），總計 2140 一般成功。定向集合與完整集合重疊不累加，未重跑 ignored／原生長測；不以本機回覆當作真正 Gemini 小聲音源的準確率或延遲驗收。

部署前確認無 SubtitleNow 程序，舊標準與備份 `target/phase11-soft-audio-92c7bc49fa7c42e5ae5067dce345d51b/subtitlenow.before.exe` 均為 B92E8327…；候選及更新後標準 `target/release/subtitlenow.exe` 均為 SHA256 5C253A5C01624F82E6A022D54DBF0B26A2C70F487DEC8AB138E6C073E3048DE2、9,958,912 bytes。已開啟 --ui，PID 5616、MainWindowHandle=26612660、Responding=True；只讀程序中繼資料，沒有讀取正式金鑰或代按開始。真實低音量效果待本次人工回報，Phase 11 未結案。

## 2026-10-26：Auto 首句延遲與持續靜音的收尾

使用者對 PID 31492 的裝置切換測試回覆選項 1、翻譯正常，並回報 Auto 首句及長停頓後首句約三十秒才翻譯、逾期導致丟棄較多；追問確認連英文原文亦延遲，未提供當下診斷數值。此為辨識入口方向的證據，不足以認定全部延遲或逾期的根因。

原 Gemini 音訊路徑不處理既有 PreparedBlock.end，傳輸閒置與語言等待收尾皆無法涵蓋「持續靜音封包、首個轉錄尚未到達」。新增協定測試先確認失敗：應有 PCM＋收尾兩訊息，實際只有一訊息（1 failed）；初次命令誤用短名稱搭配 exact，0 項執行，不列為驗證。修正後五項新增測試通過（3.52 秒），其中真正 localhost WebSocket 在收尾之前刻意不回任何轉錄：兩段合成音訊、中間持續靜音超過 1.1 秒，均收到真正 final 並分別交付英文／日文語言，無媒體時間、無語言沿用，取消正常。

本批只啟用 Gemini Transcribe Live／Auto 的既有能量端點；四百毫秒低能量或四秒區段後要求收尾，後續新鮮 PCM 不等待結果。靜音不反覆要求、語言等待收尾去重、不重設已有回應期限，其他模型／明選語言不變。成功傳送另記「本機音訊收尾要求」純數值，沒有內容保存或新相依。見 [ADR 0105](adr/0105-auto-client-audio-endpoints.md)。

審查另補「收尾要求已送出、缺語言 partial 之後才到」的去重案例；追加後六項新增案例納入 Phase 11 最終定向 114 成功、零失敗、1 ignored（50.24 秒）。先前定向 113 成功為追加前的紀錄，不重複累加。最終格式檢查、零警告 Clippy（23.15 秒）、Debug（30.43 秒）及獨立 Release（66 秒）通過。

Release 後獨立執行 `cargo test --locked --offline --quiet -- --test-threads=1`：2108 library 成功、零失敗、18 ignored（832.46 秒），整合 13＋4＋10＝27 成功（0.58／0.00／1.37 秒），總計 2135 一般成功。未重跑 ignored／原生長測，定向集合與完整集合重疊，不另累加；不把本機 mock 當成真正雲端或播放器矩陣通過。

部署前確認無 SubtitleNow 程序；舊標準及備份 `target/phase11-hybrid-f3f636c54e914b019de649ef1411f231/subtitlenow.before.exe` 均為 A8A43962…；候選及更新後標準 `target/release/subtitlenow.exe` 均為 SHA256 B92E832734ED67C2427A413ED3C6AF6242A526F2FD809F2BB7A77C0CB2855ABB、9,956,864 bytes。新版 --ui 已開啟，PID 31548、MainWindowHandle=22744074、Responding=True；只核對程序中繼資料，未讀取正式金鑰或代按開始。真實首句延遲及逾期改善待本次人工驗收；不延長三秒翻譯總年齡來保留過時要求，Phase 11 未結案。

2026-10-26 最新人工結果：使用者對本機語言備援版的 Chrome 英文來源／Gemini Transcribe Live／Auto／Standard 測試回覆「可用」，記為該組合的基本實測正例。未提供診斷計數，無法確認本次採用服務標記或本機估計；不據此擴大為五語、暫停／停止各步驟或其他服務全面通過。Phase 11 尚有人工驗收與既有媒體間歇逾時追查，未結案；以下保留各批歷史紀錄。

## 2026-10-26：真正定稿缺語言標記，改用保守本機文字備援

最終獨立串行完整回歸完成：2102 library 成功、零失敗、18 ignored（783.08 秒）；整合 13＋4＋10＝27 成功（0.57／0.00／1.41 秒），共 2129 一般成功、零失敗。108 項定向及新 11 項與完整集合重疊，不相加。未重跑 ignored／原生長測，也沒有以工具啟動正式雲端音訊測試。

部署前確認無 SubtitleNow 程序，舊標準及備份 `target/phase11-diagnostics/prior-auto-local-language-8dcd565a622043be8737f4b5fb05a5f3/subtitlenow.exe` 均為 05FF7050…；候選及更新後標準 `target/release/subtitlenow.exe` 均為 SHA256 A8A439628B58CA7199FB5F700AB13C3D7DC646FC74FE63E149F3F26780FA828E、9,948,160 bytes。已開啟 --ui，PID 31984、HasExited=False、MainWindowHandle=463754、Responding=True，只核對程序中繼資料，沒有讀取正式金鑰、代按開始或擷取／傳送使用者內容。待人工確認真實 Auto 的原文、繁中、語言依據及採用／略過數；Phase 11 未結案。

Debug（1 分 16 秒）與獨立 Release（1 分 44 秒）通過，二十六個本批檔案嚴格 UTF-8 及八份文件本機連結檢查通過。候選 9,948,160 bytes、SHA256 A8A439628B58CA7199FB5F700AB13C3D7DC646FC74FE63E149F3F26780FA828E；Release 後已獨立啟動 `cargo test --locked --offline --quiet -- --test-threads=1`，尚未宣告完整結果，未覆寫標準版。

使用者實測：收到／送出 348 包、服務訊息 7、暫定 5、真正定稿 1、交付 0、語言收尾要求 1，仍 LanguageNotReported。確認收尾取得了定稿，卡點是本次缺少可核對標記；不能再把欄位必填當成 Auto 前提。沒有取得私人本文或正式原始 JSON，不宣稱所有服務回應都缺標記。

新增 whatlang 0.18.0 本機文字模型備援，使用全語言模型後才篩選五語，至少十二個字母及函式庫可靠性旗標；不增加雲端要求。缺標記的新修訂逐一重新判定，不沿用前段或冒充服務標記；不確定真正 final 明示略過並繼續，外語可靠判定仍拒收。CPU 使用既有有界 lane、四十 ms 預算，工作綁定 Session／generation、服務訊息序號、精確本文、種類與期限。詳見 [ADR 0104](adr/0104-local-text-language-fallback.md) 及 [授權紀錄](../licenses/NOTICE.md)。

首次模型實驗為 1 成功、1 失敗（0.62 秒）：某支援外語言被安全地回報 Uncertain，fixture 卻要求一定 Unsupported；改為檢查不得強迫採用五語，沒有降低模型門檻。後續完整新集合 11 成功、零失敗（3.06 秒），涵蓋五語、外語／短句不強制、同一真實本機 WebSocket 無語言標記仍連續英／日交付、取消、飽和、逾時、真正 permit 回收、舊訊息／本文／期限拒收、逐修訂重判及略過後下一段。八千一百九十 bytes 合成文字的一百次 Debug CPU probe 為 P50 6.1073 ms、P95 8.2354 ms、最大 9.3298 ms；不是字幕延遲或模型準確率量測。

最終定向 `cargo test --locked --offline --lib --quiet phase11_ -- --test-threads=1` 為 108 成功、零失敗、1 項既有 ignored（48.04 秒）；集合重疊不相加。格式與零警告 Clippy（59.16 秒）通過，Debug／Release 及獨立完整回歸待結果。Cargo.lock 僅增加五個必要套件，既有版本未移除／更新；沒有 build script 或額外 Windows 元件，原始授權已保留。舊標準版備份 `target/phase11-diagnostics/prior-auto-local-language-8dcd565a622043be8737f4b5fb05a5f3/subtitlenow.exe` 為 05FF7050D0B5D8235020ED9C10A761DE3218896A3B1CE578887FBC7B813CEBAF、9,631,744 bytes，已核對。尚未覆蓋標準版或讀取正式金鑰；真實 Auto 與 Phase 11 未結案。

## 2026-10-26：Auto 有 partial、無 final 的收尾修正

最終獨立串行完整回歸完成：2091 library 成功、零失敗、18 ignored（820.85 秒），整合 13＋4＋10＝27 成功（0.51／0.00／1.58 秒），共 2118 一般成功、零失敗。97 項定向與完整集合重疊，不累加；本批沒有重跑 ignored／原生長測或正式服務。

更新前再次確認無 SubtitleNow 程序，舊標準及備份 `target/phase11-diagnostics/prior-auto-language-end-329afa304af54b7284e2024c3c66d834/subtitlenow.exe` 均為 7B53CB89…；候選與更新後標準 `target/release/subtitlenow.exe` 均為 SHA256 05FF7050D0B5D8235020ED9C10A761DE3218896A3B1CE578887FBC7B813CEBAF、9,631,744 bytes。已開啟 --ui，PID 4968、HasExited=False、MainWindowHandle=98505144、Responding=True，僅核對程序中繼資料。未讀取正式金鑰、代按開始或擷取／傳送使用者內容；真實服務定稿／語言及字幕效果由使用者下一次重試確認，Phase 11 未結案。

獨立 Release 通過（1 分 15 秒），候選 9,631,744 bytes、SHA256 05FF7050D0B5D8235020ED9C10A761DE3218896A3B1CE578887FBC7B813CEBAF。Release 後已獨立啟動 `cargo test --locked --offline --quiet -- --test-threads=1`，尚未宣告結果；舊標準及可回復備份保持 7B53CB89…，未提前覆寫或啟動正式服務。

最終格式、零警告 Clippy（21.67 秒）、Debug（39.53 秒）及十個本批檔案嚴格 UTF-8／七份文件本機連結檢查通過；Release 與後續獨立完整回歸待結果。未改動正式請求設定、憑證、模型或供應商，沒有工具驅動的雲端音訊測試。

定向 `cargo test --locked --offline --lib --quiet phase11_ -- --test-threads=1` 為 97 成功、零失敗、1 項既有 ignored（45.20 秒）。四項新增案例均執行，本機同一 WebSocket 的連續合成音訊取得英／日兩段真正 final 並核對兩次成功收尾計數；協定測試核對兩秒門檻、一次限制、自然閒置去重、持續送音訊、原八秒期限、不製造 final、同段退休及已知／明選語言隔離。原數值計數測試追加新計數與文字核對；集合重疊不累計。

舊標準 7B53CB89CAD3BE92E7ECC358A425914F5569D6D1A87606ABCAC80656830D1218（9,622,528 bytes）已備份至 `target/phase11-diagnostics/prior-auto-language-end-329afa304af54b7284e2024c3c66d834/subtitlenow.exe` 並核對相同雜湊；尚未更新。格式／Clippy、Debug／Release、最終完整回歸待追加實際結果。

舊版新增回歸 `phase11_auto_continuous_audio_requests_real_finals_before_language_timeout` 實際為 0 成功、1 失敗（8.04 秒），runner 因 LanguageNotReported 提早結束，沒有取得兩個真正 final。本機服務只在收到收尾後才回傳有語言的 final；客戶端持續送合成音訊，沒有使用者內容或正式服務。修正後定向及品質／完整結果待追加。all-targets check 已通過（19.71 秒）。

使用者回報實際計數：收到／送出音訊各 915 包、服務訊息 18、暫定 17、定稿 0、交付 0，Auto 待核對語言後因 LanguageNotReported 結束。確認前端音訊與辨識回應已進入；本次未收到 final，不能推論服務定稿一定缺少語言。使用者已回覆 PID 19960 關閉。

檢查發現既有排出訊號只在真實輸入中斷後送出，持續播放的待語言段未主動要求 final。新增本機 WebSocket 使用合成音訊及只在 audioStreamEnd 後回傳 final／語言的合法序列，先驗證舊版失敗，再加入 Auto 待核對時兩秒、每段一次的收尾要求，仍保留原八秒總期限及五語閘門。實際服務行為仍待下一次人工回報，不把模擬序列當成雲端根因。見 [ADR 0103](adr/0103-auto-language-finalization-request.md)。

## 2026-10-26：同來源英文正常／Auto 無字幕的進度診斷

最終獨立串行完整回歸完成：2087 library 成功、零失敗、18 ignored（971.64 秒）；整合 13＋4＋10＝27 成功（0.49／0.00／1.43 秒），共 2114 一般成功、零失敗。93 項定向與完整集合有重疊，不累加。沒有重跑 ignored／原生長測或正式服務，前批失敗／修正紀錄保持。

部署前再次核對無 SubtitleNow 程序，標準舊檔及 `target/phase11-diagnostics/prior-auto-progress-cada2315599047d98bbca17952b4c17a/subtitlenow.exe` 備份皆為 CE00C131…；候選與更新後標準 `target/release/subtitlenow.exe` 都為 SHA256 7B53CB89CAD3BE92E7ECC358A425914F5569D6D1A87606ABCAC80656830D1218、9,622,528 bytes。已開啟 --ui，PID 19960、HasExited=False、MainWindowHandle=443879336、Responding=True，只核對程序中繼資料。未讀取正式金鑰、代按開始或擷取／上傳使用者內容；下一步由使用者回報 Auto 的完整數值與階段，實際根因及 Phase 11 仍未結案。

獨立 Release 建置通過（2 分 10 秒），候選 9,622,528 bytes、SHA256 7B53CB89CAD3BE92E7ECC358A425914F5569D6D1A87606ABCAC80656830D1218。Release 後已獨立啟動 `cargo test --locked --offline --quiet -- --test-threads=1`，待實際結果；舊標準版尚未覆蓋。沒有變更 Auto 請求或新增正式服務要求。

最終格式、零警告 Clippy（1 分 19 秒）及 Debug（1 分 27 秒）通過，14 個本批修改檔嚴格 UTF-8 與七份文件本機連結檢查通過。Release 與其後獨立完整回歸待結果；已確認的本機無限等待缺口不等於已確認此次服務無字幕根因。

本批定向 `cargo test --locked --offline --lib --quiet phase11_ -- --test-threads=1` 完成：93 成功、零失敗、1 項既有 ignored（48.28 秒）。五項新增案例全部執行，含真正九秒期限、本機真正 WebSocket 的 Text／Binary JSON、未知 partial 不交付、有效語言才交付、取消、計數飽和、語言等待跨段退休及 UiFrame 身分／字幕 Arc 保留。集合與既有測試有重疊，不相加當完整總數。

舊標準版 CE00C1316FF976D6C39112B3176D7562F7F5497A4E563C30FDF36EE61EB112C8（9,620,992 bytes）已備份至 `target/phase11-diagnostics/prior-auto-progress-cada2315599047d98bbca17952b4c17a/subtitlenow.exe` 並核對雜湊。尚未覆蓋標準版，未讀取金鑰或啟動擷取／正式服務。

使用者已確認同一來源手選英文有字幕、Auto 沒有，並回覆 PID 30444 已關閉。尚無正式回應資料，不將「正在聆聽」當成成功或已確認根因。新增九秒本機回歸先得到 0 成功、1 失敗（9.04 秒）：未知語言 partial 更新一般回應計時，導致沒有總等待上限。修正加入獨立八秒語言等待、遲到拒收及同段語言確認退休；五語閘門保持。

另新增每個 SpeechSession 固定六個純數值計數與一個旗標，Auto 使用既有每秒 report tick 經同一 UiFrame 更新，錯誤保留已停止標示的最後取樣。可分辨音訊未到／未送出、服務無轉錄或正在等語言。資料沒有音訊、本文、來源識別、金鑰或播放位置，不新增工作執行緒、channel、鎖、記錄檔或依賴。詳見 [ADR 0102](adr/0102-bounded-auto-language-wait-and-numeric-progress.md)。

首次 all-targets check 通過（1 分鐘），只指出 expired 現在僅供測試使用的 dead_code 警告，已加 cfg(test) 而非隱藏警告。定向、最終品質／完整回歸與部署待本批結果；不把前批 2109 成功當成本批通過，也不宣稱實際 Auto 根因已確定。

## 2026-10-26：Auto 無字幕，重新啟動供同來源比較

使用者重試 Auto 後回報沒有任何字幕，主頁只有 Gemini Transcribe Live／正在聆聽／等待字幕，沒有錯誤碼；不把該狀態當成辨識成功，也不能僅由此判定是音訊未進入、服務未回應或語言 partial 等待。詢問持續播放時間及同來源手選英文是否正常後，使用者要求先重新啟動軟體測試。

本次未修改 Rust、重建、重跑既有測試或讀取金鑰。核對標準版仍為 CE00C1316FF976D6C39112B3176D7562F7F5497A4E563C30FDF36EE61EB112C8，確認沒有既有主視窗後開啟 --ui：PID 30444、HasExited=False、MainWindowHandle=64491900、Responding=True，使用標準路徑。只核對程序中繼資料，沒有代按開始、擷取或正式服務要求。等待使用者同一英文測試來源的手選英文／Auto 比較結果，再決定修正；狀態視窗縮放人工結果亦尚未取得。前批 2109 工程測試不當作此次真實 Auto 通過，Phase 11 未結案。

## 2026-10-26：英文 Auto 回報與狀態視窗修正

結案前本批 17 個修改檔嚴格 UTF-8 與七份文件本機連結檢查通過；再次核對 PID 27164 使用標準路徑、MainWindowHandle=50139642、Responding=True。未操作擷取／服務。工程修正已交付，以下保留各階段與首次失敗紀錄；真實 Auto 及視窗外觀人工項目仍未勾選。

最終第二輪 `cargo test --locked --offline --quiet -- --test-threads=1` 完成：2082 library 成功、零失敗、18 ignored（1014.41 秒），整合 13＋4＋10＝27 成功（1.17／0.00／2.38 秒）；共 2109 一般成功、零失敗。來源對話框十項定向及 fixture 修正後 Clippy（28.59 秒）通過，格式檢查亦通過。沒有重跑 ignored／原生長測，合成／本機協定結果不代表正式 Auto、實體多螢幕或外觀驗收。

部署前再次確認無 SubtitleNow 程序，舊標準及備份 6677326A…、候選 CE00C131… 核對相同，再複製至標準 `target/release/subtitlenow.exe`，部署後完整 SHA256 CE00C1316FF976D6C39112B3176D7562F7F5497A4E563C30FDF36EE61EB112C8、9,620,992 bytes。舊版備份位於 `target/phase11-diagnostics/prior-auto-status-aaa0ed03c5674b2db80cdea0b53c68ad/subtitlenow.exe`。新版 --ui 啟動 PID 27164、HasExited=False、MainWindowHandle=50139642、Responding=True；只核對程序中繼資料，不宣稱目視。沒有讀取正式金鑰、代按開始、擷取使用者音訊或呼叫正式服務；待使用者重試 Chrome 英文 Auto 與可調整狀態視窗，Phase 11 未結案。

fixture 修正後，來源設定對話框整個模組 10 成功、零失敗（1.31 秒），原快照／清理／遮蔽斷言保持。先前一次短名稱加 --exact 的篩選只選到 0 項，不算通過證據；本次使用完整模組篩選確認十項皆執行。唯一追加 Rust 變更位於測試專用檔，Release 候選仍為 CE00C131…，正式能力閘門未改。

第一輪完整 library 回歸為 2081 成功、1 失敗、18 ignored（1065.03 秒），Cargo 因失敗未進入後續整合組。唯一失敗是既有 `panel_start_carries_immutable_settings_and_completion_clears_them` 在選 GPT 後留著 Auto，於 request 正確得到 ExplicitLanguageRequired。這是前批能力前置檢查後漏改的測試設定，不是服務實測或本批語言解析失敗。fixture 現在明選英文並追加 request.language 核對，原 immutable subtitle settings、繁忙時忽略清除、完成後清理與私有 URL 遮蔽斷言保持；不放寬正式啟動閘門。定向及第二輪獨立完整回歸待實際結果。

Debug 建置通過（27.87 秒），獨立 Release 通過（1 分 58 秒）；候選 9,620,992 bytes、SHA256 CE00C1316FF976D6C39112B3176D7562F7F5497A4E563C30FDF36EE61EB112C8。Release 後已單獨啟動 `cargo test --locked --offline --quiet -- --test-threads=1`，尚未宣告結果。舊標準版備份 `target/phase11-diagnostics/prior-auto-status-aaa0ed03c5674b2db80cdea0b53c68ad/subtitlenow.exe` 為 9,629,696 bytes、6677326A6720AF368F60F0444C08B6B95B481EC133B54D382896F68DFB92052F，與原檔核對相同；標準版尚未更新。

定向結果：`cargo test --locked --offline --lib --quiet phase11_ -- --test-threads=1` 為 88 成功、零失敗、1 項既有 ignored（45.11 秒）。包含四項 Gemini Auto 協定／計畫回歸、一項有界標籤解析及三項狀態視窗回歸；非重試清單亦補 LanguageNotReported。原有案例一併通過，集合不累加冒充完整總數。格式與零警告 Clippy（49.01 秒）通過，16 個本批檔案嚴格 UTF-8 及七份文件的本機連結檢查通過。Debug／Release 及 Release 後獨立完整回歸待追加。

使用者在 Gemini Transcribe Live／自動偵測播放英文仍收到五語限制錯誤，另要求狀態框調整寬、高。未取得正式服務原始回應，不宣稱已確認雲端失敗根因。先補三項回歸：合法地區標籤、同一句有語言 partial 到省略標籤 final、Auto 初始化空 languageCodes；修正前 0 通過／3 失敗（0.04 秒）。

已修正請求與語言 metadata，保留五語拒收／同段身分，不猜測語言、虛構時間或套用前一段。新增 LanguageNotReported 非重試提示。狀態標題及選單開啟單一可調整寬、高的獨立原生視窗，主頁預覽保持；關閉只隱藏，主視窗 Drop 同步回收 HWND／HFONT，不影響翻譯、字幕或捲動。詳見 [ADR 0101](adr/0101-gemini-auto-language-metadata-and-status-details.md)。

第一次加入視窗測試時，測試呼叫 refresh 遺漏既有 visible 參數，編譯失敗後改為 false；沒有修改正式函式契約。修正前 all-targets check 已通過（1.61 秒），但不代替後續測試。定向、完整回歸與部署待本批實際結果追加；不把前批 2099 成功當作新碼通過。

## 2026-10-26：語言能力前置版標準更新完成

使用者回覆「已關閉」後，重新核對沒有 SubtitleNow 主程序或輔助程序。舊標準版及 `prior-auto-preflight-45ebfc1d5d624e53a2ed7f82937c0800/subtitlenow.exe` 備份均為 0FBFA316268C7BC67FDBCD710E6E7CFE9E3AC4EB0946C714E7F234F68C85E53C、9,627,648 bytes。候選 `target/phase11-diagnostics/release/subtitlenow.exe` 與覆寫後標準 `target/release/subtitlenow.exe` 均為 6677326A6720AF368F60F0444C08B6B95B481EC133B54D382896F68DFB92052F、9,629,696 bytes。

以 --ui 開啟標準版：PID 29304、HasExited=False、MainWindowHandle=125701560、Responding=True。只核對程序中繼資料，不宣稱目視；沒有讀取欄位／安全憑證、代按開始或啟動音訊／雲端處理。原生 59 及 Phase 11 定向 81 一般成功（各 1 ignored、有重疊）、格式／Clippy／Debug／Release 已在前批執行，本次未修改 Rust／重建／重測，不增加新的通過數。

下一步由使用者確認 Live Translate 第一項為「請手選語言」、必要語言未選時開始停用，明選英文後可繼續既有模式。需要 Auto 時，使用者明確切到 Gemini Transcribe Live，再選 Auto／Standard、所選 Chrome 音源及已存 Gemini key；按開始才傳所選程序音訊至 Gemini，觀察原文與臺灣繁中後停止。沒有自動改模型或 fallback，真實 Auto 與新版介面仍待驗收，Phase 11 未結案。

## 2026-10-25：Auto 選項與啟動前能力提示

使用者確認同一正常 Live Translate 視窗的暫停／恢復、最新一行、人工上捲／回底部跟隨及停止保留均正常，另指出選 Auto 會報檢查來源或所選服務。依此次回覆將該組操作記為單次人工通過，不延伸為手動清除、長測、其他語言或 OpenAI 正例。

檢查確認目前整合既已在 LiveSpeechModel.validate_live 拒絕 Native Live Translate／GPT 的 Auto，但 UI 仍提供相同 Auto 並在背景啟動後才遇錯。此次不改五語界線、不新增 Auto 辨識或模型 fallback，而是把現有規則前移至選項與 request。Gemini Transcribe Live 保留 Auto，其他模型索引零顯示「請手選語言」；語言及品質無效時開始停用、主頁最前顯示原因，直接開始／Gemini 診斷亦在欄位 key 讀取前拒絕。偏好及金鑰不暗改，見 [ADR 0100](adr/0100-live-option-capability-preflight.md)。

第一輪 cargo check --locked --offline --all-targets 通過（48.96 秒）。新增測試首輪編譯因 ControlPreferences 未實作 Debug 而失敗，改用等值布林斷言，沒有新增 Debug 或弱化條件。原生視窗首輪 57 成功、2 失敗、1 ignored（76.62 秒）：兩個舊 mock 啟動 fixture 用 OpenAI＋Auto 作成功前提，與既有後端限制不符；只改成明選英文，保留定位資源、金鑰鎖定及呼叫斷言。修正後原生視窗 59 成功、零失敗、1 ignored（74.08 秒），新增三模型／五語／Auto／偏好還原／選取保留／金鑰不消耗／直接命令防繞過通過；Phase 11 定向與品質檢查接續進行。本批範圍為 UI 選項與既有能力前置，未重跑未修改的完整媒體矩陣，也不將前批 2099 項完整成功算作本批新結果。

更正先前程序數與視窗數的說法：本次程序中繼資料確認 PID 29612 有主視窗，PID 29004 無主視窗且 ParentProcessId=29612；capture_helper 原實作會以同執行檔及 --capture-helper 啟動無視窗子程序。不能把兩個程序稱為兩個主視窗。未擅自終止程序、未讀 key／本文或代按開始；更新只需使用者正常停止並關閉主視窗，輔助程序由原 owner 回收。

本批最終相關驗證：原生視窗 59 一般成功、零失敗、1 ignored（74.08 秒）；Phase 11 定向 80 library＋1 config 整合一般成功、零失敗、1 ignored（30.74／0.04 秒），兩組有重疊不相加。格式、零警告 Clippy（24.72 秒）、Debug（1.08 秒）與獨立 Release（1m04s）通過，13 個本批 Rust／Markdown 嚴格 UTF-8／無替代字元通過。此次沒有修改媒體或串流處理，不重跑未修改的完整媒體矩陣；前批完整 2099 是歷史基準，不冒稱本批新完整結果。

舊標準版 0FBFA316268C7BC67FDBCD710E6E7CFE9E3AC4EB0946C714E7F234F68C85E53C、9,627,648 bytes 已備份至 `target/phase11-diagnostics/prior-auto-preflight-45ebfc1d5d624e53a2ed7f82937c0800/subtitlenow.exe` 並核對一致。獨立 Release 新候選已完成；主視窗 PID 29612 仍在執行，已請使用者停止並關閉。標準版尚未覆寫或新開另一個主視窗，待實際關閉後直接更新，不要求重新儲存金鑰。新的選項介面與 Gemini Transcribe Live 真實 Auto 仍待人工驗收，Phase 11 未結案。

候選 `target/phase11-diagnostics/release/subtitlenow.exe` 核對為 SHA256 6677326A6720AF368F60F0444C08B6B95B481EC133B54D382896F68DFB92052F、9,629,696 bytes。8 份本批文件本機連結全部存在。主視窗關閉條件仍待完成，不自動終止使用者正在翻譯的程序。

## 2026-10-25：第一句回報後確認持續中文正常

使用者在 0FBFA316… 新版交付後先回報「翻譯只出現了第一句，後面就沒出現了」。詢問狀態時回覆「正常」，再明確以選項 1 澄清為「中文已持續更新，問題已消失」，而不是只有狀態正常。依此次人工澄清，將目前 Gemini 3.5 Live Translate 的單次連續中文顯示記為通過；不推論實際等待多久、發生原因、自動復原或長時間穩定性，也不擴大為其他語言／供應商或捲動／停止全部通過。

再次核對標準 `target/release/subtitlenow.exe` 為 0FBFA316268C7BC67FDBCD710E6E7CFE9E3AC4EB0946C714E7F234F68C85E53C、9,627,648 bytes；PID 29612 與新啟動的 PID 14560 皆從同一路徑執行、雜湊相同且 Responding=True。沒有擅自關閉任一視窗、操作開始或變更來源；後續請使用同一個正在正常翻譯的視窗測試。工具只檢查程序／檔案中繼資料，沒有取得金鑰、字幕或服務原始訊息。

此次僅查閱既有程式、測試與官方事件文件並更新人工紀錄；未修改 Rust、模型設定或執行檔，沒有新增／執行測試。前批 2099 項一般通過及其 187 項首輪失敗紀錄維持原意，不冒充本次新測試。暫停／恢復、捲動／回看、停止與其餘 Phase 11 人工條件仍待各自確認，Phase 11 未結案。

## 2026-10-25：Live Translate 無可見更新及大量字幕捲動

使用者回報新 Live Translate 沒有字幕動作，大量字幕最後行仍不可見；重新開啟標準 A0B244… 版本 PID 17432 後，人工回報 PID 6108／Gemini／正在翻譯、正確 gemini-3.5-live-translate-preview 與未啟用獨立內容服務，沒有錯誤文字。只核對程序、未讀取金鑰或字幕，正式測試由使用者明確開始。後續使用者已按停止並關閉，允許驗證後直接更新重開。

本機重現一：`phase11_native_partial_snapshots_continue_without_a_generation_completion` 在修正前失敗（0 成功、1 失敗，0.26 秒），第二個完整 partial 快照被一般 ASR 重譯保護擋住。修正只將完整 native 快照視為本次替換完成，不冒充語句 Final，不動一般 ASR 規則。此缺陷可解釋第一版為空白／短文字時一直等服務完成才可見，但尚未觀察這次正式原始回應，不宣稱全面根因已確認。

本機重現二：`phase11_native_scroll_end_and_returning_to_bottom_resume_follow` 修正前失敗（0 成功、1 失敗，0.36 秒），底部收到 SB_ENDSCROLL 會誤停跟隨。新修正於 EDIT 實際操作後核對 32 位元捲動底部，無選取／搜尋才恢復；跟隨動作移到版面換行後。首次較簡單、無主控制列的大文字原生測試通過（1 成功、51.59 秒），未據此宣稱問題不存在；後續測試改用真實主控制列，維持超過 65,535 UTF-16 單位、64 列淘汰、設定頁切換與真正最後文字行斷言，僅減少重複的大快照發布以降低測試耗時。設計見 [ADR 0099](adr/0099-native-caption-snapshots-and-bottom-follow.md)。修正後測試仍在進行，尚未更新標準版或宣稱實機通過。

修正後第一輪 Phase 11 定向完成：library 78 成功、零失敗、1 ignored（37.55 秒），config 整合 1 成功（0.02 秒）。新兩項失敗回歸均已通過，受控主頁的大文字／換行／64 列淘汰／設定頁返回亦通過；既有取消、Session、手動清除及普通 ASR 規則仍通過。接續原生視窗完整定向與品質／獨立 Release／完整串行回歸；未提前部署或宣稱人工通過。

原生視窗完整定向完成：57 成功、零失敗、1 ignored（99.80 秒），含本批長文字與底部跟隨；與 Phase 11 定向有重疊，不累計。開始最終格式、零警告 Clippy、Debug／獨立 Release 與 Release 後獨立串行完整回歸。標準舊版 A0B2441EA056670F4D366491AE29390DB51125EB329CAEFF7BD26700D01FDD2F、9,625,088 bytes 已備份至 `target/phase11-diagnostics/prior-native-snapshots-ebe41028886e4beea29a602b0016adc9/subtitlenow.exe` 並核對一致；使用者已回報關閉，部署前仍會重新確認。

品質第一輪格式、零警告 Clippy（22.24 秒）、Debug（1.51 秒）及獨立 Release（1m22s）通過。之後只將長文字測試的可見性斷言從末端標記第一個字元收緊至真正最後一個字元，未變更產品碼；重新執行格式／Clippy，再接 Release 後獨立串行完整回歸。11 個本批 Rust／Markdown 嚴格 UTF-8 與無替代字元檢查通過。仍未提前部署、未讀正式憑證或開始擷取，實機及正式服務保持待驗收。

收緊測試斷言後格式與零警告 Clippy 再次通過（23.99 秒）。Release 後獨立 `cargo test --locked --offline --quiet -- --test-threads=1` 已開始執行 2090 個 library 案例，尚未宣告完整通過。新版候選 `target/phase11-diagnostics/release/subtitlenow.exe` 為 SHA256 0FBFA316268C7BC67FDBCD710E6E7CFE9E3AC4EB0946C714E7F234F68C85E53C、9,627,648 bytes，尚未覆蓋標準版。

第一輪完整串行結果：1885 library 成功、187 失敗、18 ignored（777.10 秒），整合因 library 失敗未執行。首個 `video_mp4_stop_error_and_chinese_sidecar_preference_release_resources` 於 prepared_asr_tests.rs:126 回報 synthetic Prepared worker did not exit，後續完整 186 項均為共用 TEST_OWNERS poisoned 鎖連帶失敗；本批譯文／捲動回歸沒有失敗。保留失敗，不將前批串行成功當成舊間歇問題已修復。

未修改碼先以 RUST_BACKTRACE=1 單獨重跑首案，再度失敗（0 成功、1 失敗，7.90 秒）。backtrace 精確指出 assert_aac_stop_error_sidecar 的 1403 行第一個等待：13 秒原生 MP4 解碼後的 mock Network 失敗路徑，尚未進入 cancel／中文 sidecar。該 helper 沿用五秒短樣本預算，而同檔原生 ready／MP3 案例已用三十秒。只將原生失敗啟動及等候 mock 開始訊號改用既有三十秒 codec fixture 預算，保留取消與中文 sidecar 的五秒、全部檔案 pin／錯誤／服務呼叫／清理斷言；新增只含各階段耗時的合成測試診斷。未改產品碼、產品期限、worker 上限或忽略測試；修正後首案量測及完整重跑待結果，不提前宣稱媒體全面修復。

修正後首案以 --nocapture 量測通過（1 成功、零失敗，26.37 秒）：44.1／48 kHz 合成原生影片的失敗啟動 6.2628／6.2266 秒、等候 mock Pending 開始 6.1394／5.7182 秒，確實超過原五秒；觀察到的取消清理僅 1.8039／3.5691 毫秒，仍通過原五秒停止上限。這是測試啟動預算修正，不是產品解碼效能改善，也不把單次量測當成 SLA。先前 187 項失敗紀錄保留；接續重新執行格式／零警告 Clippy／Debug／Release 與獨立完整串行回歸，尚未宣告第二輪完整通過。

含測試啟動預算修正的格式、零警告 Clippy（22.90 秒）、Debug（1.12 秒）與獨立 Release（1.53 秒）均通過；產品碼未再變更。第二輪 Release 後獨立完整串行回歸正在執行。第一輪失敗不刪除、不與第二輪混算，也未宣稱媒體效能改善或正式服務已通過。

本批最後完整結果：第二輪 Release 後獨立 `cargo test --locked --offline --quiet -- --test-threads=1` 完成，2072 library 成功、零失敗、18 ignored（1213.16 秒）；整合 13＋4＋10＝27 成功（1.38／0.00／2.72 秒），共 2099 一般成功、零失敗。包含本批最後字元可見性與全部既有媒體案例，未執行 ignored 或長測。第一輪 187 失敗與單案量測完整保留，不把校正測試啟動預算宣稱為產品解碼效能改善。12 個本批 Rust／Markdown 嚴格 UTF-8／無替代字元及 7 份文件本機連結檢查通過。

部署前確認 SubtitleNow 無執行中程序，舊標準版與本節 prior-native-snapshots 備份的 A0B2441EA056670F4D366491AE29390DB51125EB329CAEFF7BD26700D01FDD2F 一致；新候選與更新的標準 `target/release/subtitlenow.exe` 雜湊 0FBFA316268C7BC67FDBCD710E6E7CFE9E3AC4EB0946C714E7F234F68C85E53C 一致，9,627,648 bytes。--ui 啟動 PID 29612、HasExited=False、MainWindowHandle=173215990、Responding=True；只核對程序，不冒充目視或正式服務驗收，沒有讀金鑰、代按開始或擷取音訊。請使用者以目前 Chrome／English／Gemini Live Translate／Standard、金鑰欄留空，明確開始驗證連續中文、暫停／恢復、最後行與手動回看／到底部恢復跟隨，最後停止。Phase 11 仍未結案。

## 2026-10-25：Live 模型與主頁來源／最後待譯段

使用者指定 gpt-live-transcribe 與 Gemini 3.5 Live Translate，並回報最後一段不可見、來源／語系要放主頁、設定頁不重複狀態。本批增加獨立 Live 模型與 schema 2 偏好，舊 schema 1 僅 RAM 遷移、舊供應商預設保持。GPT 真實 delay 依品質 minimal／low／medium。Gemini 新模式固定 gemini-3.5-live-translate-preview、zh-Hant，取原生譯文並作臺灣在地化；無假 ASR、無 HTTP 翻譯工作、無猜測原文配對／capture timestamp、無音訊播放或落盤。原文無保證對應，故明示僅譯文；Fast／Standard 真實合併，Auto／High／Prepared 定位或獨立內容設定組合拒絕，不靜默改模型。細節及官方依據見 [ADR 0098](adr/0098-live-model-profiles-native-captions-and-main-source-controls.md)。

來源／語言／品質改為共享主頁欄位，三操作常駐，系統狀態與錯誤只顯示主頁。開始或無音訊診斷先回主頁，來源／模型披露仍須可見確認後才啟動。SideChat 顯示最新原文待譯段，保持最近中文、同段配對與人工歷史錨點；未觸碰 ExactCueVerifier、capture-only LivePlaybackClock 或擷取時鐘。

第一輪 `cargo test --locked --offline phase11_ -- --test-threads=1`：68 成功、3 失敗、1 ignored（library 18.99 秒）。失敗為舊 GPT setup fixture 未含新 delay、兩個舊原生可見性預期仍將來源視為隱藏；精確修正為所選 profile setup 與共享來源可見，保留原隔離／幾何／金鑰斷言。第二輪 library 71 成功、零失敗、1 ignored（22.34 秒），新增 config 整合 1 成功（0.06 秒），有重疊不累計。原生純譯文、有界通道、真實 localhost WebSocket／取消、清除防回填、臺灣在地化與無 ASR／額外工作測試均通過。後續補充主頁／診斷／披露測試及完整品質檢查仍在進行；未更新標準執行檔或宣告正式服務通過。

補充測試後：原生視窗定向 55 成功、零失敗、1 ignored（61.78 秒），Phase 11 定向 library 75 成功、零失敗、1 ignored（21.01 秒）＋config 整合 1 成功（0.02 秒）。兩組有重疊不累計。主頁／設定頁可見性、三模型選取傳遞、診斷回主頁、精確 metadata 名稱、供應商不一致拒絕及換來源重設模型披露通過。格式、Clippy、Debug／獨立 Release 與最後完整串行回歸接續執行；標準版尚未更新，正式服務、外觀及延遲未宣稱通過。

最終品質：首次 Clippy 拒絕新增分支的 suspicious_assignment_formatting 及兩處 collapsible_if；依建議改寫，沒有 allow 或隱藏警告。重新格式檢查、零警告 Clippy（49.03 秒）、Debug（21.63 秒）及獨立 Release（1m04s）通過。Release 後獨立 `cargo test --locked --offline --quiet -- --test-threads=1` 已啟動，尚未宣告完整通過。標準版 3C3C4A0655AC5E2D4FA79EF0864E07E12E33429828F9E554EF0A847B29CD648B、9,579,008 bytes 已備份至 `target/phase11-diagnostics/prior-live-models-8b3999d890e446b5a52f1dffdf6d2a15/subtitlenow.exe` 並核對一致，尚未覆寫。33 個本批 Rust／Markdown 檔案嚴格 UTF-8、無替代字元檢查通過；隨後另更新固定金鑰說明頁，最後再核對。

完整最終結果：最後 Release 後獨立串行命令完成，2069 library 成功、零失敗、18 ignored（780.44 秒）；整合 13＋4＋10＝27 成功（0.80／0.00／1.36 秒），共 2096 一般成功、零失敗。未執行 ignored 或長測，前批原生媒體間歇逾時根因未宣稱修復。包含固定金鑰說明的 34 個本批 Rust／Markdown 嚴格 UTF-8、無替代字元檢查通過，8 份本批文件本機連結全部存在。

部署前重新確認沒有 SubtitleNow 程序，標準舊版與備份 3C3C4A0655AC5E2D4FA79EF0864E07E12E33429828F9E554EF0A847B29CD648B 一致。新候選及更新標準 `target/release/subtitlenow.exe` 的 SHA256 A0B2441EA056670F4D366491AE29390DB51125EB329CAEFF7BD26700D01FDD2F 一致，9,625,088 bytes。--ui 啟動 PID 30696、HasExited=False、MainWindowHandle=302255626、Responding=True；只核對程序中繼資料，不冒充目視。未讀取正式金鑰、未開始擷取、未發送正式服務要求。請使用者在主頁選目前自有 Chrome／English／Standard，服務頁明選 Gemini 3.5 Live Translate、金鑰欄留空並儲存偏好，回主頁明確開始；核對中文持續更新、暫停收尾／保留／恢復／停止及版面。OpenAI 實際測試另由固定遮蔽欄手動處理，Phase 11 未結案。

## 2026-10-25：主頁操作、獨立服務設定及自動跟隨

使用者選擇前版五分鐘以上持續翻譯、停止與紀錄保留正常，另指出速度仍慢，要求金鑰／服務獨立選單且可儲存，開始／停止留主頁並與清除同區，設定不強制靠右，直式字幕須持續跟隨。

本機先加原生 EN_VSCROLL 非人工通知回歸，修正前 0 成功、1 失敗（0.27 秒），第 2 段即停用 auto_follow。改採本視窗 EDIT 輸入 subclass、refresh／layout 可巢狀旗標後單項通過（5.79 秒）；當時出現舊 PANEL_HEIGHT 未使用警告，已移除，未隱藏警告。新增 90 段跨容量、實際手動 scroll、回即時原生底部、獨立頁／原位展開／偏好儲存及實際設定檔往返測試。第一輪完整原生 52 成功、1 失敗、1 ignored（54.25 秒），失敗是舊測試仍在來源頁要求金鑰欄可見；改為開啟獨立服務頁，保留遮蔽／啟用／標籤尺寸／不重疊等斷言。第二輪完整原生 53 成功、零失敗、1 ignored（54.19 秒）；設定檔單項成功（0.03 秒），核對其他 display mode 不被覆蓋，無所選服務或未知 schema 不覆蓋原檔。格式／Clippy／建置及完整回歸接續中，尚未部署。見 [ADR 0097](adr/0097-main-actions-service-settings-and-history-follow.md)。

沒有修改雲端模型、品質預設、期限、並行、擷取時鐘或仲裁；本批沒有讀取正式憑證或啟動擷取／服務。速度仍待真實來源量測，不以 native fixture 時間當作遠端 latency。前批媒體原生間歇逾時亦未宣稱修復。

## 2026-10-25：多分鐘後翻譯逾時

使用者回報手動清除／停止保留／重新開始上捲均正常，但放置數分鐘後 PID 6108／Gemini 出現「翻譯要求逾時，未顯示過時結果」。此人工正例只涵蓋本次所選 Chrome／英文／Gemini／Standard 流程，不視為所有播放器或多分鐘穩定性驗收完成。

本機重現：新增第一段 Timeout、第二段成功的明確 mock，修正前單項測試失敗（0 成功、1 失敗，0.01 秒），實際回傳 Err(Timeout)，預期不中斷 Live。修正隔離單次期限失敗，完整來源／修訂／job 核對後退休該要求，三次連續失敗仍升級原本有限復原；其他錯誤與 Prepared 維持契約。前次編譯後此單項通過（0.01 秒），後續完整補測／品質驗證接續中，尚未部署。見 [ADR 0096](adr/0096-live-translation-deadline-isolation.md)。未讀取正式憑證、未啟動擷取或雲端測試，遠端變慢原因未確認。

## 由使用者清除字幕，停止與重連保留畫面（2026-10-25）

使用者回報 PID 22588「基本 ok」，仍偶發全字幕清空，明確要求改為往上捲並由新按鈕手動清除。已更新 SPEC／AGENTS 的顯示保留例外；後端仍取消舊工作與釋放 Session 資源，保留的是本次開啟期間最多 64 列的 UI RAM 文字，不供網路或定位重用。每列完整身分避免跨連線同號覆寫，晚到舊工作仍拒收；先前連線／撤回來源的文字標示「先前字幕」。

增加固定「清除字幕」按鈕及選單操作。清除不停止辨識，設定最高已收段落水位防止舊完整快照回填；下一段正常顯示。搜尋／選取不再於停止與重連重設，完整身分與本文起點保護歷史錨點；保留／已清除的列不冒稱新段落 UI latency 樣本。見 [ADR 0095](adr/0095-user-owned-bounded-subtitle-display-history.md)。

迭代紀錄：第一輪 UI 80 項成功、3 項舊自動清除預期失敗（0.26 秒）；依新使用者政策改為停止保留、手動清除，保留舊身分與晚到結果拒收斷言。初次原生重跑 50 成功、1 失敗、1 ignored（34.10 秒）：新增按鈕擠壓 760 DIP 短視窗的既有 48 DIP 字幕空間；未降低門檻，改採寬視窗同行／窄視窗獨立列，重跑 51 項成功、零失敗、1 ignored（34.51 秒）。UI 85 項（0.23 秒）、Live 16 項（0.71 秒）、Phase 11 一般 52 項通過、1 ignored（2.76 秒），重疊不累計。格式與 Clippy 通過後，再補來源撤回時不可用空譯文抹除舊配對的顯示回歸，最終驗證接續中，尚未部署。

來源撤回保留回歸加入後，UI 全部 86 項通過（0.26 秒）；最後 Phase 11 一般 53 項通過、1 項既有 ignored（2.75 秒），包含實際按鈕、跨連線選取、80 個 Session 容量及來源撤回。最後 UI mailbox 暫停案例也改核對「後端停止後仍保留顯示、手動清除才清空」；沒有取消後端終止／舊快照拒收斷言。格式通過，最終 Clippy／建置／獨立完整回歸接續中。

最終零警告 Clippy 通過（25.22 秒），Debug 建置通過（1.14 秒）。使用者已回覆關閉舊程式，並再次確認無同名程序；舊標準版 9,548,800 bytes、SHA-256 `C06068F5F0FE5DE56E82ED198A12A2FD304458129AF1D674818080705383C681`，已備份至 `target/phase11-diagnostics/prior-manual-clear-fe7e9ab23a3d49839036002e72819af3/subtitlenow.exe` 並核對相同雜湊。

獨立 Release 建置通過（1 分 03 秒）。候選 `target/phase11-diagnostics/release/subtitlenow.exe` 為 9,539,072 bytes、SHA-256 `3C38A564A08E94AABF5EBAFF8A766EC2595062B0424D057E32E507193340AACD`。接續以 `cargo test --locked --offline --quiet -- --test-threads=1` 執行 Release 後獨立完整回歸；沿用前批已記錄的單一測試工作模式，不改產品／測試期限或聲稱既有媒體 worker 間歇逾時已修復。

最後上述獨立完整回歸通過：2047 library＋12／4／10 整合＝2073 一般測試成功、零失敗、18 既有 ignored，library 816.59 秒。未重跑 ignored 或長測，沒有將重疊定向數據相加，既有媒體間歇逾時仍待追查。

完整回歸後再確認無同名程序，核對標準舊版／備份／候選雜湊後更新 `target/release/subtitlenow.exe`，新雜湊與獨立 Release 相同（`3C38A564A08E94AABF5EBAFF8A766EC2595062B0424D057E32E507193340AACD`）。新版 `--ui` PID 28940 存活、有非零主視窗 handle 且回應，沒有自動擷取、讀取正式金鑰或目視正式畫面。

下一次人工驗收：原 Chrome／英文／Gemini／Standard、金鑰留空，停止應保留紀錄、重新開始後新段上捲；按「清除字幕」才清空，處理不停止、舊段不回填，新段照常顯示。正式服務及新外觀仍待回報，Phase 11 未結案。

## 影片暫停時完成待譯並保留字幕（2026-10-25）

使用者回報 PID 27032 仍有長句更新過快，影片暫停後停止翻譯並消失，要求完成剩餘翻譯及保留字幕。已詢問消失時的系統狀態以區分正常閒置與真正錯誤，尚不把其他來源／服務或長句人工驗收標記通過。

後續使用者提供的完整狀態仍為 PID 6108／Gemini「正在翻譯」，沒有錯誤文字；因此不將實際消失直接歸因重連。另確認顯示缺口：新尾段尚無中文時 overlay 原本取最後列的空譯文，SideChat 自動跟隨亦可能被純原文長尾推走中文。本批改為從有效同 Session 快照保留最近有中文的同段字幕、自動跟隨不投影未譯尾段；手動原文歷史與搜尋仍完整可見，新段中文可用才交接。使用者已回覆關閉舊視窗。

新增 localhost 重現先在舊程式實際失敗：服務只在收到 `audioStreamEnd` 才交付 Final，四秒內沒有新 PCM 就無法完成（4.01 秒），沒有真實音訊、金鑰或雲端請求。依官方契約補上有界輸入閒置收尾：1100 ms 無新資料且真實三段佇列排空，送一次結束輸入，服務 Final 仍以真正回覆為準；恢復 PCM 使用原連線、Session。翻譯與 UI 不因正常閒置停止或清空，八秒 Final／心跳／輪替／翻譯期限及真正停止清理保持。見 [ADR 0094](adr/0094-gemini-idle-input-drain-and-caption-retention.md)。本批定向、完整回歸與部署結果接續記錄。

加入顯示保留後的定向結果：Phase 11 一般 47 項通過、1 項既有 ignored（2.53 秒）；完整串流模組 26 項通過、2 項既有 ignored（6.06 秒），UI mailbox 30 項（0.24 秒）、Presentation 4 項通過。原生視窗第一次 44 項成功、4 項失敗、1 ignored（36.93 秒）：四項捲動 fixture 在自動跟隨時尋找尚未翻譯的尾段，與新保留規則衝突，均在字串搜尋 unwrap 失敗，非已找到內容後的選取斷言失敗。將該捲動 fixture 改為明確有合成譯文的可見歷史，保留原 UTF-16／淘汰／選取／捲動斷言；另新增真正原生 pending 尾段／手動回看／新中文交接／停止清除回歸，接續重跑。重疊案例不累計。

最終定向重跑：原生視窗 49 項成功、零失敗、1 既有 ignored（46.99 秒）；Phase 11 一般 48 項成功、零失敗、1 既有 ignored（3.33 秒）。格式、零警告 Clippy（52.66 秒）、Debug 建置通過，獨立 Release 與其後完整回歸接續中。沒有放寬產品／測試期限或修改前批原生媒體間歇問題。

獨立 Release 建置通過（2 分 09 秒），接續執行 `cargo test --locked --offline --quiet -- --test-threads=1`。沿用前批單一測試工作模式以降低既有原生媒體案例間競爭；各案例內部並行與取消仍啟用，不改期限，原間歇逾時並未因本批被宣稱修復。

已確認舊程式關閉並備份標準版至 `target/phase11-diagnostics/prior-idle-caption-9697a5d8b3d948058f3681020e8f1a83/subtitlenow.exe`，9,549,824 bytes、SHA-256 `177B0B7F27EE0A11F3D8ED7A69006FF1EC0F1BB4DE63521A806E9B25C8EB285F`，備份與更新前標準版相同。

最後 Release 後獨立完整回歸通過：以單一測試工作執行，2042 library＋12／4／10 整合＝2068 一般測試成功、零失敗、18 既有 ignored，library 1035.87 秒。沒有重跑 ignored 或長測、沒有調整期限；前批媒體 worker 間歇逾時仍待追查，不將本輪通過當成根因已修復。

本批獨立 Release `target/phase11-diagnostics/release/subtitlenow.exe` 為 9,548,800 bytes，SHA-256 `C06068F5F0FE5DE56E82ED198A12A2FD304458129AF1D674818080705383C681`；完整回歸成功後再確認無同名程式，核對舊標準／備份／候選雜湊，更新 `target/release/subtitlenow.exe` 並核對相同新雜湊。新版 `--ui` PID 22588 存活、有非零主視窗 handle 且回應，僅為待命確認，沒有按開始、讀取正式金鑰、擷取 Chrome 或目視畫面驗收。

下一次人工測試為原 Chrome／英文／Gemini／Standard、金鑰留空：播放約十秒、暫停約十秒，確認待譯完成且字幕保留；恢復後接續翻譯，最後明確停止才清除。若仍異常只回報狀態框，勿提供金鑰或字幕本文。保留只在同一有效 Session，真正錯誤／重連及來源切換仍清理；新修正未被標成正式 Gemini 通過，Phase 11 未結案。

## 持續播放通過後的長句可讀性與側邊預設（2026-10-25）

使用者回覆 PID 5192 的原 Chrome／英文／Gemini／Standard 測試「1」：持續播放就有中文，停止正常；同時確認語速快、句子長時，同段中文被下一版整段換掉，來不及讀。此為單次人工正例，非其他語言、來源、服務或延遲 SLA 通過。使用者已停止並關閉舊程式。

共用時間軸在已有可讀中文且原文修訂時，保留舊暫定中文，直到下一版完整輸出再替換，第一版仍即時串流；不以長度猜測語意或捏造穩定／定稿。資料驗證先執行，沒有增加本文歷史副本、第二條處理管線、音訊等待或放寬期限。

依使用者指定，側邊預設先右側定位再自由移動，外框半螢幕解析度高度、寬為該高度三分之一，限制在工作區且不二次套用 DPI。窄字幕模式收合來源／金鑰區，從「選單 → 來源與金鑰設定／回到字幕視窗」展開及還原；系統狀態與字幕保持分框。側邊採 Windows 的每螢幕 DPI 系統訊息字體；底部獨立外觀選項保持。未啟用內容辨識時改明示歌曲／影片獨立服務未啟用、不影響即時翻譯，未配置或啟動其他服務。見 [ADR 0093](adr/0093-readable-draft-replacement-and-compact-side-defaults.md)。本批測試／部署結果接續記錄，尚未宣稱新外觀與長句人工通過。

本批定向結果：Phase 11 一般 42 項通過、1 項既有 ignored（2.79 秒）；完整原生視窗模組 48 項通過、1 項既有 ignored（32.09 秒），其中直接 tests 21 項另行通過（3.20 秒）。翻譯核心 22 項（0.02 秒）、Live source 16 項（0.73 秒）、內容提示 3 項通過；重疊案例不重複累計。格式、零警告 Clippy（38.72 秒）、Debug 與獨立 Release 建置（1 分 20 秒）通過，第一輪完整回歸失敗及最後串行成功另列下方。未擷取 Chrome、讀取正式金鑰或重新嘗試失敗的畫面擷取工具。

舊標準版 `90BD10A2D93710AEB898DCF76498932FA2D92381C48EAAE4FE6D2BE9B3093A7D`、9,531,904 bytes，已備份至 `target/phase11-diagnostics/prior-readable-side-a580164dea734ab384d578c5675ac127/subtitlenow.exe`，雜湊一致；確認無同名程式，完整回歸通過後才更新標準版。

獨立 Release `target/phase11-diagnostics/release/subtitlenow.exe` 為 9,549,824 bytes，SHA-256 `177B0B7F27EE0A11F3D8ED7A69006FF1EC0F1BB4DE63521A806E9B25C8EB285F`；最後完整回歸成功後核對舊標準／備份／候選雜湊，更新 `target/release/subtitlenow.exe` 並核對相同新雜湊。新版 `--ui` PID 27032 存活、有非零主視窗 handle 且回應，僅待命，不自動擷取或連線。新外觀未以畫面擷取工具目視驗收。

第一輪 Release 後完整回歸失敗：1849 library 通過、187 失敗、18 ignored，476.58 秒，整合測試未接續。唯一非 PoisonError 的首錯為既有 `video_mp4_stop_error_and_chinese_sidecar_preference_release_resources` 在 `prepared_asr_tests.rs:126` 回報 `synthetic Prepared worker did not exit`，即既有五秒 terminal 期限；其餘 186 項皆為共用 `TEST_OWNERS` 鎖 poisoned 連帶失敗。單獨 exact 重跑先再次失敗（13.27 秒整案），加 backtrace 再診斷則通過（16.66 秒整案），未取得失敗的詳細呼叫階段，不能斷言根因已修復或歸因本批 UI。沒有延長期限、改媒體程式、清除 poisoned 後冒稱通過或部署失敗版本。接續以 `cargo test --locked --offline --quiet -- --test-threads=1` 執行獨立完整回歸，減少測試案例間原生資源競爭；各案例內部既有並行、取消與容量測試保持。

最後上述獨立串行完整回歸通過：2036 library＋12／4／10 整合＝2062 一般測試成功、零失敗、18 既有 ignored，library 756.02 秒。沒有重跑 ignored 或長測、沒有修改產品或測試期限；先前影片 worker 間歇逾時仍是已知待查問題，串行成功不證明其根因已解決。新 GUI 長句可讀性、預設比例與設定切換待使用者實測，Phase 11 未結案。

## 持續語音翻譯要求被修訂反覆取消（2026-10-25）

使用者在連續轉錄／分框版回報：Gemini 已顯示「正在翻譯」，但中文要暫停 YouTube 才出現；此結果不能當作即時中文驗收通過。已在本機重現：每 40 ms 的相容 Partial 修訂與 240 ms mock 翻譯反覆互相取消，無 Final／暫停時未出中文，新增回歸實際失敗（1.21 秒）。沒有擷取使用者內容或使用正式金鑰來重現。

Live 入口新增每段最新一筆、有界的相容 Partial 合併，讓已啟動要求完成串流，不再每版都取消。回退、Final、來源 metadata 變更與停止仍走原取消；核心修訂／job 核對不放寬，共享時間軸不重建。合併原文接收時記錄三秒期限，釋放時不重設。設計與取捨見 [ADR 0092](adr/0092-live-partial-translation-coalescing.md)。

使用者已回覆停止並關閉舊視窗；完整回歸通過後再確認無同名程序，已更新並啟動新版。真實持續播放中文與停止仍待回報，Phase 11 未結案。

修正後 LiveTranslation source_tests 全部 16 項通過（0.90 秒），含四項新回歸：不停更新的 Partial 在沒有 Final／暫停時即產生合成中文、每段只保留最新及 Final／回退取消、期限不重設／停止不重送、多批中文串流完整輸出後才接續合併來源。原翻譯核心 22 項通過（0.04 秒）；Phase 11 一般定向 38 項通過、1 項既有 ignored（2.63 秒），重疊案例不重複累計。這些為明確 mock 排程驗證，不是正式 Gemini 的延遲 benchmark。格式、零警告 Clippy、Debug 與獨立 Release 建置通過；Release 後獨立完整回歸 2032 library＋12／4／10 整合＝2058 一般測試通過、零失敗、18 既有 ignored，library 560.21 秒。未重跑 ignored 或原生長測；前批數據保留，不累計成新證據。

獨立 Release 為 `target/phase11-diagnostics/release/subtitlenow.exe`，9,531,904 bytes、SHA-256 `90BD10A2D93710AEB898DCF76498932FA2D92381C48EAAE4FE6D2BE9B3093A7D`。舊標準版 9,551,360 bytes、SHA-256 `A242AFE9B6B23767568C088DA66B2A31035D1A8DEEDDDD111E227195BD48B624`，已備份至 `target/phase11-diagnostics/prior-partial-coalescing-6f796cd7aac641f89c3dcace39833493/subtitlenow.exe` 並核對相同雜湊。完整回歸通過後更新 `target/release/subtitlenow.exe`，新標準版雜湊與獨立 Release 相同，`--ui` 新 PID 5192 存活、主視窗 handle 非零且回應。這是程序待命確認，沒有按開始、讀取金鑰、擷取音訊或重新嘗試已兩次逾時的畫面擷取。

下一次人工測試：原 Chrome 英文內容、英文／Gemini／Standard、Gemini 金鑰欄留空，核對當前來源後開始；保持 YouTube 播放，確認中文是否在不停播時出現並持續更新，再按停止。只回報狀態或錯誤，不貼金鑰或私有本文。只有這次結果才能確認正式服務的持續中文與停止，不以 mock 延遲宣稱符合效能目標。

## 連續轉錄與狀態／字幕分框（2026-10-24）

使用者在 PID 23016 回報只有英文、仍反覆清除，最終明確為音訊傳送佇列滿（12 項），並建議將錯誤與字幕分框。舊佇列取出只在等 Final 時停下，由路徑推論問題仍涉及等候最終結果；不能由此宣稱服務端究竟延遲多久。

Gemini 改用官方連續音訊／自動活動偵測，固定模型、語言、WSS、TLS 與所選服務保持；不再把本機能量段落當成服務轉錄 ID，不等 Final 而累積下一段 PCM。轉錄使用 server utterance 順序及修訂，未知精確擷取時間保持 None；沒有以回覆抵達時間補造來源時間，也不繞過精確驗證／仲裁。詳細取捨與限制見 ADR 0091。

原生主視窗新增獨立「系統狀態與錯誤」標籤／只讀可捲動框架。字幕框不再前置狀態／錯誤，搜尋與捲動錨點只處理字幕，狀態未變不反覆重設框架；底部字幕也只顯示字幕內容。真正停止／重連仍清除舊 Session，錯誤原因留於狀態區，沒有保存舊 Session 本文。

第一輪編譯發現新分支引用不存在的 audio_message helper，已改用既有 PCM 序列化；另刪除已無用途的字幕 header prepend 方法，未壓掉 warning。延後 Final 測試第一輪將初始 Ping 誤當結束而失敗，已修正本機測試服務對控制訊息的處理，沒有放寬產品音訊期限。

定向結果：Phase 11 一般 34 項通過、1 項既有 ignored（2.80 秒）；原生字幕視窗 19 項通過（1.76 秒），驗證狀態／字幕分離、狀態變化不改本文、兩區不重疊及原停止／Session 隔離。串流模組 23 項通過、2 項既有 ignored（6.03 秒），包含一秒才回 Final 的持續新鮮音訊、四段 10 ms 音訊、兩服務五語及取消。Standard 未知時間的 Partial／Final 翻譯為明確 mock，只證明流程可產生中文，不宣稱正式 Gemini 翻譯已成功。最後只補欄位名稱澄清與 Auto VAD 配置檢查，最終格式／Clippy／建置及完整回歸接續中，尚未部署，Phase 11 未結案。

最終格式檢查通過；Clippy 第一次僅因測試手寫範圍判斷而失敗，已改用 Range::contains，沒有忽略 lint。再次零警告 Clippy 通過（21.39 秒），Debug 通過（18.86 秒），Release 通過（1 分 01 秒）。新版 `target/phase11-diagnostics/release/subtitlenow.exe`，9,548,800 bytes，SHA-256 `E37FC4FB95AB308EDD27B4526A2C909254F9C294E9F8328EC3E0B7F72CE42710`。

舊標準版 SHA-256 `AF7F8E6832EB18E7E4975CFF5BB3907830AC97D7A3B7AB385C958912D31F2270` 已備份至 `target/phase11-diagnostics/prior-gemini-continuous-c34e4b82a8864bf19b60ea5bc827f0c1/subtitlenow.exe` 並核對相同雜湊。Release 後完整回歸執行中，尚未更新標準版或使用正式金鑰。審查確認效能的擷取起點關聯依 event.start_us 查找；None 不會誤用 server 段落序號量測擷取延遲。對齊／精確驗證仍需原本的完整可信時間，沒有改動其採用規則。

第一輪完整回歸未通過：2024 library 通過、3 失敗、18 ignored，565.76 秒；三項為內容／歌詞／字幕服務狀態測試，仍假設狀態位於字幕框。已將原狀態檢查移到新的獨立狀態框，保留原字幕／snapshot／候選未採用／停止清除檢查，並明確檢查狀態不進字幕。這輪結果保留為失敗，不當成完整通過；標準版未更新。

另補短版控制視窗的配置：狀態框在低高度時縮小並以自身捲動顯示診斷，優先留下字幕高度；新增含完整控制面板的 760 DIP 高度原生幾何測試。未調整系統 DPI 或使用者視窗／顯示設定。針對整個 subtitle_window 模組（含先前未在十九項直接視窗測試內的 interaction／subtitle_source）重新回歸，最終檢查與新一輪完整回歸待結果。

整個原生字幕視窗模組已重新通過：47 項成功、零失敗、1 項既有 ignored（36.45 秒），含三項先前失敗的狀態分框與新短視窗案例，沒有排除失敗案例或放寬字幕／候選採用檢查。格式再查通過；最終 Clippy／Debug／Release 與第二輪完整回歸接續，標準版保持前版。

2026-10-25 接續確認：最終格式、零警告 Clippy（28.76 秒）、Debug（0.98 秒）、Release（1 分 00 秒）皆已執行通過；最後候選為 9,551,360 bytes，SHA-256 `A242AFE9B6B23767568C088DA66B2A31035D1A8DEEDDDD111E227195BD48B624`。它包含短視窗高度調整，取代前述未部署的 E37FC4… 候選。標準版與 `prior-gemini-continuous-c34e4b82a8864bf19b60ea5bc827f0c1` 備份仍為相同 AF7F8E… 雜湊，確認無 SubtitleNow 程序；第二輪 Release 後獨立完整回歸已啟動，尚未宣告成功或更新標準版。

### 連續轉錄／分框版完整驗證與部署完成（2026-10-25）

最後候選 Release 後獨立 `cargo test --locked --offline --quiet` 通過：2028 library、零失敗、18 ignored，577.55 秒；整合 12＋4＋10 通過，共 2054 項一般測試。本批新增未知時間翻譯與短視窗兩項一般案例；沒有新增或執行 ignored，也沒有重跑原生時鐘／三十分鐘 soak／三百次重啟。第一輪失敗與修正保留於前節，不重算為通過或以局部代替完整回歸。

部署前再確認無 SubtitleNow 程序，核對標準與備份舊 SHA-256 `AF7F8E6832EB18E7E4975CFF5BB3907830AC97D7A3B7AB385C958912D31F2270`，最後候選為 `A242AFE9B6B23767568C088DA66B2A31035D1A8DEEDDDD111E227195BD48B624`。已更新標準 `target/release/subtitlenow.exe` 並核對相同新雜湊，9,551,360 bytes；回復路徑為本節 `prior-gemini-continuous-c34e4b82a8864bf19b60ea5bc827f0c1`。

新版 --ui 已啟動（PID 3248），主視窗 handle 7736228、存活且 Responding。computer-use 僅選取該程式的唯一返回視窗「SubtitleNow — 即時字幕」，未讀 accessibility 本文／金鑰、未按開始；第一次唯讀截圖回報 FrameArrived timed out，重新列舉同一唯一視窗並啟用後，重試仍回報 window capture timed out。依技能復原規則停止重試，未取得可檢查畫面，不宣稱目視成功；之後程序中繼資料仍顯示回應。原生框架／幾何測試的通過與實際外觀驗收分開。

仍需使用者手動核對分框，並以原 Chrome 英文來源／英文／Gemini／Standard／金鑰欄留空啟動，確認中文、持續顯示與停止，或回報獨立狀態框的新錯誤。沒有擷取 Chrome、取用正式金鑰或代作雲端測試；實際故障／中文與其他 Phase 11 人工項目保持未通過，尚未結案。

## Gemini 最終轉錄屏障與中斷空畫面（2026-10-24）

使用者在數值診斷版（PID 30752）回報改為「語音辨識佇列已滿」，仍有英文原文短暫出現後被「等待字幕」替換。此訊息原先合併音訊輸入、重採樣、傳送、辨識事件與段落限制，無法僅由人工回報認定實際填滿哪個佇列。

核對 [Google 轉錄專用文件](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe)：`interimInputTranscription` 是暫定文字，`inputTranscription` 是語音段落的最終轉錄。舊程式誤把對話代理的 `turnComplete` 也列為放行下一段的必要條件。新增純協定回歸只回覆兩段最終轉錄、不提供代理完成訊號；修正前實際失敗（0.01 秒），第一段最終結果後 current 仍未釋放。此為已重現的程式缺陷，不代表已證實 Chrome 那次佇列溢出的唯一原因。

修正保留同時最多一段待最終結果及原音訊期限，在已送 activityEnd 且收到最終轉錄時釋放；延後的 `turnComplete` 不會提早釋放後一段。新增八種具名佇列／段落容量錯誤，根因仍為 Overloaded，共用三次復原預算與原容量不變。預處理輸出關閉改為工作已結束，不冒稱佇列已滿。

UI 已確認在錯誤／重連時清空 Session snapshot，原本卻一律顯示「等待字幕」。改用真實狀態的空畫面提示，說明中斷或重新連線／字幕已清除；不保留或復活舊 Session 本文。真實 Windows 控制項回歸補查重新連線、錯誤與停止的文字及隔離，未操作使用者視窗。工程驗證、部署及後續人工結果待追加，Phase 11 未結案。

定向驗證：修正後串流模組 22 項通過、2 項既有 ignored（6.00 秒）；新增連續四段／10 ms 原生 packet 形狀的本機 WebSocket 案例，只回覆最終轉錄，模擬 30 ms 回覆延遲，逐段驗證身分、序號、480 ms 長度與取消。最終 Phase 11 一般定向 33 項通過、1 項既有 ignored（2.76 秒），其中包含八種具名佇列錯誤與原始 Overloaded 相同的三次延遲／預算。Windows 原生控制項生命週期案例通過（0.58 秒），重連／錯誤不顯示「等待字幕」、清除舊本文、拒絕舊 Session、停止仍正確。定向案例有重疊，不累計為不重複總數。

格式、零警告 Clippy（48.82 秒）、Debug（0.94 秒）與 Release（56.30 秒）通過。獨立新版 `target/phase11-diagnostics/release/subtitlenow.exe`，9,547,776 bytes，SHA-256 `AF7F8E6832EB18E7E4975CFF5BB3907830AC97D7A3B7AB385C958912D31F2270`。舊標準版 SHA-256 `5E0A9FC6DD62617C474B69772441A1CD06BC6E98BBAAAF92F632E0814FFD9306` 已備份至 `target/phase11-diagnostics/prior-gemini-final-gate-8cbd1ed708bf4c2c89c6509b8be5cfe0/subtitlenow.exe` 並核對相同雜湊。Release 後獨立完整回歸執行中；標準版尚未更新、未使用金鑰或擷取 Chrome。

### 最終轉錄屏障修正版驗證與部署完成

Release 後獨立完整回歸通過：2026 library、零失敗、18 ignored，504.98 秒；整合 12＋4＋10 通過，共 2052 項一般測試。本批新增三項一般測試，沒有增加或執行 ignored，沒有重跑原生時鐘／三十分鐘 soak／三百次重啟。安全檢查確認沒有更動原生 callback、helper IPC、音訊期限／容量、定位／仲裁、固定模型／端點、TLS、語言／品質或金鑰流程；新增錯誤只保留固定具名邊界及數值，沒有持久化內容。

部署前再次確認沒有 SubtitleNow 程序，核對標準版與備份舊雜湊 `5E0A9FC6DD62617C474B69772441A1CD06BC6E98BBAAAF92F632E0814FFD9306` 及新版 `AF7F8E6832EB18E7E4975CFF5BB3907830AC97D7A3B7AB385C958912D31F2270`。標準 `target/release/subtitlenow.exe` 已更新並核對相同新雜湊，9,547,776 bytes；舊版回復路徑沿用本節 `prior-gemini-final-gate-8cbd1ed708bf4c2c89c6509b8be5cfe0`。

新版 --ui 已開啟（PID 23016），主視窗 handle 10817608、存活且 Responding。僅檢查程序中繼資料，未讀取金鑰、擷取 Chrome 或代按開始。需由使用者維持原 Chrome 英文來源／英文／Gemini／Standard／金鑰欄留空，明確啟動後確認英文、臺灣繁中與停止，或回報完整新錯誤／具名佇列容量。已修正可重現的程式缺陷，真實故障與 Phase 11 仍待人工結果，未宣告結案。

## 零位置修正後仍中斷：數值連續性診斷（2026-10-24）

使用者在 PID 1532 版本以原 Chrome 英文來源回報相同 Receive／DiscontinuityFlag，另有短暫英文原文出現後消失。這表示已到部分辨識輸出，不能當成臺灣繁中或持續辨識驗收通過；也不能由彙總旗標認定為原生 Windows 丟音。錯誤後 Session 清理可能解釋文字消失，但尚未確認該次狀態轉移。

新增明確 ignored 原生案例 `win32::soak_tests::native_process_capture_active_clock_metadata_is_continuous`，只建立自有低振幅合成音程序，檢查一千包與九次播放／靜音切換。首次執行通過（10.29 秒）：後續旗標 0、位置缺口 0、未知時間 0，stop 後 worker 歸零。沒有擷取 Chrome、保存音訊或使用正式服務，尚未重現使用者的剩餘問題。

辨識取出端改為先指出可直接觀測的序號／位置缺口，另區分帶旗標且位置未知；其他彙總旗標仍保留原訊息。所有同樣的失敗條件仍拒收，首次旗標容許、250 ms 期限、共用有限重試與 Session 隔離不變。只在失敗時附前後序號、前包 frames、相對預期位置差、相鄰 QPC 差與旗標；未知明示，不加入絕對時間、來源識別、PCM、字幕或供應商本文，不更動 helper IPC 或原生 callback。

新增回歸要求佇列／位置缺口即使同時帶不連續旗標仍正確分類，並驗證數值與第一個取消原因。定向測試首次編譯因測試使用 `unwrap_err` 要求音訊型別提供 Debug 而失敗，已只改測試取錯誤方式，沒有替音訊增加可輸出內容的 Debug。最終工程驗證與部署結果待追加；Chrome、實際翻譯與 Phase 11 仍未通過。

定向檢查已通過：`cargo fmt --check`、零警告 Clippy（50.19 秒）、十四項語音邊界測試（0.30 秒）、三十項 Phase 11 一般測試（1.66 秒，另 1 項既有 ignored，重疊案例不重複累計）、Debug build（0.79 秒）。定向測試曾使用過不正確模組篩選而執行零項，已改為 `ai::speech::tests::` 並確認十四項實際執行，不以零項當作測試通過證據。

獨立 Release build 通過（1 分 06 秒）：`target/phase11-diagnostics/release/subtitlenow.exe`，9,529,856 bytes，SHA-256 `5E0A9FC6DD62617C474B69772441A1CD06BC6E98BBAAAF92F632E0814FFD9306`。標準版仍為前批時鐘版 `2A534ECF7676F1FAB8822323DD435F6CA561BBEE84440700A4FF805CB778398D`，已備份至 `target/phase11-diagnostics/prior-continuity-detail-99893a50cc944227ae8fe7b0fc5f4158/subtitlenow.exe` 並核對舊雜湊。Release 後完整一般回歸正在執行；尚未覆寫標準版、操作金鑰或啟動擷取。

### 數值連續性診斷版完整驗證與部署完成

Release 後獨立 `cargo test --locked --offline --quiet` 完成：2023 library 通過、零失敗、18 ignored，496.48 秒；整合 12＋4＋10 通過，合計 2049 項一般測試。ignored 增加一項自有 active 音源時鐘案例；本批該項已明確通過，並在完整回歸後另行執行原靜音時鐘案例通過（1.28 秒）：100 包、後續旗標／位置缺口／未知時間皆零、停止 worker 歸零。沒有執行所有 ignored，沒有重跑或冒稱先前三十分鐘 soak／三百次重啟數據。

部署前確認沒有 SubtitleNow 程序，再核對標準版與備份舊雜湊 `2A534ECF7676F1FAB8822323DD435F6CA561BBEE84440700A4FF805CB778398D`、獨立新版 `5E0A9FC6DD62617C474B69772441A1CD06BC6E98BBAAAF92F632E0814FFD9306`。已複製至標準 `target/release/subtitlenow.exe` 並核對相同新雜湊，9,529,856 bytes；回復路徑沿用本節 `prior-continuity-detail-99893a50cc944227ae8fe7b0fc5f4158`。

新版 --ui 已啟動（PID 30752），主視窗 handle 15795140、程序存活且 Responding。只核對程序中繼資料，沒有讀取金鑰或代按開始；尚未擷取 Chrome 或作真實雲端驗收。下一步由使用者維持原 Chrome 英文來源／英文／Gemini／Standard／已存金鑰欄留空，明確啟動後回報完整階段與數值，或確認英文、臺灣繁中及停止正常。剩餘故障根因與 Phase 11 結案仍待此項人工結果。

2026-10-23 使用者提供實際錯誤畫面，並確認按開始後立即出錯，尚未出現字幕。畫面只有「語音辨識網路連線已中斷」，不能由此判定金鑰、帳號、模型或傳輸的確切原因。Phase 11 的真實服務驗收仍未通過。

## 已確認的範圍

- 本機以 .NET SslStream 對固定 Gemini 主機的 TCP 443 與預設信任驗證的 TLS 1.2 探測通過；沒有 HTTP／WebSocket 認證要求、金鑰或音訊。這也不是程式內 rustls／WebSocket、模型權限、配額或實際服務成功的證明。
- 程式的模型、固定端點與基本初始化欄位對照 [Google Live transcription 文件](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe) 及 [WebSocket 參考](https://ai.google.dev/api/live)；未因猜測而換成其他模型／供應商。
- 現有 Live 流程先完成連線與 setup，再啟動擷取；未通過 setup 的嘗試不會開始新的音訊擷取。畫面本身無法證明原嘗試已到哪一階段。
- 「內容辨識：尚未設定服務」為獨立功能狀態，沒有作為 STT 失敗原因的證據。

## 本次變更

錯誤新增三個固定階段：建立安全串流連線、送出服務初始化設定、等待服務初始化完成。HTTP 拒絕與 WebSocket 關閉只保留數字代碼及原有具型別原因；遠端 reason、回應本文、金鑰、認證 header、網址與 transcript 不進錯誤顯示或 Debug。

例：`等待服務初始化完成失敗：WebSocket 關閉碼 1007：語音辨識資料格式或工作階段無效。` 這是本機測試案例，不是本次真實 Gemini 結果。

連線的二十秒總期限由各步驟共用同一 deadline，不改為每階段二十秒。原有每次寫入一百 ms、三次有限復原與選定來源／服務保持。新增的診斷包裝依原始 cause 判斷重試，不將全部關閉碼當成可重試。沒有讀取待命程式的金鑰或記憶體，也沒有終止該程式。

另核對本機固定版本 `tokio-tungstenite 0.28.0/src/tls.rs` 與 `tokio-rustls 0.26.6/src/common/mod.rs`，確認 rustls 驗證錯誤實際會放在 `io::Error` 內。原本只有 `Error::Tls` 會分類為 SecureTransport，這種實際包裝會錯判成 Network 並重試。已僅按內層具體 `rustls::Error` 型別改判 SecureTransport，不解析或印出錯誤字串；一般 ConnectionReset／UnexpectedEof／TimedOut 及非 rustls 的 InvalidData 仍維持 Network。此缺陷已在本機測試重現分類差異，但不是本次真實 Gemini 根因的證明。沒有停用驗證、安裝憑證、修改信任庫、改走其他服務或捕捉另一來源。

本機測試涵蓋實際 handshake 拒絕、setup 後關閉、正常 setup、共享期限、有限重試與 rustls／一般 I/O 分類。測試使用 localhost、明確假金鑰與人工私有標記，驗證 Display／Debug 不含標記；不使用真實帳號或音訊。

## 接續手動重試

新版輸出至獨立目錄，不覆寫執行中的舊程式。既有視窗不會因來源檔更新而自動換版；本批後續只讀程序檢查發現舊程式已關閉，沒有由代理程式終止。建置與驗證完成後，可從專案根目錄執行：

```powershell
.\target\phase11-diagnostics\release\subtitlenow.exe --ui
```

驗證完成並再次確認無 SubtitleNow 程序後，已備份舊版至 `target/phase11-diagnostics/prior-release-df25fe27b435448cbb1aba9576fa795c/subtitlenow.exe`，原 SHA-256 `1E69847AEFD52B81CF67D4EDC3FA3AD1FE8D697B6E82A89B39380004ABD44385`；再將新版同步至標準 `target/release/subtitlenow.exe`，重新核對與獨立建置 SHA-256 相同。一般手動操作文件的標準路徑因此也已是新版。備份只含先前建置的執行檔，沒有金鑰、媒體或字幕。

標準路徑新版已以 `--ui` 開啟待命（PID 20704）；程序仍存活、有主視窗 handle 且 Responding。這不是畫面視覺或真實服務通過的證據。代理程式未讀取金鑰或代按開始，已要求使用者依原固定遮蔽介面手動重試。

請仍在固定遮蔽欄位手動輸入 Gemini 金鑰，不貼入聊天。重試後只回報完整的新錯誤階段與數字代碼；若成功出現字幕，另依 [手動驗收](phase-11-manual.md) 核對停止與 Session 隔離。未取得真實新結果前，不宣稱已修復這次雲端連線。

## 本批驗證

格式、零警告 Clippy 與 Debug 建置通過；`cargo test --locked --offline phase11 --quiet` 共十七項一般測試通過、一項長 heartbeat 依既定方式 ignored，0.37 秒。較前批新增五項一般測試，涵蓋三項實際連線／初始化、共用重試與 rustls I/O 分類。最終 `cargo build --release --locked --offline --target-dir target/phase11-diagnostics` 通過（1 分 01 秒）；新版 9,424,896 bytes，SHA-256 為 `9E9E1DD178D25BC70199CCFE3038260F0B308B5EA8F6532E777182AA07C68C74`。

Release 後獨立完整 `cargo test --locked --offline --quiet` 通過：1996 library、零失敗、16 項 ignored，524.36 秒；另 12 設定、4 探索及 10 啟動測試通過，合計 2022 項一般測試。前批指定的 native／heartbeat／五分鐘探測結果保留原本範圍與時間，本批沒有改擷取／IPC／正規化或時鐘路徑，也沒有將那些長測冒充再次執行。

第一輪新增 HTTP fixture 使用套件 callback，Clippy 指出其規定的 Response 錯誤型別過大；已改為有界四 KiB 的本機 HTTP fixture 並重新通過，沒有停用 lint、降低斷言或放寬產品限制。

## 本機儲存狀態與 1011 的後續回報

使用者回報按儲存後欄位空白，不確定是否存在本機，並提供新錯誤：等待服務初始化完成時，遠端 WebSocket 以 1011 關閉。已使用 Windows `cmdkey /list:SubtitleNow/Gemini/ApiKey` 的指定目標中繼資料確認本機目前使用者存在該項目，複核精確 `Target: SubtitleNow/Gemini/ApiKey` 列，不把含有目標名稱的查詢標頭當成存在證據。工具只輸出指定目標名稱或 presence，不顯示帳號或金鑰內容；沒有要求金鑰值。這只證明項目存在，不能代替可讀性、帳號權限或雲端成功。

[RFC 6455 §7.4.1](https://www.rfc-editor.org/rfc/rfc6455#section-7.4.1) 將 1011 定義為遠端服務遇到意外狀況而不能完成要求。它不是金鑰未儲存的證據，也不單獨證明帳號／模型可用。程式仍以相同 provider、模型與 setup，在原有三次預算內有限重試；未修改雲端要求來猜測修復、換服務或停用 TLS。

由程式步驟可推知，這次 SetupReceive 錯誤發生在安全 WebSocket 建立、初始化設定送出之後，服務尚未回覆 setupComplete；本機擷取仍尚未啟動。這個推論只限本次具階段的錯誤，不將服務端關閉當作金鑰未存、電腦無法上網或雲端驗證通過的證據。遠端內部錯誤的帳號／模型／服務狀態根因仍未確認。

本批補上以下行為：

- OpenAI／Gemini 固定列各有「檢查本機」，不必先選音訊來源。只有點選才讀取該服務的 app-owned WinCred 項目；短暫驗證副本清除，UI 只取得狀態，不回填原文／星號假內容、讀另一服務或自動連線。
- 本機可讀、未儲存、資料無法使用與無法確認各有不同訊息。不保存狀態快取，重新開啟後可再次明確檢查；保留使用者尚未儲存的欄位內容。
- 儲存後清空欄位與 undo 保持；同一服務寫入後讀回內容一致才顯示已儲存且可讀。寫入完成但讀回不可確認時明確顯示未知，避免錯報為未儲存或成功。
- 本機提示只描述本機操作，雲端結果請看下方狀態，避免舊的「未連線」提示被誤讀成即時雲端狀態。工作中檢查按鈕停用，程序化通知也忽略。
- 遠端 1011 改用 ServiceInternal 原因，顯示「語音辨識服務回報內部錯誤；請稍後重試」，仍保留階段、數字代碼與同一個三次預算，不再誤報為一般網路斷線。

新增四項一般測試（讀回核對一項、本機檢查路由／失敗與儲存未知三項），並擴充原生按鈕尺寸／遮蔽／啟動鎖定及 1011 本機關閉／有限重試測試。二十一項 Phase 11 一般測試通過、一項長 heartbeat ignored（0.43 秒）；格式、零警告 Clippy（35.48 秒）、Debug 建置（0.80 秒）、獨立 Release（1 分 02 秒）通過。獨立完整回歸 2000 library、零失敗、16 ignored（528.49 秒），另 12 設定、4 探索、10 啟動通過，共 2026 項一般測試。本批沒有新增 unsafe block、相依、擷取／IPC／時鐘變更、內容保存、憑證列舉、遙測或服務 fallback；本機測試不是實際 Gemini 連線通過。

更新前已再次確認沒有 SubtitleNow 程序，沒有由代理終止使用者程式。舊標準版備份在 `target/phase11-diagnostics/prior-key-status-d535a1b2689842cb8ba149d1a61b8668/subtitlenow.exe`（原 SHA-256 `9E9E1DD178D25BC70199CCFE3038260F0B308B5EA8F6532E777182AA07C68C74`）。新版 9,430,528 bytes、SHA-256 `5087F0888BBB41F1F1B4C0F99220B496A70181F031DFFDB3AFDE720B1580FA18`，同步至標準 `target/release/subtitlenow.exe` 後雜湊相同。

新版已以 `--ui` 開啟待命（PID 16888），程序存活、有主視窗 handle 且 Responding；代理未讀金鑰、按檢查或開始。使用者先按 Gemini 列「檢查本機」，確認可讀後保持輸入欄空白即可使用已存金鑰。若仍在初始化收到 1011，保留具型別失敗與同服務有限重試，不要求反覆儲存，也不宣告雲端／Phase 11 結案。人工狀態仍待使用者回報。

### 使用者本機檢查結果

2026-10-24 使用者回覆新版「檢查本機」結果為選項一「本機已儲存且可讀取」。據此將 Gemini 本機儲存／讀取人工驗收標記通過；這是使用者回報，代理沒有讀取金鑰內容。接續可讓金鑰欄留空，用已儲存金鑰執行所選測試來源；實際 Gemini 初始化、辨識與翻譯，以及先前 1011 是否解除，仍待另行回報。沒有重寫金鑰、重新啟動程式或把本機成功當成雲端成功。

### 本機可讀後仍以 1011 初始化失敗

2026-10-24 使用者回覆接續真實測試為選項二「初始化仍以 1011 失敗」。本機可讀性維持通過，實際 Gemini 初始化仍未通過；不要求反覆重存金鑰、不把服務內部關閉當作已確認的帳號／模型根因。

主視窗新增明確「Gemini 診斷（連線、不傳音訊）」按鈕。使用者按下才以 Gemini 欄位或其已存憑證，向固定 Google HTTPS 模型端點發出單次 GET，再使用相同固定模型、目前來源語言與既有 WSS setup 嘗試初始化；不選取程序、不啟動捕捉或 recognizer.run、不傳音訊／辨識文字或字幕、不自動翻譯。另一服務的未儲存輸入保留；只清空 Gemini 欄位及 undo，憑證不回填。

依 [Google Models API](https://ai.google.dev/api/models)，單一模型資訊使用 `GET /v1beta/models/{model}`，可列出支援方法。查詢失敗仍會獨立嘗試一次初始化，查詢成功也不作執行權限或實際辨識驗收證據。只有固定名稱能核對為模型資訊可讀；沒有改用查詢結果中的其他模型，也不提供任意端點。回應本文與遠端錯誤文字不顯示或寫入日誌，只保留 HTTP 數字及串流原有安全階段／關閉碼。

模型查詢最多八秒、回應十六 KiB、整次共用三十秒期限；原串流二十秒期限保持且受外層剩餘期限約束。TLS 驗證、HTTPS-only、不跟隨重新導向；不重試、不 fallback。沿用既有單一 UiWorker／取消／join，診斷通道只容一份結果。停止／關閉撤回未完成 future，無第二條音訊或字幕管線；本次結果只在 RAM。服務結果優先顯示，避免來源探索通知遮住無音訊診斷；下次正式開始恢復原來源通知規則。

本機測試只使用 loopback HTTP、假金鑰、受控 future 及隱藏原生視窗。代理沒有用正式金鑰執行診斷。真實結果與 Phase 11 結案仍待人工回報。

### 無音訊診斷結果與工程檢查

2026-10-24 計費頁唯讀複核：使用者提供的是 Spend 截圖，頁面金額表示月支出上限，不是預付餘額。經目前瀏覽器側欄 Billing 導覽，帳戶顯示已切換 Prepay、已有正值預付餘額、自動儲值關閉；其 Project 區列出使用者指定的同一專案。此觀察不讀取 API 金鑰，不能以此獨立證明 WinCred 金鑰的專案歸屬或實際串流成功。僅保留必要狀態，不保存帳戶／專案識別碼、帳號、付款方法、交易明細或精確餘額。未按 Switch、Buy credits、設定自動儲值或更動支出上限。

隨後使用者澄清已自行預付，所以前述瀏覽器觀察是帳戶變更後的狀態。不能以現在正餘額回推先前 1011 發生時的計費狀態，或當作排除計費原因的證據；目前也不要求追加儲值。先前建議 English 的對照暫緩，改先保留原失敗嘗試的語言與其他設定，使用已存 Gemini 金鑰由使用者按同一無音訊診斷，觀察預付後是否改變。若已同時改語言，須註記而不能只歸因於計費。本次只更正證據時序與手動步驟，不改模型／程式、不重建或重跑舊測試、不代按正式金鑰診斷或開始擷取；Phase 11 維持未結案。

2026-10-24 額度／帳戶補充：使用者回報可找到本模型且有可用額度，另外提供計費頁遷移通知，要求一個或多個 billing accounts 改為 prepay 以避免服務中斷。這是使用者回報，未取得私人帳戶識別碼、金鑰、付款資料或直接登入權限。額度存在與計費帳戶可用是不同條件；目前仍未確認通知對應的帳戶是否支付本專案、遷移期限是否已過或帳戶是否已停止服務，不能直接將 1011 定為計費原因。

依 2026-10-24 核對的 [官方 Billing 遷移章節](https://ai.google.dev/gemini-api/docs/billing#migrate-to-prepay)，Google 正將部分既有計費帳戶由 Postpay 轉為 Prepay；受影響帳戶須在個別通知期限前切換並加入餘額，僅使用 Free Tier 功能的帳戶不需動作。最低預付金額目前為 USD 5，自動儲值為選用，均非代理自動啟用項目。下一步僅請使用者在同一專案的 Billing 頁確認帳戶狀態及截止日期；尚未代按切換／儲值／開自動儲值／停用計費或更動金鑰。涉及付款的遷移由使用者自行決定與操作；完成帳戶處理後才由使用者再按既有無音訊診斷核對初始化，不將付款或額度畫面當成辨識／翻譯通過。本次無程式變更或新測試，保留前批驗證範圍。

補充人工回覆：使用者對來源語言問題選擇第三組「日文／法文／德文／韓文」，沒有指定其中哪一語。只記錄這個群組回覆，不補寫特定語言，也不宣稱五語皆已實測。接續需要使用者登入 [AI Studio 模型額度頁](https://aistudio.google.com/rate-limit?timeRange=last-28-days)，選同一金鑰所屬專案，回報 `gemini-3.5-transcribe-live` 的可用額度是否存在／為零／已耗用。官方公開額度連結導向 Google 登入，代理未取得私人專案額度；公開狀態頁亦未取得可判讀服務狀態，不當作「沒有服務事故」的證據。這是讀取帳號狀態的人工步驟，不要求修改付費設定、提供金鑰或重新儲存。

2026-10-24 使用者已實際執行新版無音訊診斷並回報完整狀態：固定模型資訊可讀、列有串流方法；相同模型的串流仍在 SetupReceive 收到 WebSocket 1011，沒有通過初始化。此為使用者回報，代理未取用正式金鑰或代按診斷。模型查詢人工項目可標記通過，實際辨識／翻譯仍不可標記；無新音訊擷取或上傳。

本次重新逐欄核對 [官方 Live transcription](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe) 的固定模型／WSS 範例、TEXT、BCP-47、VERBATIM 與手動 VAD，尚未找到可確認的 setup 格式差異。五語 Auto 的 languageCodes 與單語要求仍需先確認此次選項，不能以猜測更改正式設定或解除五語限制。另查 [官方價格表](https://ai.google.dev/gemini-api/docs/pricing#gemini-3.5-transcribe-live) 仍列此模型免費用量，不能由 1011 斷言必須付費；[官方額度文件](https://ai.google.dev/gemini-api/docs/rate-limits) 說明額度按專案且可於 AI Studio 查詢，專案實際可用額度尚未取得。根因仍未確認，不將模型 GET 成功當成執行授權或全面網路狀態證明。本次只更新人工記錄／排查範圍，未修改或重新建置程式、重跑前批測試或要求反覆儲存金鑰。

本批新增十項一般測試：八項模型 HTTP／回應容量／HTTPS-only／獨立初始化／共享期限／取消／無來源狀態，以及兩項原生診斷路由／停止／重入／modal owner／非法輸入。`cargo test --locked --offline service_check --quiet` 十項通過（1.18 秒）；`cargo test --locked --offline key_tests --quiet` 十一項通過（1.99 秒，含前述兩項，不能重複累計）。本機可讀檢查與 WinCred 保存流程未改，無新增 ignored 或依賴。

定向測試發現原先嘗試透過 terminal clear 設定 Starting 會被既有橋接拒絕，已改為專用 CheckingService 與只允許 fresh channel 的準備入口；未放寬原有 clear 狀態契約、捏造來源或 Session。另核對原生無來源狀態，確保不會在診斷連線時仍誤稱「未啟動雲端」。

最終 `cargo fmt --check`、`cargo clippy --locked --offline --all-targets --all-features -- -D warnings`（33.39 秒）與 Debug 建置（0.97 秒）通過。獨立 Release 建置通過（1 分 16 秒），9,496,576 bytes，SHA-256 `35C8BCACBBDA58EFD7054BA458FDF659A13562F29F9D62C87090237A937F3322`。未改擷取／IPC／重採樣、重試策略、精確／模糊採用、播放時鐘或單一時間軸；先前長測保留原有範圍，未冒充本次重跑。

Release 後獨立 `cargo test --locked --offline --quiet` 通過：2010 library、零失敗、16 ignored（514.33 秒）；另 12 設定、4 探索、10 啟動通過，共 2036 項一般測試。新增 Win32 查詢只操作本 UI thread 擁有的 combo／owner HWND，各有 SAFETY 說明；沒有 callback I/O、無界工作、內容保存或真實服務測試。

使用者回報已關閉舊版後，更新前再次確認無 SubtitleNow 程序；未由代理終止程式。舊標準執行檔備份至 `target/phase11-diagnostics/prior-service-check-eb2925b86d5243bdbf7748a1f6c985f6/subtitlenow.exe`，SHA-256 `5087F0888BBB41F1F1B4C0F99220B496A70181F031DFFDB3AFDE720B1580FA18`。新版同步至 `target/release/subtitlenow.exe` 並核對與獨立建置雜湊相同，開啟 `--ui` 待命（PID 29668），程序存活、主視窗 handle 3541190 且 Responding。僅查程序中繼資料，未讀取畫面／金鑰、代按診斷或開始；啟動確認不等於視覺或 Gemini 通過。接續請使用者保持原測試語言、Gemini 欄留空，明確按無音訊診斷並回報兩階段狀態。Phase 11 保持未結案。

## 預付後 InvalidData 與 Gemini Binary JSON 接收修正

2026-10-24 使用者完成預付後，在新版 PID 3624 重新執行無音訊診斷：固定模型資訊仍可讀且列有串流方法，串流初始化在 SetupReceive 回報「語音辨識資料格式或工作階段無效」，本次回報不含原先的 WebSocket 1011。這是不同結果，但尚未通過初始化；沒有新音訊擷取或上傳。不能據此單獨確認原先 1011 的計費原因，或將新的 InvalidData 當作服務成功。

檢查接收端發現 read_message 對所有 Binary frame 一律回傳 InvalidData，即使內容是有效的 setupComplete JSON。對照 Google 官方 Python SDK 的初始化及串流接收路徑，raw_response 可為 bytes，再交給 json.loads；不應僅因 WebSocket Binary opcode 拒收。參考：https://github.com/googleapis/python-genai/blob/main/google/genai/live.py 。此為可確認的本機相容性缺陷；尚未讀到這次真實原始回覆，不能宣稱它就是本次真實失敗的唯一原因。

本批只讓 Gemini 接受上限內的 Binary payload，交回與 Text 相同的 UTF-8／JSON object／setupComplete／服務錯誤及事件語言驗證；不當 PCM 或音訊解碼。初始化及正式 run_socket 沿用同一條接收路徑，單一時間軸、bounded queues、取消／Session 隔離、TLS、模型／端點／語言計畫／manual VAD／重試預算不改。原 64 KiB 訊框／訊息限制與解析上限保持，Binary 在複製前另核對大小；沒有新增依賴、unsafe、worker、內容保存或 raw payload／錯誤 reason logging。OpenAI 保持 Text 契約，Binary 仍拒收。

新增實際 loopback WebSocket 測試涵蓋六個來源語言選項的 Binary setupComplete，確認 connect-only 完成後關閉而不發音訊。另涵蓋恰為上限、超限、非 UTF-8、壞 JSON、非 object、無效 setupComplete、具型別 429／私有訊息遮蔽及 OpenAI Binary 仍拒收。原兩服務五語的合成 PCM 端到端 fixture 擴充 Gemini Binary partial／Text final 混用，保留真實傳輸、重採樣、時間、取消及順序檢查。所有資料為本機假金鑰與合成內容，不使用正式服務。最終驗證與新標準執行檔資訊待完成後追加。

### Binary JSON 批次工程驗證

串流模組定向 `cargo test --locked --offline ai::streaming::tests --quiet`：20 項通過、零失敗、2 項既有 ignored（5.97 秒）；含新增兩項一般測試及擴充的兩服務五語合成 PCM fixture。兩項 ignored 的真實服務／長 heartbeat 不在本次定向執行範圍。`cargo fmt --check`、零警告 `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`（22.65 秒）、Debug（0.76 秒）及獨立 Release（55.09 秒）通過。

本批 Release 9,504,768 bytes，SHA-256 `352917B69A23BADB9D92106B93536D910F75F68095BA207FA7D11682DAEA6B78`。最終獨立完整回歸與標準路徑更新結果後續追加；不能把本機 Binary fixture 通過當成預付後 Gemini 初始化、辨識或翻譯已通過。

### Binary JSON 完整回歸與標準版更新

Release 後獨立 `cargo test --locked --offline --quiet` 全數通過：2012 項 library、零失敗、16 項既有 ignored（531.73 秒）；另 12 設定、4 探索、10 啟動通過，共 2038 項一般測試。前批 capture／原生長測保持原結果與範圍，本批沒有冒充重跑或真實雲端驗收。

更新前再次確認沒有 SubtitleNow 程序，未由代理終止舊視窗。舊標準版備份在 `target/phase11-diagnostics/prior-binary-json-2158336f3c714265bc2298fd4220d9f6/subtitlenow.exe`，SHA-256 `35C8BCACBBDA58EFD7054BA458FDF659A13562F29F9D62C87090237A937F3322`。新版同步至 `target/release/subtitlenow.exe`，與獨立建置 SHA-256 `352917B69A23BADB9D92106B93536D910F75F68095BA207FA7D11682DAEA6B78` 相同，9,504,768 bytes。

新版 `--ui` 已開啟待命（PID 21104），主視窗 handle 7146524、程序存活且 Responding；僅查程序中繼資料，不當作視覺或服務通過。代理未按診斷、輸入金鑰或開始擷取。請使用者選回本次失敗的來源語言，留空沿用 Gemini 已存金鑰，再按既有無音訊診斷，回報目前語言與兩階段提示；初始化、真實辨識／翻譯及整個 Phase 11 維持未結案。

## 英文初始化人工驗收成功（2026-10-24）

使用者在標準 Binary JSON 修正版重新啟動後（PID 27532）明確回報來源語言為英文；固定模型資訊可讀、列有串流方法，且串流初始化成功。沒有擷取或傳送音訊，畫面仍等待字幕。本次只將該次英文初始化標記通過；其他語言、實際語音辨識、臺灣繁中翻譯及 Phase 11 結案條件仍待人工驗收。

上一輪新版仍回報 InvalidData，當時語言未明；本輪確認英文成功，不能由此推論其他語言皆成功或失敗，也不能把前次問題單獨歸因於預付、Binary JSON 或語言變更。尚無原始服務回覆證據，因此暫不為已通過的英文初始化追加猜測性協定修正或再做計費操作。

接續使用既有獨立英文 TTS 測試視窗，由使用者明確播放、選同一 PID、英文／Gemini／Standard，再按開始翻譯，確認英文原文、臺灣繁中與停止清空。金鑰欄留空沿用已存 Gemini 憑證；不要求再次儲存、不由代理讀取或使用金鑰。此次只更新文件，未修改程式、重新建置或重跑自動測試；前批 2038 項結果仍是前批工程證據。

接續已開啟既有合成英文語音測試視窗（PID 26136，主視窗 handle 18222514、Responding），SubtitleNow PID 27532 仍回應。僅確認啟動中繼資料，沒有按 Start、啟動擷取或使用正式金鑰；等待使用者明確操作。首次隱藏啟動的自有待命 helper 未提供可見主視窗，已核對 PID／腳本身分後只結束該測試 helper，再以供使用者操作的可見視窗重開；沒有終止 SubtitleNow 或其他使用者程序。

## 初始化成功後的音訊不連續回報（2026-10-24）

使用者接續回報 Gemini 開始後顯示「語音辨識音訊已過期或不連續」，畫面來源為 PID 9772；本次查詢程序中繼資料確認該 PID 為 Chrome，與前次安排的合成英文 TTS PID 26136 不同。舊 TTS 與 SubtitleNow PID 27532 已不在執行中。這只能確認來源不同，不能斷言選錯來源就是錯誤根因；待使用者確認實際播放來源。英文無音訊初始化成功仍成立，實際辨識／翻譯保持未通過。

已檢查 SpeechInput、SpeechAudio、重採樣、PreprocessWorker、串流送出、native packet 與 helper 傳遞：此具型別錯誤可能來自超過原 250 ms 期限、資料缺口、discontinuity 旗標或不可靠時間戳記；沒有本次原始數值，無法單獨定位。沒有放寬期限、移除缺口保護、偽造連續時間或變更擷取／重試／定位行為，也未讀取、擷取或上傳 Chrome 內容。

本次執行 `cargo test --locked --offline ai::speech::tests:: --quiet`：12 通過、零失敗，0.29 秒；`cargo test --locked --offline win32::capture::packet_tests:: --quiet`：6 通過、零失敗，0.04 秒。它們驗證既有本機邊界，不代替 Chrome 或 Gemini 端到端驗收。最初模組篩選 `ai::tests::` 沒有匹配任何測試，未計為通過項目。本次未修改程式碼、重建或重跑完整回歸，前批 2038 項數據保持歷史範圍。

已重新啟動標準 SubtitleNow PID 28620 與供使用者手動操作的英文 TTS 視窗 PID 26536；兩者主視窗 handle 非零且 Responding。標準執行檔 SHA-256 仍為 `352917B69A23BADB9D92106B93536D910F75F68095BA207FA7D11682DAEA6B78`。未按播放／開始、未讀取或使用正式金鑰。下一步先按 TTS Start 使來源出現，再明確選 PID 26536、英文／Gemini／Standard、沿用已存金鑰，手動測試字幕與停止。若來源清單找不到此 PID，先回報，不以 Chrome 或其他程序代替。

## Chrome 英文來源確認與音訊階段診斷（2026-10-24）

使用者已明確回覆來源為 Chrome 播放英文內容，PID 9772 是使用者有意選取的來源，不是來源選錯；前批「待確認實際來源」由本項澄清。先前與獨立 TTS PID 不同只是觀察，不能解釋此次失敗。英文無音訊初始化通過保持，Chrome 實際辨識／翻譯仍未通過。

本批追加固定的 AudioStage／AudioIssue 與 SpeechError::AudioFailure：保留送入／取出辨識佇列、重採樣、協定送出、傳送佇列及等待 Gemini 上一段結果的失敗位置，分別說明期限／時間戳記、擷取旗標、序號、位置、重採樣器拒收或 worker 結束。不保存任何音訊、本文、來源名稱、數值 payload 或秘密；只在錯誤發生時包裝原 AudioDiscontinuity。正常音訊不增加配置、工作或佇列；未修改 native capture／helper／時鐘／定位／仲裁、模型、語言、TLS 或憑證處理。

Recovery 對詳細錯誤先取同一 root cause，保留原 AudioDiscontinuity 的最多三次重試、相同延遲／jitter 及共用預算；不是新增不受限重試。所有原時間、旗標、序號與位置拒收條件保留，首塊原生不連續仍可接受，後續缺口仍終止並清理。

新增本機測試涵蓋具體缺口／位置／旗標／未來時間與首個取消原因、preprocess 的過期／拒收、詳細與原始錯誤的三次相同復原預算，以及真正 loopback WebSocket 延遲 final 的情境。後者刻意不回覆第一段 final，繼續讀取客戶端；驗證下一段舊音訊不會越過屏障送出且出現等待結果階段。這是可重現的可能路徑，不能當作這次 Chrome 失敗的根因。工程驗證與部署結果待實際完成後追加。

### 音訊階段診斷定向驗證

本批 `cargo fmt --check`、`cargo clippy --locked --offline --all-targets --all-features -- -D warnings` 通過（Clippy 1 分 01 秒）；`cargo build --locked --offline` 通過（0.85 秒）。`cargo test --locked --offline phase11 --quiet` 為 29 通過、零失敗、1 項既有 ignored，1.30 秒；`ai::speech::tests::` 為 13 通過、零失敗，0.29 秒；`ai::streaming::tests::` 為 21 通過、零失敗、2 項既有 ignored，5.98 秒。這些集合互有重疊，不相加冒充新增總數。四項新增測試均已執行，沒有正式端點、金鑰或 Chrome 音訊。

錯誤原因與原 AudioDiscontinuity 的對應已審查：首塊不連續例外、後續旗標／序號／位置／時間拒收順序保持；詳細錯誤穿過取消通道後仍保留首個失敗，三次復原延遲與原始錯誤相同。沒有新增 unsafe、依賴、正常音訊動態配置、worker、內容保存或原生擷取工作。Release 與其後獨立完整回歸結果待追加。

音訊階段診斷 Release 建置已通過（1 分 16 秒），獨立輸出 `target/phase11-diagnostics/release/subtitlenow.exe`，9,517,056 bytes，SHA-256 `84F5B8534AC371906DE88DFC4DFC09096B39E83B87621764B0F2AB99F0BE4233`。Release 後完整回歸已獨立啟動，尚未完成；標準程式仍為前版，待使用者關閉及驗證通過後才更新，未提前覆寫執行中的程式。

### 音訊階段診斷完整回歸與待更新狀態

Release 後獨立 `cargo test --locked --offline --quiet` 已完成：2016 library 通過、零失敗、16 項既有 ignored，535.57 秒；整合測試另有 12＋4＋10 通過，共 2042 項一般測試成功。未執行全部 ignored，也沒有重跑先前原生長測／實際 heartbeat／五分鐘 probe；不能把那些歷史數據當成本批新量測。格式、零警告 Clippy、Debug、Release 與定向測試已全部通過。

更新前再次核對：目前 SubtitleNow PID 28620 仍在執行、主視窗 handle 非零且 Responding，使用標準路徑；標準版仍為 SHA-256 `352917B69A23BADB9D92106B93536D910F75F68095BA207FA7D11682DAEA6B78`。已建立並核對相同雜湊的備份 `target/phase11-diagnostics/prior-audio-stage-5b44659edb724c068b86a93563eb6ebf/subtitlenow.exe`；本批新版獨立檔仍為 SHA-256 `84F5B8534AC371906DE88DFC4DFC09096B39E83B87621764B0F2AB99F0BE4233`。

標準版尚未覆寫，也未啟動新版或終止使用者程序。因執行中的 Windows 映像檔鎖定，停在必要手動條件：請使用者停止翻譯並關閉 SubtitleNow，之後先確認無同名程序、重新核對兩個雜湊，再複製新版至標準路徑並啟動。正式金鑰不需重存。更新後以原 Chrome 英文測試來源回報完整音訊階段／原因或字幕與停止結果；Chrome 根因及 Phase 11 結案仍未確認。

### 音訊階段診斷標準版更新完成（2026-10-24）

使用者回覆已關閉後，更新前確認沒有 SubtitleNow 程序，再核對舊標準版及既有備份 SHA-256 均為 `352917B69A23BADB9D92106B93536D910F75F68095BA207FA7D11682DAEA6B78`、新版獨立 Release 為 `84F5B8534AC371906DE88DFC4DFC09096B39E83B87621764B0F2AB99F0BE4233`。已複製至 `target/release/subtitlenow.exe` 並核對相同新雜湊，9,517,056 bytes；備份路徑保持前節所列，未終止使用者程序或修改金鑰。

新版 `--ui` 已重新啟動（PID 3088），程序存活、主視窗 handle 721792 且 Responding。只核對程序中繼資料，沒有讀取 UI 金鑰、代按開始、啟動擷取或呼叫正式服務。請使用者選回 Chrome 英文測試來源、英文／Gemini／Standard、金鑰欄留空，再明確開始，回報完整新錯誤階段／原因或字幕及停止結果。工程驗證仍採本批 2042 項結果；此次只部署及更新文件，沒有修改程式碼或重跑測試。Chrome 實際問題與 Phase 11 仍未結案。

## 辨識取出端旗標與 Process Loopback 零位置重現（2026-10-24）

使用者在音訊階段診斷新版回報：Chrome PID 9772／Gemini，辨識端取出音訊時收到擷取端不連續或不可靠時間戳記標記；本次最先觸發的是 Receive／DiscontinuityFlag，不是等待 final 時過期。旗標仍可能合併原生狀態與本機位置／佇列缺口，不能僅由訊息判斷 Windows 遺失了音訊。

新增明確 ignored 原生回歸 `win32::soak_tests::native_process_capture_clock_metadata_is_continuous`，只建立並擷取本測試的獨立靜音程序，不接觸 Chrome、金鑰或網路。首次執行失敗：100 個封包各 480 frames，QPC 前八筆相鄰差均為 100,000 個 100 ns ticks，但位置持續為零；未知時間戳記 0、後續不連續旗標 99、位置缺口 99。100 包完成後已先 stop 並確認 worker 歸零，再由斷言回報失敗（1.32 秒）。這個失敗保留為原始重現，沒有改成假通過。

核對 Microsoft GetBuffer 與 Chromium 官方實作後，確認一般裝置位置契約不能直接套用至 Process Loopback。新增 Windows 原生 adapter 專用、每次擷取獨立的 LoopbackClock：零位置時用實際 frame 數且核對原生 QPC 間隔；只容許一個 100 ns tick 的整數取整差，保留真實 QPC 缺口、原生 glitch、時間不可靠、序號／queue 缺口與非零原生計數器重設。沒有補音訊、放寬 STT 或用牆鐘偽造媒體位置。設計與來源見 ADR 0090；修正後原生／單元／完整驗證與新版部署待追加。

### 零位置修正後原生驗證

同一原生回歸修正後通過（1.26 秒）：100 個 480-frame 封包，位置依序 0／480／960／1440…，QPC 仍按原生 100,000 ticks 前進；後續不連續 0、位置缺口 0、未知時間戳記 0，停止後 worker 歸零。只擷取本測試的靜音子程序，沒有讀取或傳送 Chrome 音訊。

既有 `win32::tests::native_process_capture_isolates_tones_and_includes_audio_children` 也明確執行通過（2.44 秒）：自有 A／B 音源及新增／既有子程序仍隔離，目標振幅 0.031250、非目標 0.000000、靜音峰值 0.000031，原門檻沒有放寬。此為本批新原生結果，與前批長測分開。

原先五項純時鐘測試通過；另補可變 packet 長度的千包取整回歸，最終格式／Clippy／定向測試、Debug／Release 及獨立完整回歸待完成後追加。新增原生回歸預設 ignored，只有本次明確指定才執行；完整一般測試不會自動擷取使用者來源。舊標準 PID 3088 待使用者關閉後才能更新，不先將原生修正當成 Chrome／Gemini 驗收。

### 零位置修正定向工程檢查

最終 `cargo fmt --check` 通過；零警告 `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` 通過（49.60 秒）。六項純時鐘測試通過，包含可變 packet 的千包取整回歸；`win32::capture::packet_tests::` 六項通過（0.04 秒），`audio::capture::tests::` 十七項通過（0.39 秒），均零失敗。Debug build 通過（0.75 秒）。新增可變長度測試後，最初 fmt check 僅指出鏈式呼叫換行，已 cargo fmt 修正並重查通過，沒有隱藏檢查失敗。

審查範圍：原生 packet acquire/release 與 COM／event 的 owning thread、RAII 生命週期保持；新增時鐘沒有 unsafe、配置、鎖、I/O 或 worker，且每個 native capture 重新建立。一般 sink、helper IPC、SpeechAudio／重採樣缺口拒收及 LivePlaybackClock／alignment／arbiter 沒有更動。原本未知 QPC 不會被新計數器當成可信位置；真正非零裝置計數器仍走原檢查。Release 及其後獨立完整回歸待完成；使用者已回覆舊 PID 3088 已關閉，部署前仍會重新檢查程序與雜湊。

零位置修正的 Release 已通過（1 分 11 秒）：`target/phase11-diagnostics/release/subtitlenow.exe`，9,519,104 bytes，SHA-256 `2A534ECF7676F1FAB8822323DD435F6CA561BBEE84440700A4FF805CB778398D`。Release 後獨立完整回歸已啟動，尚未宣告完成。舊標準版 SHA-256 `84F5B8534AC371906DE88DFC4DFC09096B39E83B87621764B0F2AB99F0BE4233` 已備份至 `target/phase11-diagnostics/prior-zero-position-60fdb75664914c08af87f9e97078e88d/subtitlenow.exe` 並核對相同雜湊；標準版尚未覆寫。使用者已回報舊版關閉，待最終檢查通過後直接更新並重開，不要求修改憑證或計費。

### 零位置修正完整驗證與標準版更新完成

Release 後獨立 `cargo test --locked --offline --quiet` 完成：2022 library 通過、零失敗、17 ignored，514.57 秒；整合測試 12＋4＋10 通過，共 2048 項一般測試成功。ignored 比前批增加一項新原生時鐘回歸；本批只另外明確執行該項及既有音源隔離／程序樹兩個原生案例，皆通過，沒有執行所有 ignored 或冒充長測重跑。格式／零警告 Clippy／Debug／Release、六項純時鐘、六項 packet 與十七項 capture 定向測試均通過。

使用者回覆已關閉後，部署前重新確認無 SubtitleNow 程序，再核對標準版與備份的舊雜湊 `84F5B8534AC371906DE88DFC4DFC09096B39E83B87621764B0F2AB99F0BE4233`，及獨立新版雜湊 `2A534ECF7676F1FAB8822323DD435F6CA561BBEE84440700A4FF805CB778398D`。標準 `target/release/subtitlenow.exe` 已更新並核對相同新雜湊，9,519,104 bytes；舊備份路徑沿用前節 `prior-zero-position-60fdb75664914c08af87f9e97078e88d`。

新版 --ui 已啟動（PID 1532），主視窗 handle 20777504、程序存活且 Responding。僅檢查程序中繼資料，不宣稱已作視覺、Chrome 或雲端驗收；沒有讀取金鑰、代按開始或擷取使用者內容。請使用者以原 Chrome 英文測試內容、英文／Gemini／Standard、Gemini 欄留空，明確開始並回報英文原文／臺灣繁中與停止結果，或完整新階段與原因。工程缺陷已在自有原生來源重現並修正，真實 Chrome／服務及其餘 Phase 11 人工矩陣仍待完成。

接續作業啟動核對：PID 28940 已不在執行，標準執行檔 SHA256 仍為 3C38A564A08E94AABF5EBAFF8A766EC2595062B0424D057E32E507193340AACD、9,539,072 bytes；16 個本批檔案嚴格 UTF-8 檢查通過。重新啟動 --ui 後，WaitForInputIdle 呼叫回報無圖形介面例外，但隨後直接程序核對確認 PID 19872 存活、主視窗控制代碼 17895594、Responding=True，路徑為標準版 target/release/subtitlenow.exe。未操作擷取或雲端處理；待使用者驗收，未重跑已通過測試。

單次期限隔離補測：第一輪 Phase 11 57 成功、3 失敗、1 ignored（15.20 秒）。兩個新測試 fixture 誤將已 Final 來源當可再修訂，以及把既有 mock 的兩次 partial＋一次 complete 當成一個更新；另一個測試在驗證新 revision 已接受後仍斷言舊 revision。修正 fixture／斷言順序，沒有放寬 Final 不可覆寫、輸出核對或期限；第二輪 60 成功、零失敗、1 ignored（13.73 秒）。涵蓋原有回歸及本批七項新增案例，有重疊不累計。接續串行執行格式、Clippy、Debug／獨立 Release 與 Release 後完整回歸。

最終品質檢查：首輪 Clippy 僅在新增 mock 的倍數判斷觸發 manual_is_multiple_of，依 lint 改用 is_multiple_of，未隱藏警告。重新執行格式通過、零警告 Clippy 通過（41.47 秒）、Debug 通過（0.95 秒）、獨立 Release 通過（55.00 秒）。Release 後的獨立串行完整回歸仍在執行，未提早宣稱通過。舊標準版已備份至 target/phase11-diagnostics/prior-live-deadline-64eda625375448f0a68a13cedee9be1d/subtitlenow.exe，雜湊 3C38A564A08E94AABF5EBAFF8A766EC2595062B0424D057E32E507193340AACD 核對一致；新候選 9,556,480 bytes、SHA256 B74B8981C49A4DB8BE665582CBDF5ECE3AE0E0CF95CBBF3C1FC8138FDD3A82B0，尚未覆蓋標準版。

本批最終完整結果：最後 Release 之後獨立執行 cargo test --locked --offline --quiet -- --test-threads=1，2054 library 成功、零失敗、18 ignored（734.54 秒），整合 12＋4＋10＝26 項成功（0.44／0.00／1.30 秒）；共 2080 一般成功、零失敗。沒有重跑 ignored 或長測，不宣稱舊媒體間歇逾時根因已修復。標準版覆蓋前確認 SubtitleNow 無執行中程序，舊版／備份 3C38… 雜湊一致、新候選及部署 B74B8981C49A4DB8BE665582CBDF5ECE3AE0E0CF95CBBF3C1FC8138FDD3A82B0 一致（9,556,480 bytes）。新版 --ui 啟動 PID 24216、HasExited=False、MainWindowHandle=39915408、Responding=True；只待命，未讀取正式憑證、未啟動擷取或服務要求。13 個本批檔案 UTF-8 檢查通過；程式畫面及正式服務多分鐘結果仍需使用者驗收，Phase 11 未結案。

主頁操作批次最終品質檢查：格式、零警告 Clippy（40.13 秒）、Debug（0.86 秒）及獨立 Release（1m00s）通過。Release 後獨立串行完整回歸仍在執行，未提早宣稱新碼通過。舊標準版已備份至 target/phase11-diagnostics/prior-main-actions-565930940a644ba286ba9827dfa01859/subtitlenow.exe，SHA256 B74B8981C49A4DB8BE665582CBDF5ECE3AE0E0CF95CBBF3C1FC8138FDD3A82B0 及 9,556,480 bytes 核對一致，尚未更新標準版。13 個本批檔案嚴格 UTF-8 檢查通過。

主頁操作批次最終完整結果：最後 Release 後獨立執行 cargo test --locked --offline --quiet -- --test-threads=1，2057 library 成功、零失敗、18 ignored（760.96 秒），整合 12＋4＋10＝26 項成功（0.41／0.00／2.13 秒），共 2083 一般成功、零失敗；沒有重跑 ignored 或長測。部署前確認標準 SubtitleNow 未執行，舊版與備份 B74B8981C49A4DB8BE665582CBDF5ECE3AE0E0CF95CBBF3C1FC8138FDD3A82B0 一致；新候選與標準版 SHA256 3C3C4A0655AC5E2D4FA79EF0864E07E12E33429828F9E554EF0A847B29CD648B、9,579,008 bytes 核對一致。新版 --ui 啟動 PID 24580、HasExited=False、MainWindowHandle=88215258、Responding=True，只待命，未讀取正式憑證、未啟動擷取／雲端。13 個本批檔案嚴格 UTF-8 檢查通過。實際介面與來源長時間捲動仍待人工驗收，遠端速度未量測改善，前批原生媒體間歇逾時根因未宣稱修復，Phase 11 未結案。
