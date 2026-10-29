# Phase 8 — Prepared Mode

2026-10-11 使用者明確批准啟動。基準仍是目前 Windows 19045；外部歌曲辨識服務維持暫緩。有效 OpenAI／Gemini 金鑰與真實雲端驗收留至結案，在固定遮蔽介面手動輸入。

2026-10-22 接續狀態：第四十三批工程驗證完成後，已開始 [Phase 09 第一批](phase-9.md) 的獨立有界模糊定位候選核心。以下各批「尚未具備條件／未啟動」為當批歷史狀態；Phase 08 的真實媒體、雲端品質及人工驗收限制仍保留，不因後續核心開發而視為通過。

## 首批：本機外掛字幕的 RAM 準備

實作 PreparedRequest、明確字幕選取規則、MediaInput／LocalSubtitleInput、PreparedSession 與 caller-driven PreparedTask。真正讀取使用者明確指定的本機 SRT／WebVTT，保留原文／時間，繁中／簡中以現有臺灣片語字典本地化，成功後交付本次 RAM Ready document。沒有自動播放、媒體內容保存或字幕匯出。

輸入型別沒有 Debug／Serialize。路徑只接受有界、絕對的本機磁碟路徑；Windows 開檔前核對實際磁碟類型，拒絕 UNC、映射網路磁碟、裝置、ADS、特殊名稱及 reparse／symlink。原媒體只開啟唯讀一般檔案並持有 handle 與大小，拒絕空檔、writer／刪除；不掃描影音、不宣稱任何 codec 或影片配對已通過。檔案不會被修改。

最多八份使用者明確聲明格式／語言的外掛字幕，依 zh-TW、其他繁中、簡中、英／日／法／德／韓排序；同一最高優先級多份資料回報歧義，沒有字幕回報 ASR 未接入。原文需要全文翻譯，首批回報明確不可用；不是假翻譯或悄悄降級。選定高優先資料無效時不偷偷改用較低優先來源，檔名與來源語言偏好不代替 metadata。

真正讀檔每八 KiB 檢查取消，上限八 MiB 並最多探測上限後一 byte。共用原有嚴格純文字 SRT／WebVTT parser，拒絕非法 UTF-8、時間、倒序與未支援標記；Prepared 另採 8192 cues／四 MiB 本文預算，本地化輸出另限四 MiB。Live 原有 256 KiB 傳輸、1024 cues／128 KiB 本文及五秒期限完全保留。Prepared 不建立假 capture stamp、不延長 Live lease；獨立 document 驗證允許整份媒體字幕準備。解析後釋放原始讀取 buffer。

新工作與 Live 共用 checked Session ID allocator；每次要求取得唯一 Session、sequence 及實際完成時間。新要求取消舊工作、清空舊文件，舊 completion 被拒收且不能清掉新 Ready。停止、owner／task Drop 及執行 Future Drop 撤回。caller 明確 apply 目前 completion 才進入 PreparedReady；不由工作直接寫 UI，也沒有自動播放。Ready getter 只借用目前 owner，不提供延長生命週期的 Arc 或複本。

所有 I/O、解析與本地化在依可用核心數限制、最多兩項工作的共用 CPU lane；零等待佇列、過載明確拒收。已開始 worker 在 Future Drop 後保留許可，逐 chunk／line／cue 合作取消，退出才釋放。沒有持鎖 await、每句新建 OS thread 或 detached provider 工作。本機磁碟系統呼叫若阻塞，需待它返回後的檢查點才能中止；沒有假稱能強制取消 OS 磁碟操作。

這條本機路徑不需要 temp，沒有建立產品暫存檔或啟動清掃其他檔案。所有本文、路徑與 metadata 只留本次 RAM。設計見 [ADR 0036](adr/0036-prepared-document-lifetime-and-local-input.md)。

## 第二批：來源字幕全文翻譯

新增 PreparedTask::run_with_translator，接受 caller 明確選定的 Translator、Fast／Standard／High 品質與可選的本次 SessionGlossary。英、日、法、德、韓來源字幕使用既有供應商抽象及真正的 HTTP adapters；沒有讀取憑證、自動選服務或跨供應商 fallback。中文優先來源仍只在 CPU worker 本地化，即使傳入不可用服務也不呼叫它。沒有配置服務的原有 run() 回報明確 TranslationRequired。

全文翻譯使用獨立文件生命週期，逐要求三十秒而非 Live 三秒總期限；Live 五秒視窗與原有 transport 三秒維持。每批同時要求最多等於服務宣告上限（最多四個，既有 HTTP client 為兩個），每批還原原始順序後再排下一批。共用服務 Semaphore／速率限制仍跨 caller 生效；沒有整份 cue 數量大小的待執行 future 佇列、detached 工作或每句一個 OS thread。網路期間釋放 CPU 許可，仍持有原媒體唯讀 handle；最後本地化／合併再取得 CPU 許可，過載明確回報失敗。

每句保留 Session、序號、原始媒體時間、前一批已完成的譯文、前後原文窗口與同一份詞彙表。Fast 前後各一個窗口、只做 Draft；Standard 各三個、High 各八個，兩者完成整份草稿後再做整份 Review，校訂可看跨批次的相鄰草稿。新 PreparedContext 明確區分來源提示與草稿；供應商固定指示只翻譯當前 cue，所有本文、脈絡、詞彙仍是 user JSON 資料。詞彙表核對完整身分，拒絕舊 Session；目前支援明確提供的詞彙表，自動全文專名抽取／建立詞彙仍待後續。

每個服務要求使用取消 Drop guard，停止、切換或 Future Drop 取消所有已開始要求。失敗停止後續排程；服務在要求取消後仍回成功、panic、空白、非法控制字元、超長文字或超限總結果都拒收。各 pass 最多四 MiB，單句 8192 bytes；CPU 合併再驗證原始時間、完整數量與臺灣片語轉換後的容量。整份成功才有 Ready，不提交部分文件、不自動播放。沒有加入磁碟快取、輸出／暫存檔或額外字幕時間軸。

兩個 HTTP adapter 都序列化本次脈絡及草稿，Prepared payload 上限四 MiB，保留 Live 256 KiB。較大上限涵蓋最多脈絡資料及兩層 JSON escaping，並不表示雲端服務必然接受最大 token 數。設計見 [ADR 0037](adr/0037-bounded-prepared-document-translation.md)。實際有效金鑰、媒體與雲端翻譯品質仍留到結案由使用者手動測試。

## 第三批：本次 Prepared GUI 與完成通知

2026-10-12 接續第二批後端，新增一般 --ui 的「預先準備（本次）」入口。指定本機媒體與一份本機 SRT／WebVTT 外掛字幕的完整路徑，明確選擇格式、臺灣繁中／其他繁中／簡中／五語原文及 Fast／Standard／High；格式與語言不猜測，預設品質仍為 Standard。中文在本機處理；五語原文必須選擇 OpenAI／Gemini，介面先顯示傳送主機與原文字幕／脈絡用途。本批不擷取或上傳媒體音訊。

準備視窗提供兩列固定遮蔽金鑰，只有有效的明確準備動作才讀所選欄；中文不讀任何欄的金鑰，也不建立 client。留空在工作執行緒使用所選 Windows 已儲存憑證；本次輸入不儲存。準備、取消、清除及關閉會清空金鑰與原生 undo buffer。原主視窗的固定金鑰與明確安全儲存功能保持可用，沒有背景終端機提示或新增帳號登入。

PreparedController 持有一個 PreparedSession、最多一個工作執行緒與容量一的結果通道。UI 不讀檔、不連雲端，也不等待尚未退出的執行緒；100 ms 輪詢在完成後回收再 apply。取消立即撤回 Ready，工作未退出前保持 Stopping，不允許重啟；buffered 舊 completion 也拒收。正常關閉先取消並保持訊息迴圈，父視窗銷毀／WM_QUIT 亦保留 owner，回收後才離開；服務建立前後檢查取消，停止後不啟動翻譯。

處理及 Ready 期間固定本次來源、服務、品質與金鑰；Ready 清除後才可改下一份。準備期間主視窗停用，Live 開始及新的 Prepared 入口也核對 enabled state，避免訊息繞過。視窗按目前螢幕工作區限制大小，內容可垂直捲動，底部操作按鈕固定；本機驗證錯誤持續顯示到下一個有效動作，不被定時 poll 蓋掉。來源路徑只存在本次欄位，關閉清除內容與 undo。

每個 Ready 交付一次 Windows Shell 完成通知，通知只有固定一般文字；取消、清除、重啟及銷毀撤回通知區圖示。送出失敗顯示明確提示並保留 Ready；Windows 通知設定仍可能抑制 popup。通知不含路徑、標題、字幕、金鑰或原始服務內容。Ready 文件只有本次 owner，沒有保存、匯出或自動播放，介面明確標示「播放尚未提供」。關閉準備視窗會釋放本次文件；媒體解碼、播放／音訊定位與後續交付同一時間軸仍待後批。

架構理由見 [ADR 0038](adr/0038-prepared-gui-owned-worker-and-notification.md)。固定介面已可接受結案時使用者手動輸入金鑰，本批不使用正式金鑰或呼叫雲端。

## 第四批：本機 WAVE 的有界 RAM 解碼

2026-10-12 以目前 Windows 19045 為開發與驗收基準，新增獨立的 decode_local_audio library 入口及 provider-neutral MediaDecoder／DecodedAudioBlock／DecodedAudioGate。只讀取明確指定的本機 RIFF/WAVE；沿用既有路徑、磁碟、reparse 及唯讀一般檔案檢查，將已開啟的 pinned handle 交付自訂 IStream，不以 pathname 再次開檔。先讀取固定十二 bytes 核對 RIFF／WAVE，再建立 Media Foundation 來源；不傳入 URL、來源檔名或 origin metadata，拒絕一般播放清單與其他容器。原媒體不能被同時寫入或刪除，最後一個 stream 參照釋放後才解除 pin。

IStream 是唯讀、可 seek 的輸入，複本共用同一檔案並各自保留 logical cursor；seek／read 以固定鎖定順序保護，避免複本讀錯位置。單次 native read 上限一 MiB，每六十四 KiB 檢查取消，短讀只代表真正 EOF；過大要求明確失敗，不以假 EOF 截斷音訊。寫入、改大小及輸出複製均拒絕，Stat 不回傳私人檔名。COM ABI 將 Rust panic 轉成固定原生錯誤，不讓 panic 穿越 vtable。

COM MTA、MF startup、reader、byte stream 及 media buffer 都在同一個 bounded blocking worker 建立、使用與釋放；decoder 不具 Send／Sync。使用 MFSTARTUP_LITE，不初始化 socket 子系統，只選第一個音訊串流，關閉其他串流並協商 PCM16，再檢查實際取樣率、聲道及 frame alignment。PCM 契約允許八千至十九萬二千 Hz、一至八聲道，實際格式相容性仍須個別驗證；保留實際協商格式，不宣稱已做單聲道或服務取樣率轉換。native sample 先核對總長不超過一 MiB，再取得 contiguous buffer；每次 pull 只複製 frame-aligned、最多六十四 KiB 的 PCM block。Lock 成功立即建立 Unlock guard，取消或錯誤路徑也釋放；讀取小端 byte pairs，不假設 i16 pointer 對齊。

每份 PCM block 具有完整 Session／generation、checked 序號及原生媒體時間；分片時間由原始 timestamp 與 sample offset 計算，不用牆鐘或虛構 capture stamp。gate 拒絕舊身分、格式變更、重複／跳號及向後或重疊時間；前進缺口明確計數，不插入合成靜音。允許一微秒的獨立整數取整差異；單一媒體 timeline 限二十四小時，超限在交付 consumer 前拒收。成功也可能沒有 sample 的 ReadSample 以有限次數讀取，不捏造 PCM 或 EOF；格式／串流變更、破損與內容保護回報 typed error，不改用其他 decoder、供應商或繞過保護。

解碼與 Prepared 字幕讀取／解析／本地化共用 process-wide CPU lane，由 available_parallelism 決定，最多兩項工作；過載立即拒收，不累積待執行要求。consumer 只借用當前 block 做短且有界的同步 CPU 工作，不得連網、寫檔、操作 UI、等待佇列或保留 PCM 複本；每輪釋放 block 後才拉下一份。結果僅有同 Session 的格式、frame／block 計數及原生時間摘要，不留音訊，也不是字幕 Ready 或播放定位證據。

未 poll 的 Future Drop 也會取消；執行中 Drop、外部停止、decoder 或 consumer 失敗會停止後續拉取並拒絕部分摘要。已開始 worker 持有 CPU 許可直到真正退出及 native 資源銷毀，不能靠 JoinHandle::abort 假稱停止同步 codec。檔案與 MF 同步呼叫只能在返回後合作取消；本批沒有強制中斷這些呼叫或提供固定取消時間保證。產品流程沒有建立暫存檔、輸出音訊或任何內容快取。

本批入口尚未接入一般 GUI、PreparedSession 的 ASR fallback 或播放。現有外掛字幕準備仍不解碼或上傳媒體音訊，固定介面的用途揭露保持符合實際行為；可靠字幕不額外跑完整 ASR。MP3／AAC／影片容器與 URL 解碼入口尚未提供，其他 WAVE 編碼也未完成相容性驗收，不能把本機 PCM WAV 測試外推為任意影音支援。後續 Prepared ASR 需使用獨立離線要求與媒體時間契約，不能更新 Instant 讓離線音訊通過 Live 的 MAX_AUDIO_AGE 檢查。

決策及 Microsoft primary 文件見 [ADR 0039](adr/0039-bounded-local-media-decoding.md)。本批測試 fixture 範圍為呼叫端自行建立的合成 PCM WAV、真正的本機 MF 呼叫、自訂 IStream 及離線 mock decoder；不使用使用者媒體、喇叭播放、擷取音訊、正式金鑰或雲端。最終指令與測試數字另列於本批驗證紀錄，不將測試設計或編譯中狀態列為通過。

## 第五批：媒體時間的單聲道前處理與 EOF 尾段

2026-10-12 接續本機 WAVE 解碼，新增獨立的 PreparedAudioProcessor、PreparedPcmBlock、PreparedAudioSummary 及 prepare_local_audio library 入口。真正解碼與前處理在同一個 worker 完成，從明確指定的本機 RIFF/WAVE 拉取原生 PCM16，再等權混合一至八聲道，明確選擇 16 kHz 或 24 kHz 單聲道。consumer 每次只借用最多二十毫秒的 PCM，EOF 或 gap 前也交付較短尾塊；沒有音訊保存、匯出、服務要求或播放。

多聲道使用 i64 累加避免溢位，平均值保留小數到最終量化；這是明確等權策略，不推論 surround speaker layout。不同取樣率使用 257 taps／256 phases 的 centered Blackman windowed-sinc FIR，截止頻率為 `0.45 * min(1, output_rate / input_rate)`，每個 phase 正規化。係數表固定 526,336 bytes、ring history 固定 2,056 bytes，128 native frames lookahead；不因媒體長度或奇數取樣率的 rational denominator 增加表格。相同取樣率直接量化，不配置 FIR table、不增加 filter 延遲。有限 filter 的過渡帶仍需實測，不以理想 sinc 名稱或測試門檻宣稱所有頻率已驗收。

每個連續區段從實際 native timestamp 建立唯一 frame clock，使用 checked 累積 native／output frame 與 rational sampling position；不建立 AudioStamp、Instant 或調整 Live 音訊年齡。DecodedAudioGate 的 Session／generation、序號、格式與時間檢查保留，另外核對每份 block 相對區段起點與累積 native frames 的時間，最多容許一微秒取整差；累積漂移不以新起點掩蓋。真正的前進 gap 先完成前區段，再清空 FIR／clock，下一個 output 明確標示 discontinuity；不跨 gap 混合、不插入合成靜音。

首尾 sample extension 僅供 filter 邊界計算。每個區段總 output frames 固定為 `floor(native_frames * output_rate / input_rate)`，最後可能比原區段少於一個 target frame；不把 lookahead 當成媒體時間，也不交付超出原區段的額外 output。整份音訊不足一個 target frame 時回報 InsufficientAudio，不假造內容。Processor 完成、取消或失敗後不能繼續使用；輸出具有同一身分、全程 checked 序號及實際媒體時間，沒有 Debug／Clone／Serialize 的音訊副本。

decode worker 加入私有 EOF-aware consumer，只允許明確有界的 FIR／pending output state。原有 decode_local_audio 的公開 consumer 仍只借用當前 native block；prepare_local_audio 使用相同 dispatch、cancel guard 及 CPU lane，EOF 後在同一 worker flush 尾段，再交付不含 PCM 的 PreparedAudioSummary。解碼、前處理、flush 與 native 銷毀共享一份許可，依可用核心數限制、最多兩項工作；無音訊佇列、額外 thread 或排隊等待。公開 consumer 不得保留 PCM、連網、寫檔、操作 UI 或等待佇列容量。

未 poll／執行中 Future Drop、停止、callback error 或 native error 取消 scope，失敗不返回部分摘要；已開始 worker 仍持有容量至真正退出。初始化、下混音、FIR、flush 及 callback 前後檢查取消，回應若在取消後才完成也拒收。Native 同步呼叫仍只能在返回後合作停止，沒有固定取消時間保證。本批沒有新增 crate、unsafe、元件安裝、產品暫存檔或內容快取。

本批仍是 library 音訊準備，不是 ASR、字幕 Ready 或播放定位證據。沒有接入 GUI／Prepared ASR、OpenAI／Gemini 音訊連線或上傳；現有外掛字幕流程仍不額外解碼，固定介面的用途揭露與金鑰結案安排保持不變。MP3／AAC／影片容器、內嵌字幕、URL、辨識結果合併／全文脈絡與 playback 尚待接續。架構理由見 [ADR 0040](adr/0040-prepared-audio-clock-and-bounded-preprocessing.md)。本批實際指令與數字另列驗證紀錄，不將新增測試或測試門檻當成已通過結果。

## 第六批：離線 ASR 分段、原生時間合併及有界排程核心

2026-10-12 新增 PreparedSegmentBuilder、PreparedSpeechRecognizer 契約、PreparedTranscriptMerger 及 PreparedRecognitionSession。這是獨立於 native 解碼與一般 GUI 的 portable library；要求包含完整 Session／generation、checked chunk sequence／run ID、明確的 16／24 kHz 單聲道 rate、音訊範圍及唯一正文範圍。沒有讀取金鑰、自動選供應商、fallback、服務連線、音訊上傳或播放。Live 的 MAX_AUDIO_AGE、capture stamp、重新連線與字幕翻譯期限不變；離線工作不藉重設 Instant 通過 Live freshness。

每個連續 run 的正文以半開區間分成十秒，左右各最多五百毫秒的真實音訊作為 context，單份 owned chunk 上限十一秒；首段沒有左 overlap，EOF／gap 不填假音訊。分段逐 sample 消費 owned DecodedAudioBlock，working buffer 最多十一秒，ready 最多兩份，另加目前六十四 KiB input；不累積整部媒體 PCM。只複製下一份所需的有界 overlap／真實後續片段，主 buffer 移交 chunk。未取走任何 ready 時新的 push 明確回報 Overloaded 並成為 terminal；EOF 或 gap 可留下完整正文加短尾兩份，若 finish 將超過兩份同樣拒收並清除，不能靜默掉資料。呼叫端須按需求拉取、排空 ready，再交付下一份 input。

DecodedAudioGate 拒絕舊 Session／generation、錯誤 rate／聲道、跳號、倒序及重疊。另以 run 原生起點及累積 frames 計時，只容許一微秒取整差，逐塊看似連續的累積漂移仍拒收。傳入 discontinuity 必須與實際 forward-gap 一致，首塊必須 false，不能用假 gap 重設 clock 遮掩漂移。真正 gap 先排空舊 run，再清空 overlap，以 checked 新 run ID 開始；新 run 只有第一份 chunk 標示 gap。整個媒體時間範圍仍限二十四小時，超限／溢位在交付辨識前拒收。

服務能力明確宣告取消、原生定稿時間、五種來源語言、限制為五語的 Auto、16／24 kHz 支援及一至四個並行要求。未具必要能力就回報 typed error，不猜測時間、語言或假成功。單份回應最多 512 cues／128 KiB、單句最多 8192 bytes；每筆非空 final 保留相對 request audio 起點的原生時間，核對完整 SID／sequence／run、語言、正長度、有序及音訊範圍。所有句子必須完整位於這份正文；句子跨越 body 時回報 BoundaryMismatch，沒有 word timing 時不裁句、移動時間或以重複文字猜測合併。Overlap 只提供辨識 context，不能產生另一份正文的字幕。

Merger 最多保留四筆待完成 metadata／response slot，不保留 request PCM；按已註冊 sequence 提交倒序完成結果。正文維持同 run 連續，跨 run 必須是 checked 下一份 ID、有真實 gap 且不攜帶前 run overlap，提交的 cues 全程不倒序或重疊。切換 run 前及整份 finish 時，前／最後 chunk 的 audio_end_us 必須等於 body_end_us，確認最後實際音訊都已有正文，不把尚未交付的右 context 冒充完整 EOF；不符就回報 typed error，不交付部分 transcript。Auto 的非空結果若偵測成不同來源語言，整份工作回報 MixedLanguages；空回應不改變語言，全篇無語音回報 NoSpeech。全文最多 8192 cues／四 MiB，前文只包含同 run 已按序提交的最多八筆／六十四 KiB，尚未提交的倒序結果及前 run 文字不會成為 context。

PreparedRecognitionSession 每批接收最多四份 owned chunk，在呼叫端 Future 內按所選能力限制一至四個 provider 要求；完成整批後才能排下一批，不產生全媒體大小的 future queue、OS thread 或 detached 網路工作。每個要求的 offline timeout 為六十秒，服務 adapter 仍須維持自身共用的並行／速率限制並支援取消，本批尚沒有正式 adapter。取消、timeout、provider error／panic、身份／時間錯誤、超量結果或 Future Drop 均停止後續排程、取消已開始要求並釋放本批 PCM 與累積本文，沒有部分成功文件；未 poll 的 batch Future Drop 也取消 owner。完整 EOF 才交付獨立 PreparedTranscript，本文僅在本次 RAM；此辨識來源型別不是 ExistingSubtitle、Prepared Ready 或播放定位證據，不直接寫 UI。

本批沒有改寬公開 decode_local_audio／prepare_local_audio consumer：它們仍只借用當前 PCM 做短、有界 CPU 工作，不得保留複本、連網或等待佇列容量。不能在 callback 內複製音訊後等待 provider 來假裝完成檔案 ASR；後續需另外建立私有、依需求拉取的 native actor，保留 COM 執行緒／CPU 許可／取消生命週期後再接上 owned chunk producer。一般 GUI 無字幕仍明確回報 AsrUnavailable，現有可靠外掛字幕優先流程不額外解碼。沒有新增 crate、unsafe、音訊落盤、快取、正式服務或 GUI 接線。

設計理由見 [ADR 0041](adr/0041-bounded-prepared-recognition-and-transcript-provenance.md)。本批合成 provider、PCM、時間與排序測試不代表真正媒體、檔案 ASR、語言／辨識品質、有效金鑰、雲端服務或播放定位已驗收；實際檢查與測試數見第六批驗證。

容量也核對所有權容器的配置，而非只核對 Vec 長度：PCM 與前文改為 boxed slices，回應及 batch 丟棄多餘 reserved capacity。超過四份的輸入 batch 在建立 Future 前立即釋放，不等首次 poll；未 poll Future 不會保留超量音訊。

## 第七批：本機解碼到離線辨識的有界接線

2026-10-12 新增私有 PreparedLocalSource 與公開 recognize_local_media library。明確指定本機 WAVE、完整 Session、來源語言、16／24 kHz 及 caller 提供的 recognizer；取消、身分及能力核對先於讀檔，沒有自動供應商、金鑰讀取、正式 adapter 或 GUI ASR 接線。

首 pull 取得共用 CPU 與 native-owner 許可後才建立一個 blocking worker。原生 COM／MF 解碼器、唯讀 pin、FIR 與分段狀態留在同一執行緒；容量一的 Pull 命令攜帶 CPU 許可及 oneshot 回覆，不含 PCM。每次完整處理當前 native block，私有 owned DSP emission 直接移交音訊並立即排空分段 ready，本次回覆最多兩份 chunk；舊公開 borrowed callback 不保留複本、不等待網路或佇列，契約不變。

CPU 在送回 chunk 前釋放，等待辨識時停止新的 decoder pull；閒置 worker 只等待命令或取消，不建 relay task。Native owner 依 available_parallelism 最多兩份，與舊正式解碼／前處理 factory 共用，許可持到原生資源真正銷毀及退出；CPU lane 仍為原有最多兩份。過載立即拒收，不排入額外工作。閒置取消必須可在同執行緒清理，不因 CPU 容量忙碌而拒絕回收。Active step panic 轉 typed error，仍持 CPU 銷毀 core；沒有把 abort 當作退出證據。

首 native run 若不足一個目標 frame 而完全沒有 mono output，後續第一份實際 mono 輸出沒有前一份可重設的音訊；私有 producer 將其第一序號標示為非 discontinuity，保留真正媒體起點，不補靜音。其他實際 mono gap 仍須通過原有 gate，不放寬正文與時間核對。

EOF 先完成 FIR／分段 flush 與合法正文尾段，正常銷毀持有 CPU，再 await 真正 native worker Join 後回傳 eof；最後辨識與有序合併仍須成功，才交付獨立 PreparedTranscript。Caller batch 最多四份，只有尚有兩份空間才繼續 pull，三或四份先交辨識；服務等待沒有 producer 背景前讀。未 poll／已開始 Future Drop、取消及失敗清理本次 PCM／本文並取消工作；開始後的 native owner 不提前釋放。

本批沒有新增 crate、unsafe、安裝、產品暫存檔、音訊落盤、快取、真正雲端或播放。Native 檔案 pin 在實際 EOF／取消回收時釋放；回傳 transcript 不持媒體、不代表 Ready 或播放配對證據，原有外掛字幕 Ready owner 與固定金鑰 UI 不變。跨正文句仍回報 BoundaryMismatch。真正 WAVE 與明確 mock recognizer 的驗證範圍、數字另列第七批驗證；正式辨識品質不能以 mock 結果驗收。決策見 [ADR 0042](adr/0042-demand-driven-native-prepared-asr.md)。

## 第八批：保留辨識來源的全文翻譯與臺灣本地化

2026-10-12 新增公開 `translate_recognized_transcript` library。Caller 明確交付 owned PreparedTranscript、Fast／Standard／High 品質、目前 Session 詞彙表與所選 Translator；建構 Future 不啟動服務，未 poll 的 Drop 也會取消本次工作並釋放原文。沒有預設服務、金鑰讀取、正式 ASR adapter 或 GUI ASR 接線。

來源字幕與辨識本文透過私有 borrowed enum／cue view 共用原有 Prepared 全文翻譯引擎。來源字幕仍須符合 ExistingSubtitle 與來源語言契約；ASR 另核對 sealed transcript 的完整 Session／generation、五語、原始正長度時間、有序且不重疊、二十四小時、8192 cues 與四 MiB。View 只借用原文與時間；只複製目前有界 batch 的要求與脈絡，不複製整份原文，也不替 ASR 製造 ExistingText、content ID、字幕出版物、來源仲裁或播放定位證據。

Fast／Standard／High 實際使用一／三／八句前後文。Fast 只做一次 Draft；Standard／High 完成全文 Draft 後，使用目前草稿、鄰近草稿、前後原文及已完成譯文做全文 Review。要求維持原始 Session、逐 cue 序號及媒體時間，倒序完成還原原順序；每批按服務能力限制一至四個要求，完整批次結束才排下一批。每要求仍是三十秒離線上限，沿用翻譯 client 跨 caller 共用的並行／速率限制與固定 zh-TW 指示；沒有放寬 Live freshness、期限或另建時間軸。

SessionGlossary 必須符合原 ASR 的 SID 與 generation；跨工作階段或 generation 的 glossary 在服務要求前拒收。前後文、草稿與詞彙都是不可信任的資料，不作指令；HTTP adapter 沿用既有 prepared_context 的有界傳輸與獨立要求期限，不因辨識來源而新增供應商條件或跨服務 fallback。本批支援 caller 提供的 Session 詞彙表，自動全文脈絡分析與 glossary 建立仍待完成。

服務等待不持 CPU 或 native-owner 許可。所有翻譯完成後，才在既有 process-wide 有界 CPU lane 本地化臺灣用語並再次核對原文、時間、句數與譯文。原始及本地化譯文各核對每句 8192 bytes、全文四 MiB；本地化後超量、空白、非法控制字元、句數不符或失敗都不交付結果。過載明確回報，沒有另建工作佇列。Worker 持有本文及許可至真正退出；Future Drop 的 abort 不被視為同步 worker 已停止，逐 cue 合作取消與完成後檢查拒收晚到結果。

完整成功後交付 PreparedTranslatedTranscript，持有原始 PreparedTranscript、同索引的 boxed zh-TW 譯文與實際品質。原 ASR 身分、來源語言、時間與本文保持不變；結果無 Debug／Clone／Serialize、永久內容快取或匯出。取消、要求 timeout、provider error／panic、Future Drop、本地化或最終驗證失敗皆釋放本次本文，不交付部分成功文件。

這份結果不持媒體 pin，不代表 Prepared Ready、媒體配對或播放定位；現有本機 actor 在真正 EOF／取消回收時釋放檔案 pin 的行為不變。後續 ASR Ready 組合須保留同一媒體 handle 的來源身分與生命週期，不能在 library 翻譯成功後重新開檔就宣稱原媒體已被 pin。一般 GUI 無字幕仍回報 AsrUnavailable，可靠外掛字幕流程不額外解碼，固定遮蔽金鑰及結案手動輸入安排不變。

本批沒有新增 crate、unsafe、元件安裝、真正雲端、產品音訊／字幕／翻譯寫檔、暫存媒體、快取或匯出。實作與合成測試只能驗證來源、脈絡、取消、時間、容量及 client 接線，不能代替正式辨識／翻譯品質或媒體驗收。理由見 [ADR 0043](adr/0043-recognized-transcript-translation-provenance.md)；本批實際檢查與測試數由完成驗證後另記，不預先列為通過。

## 第九批：同一媒體 pin 的 ASR Prepared Ready

新增 PreparedTask::run_with_recognizer，caller 明確交付辨識／翻譯服務、來源語言、取樣率、品質及可選的本次 glossary。先判定原有字幕優先序；有字幕時沿用全文翻譯／本地化，不呼叫 recognizer，也不在選定字幕無效時默默改用 ASR。無字幕時先核對 glossary 的完整 Session／generation，再進行同一媒體的辨識及 zh-TW 全文翻譯。

native worker 只開啟一次媒體，保留原 OpenedMedia，以 File::try_clone 複製同一檔案物件的 handle 給既有 IStream，不再次依路徑開檔。原 pin 不讀取或 seek，共用檔案位置仍只由 IStream 管理。EOF 在同一執行緒銷毀 decoder／MF／COM，等待真正 worker join 後移交 pin；native-owner 許可已釋放，原媒體仍在最後 ASR 要求、翻譯與 Ready 期間禁止 writer／刪除。網路等待不持 CPU 許可，原有 PCM、batch 與服務並行上限不變。

PreparedDocument 的 body 明確區分 ExistingText 與 PreparedTranslatedTranscript，original() 回傳安全借用的 Existing／Recognized 來源型別，譯文及 cue_count() 共用；辨識來源保留真正的原文、五語、時間、順序與翻譯品質，不冒充 ExistingSubtitle 或新增出版物／配對證據。原公開 recognize_local_media 與 translate_recognized_transcript 仍是獨立本文入口；其結果本身不持媒體、不代表 Ready。完整工作經目前 Session 的 apply 才成為 Ready，不自動播放。

下層辨識與翻譯使用連結到 Session 的子取消 scope，失敗保留 typed error，停止／新要求／Drop 仍會撤回子工作及媒體 owner。舊 completion 不能覆蓋新文件，取消後的成功也拒收。已開始同步工作保留資源至實際退出，不能以 abort 當作銷毀證明。本文仍只在本次 RAM；未新增 crate、unsafe、安裝、產品暫存檔、音訊保存、內容快取或匯出。

本批是 Session library 接線，沒有預設 mock、正式 ASR client 或一般 GUI 的音訊上傳入口；GUI 無字幕仍回報不可用，可靠外掛字幕及固定金鑰介面的實際用途保持不變。測試範圍為合成 WAVE 與明確 mock provider，不能當作真正辨識品質或播放定位驗收。決策見 [ADR 0044](adr/0044-pinned-recognized-prepared-ready.md)，實際指令與數字另列本批驗證。

## 第十批：OpenAI／Gemini 的正式 Prepared 辨識 HTTP 協定

新增 HttpPreparedRecognizer，caller 明確交付服務與 ApiKey，沿用既有 PreparedSpeechRecognizer／Session 接線。OpenAI 使用 whisper-1、verbose_json 及 segment 原生時間；Gemini 使用 gemini-3.5-transcribe 的 Interactions VERBATIM word_info 時間，以 caller 明確選定的來源語言為設定。兩者的五語及 16／24 kHz mono WAVE 包裝都有限制；constrained_auto_detection 為 false，Auto 在開檔或上傳前拒收。Gemini 的語言提示不冒充偵測，不宣稱可發現錯選語言或混語。

既有短期 PCM 只在 RAM 包裝 WAVE，不存音訊、不用 Files API、不傳來源路徑。OpenAI 使用固定 multipart 欄位及一般 chunk.wav 名稱，檢查 boundary 碰撞；prompt 僅保留最新 1024 UTF-8-safe bytes 原文。Gemini 使用 inline Base64，明確 store／background／stream:false，generation_config.transcription_config 使用原生 VERBATIM／word 時間；不送 context prompt、SMART／custom vocabulary、previous interaction ID 或背景工作。Session 詞彙仍由後續全文翻譯處理。

編碼與 JSON 解析在原有共用 CPU lane；借用 trait 需要先複製一份有界的 PCM／脈絡 owner，沒有假稱 zero-copy。HTTP 最多兩個要求，依共用 CPU 的固定容量調整，單核心一個；capabilities 與 Semaphore 用同一值，額滿明確拒收，不建立無限等待 queue。HTTP owner 與已開始 worker 共用 request permit lease，Future Drop／取消不會在真正 cleanup 前提早釋放容量。網路等待不持 CPU 許可。

固定 HTTPS endpoint、敏感認證標頭、驗證 TLS、無重新導向／代理及共用 connection pool。六十秒總預算包括速率等待、編碼、最多兩次嘗試、回應及解析；只有網路／逾時／限速失敗可重試。尊重 Retry-After，較長有效值保持實際冷卻且本次不重送，非法或溢位值不重試。原始 body 限 512 KiB，Content-Length 與 chunked 累積都檢查；只接受 JSON／identity encoding，錯誤 body 不讀取或記錄。建構不讀金鑰儲存庫或連線，不跨供應商 fallback。

建立 Future 前就配置連結到 caller 的內部子取消 scope，服務內部失敗／逾時／Drop 只撤回子工作；父停止仍向下傳遞。這避免解析錯誤被內部取消蓋成 Cancelled。原生時間、語言、排序、句數、本文一致性及容量嚴格核對，缺少正式輸出不冒充無語音。Gemini word_info 可使用原生 UTF-8 byte 索引，缺少索引時仍須精確核對 native word text 的全文位置；只有完整證據才按 first／last word 原生時間合併。未標註標點也保守拒收；完整 context 先驗證再排除，跨正文句子／單字回報 BoundaryMismatch，不裁時間或捏造字詞定位。

這是正式 API 協定 library，沒有把 mock 當預設服務，也尚未接入一般 GUI 的音訊上傳。固定金鑰 UI 與一般外掛字幕流程不變，真實金鑰／雲端辨識品質仍留至結案手動驗收。架構及官方來源見 [ADR 0045](adr/0045-bounded-prepared-speech-http-providers.md)，實際指令及測試數另列本批驗證。

## 第十一批：一般 Prepared GUI 的音訊辨識入口

2026-10-13 接續正式 HTTP adapters，讓一般 --ui 的「預先準備（本次）」沿用既有 Controller／Session／owned worker 執行辨識。外掛字幕欄完全留空時生成 AsrRequired，使用本機 RIFF/WAVE；非空路徑仍要求明確格式／語言，只有語法與完整資料通過才準備，不以無效、空白字元或損壞字幕觸發音訊上傳。中文只本機本地化，來源字幕只建立 Translator，可靠字幕不額外辨識。

新增獨立音訊來源語言 Auto／英／日／法／德／韓，與字幕語言分開。預設 Auto 目前尚未有可靠的限五語辨識能力，按準備會在取用金鑰前明確拒收；controller 同樣在 worker／憑證／媒體讀取前檢查。字幕準備忽略這個欄位，不因 Auto 或錯誤音訊語言影響原有流程；無字幕時也不依賴停用的字幕格式／語言。品質維持 Standard 預設，GUI 音訊使用固定 16 kHz 前處理，不新增技術取樣率選項。

目的地列依字幕欄及所選服務即時更新：中文不使用雲端、來源字幕只傳文字、無字幕傳短片段音訊供辨識與辨識原文／翻譯脈絡供翻譯。無字幕需明確選 OpenAI／Gemini；雙列固定遮蔽金鑰只讀所選列，準備後清除兩列與 undo，不查另一列補救空值。本次輸入不儲存；留空才在背景 worker 讀該服務安全憑證。用途與操作按鈕保持在捲動區之外，準備／Ready 期間欄位鎖定。

私有 factory 依 Translation／Recognition kind 建立服務，同一所選金鑰只解析一次，無字幕的 HttpPreparedRecognizer／HttpTranslator 使用相同 provider 與 key。Controller 不依 UI 狀態假設合法，服務回傳型別不符或失敗明確拒收；建構不送網路要求，不用 mock 當正式預設。既有單一 worker 呼叫 run_with_recognizer，完整原生時間、臺灣本地化、身分與同一媒體 pin 保留至 Ready。等待服務不持 CPU 許可，停止保持 Stopping 至 runtime／同步 worker 真正退出，不允許新工作重疊。

沒有新增永久內容、音訊落盤、快取、匯出或自動播放。關閉清除本次本文、pin、路徑、金鑰及通知；Ready 仍不是播放定位或仲裁證據。設計見 [ADR 0046](adr/0046-explicit-prepared-asr-gui.md)，操作見 [固定金鑰介面](api-key-ui.md#prepared-本次辨識與翻譯)。真實服務留至結案手動輸入，本批實際指令與測試數在驗證後另記。

## 第十二批：原生 MP3 解碼與壓縮時間限制

新增內容 preflight 及原生 MPEG-1 Layer III 解碼，公開 decode_local_audio library 可從原先的 pinned file object 使用 trusted audio/mpeg 及 Windows MF 交付借用 PCM／metadata summary。門檻支援 32／44.1／48 kHz、mono／stereo、兩個相鄰完整 frame，以及一 MiB 內的 ID3v2.2／2.3／2.4／footer；保留既有 WAVE gate。最多讀取 31 bytes、固定 seek，取消／短讀／I/O error 明確處理。沒有 filename／URL、metadata 字串、全檔 buffer 或第三方 codec 安裝，不接受 MPEG-2／2.5／free format／AAC／MP4 或任意 junk sync。

本機真實 MP3 decoder 測試發現，初段 PCM 的 decoder delay 移除會保留後續封包 native time，形成約 11–16.5 ms 的回報缺口。44.1 kHz 每 MPEG frame 另有 100 ns 取整，兩秒資料已觸發原有 DSP 的累積漂移檢查。本批原定的完整 Prepared 接線因此調整為可獨立驗證的原生解碼及明確時鐘門檻；不以放寬精度、補靜音、重設時鐘或中途交付部分音訊冒充成功。

Native 與 SampleAccurate 兩種 caller 要求明確分開。普通原生解碼保留真實時間／缺口；prepare_local_audio 及私有 Prepared actor／ASR 在 MP3 format preflight 後、MF／PCM／服務要求前回報 UnsupportedAudioClock。GUI 顯示這個 typed error，不上傳部分資料、不跨 provider fallback。WAVE 準備路徑及字幕優先保持可用，MP3 加外掛字幕不需進入音訊處理。壓縮時間到單聲道 sample clock 及 provider relative cue 的映射留待後續設計。

測試 MP3 由 Windows sink／encoder 在 HGLOBAL RAM 內從合成 440 Hz PCM 產生，沒有外部工具、下載檔或使用者音訊；只有測試具備 writable stream／編碼 helper。源 PCM 與 MP3 不假稱等長／逐值相同；分別核對 RMS／頻率、原生 PCM frames、缺口及完整壓縮 frame 結束時間。實際 VBR、任意使用者檔案及播放器／同步仍未驗收。設計及官方來源見 [ADR 0047](adr/0047-bounded-mp3-native-decoding-and-clock-gate.md)，實際指令及測試數完成後另記。

## 第十三批：有界解碼時鐘映射核心

新增 `media::decoded_clock`，以累積 decoded frames 建立明確 sample clock，同時保留每個 native 起點及真實 frames 推導的原生端點。PCM 原有 allocation 原樣移交，沒有複製、裁剪或補靜音。沿用原生 Session／generation、sequence、format、時間排序與一微秒相鄰取整門檻；真缺口建立新 run，sample origin 使用前 sample end 加測得的 native gap，不以重設 native origin 吞掉長 run 的量化偏移。既有 WAVE DSP 時鐘精度保持原有值。

rolling window 只持有最多 512 spans／十二秒的 metadata，超限明確 Overloaded 並清除，呼叫端只能明確 prune 已不需要的完整前綴。映射核對完整區間及所有涵蓋 spans，拒絕錯身分、跨缺口、未知／已 prune 區間或外推；單一 span 依兩種取整時間做有理內插，保留原生端點，不累積偏移。共享端點 start 採右側封包、end 採左側封包；原有一微秒相鄰取整差異保留，後續 cue 合併仍須驗證。取消／失敗成為 terminal，不能換 token 復活，沒有本文、路徑、序列化、檔案、網路或新增 worker。

真正自有合成兩秒 MP3 涵蓋三種原生 rate／兩種 channels，經此核心交付完全未放寬的 16／24 kHz DSP；核對 allocation／checksum、原生端點、每 run 精確輸出 frame、缺口、尾段、tone、pin、取消與新 Session。純 RAM 另驗證長 run 44.1 kHz 累積取整與小真缺口、窗口兩種上限、prune、半開區間、身分／順序、溢位及 terminal 清理。實際通過指令及數字於完成後列於本批驗證。

本批是可運作的映射 library，不啟用一般 MP3 DSP／ASR。接線需在 chunk 產生時擁有不可變、有界的 sample→native map，包含 context；不能讓最多四份十秒 ASR batch 等結果回來才查十二秒 rolling window。極短 native run 可能沒有輸出，native／chunk run 身分也需明確綁定。一般 WAVE 與字幕優先保持可用，MP3 仍在 PCM／服務要求前回 UnsupportedAudioClock，沒有部分 Ready 或上傳。原生 endpoint 證據與接線界線見 [ADR 0048](adr/0048-bounded-decoded-sample-to-native-clock-map.md)。

映射結果使用私有欄位及 getters，保留完整 Session／generation、來源 run、第一／最後 decoded block 序號與時間，避免後續跨 chunk 工作只傳裸時間。來源序號／run 與 ASR chunk 身分不同，不冒充 Ready 或播放證據。窗口另核對 source frame 起迄與 run origin，拒絕同一 Session 混入不同 normalizer 的不相容累積相位。

## 第十四批：分段映射與原生辨識定稿合併

新增不可變 DecodedClockSnapshot，按完整 audio／左右脈絡移交最多 512 個原始 spans；從完整 coverage 推導 source run，另外限制精確可用區間，不裁切原始端點或相位。快照沒有 PCM、本文、路徑、Clone／Debug／Serialize，窗口 prune／Drop 不會丟失已移交證據。保留原始與建立當下的準備取消 scopes，不建立 child 或取消 parent，不能綁定正常成功後也會取消的 provider request child。

PreparedSpeechChunk::with_clock_map 消費快照並核對身分、精確 audio 起迄與正文，拒絕覆寫；PCM allocation 原樣保留。stamp 仍是辨識取樣時鐘，服務 adapter 不新增 map payload。內部 Arc 只共享有界 metadata；merger 保存同一 map binding，complete 拒絕另一份等值快照重綁，也拒絕同文件直接／mapped 模式混用。相鄰 maps 的完整共用 source rows、format／run origin／frame 相位與來源順序均核對；合法擴充左脈絡使用 row 交集，音訊 end 不倒退，以免遺失尚未擁有的右脈絡。

raw 回應先通過原有 sample 身分、語言、正文／context 與時間驗證，倒序結果還原後逐 cue 轉回原生時間。正長度、原生正文邊界及整份順序仍嚴格核對，包含一微秒重疊也拒收，不裁句或移動時間。source run 與 chunk run 明確分開：DSP 沒有輸出的原生 run 可以略過；新 chunk run 的來源 run 嚴格前進。EOF 短正文可擁有前段右脈絡並共用 audio end／相同 context 範圍，最終 audio_end == body_end 規則保持，不外推合法的 DSP 短尾。

merger 保留最多八個不同準備 scope 的小型 guards，以既有 same_scope 去重；正常 Session 重用一個 scope，長媒體不增加 guards。較早已提交的 creation scope、暫存倒序／空結果與最後交付皆可撤回，取消優先於 NoSpeech。超過八個不同 scopes 明確 Overloaded，不忘記早期撤回規則或保留所有舊 maps。scope／本文清理、完整文件驗證與原有 batch／全文容量保持有界。

本批完成分段與結果合併的 library 契約。真正十三秒合成 MP3 經兩種 DSP rate、分段、移交快照與明確 mock 辨識，以原始 decoder 時間與 frames 的獨立公式核對最終原生時間；窗口／decoder 在辨識前銷毀，原檔 pin 仍持有。純 RAM 另核對四份四十秒延遲分段、脈絡、重綁／混用、原生相位、取消、共享 EOF／左脈絡、scope 上限與零輸出 run 的實際 DSP 行為。一般 MP3 producer factory／GUI 不變，仍在音訊或服務要求前回 UnsupportedAudioClock；沒有部分 Ready 或上傳。後續須將移交接入原有有界 native actor，處理大 block／長缺口／窗口邊界，再決定開放 MP3。設計見 [ADR 0049](adr/0049-chunk-owned-clock-evidence-and-native-finals.md)，實際驗證另列本批紀錄。

## 第十五批：原生 producer 映射移交

2026-10-14 將不可變分段快照接入正式依需求拉取的 native actor。內容 probe 判定 MPEG-1 MP3 使用 sample／native 映射，WAVE 保留直接取樣時鐘。仍由同一 worker 擁有 decoder、DSP、segmentation、窗口與原檔 pin；命令容量一、每次回覆最多兩份分段、辨識每批最多四份，服務等待不占 CPU 許可。

每個原始 block 先驗證完整來源相位與順序，以一筆 pending span 供本次分段建立完整 audio／context 快照，完成後才提交窗口。私有 append_retained 使用 segment 左脈絡與 DSP 尚未交付時間的最小起點作為單調 watermark，可查詢範圍仍限十二秒、存放最多 512 rows；保留完整跨界原始 span，不裁切端點或虛構 phase。被回收前綴拒絕查詢，跨左界部分額外至多一個 64 KiB／4.096 秒原始 block；公開 append 的完整端點容量檢查保持原有方式。

真正 native gap 先排完舊 DSP 尾段並關閉舊 segment run，再處理新 span。原有 gates 保留，下一個 push 仍驗證真缺口；零輸出原生 run 不產生假正文。先前已有正文但最後原生 run 沒有 target frame 時 EOF 可完成文件，全檔沒有輸出仍失敗。快照在所有 provider 等待前移交，定稿使用第十四批同一合併器還原 native time；取消與任何錯誤都不提供部分 Ready。

一般 --ui／Prepared ASR 可準備內容核對通過的本機 RIFF/WAVE 或 MPEG-1 MP3，字幕仍優先。公開 borrowed DSP 仍只接受 SampleAccurate，沒有放寬 WAVE 一微秒精度或改變 callback 契約。成功 EOF 等待原生 owner 真正退出，原媒體 pin 留到本次 Ready 清除。沒有新增產品音訊寫入、暫存媒體、快取、匯出、播放或跨服務 fallback。合成 MP3／明確 mock 的完整 Ready 證據與正式雲端／真實媒體驗收分開報告；固定遮蔽金鑰及結案手動輸入保持原有方式。設計見 [ADR 0050](adr/0050-native-prepared-clock-handoff.md)。

## 第十六批：全文專名與本次詞彙分析

2026-10-14 來源字幕及 ASR 原文先走真正所選服務的結構化分析，再開始全文 Draft／Review。所有窗口依序涵蓋完整文件，較後段發現的詞彙也供前段翻譯使用；不以假 TranscriptEvent 傳遞 JSON。GUI 使用明確的分析／翻譯組合契約，HttpTranslator 共用所選 client、兩個要求許可與速率 gate；舊 library 的手動 glossary 入口保留。中文既有字幕仍只在本機處理。

每個窗口最多 64 cues／64 KiB、至多 256 個窗口，承接上一窗口已接受的詞彙。服務逐字回傳來源詞彙，本機核對 cue 並推導 UTF-8 範圍，領域核心核對身分／序號及範圍。詞彙表最多 64 筆，source／target 各 256 bytes；target 在共用 CPU lane 本地化，轉換後再檢查容量。手動詞彙優先，同生成詞彙去重，衝突譯名或超限明確失敗，不抽樣或無聲截斷。

每窗口三十秒，caller-owned future、子取消 scope、token、worker 許可及原媒體 pin 涵蓋分析、合併與翻譯。失敗／停止不開始 Draft 或交付部分 Ready；全文與詞彙只在本次 RAM。固定目的地提示增加專名／詞彙整理用途，結案手動金鑰安排保持一致。正式供應商協定已接線，真實金鑰、雲端接受程度、語意／譯名品質仍待驗收；完整全文主題／語意摘要不列為完成。設計見 [ADR 0051](adr/0051-grounded-prepared-session-glossary.md)。

## 第十七批：AAC／ADTS 有界準備與時鐘門檻

2026-10-14 新增本機 MPEG-4 AAC-LC／ADTS 的固定二十 bytes 內容門檻，核對兩個完整相鄰 frame、格式及單一 raw block。正式 Prepared／GUI 開放 48 kHz 雙聲道，使用同一 native actor、取樣／原生快照、分段合併、全文詞彙及翻譯到 RAM Ready；既有字幕仍優先。native AAC subtype 與實際 PCM16 的取樣率／聲道在同執行緒核對，公開 borrowed DSP 仍拒收封包時鐘。

44.1 kHz 單聲道合成檔實際出現四微秒的原生起點／PCM 推導前段終點重疊。原有一微秒檢查保持；其餘辨識得到的取樣率／聲道組合在 PCM／服務要求前明確 UnsupportedAudioClock，不重設時間或假裝全 AAC 已支援。CRC 只辨識 header 大小，不宣稱 checksum 驗證。其他 profile／傳輸、前置 ID3、MP4／影片容器及真實檔案相容性仍待驗收。

共同 metadata row cap 改為 640，以容納合法最密集的 48 kHz／1024-frame packet 情形；十二秒上限、PCM／pending／回覆／ASR／worker 容量維持。真正 MF 會合併壓縮封包，測試分開核對壓縮 packet 數與 native 觀測，不虛構一 packet 一 row。合成 AAC 由內建 encoder 在 RAM 產生，另有純 RAM 的 560-row／十二秒快照驗證；沒有新增產品輸出／暫存檔或安裝。設計與官方資料見 [ADR 0052](adr/0052-bounded-adts-aac-and-native-clock-gate.md)。

## 第十八批：48 kHz AAC 單聲道與直接原生時間驗證

2026-10-14 將 AAC-LC／ADTS 正式範圍擴大為 48 kHz 單／雙聲道。先在測試內以同執行緒、純 RAM 的真正 MF Source Reader 直接查詢 IMF sample time／duration，另比對 PCM frame duration；不讓產品時鐘映射充當自己的 oracle。十三秒合成資料的 48 kHz 單／雙聲道原生 duration 觀測沒有重疊，44.1 kHz 單／雙聲道兩者仍有最多 43 個 100 ns ticks 的重疊。因此 44.1 kHz 維持前置 UnsupportedAudioClock，不藉換用 sample duration、重設起點、裁切 PCM 或放寬一微秒檢查通過。

正式變更僅擴大 native format 的已驗證聲道範圍及 GUI 格式提示；所有原生時間／PCM gate、取樣映射、十二秒／640-row metadata、PCM／pending／ASR／worker 容量、取消、Session 與原檔 pin 沿用。48 kHz 單聲道的十三秒合成檔經 OpenAI／Gemini 明確 mock 及 16／24 kHz 前處理，完整走同一 ASR、全文詞彙、Standard Draft／Review 到 RAM Ready，cue endpoints 由獨立 decoder timestamp／frame oracle 核對。既有雙聲道及中文字幕優先也驗證；沒有產品音訊／文字寫入、temp／cache、匯出、自動播放或新服務。

直接原生觀測只編譯於測試；有界 1024 次讀取、每個 native sample 最多一 MiB，要求真正 EOF，所有 COM／MF 與 sample 都在同執行緒 RAII 釋放。測試不使用使用者媒體、有效金鑰或真實雲端；不能外推為任意 AAC 編碼器、使用者檔案或播放同步已驗收。決策及直接 duration 官方文件見 [ADR 0052](adr/0052-bounded-adts-aac-and-native-clock-gate.md)。

## 第十九批：本機單音軌 M4A／MP4

2026-10-14 新增非 fragmented、單一 AAC-LC 音軌、48 kHz 單／雙聲道的本機 M4A／MP4 子集合。以相同 pinned file 做有界 metadata 門檻：最多 128 個頂層 boxes／一 MiB moov，一份 self-contained data reference，核對 sample description、所有 chunk／sample 的檔內範圍及 sample table 一致性。音訊 payload 以 seek 跳過；不新增整份媒體 buffer、下載或產品暫存檔。

影片、多音軌、外部引用、加密、fragmented、edit list、封面或其他非空 metadata 目前拒收；不是完整 MP4 容器支援。native AAC type／短 ASC／PCM 格式再次核對，48 kHz 使用既有一微秒 gate、時間快照及同一 ASR／詞彙／翻譯到 RAM Ready。44.1 kHz MP4 本批未完成時鐘驗收，仍於 PCM／服務要求前拒收；不能把 ADTS 的歷史時間重疊直接推論到 MP4。

固定 GUI 提示新增此音訊子集合及影片／多音軌限制；原有服務揭露、金鑰、中文字幕優先、取消／Session、worker／native 回收及 Ready 原檔 pin 維持。合成／mock 已核對兩個服務、16／24 kHz、native endpoint 與完整生命週期；正式金鑰、使用者檔案／雲端品質仍待結案驗收。設計及官方資料見 [ADR 0053](adr/0053-self-contained-mp4-aac-prepared-input.md)。

## 第二十批：M4A／MP4 44.1 kHz 原生時間與 Ready

2026-10-14 先直接量測真正 AAC／MP4 RAM 編碼的 mono／stereo、44.1／48 kHz、約一／二／十三秒及不整除尾段，共十二種組合。44.1 kHz 本機 native duration 的最大重疊為一個 100 ns tick，frame-derived 微秒端點沒有重疊，48 kHz 兩者皆零；全部到真正 EOF，sample／ReadSample timestamp 相同。量測沒有使用產品 normalizer／map 或保留整份 PCM；最多 1024 次讀取、每個 sample 一 MiB，維持同執行緒 COM／MF 生命週期。

通過原生量測後，只將 MP4 的 rate gate 擴大為 44.1／48 kHz 單／雙聲道；ADTS 44.1 kHz 仍明確拒收。正式 decoder 的 frame clock／一微秒 gate、chunk snapshot、bounded worker／ASR、原檔 pin 與 Session 皆沿用，沒有調整樣本或篡改原生時間。容器門檻仍是有限的單音軌 AAC-LC 子集合，不能把 stts 正值／sample count 一致性當作解碼時鐘已驗證。

新增 44.1 kHz tone／native gate／pin 與完整 Ready 測試。十三秒加短尾的真正音訊分別經 OpenAI／Gemini 明確 mock 與 16／24 kHz 前處理，native cue endpoint 由原 decoder 的 timestamp／frames 獨立 oracle 核對；完整詞彙與 Draft／Review、停止撤回、服務錯誤、中文字幕優先及許可回收都有測試。外部引用反例擴大至兩種率／聲道，仍在 speech／詞彙／translation 要求前拒收；所有 compressed borrowed DSP 繼續拒收。

固定 GUI 提示明確區分 ADTS 與 M4A／MP4 的率範圍，金鑰與結案手動驗收安排沿用。影片、多音軌、fragmented、edit list、封面與其他非空 metadata 仍未提供。合成／mock 不代表一般使用者檔案或真實雲端品質已驗收，Phase 09 尚未具備條件。決策與實際原生觀測見 [ADR 0053](adr/0053-self-contained-mp4-aac-prepared-input.md)。

## 第二十一批：有界的 M4A／MP4 描述標籤與封面

2026-10-14 在既有單音軌 AAC-LC M4A／MP4 中，開放 mdir／ilst 的少數已知描述項目。UTF-8 文字每筆 64 KiB，JPEG／PNG 封面 512 KiB，ilst 合計 768 KiB，仍受整份 moov 一 MiB 限制；封面只核對 type／開頭，不解碼、顯示或驗證完整圖片。trkn／disk／tmpo／cpil 以固定大小和數值範圍核對。單值／不重複、未知項目／外部引用／freeform／非空 Xtra／mdta／超量拒收；音訊 data reference、ES descriptor 和 sample tables 檢查保持。

檢查借用原有 moov，沒有回傳 metadata model，完成後釋放；URL 形狀的文字不解析或抓取，內容不進入詞彙、翻譯、UI、日誌或快取。原有有界 worker、native 一微秒精度、Session／取消、媒體 pin、Ready 與金鑰安排沿用，沒有新增元件或產品圖片工具。一般 metadata resolver／來源辨識仍未完成。

真正合成 M4A 只在測試內替換最後的 udta，保留原 mdat、AAC 與 sample offsets；加入自有文字、數值及一像素 PNG。四種率／聲道下，所有 PCM samples、sequence、native endpoints 及 EOF 逐筆與原檔相同；十三秒音訊經兩個服務 mock、16／24 kHz、詞彙／Draft／Review 到 Ready。詞彙與翻譯 mock 另檢查原文、脈絡、Draft 與 glossary 不含標籤標記。停止／錯誤／中文字幕優先及拒收外部引用都有測試；不是使用者檔案或真實雲端驗收。設計見 [ADR 0054](adr/0054-bounded-mp4-descriptive-tags.md)。

## 第二十二批：有限的 H.264／AAC 本機影片輸入

2026-10-14 接受單一 H.264 Baseline 視訊軌加單一 AAC-LC 44.1／48 kHz 單／雙聲道音軌，仍只取音訊。同一 pinned file 先核對每軌 self-contained data reference、描述與 sample／chunk 全部檔內範圍；兩個借用 cursor 檢查交錯與跨軌 aliases，沒有每個 sample 的新 range allocation。moov 保持一 MiB、avcC 64 KiB、尺寸 1–8192，所有迴圈檢查取消。

只選 FIRST_AUDIO 的原生 reader 沿用；不協商視訊 decoder、不渲染或送視訊到服務。H.264 Main／High、其他 codec、多音軌／無音軌、fragmented、edit list、未知結構仍拒收。AAC 軌可含 native muxer 的 version 0 ctts，核對表格大小與總樣本數，不替代原生時間或調整 PCM。音訊時鐘／ASR／詞彙／翻譯／RAM Ready、Session／取消／pin 和固定金鑰流程維持。

測試 encoder 只在 RAM 產生固定 NV12 影格及 PCM tone；獨立選視訊原生型別／尺寸、讀取非空 H.264 壓縮 samples 至 EOF，產品路徑只驗證音訊。純結構反例與真正 native／Prepared mock 證據分開，一般影片／使用者檔案、雲端與播放品質仍待驗收。設計見 [ADR 0055](adr/0055-bounded-h264-aac-mp4-prepared-input.md)。

## 第二十三批：H.264 Main／High 與有界組合時間表

2026-10-14 擴大單視訊／單 AAC-LC 音軌的本機 MP4，視訊描述接受 Baseline／Main／High，產品仍只取音訊。High 的 SPS 直接核對 ID、4:2:0／8-bit，借用 bit reader 只解析四欄位、prefix 最多五個零，不配置完整 RBSP；描述可無 extension，若有只接受完整四 bytes 的已核對格式。其他 profile／深度／chroma／SPS-ext 拒收，moov／avcC／sample rows／軌道引用／跨軌範圍與取消上限不變。

視訊 ctts 支援 version 0／1、零 flags、完整大小、正值 count 和總樣本數；音訊 ctts 保持 version 0，其他 tables 仍核對 version／flags 零。沒有把視訊 offset 用於音訊、實作 renderer 或放寬一微秒原生 gate。組合時間表的結構測試不代表真正 B frames、seek 或一般影片／muxer 已驗收，測試 encoder 仍採零 B frames。

真正原生 Main／High synthetic profiles 經型別／尺寸／壓縮 samples 與 EOF 核對；AAC 音訊 tone、clock、pin、取消與兩個所選服務 mock／16／24 kHz 的完整 Ready 可測。固定提示列出 profile 子集合，金鑰與結案手動驗收安排維持。設計與限制見 [ADR 0056](adr/0056-ordinary-h264-profiles-and-composition-tables.md)。

## 第二十四批：真正 B-frame MP4 的音訊準備驗證

2026-10-14 補上原生 Main／High、固定 NV12／PCM 的 genuine B-frame MP4，測試 encoder 於 BeginWriting 前設定 maximum B-picture count=2。原生壓縮 stream 必須有真正 B slice、時間倒序、正確影格數與 EOF；設定值或人工表格不作為證據。不改 NAL、ctts、mdat、PCM 或時間，正式容器／decoder／時鐘／ASR／Ready 維持前批實作。

Main／High、44.1／48 kHz、單／雙聲道八種組合通過 slice／reorder 證據；各四秒、六秒加尾段、十三秒加尾段共 24 種原生音訊時間有直接觀測。影片的重排不流入音訊 gate；tone、pin、取消、公開 borrowed clock、兩個所選服務 mock／16／24 kHz、標籤／PNG 的完整 Ready 與錯誤／中文字幕優先有測試。無效 video ctts／reference 在所有服務要求前拒收。

只新增有界 cfg(test) fixture／驗證與 windows 既有 OLE binding feature，不新增 crate、系統安裝、產品 encoder／renderer／媒體寫入／快取或播放。這是本機固定案例，其他 muxer／B-frame 設定／GOP／尺寸／fps／使用者檔案與真實雲端仍待驗收。金鑰維持結案固定介面手動輸入，Phase 09 尚未具備條件。設計見 [ADR 0057](adr/0057-genuine-b-frame-mp4-audio-evidence.md)。

## 第二十五批：tx3g 純文字樣本核心

2026-10-14 開始內嵌字幕的樣本層：`media::embedded_tx3g::Tx3gTrack` 接受明確的 track ID／語言／timescale、Session／generation／連續序號、原始 start／duration ticks 與借用的單份樣本 bytes。驗證兩 bytes 大端長度、嚴格 UTF-8 或 BOM UTF-16（兩種 byte order）、完整 sample 與本文。只接受無 modifier 的純文字；樣式、卡拉 OK、連結、位置及未知尾資料明確 UnsupportedSubtitle，不自動略過。文字不解釋 HTML／指示、不自動猜語言或本地化；Unicode 行／段落分隔符換成 newline。

零長度字串只代表該段清除，不延長前 cue、不填補空隙。空白／BOM-only／非法控制字元及畸形 Unicode 拒收。每份 raw sample 8194 bytes，decoded cue 8192 bytes，track input 八 MiB，本文四 MiB／8192 cues，包含清除樣本最多 16384 份；所有限制分開核對。以 u128 換算每個絕對端點，不加總截斷 duration；拒收 zero duration、時間重疊／overflow／微秒折疊與超過 24 小時的端點。任一樣本失敗立即釋放累積文字、保留 typed failure，finish 不交付部分文件；取消／舊 Session／generation／亂序同樣拒收。

完整輸出只是一份原始 `ExistingText`，content ID 是本次 Session／generation／track 的區域識別，不代表媒體版本核對、來源仲裁、Ready 或播放 alignment。核心無 I/O、provider、queue、task、unsafe、credentials 或保存功能，須在 Prepared 有界 CPU lane 由容器 reader 使用。合成樣本測試驗證與既有臺灣片語及五語 Standard Draft／Review 引擎共用原時間、跨 cue 脈絡；尚未提供實際 MP4 軌道探索、tables／references／edit list reader 或 GUI 選軌／優先接線，原有音訊格式門檻未放寬。一般 --ui 仍不支援內嵌字幕。設計及官方格式依據見 [ADR 0058](adr/0058-bounded-tx3g-plain-text-samples.md)。

聚焦測試 `embedded_tx3g` 的 22 項通過，1.15 秒（測試時間，不是字幕延遲）；格式檢查及全 targets／features Clippy 通過，Clippy 27.56 秒。首次跨階段測試將 sample 的一基序號誤當為全文引擎的零基 cue sequence，修正 fixture assertion 後通過，正式全文序號規則未改。Clippy 的固定大小切片建議採 `as_chunks::<2>()`，未新增 lint suppression。

## 第二十六批：同一 pin 的指定 MP4 tx3g 軌讀取

2026-10-15 接上 `media::embedded_mp4::extract_local_mp4_subtitle` Rust 呼叫介面，輸入本機媒體、本次 identity、非零 track ID 及明確 TextLanguage。以既有本機路徑／一般檔案／唯讀 sharing 檢查取得 pin，再只複製該 handle 供同一 worker 讀取，不依副檔名選格式、不重開 path。成功回傳完整 ExistingText 及同一原檔 pin；owner 停止 Session 時須丟棄結果。這是讀取邊界，不是 GUI 選軌、來源仲裁、Prepared Ready 或播放。

共用既有有界 MP4 top-level loader，保留 moov 一 MiB、128 boxes、單一 mdat／已知標籤／UUID 及 seek 跳過 payload；該 loader 會先讀 mdat 最後一 byte 核對可讀性。完整 sample 讀取前，檢查全部軌道、檔內 self-contained reference、sample description、stts／stsc／stsz／stco 或 co64／mdat 範圍，借用 chunk cursors 做每對軌道的 byte overlap 核對。限定一個既有 AAC-LC 音軌、可有一個既有 H.264 視訊軌及一個指定 `text` handler／`tx3g` 文字軌，最多三軌；多文字軌／音軌、未知 handler、外部引用、edit list、fragmented／加密及文字 ctts 都拒收。原有音訊 decoder gate 仍拒收帶文字軌的輸入，沒有以字幕 API 自動擴大 ASR 或原生 reader 的格式門檻。

tx3g 描述接受單一 data reference=1／零 reserved、零動態 displayFlags、已知 justification、合法文字框、零 default style 字元範圍、已知 face flags，以及最多 32 項唯一 font IDs／UTF-8 font names（每份一 byte 長度）、存在的 default font 或空表／未指定 default。只提取文字內容，未載入媒體字型或重現原文字外觀，使用者介面外觀仍由本產品管理；UTF-16 font names 及其他描述擴充暫不支援。原 sample modifiers 保持拒收。mdhd packed ISO 語言須匹配明確聲明或為 und，不能從 zho 推定中文地區。

文字 stts 正值 count／delta／總 sample 數與 mdhd duration 相符；tkhd duration 只接受依 movie timescale 換算的 floor／ceil，mvhd duration 必須涵蓋，媒體時間仍採原始文字 ticks，不用 movie rounding 改寫。無 edit list／ctts，所以不估計 offset；每份樣本以固定 8194 bytes scratch 讀取，原有八 MiB wire／四 MiB text／8192 cues／16384 samples／24 小時上限保持。任一錯誤、取消或 Future Drop 都不交付部分文件。

工作使用既有共用 Prepared CPU semaphore／available_parallelism 上限，不為此建立第二個 CPU pool；飽和立即 Overloaded，不等候排隊。未 poll 的 Future Drop 也取消；已開始的 blocking worker 持有許可與 pin 到真正退出，abort handle 不代表 I/O 已完成。取消後仍回成功、panic、過載與復原都有明確結果，沒有 provider、API key、音訊／視訊 decode、產品寫入／temp／cache／export。詳細決策見 [ADR 0059](adr/0059-selected-pinned-mp4-tx3g-reader.md)。

## 第二十七批：內嵌字幕 Prepared／GUI／Ready 接線

2026-10-15 `PreparedRequest` 新增一份明確 Mp4SubtitleSelection，可與外掛字幕共用原有語言優先順序與八份總預算。臺灣繁中、其他繁中、簡中、來源字幕依序選用；同順位回報 AmbiguousSubtitle，不以 embedded／sidecar 先後猜測。metadata-only 的 text_language 規劃發生在憑證／client 建立前，中文不建服務、不取用傳入金鑰；來源字幕只建立 Translation service，不建立 recognizer。內嵌選取不能失敗後退回較低順位文字或 ASR。

Session 的既有 bounded blocking load 開啟一次 media pin，再由前批 reader 的 read_pinned helper 複製同一 handle 讀取；不重開 path、不呼叫另一 async extractor 或取得第二個 CPU permit。完整 ExistingText 與 pin 進入既有全文詞彙、品質 Draft／Review、Taiwan phrase conversion／最終 validation，再以同一 completion gate 發布 RAM Ready。原始文字／時間保留，cloud await 不持 CPU 許可；取消、舊 Session／generation／sequence、停止／清除與 Drop 保持原規則。

一般 --ui 的準備視窗在既有格式欄新增「MP4 內嵌 tx3g（填軌 ID）」，字幕欄改為路徑／軌 ID 的明確雙用途。選此模式須填 1 至 4294967295 的 ASCII 十進位整數及明確字幕語言；不 trim，不接受符號、小數、全形數字、空白或溢位。格式控制項在空白字幕時也可選取，embedded 模式即使 ID 空白仍視為已選文字，停用音訊語言，不會因驗證失敗改為 audio upload。來源改變會更新 disclosure／欄位可用性；中文顯示本機處理，來源語言顯示只傳文字與脈絡至所選主機。既有固定雙列遮蔽金鑰／清空 undo／Ready 鎖定與通知流程共用，不新增 key、路徑或本文偏好設定。

這是手動指定已知軌 ID 的入口，沒有軌道探索清單／自動語言或版本判定；前批限定的 AAC-LC／可選 H.264／單 text／tx3g、無 edit list／文字 ctts／modifier 等門檻未放寬。普通 MP4 音訊解碼器仍拒收帶文字軌的 ASR 輸入；選取文字時使用獨立 reader，並不解碼媒體。完成僅代表文件準備，不播放、不授予 Live 仲裁或 alignment 證據，Phase 09 尚未具備條件。設計見 [ADR 0060](adr/0060-explicit-embedded-prepared-ready-and-ui.md)。

## 第二十八批：本機 MP4 字幕軌探索

2026-10-15 一般 Prepared 視窗新增「探索 MP4 字幕軌」，在背景讀取所選本機容器、驗證原有單文字軌範圍，填入 ID／內嵌模式及五語 ISO 提示。中文 zho 不能證明繁簡或臺灣用語，und 也不能自動選語言，兩者都重設語言供使用者指定。多文字軌清單／最佳字幕自動選用及其他格式仍不在本批。

探索沿用同一 MP4 metadata／description／tables／跨軌 byte range gate，不讀完整文字樣本，但保留共用 loader 的最後一 byte mdat 可讀性探測。原有單 AAC-LC、可選 H.264、單 text／tx3g、無 edit list／文字 ctts／modifier 等限制保持。結果只有有界 ID／語言提示，不是本文有效性、版本或 Ready 證據；按準備仍重新開檔／驗證 ID、語言及本文，再沿用同一 Session／全文／RAM Ready 管線。

async library 與 UI owner 共用既有 bounded CPU lane，一個探索 thread／唯一 StreamIdentity／一個 terminal reply，不新增 CPU pool。poll 只 reap 已退出工作；取消退休 buffered success，退出前禁止重啟，Drop 取消及 join。探索期間欄位／準備鎖定，金鑰及 undo 清空但不讀取、不建 provider；不自動準備／發 Ready 通知或播放。來源路徑變更退休探索及自動填入 ID／語言，程式化編輯也取消 pending scan；清除／關閉回收本次資料與 worker。設計見 [ADR 0061](adr/0061-bounded-local-mp4-subtitle-discovery.md)。

## 第二十九批：有界多字幕軌與明確選擇

2026-10-16 既有 MP4 text／tx3g 門檻擴大至最多八條字幕、一條 AAC-LC 音軌及可選一條 H.264，總計最多十軌。每軌仍驗證唯一 ID／描述／時間／檔內引用及全部軌之間的 byte overlap；moov 一 MiB、sample／wire／本文／cue、共享 CPU、取消及原檔 pin 規則保持。只抽取明確選定文字，未選軌結構仍需有效，本文不解碼、不送翻譯。原音訊 ASR gate 保持拒收文字軌。

新 plural discovery 返回固定八格 catalog，原 singular API 遇多軌明確 AmbiguousSubtitle。一般 GUI 在字幕行提供下拉清單；單軌沿用 ID 填入，多軌先留 ID／語言空白，由使用者明確選擇；不按中文提示推測繁簡或臺灣用語，也不按容器順序挑最佳字幕。手動 ID 保留並取消 combo 選取，語言／品質編輯保留同來源清單，來源變更／重新探索／取消／清除會退休舊 catalog；錯誤不保留部分清單。

準備依然只含一份明確 embedded selection，共用既有 Session／詞彙／品質翻譯及 RAM Ready，不新增處理時間軸／pool。固定遮蔽金鑰與目的地揭露沿用；探索不取用金鑰或 client，介面補上探索會清空雙列／undo 的說明。新 combo／bounded labels／scalar index 的 Win32 操作具 SAFETY，沒有 pointer payload 或 COM 逸出。設計、操作與限制見 [ADR 0062](adr/0062-bounded-mp4-multiple-subtitle-tracks.md)。

## 第三十批：完整文字軌 edit list 的前置延遲

2026-10-16 tx3g 文字軌新增有限 version 0 edts／elst：一筆完整正常速度 edit，可有一筆前置 empty edit。active media_time=0、empty=-1、rate=1、正 duration 與精確表格大小才接受；active 長度必須能以 movie timescale 精確表示完整 mdhd，tkhd 等於總長、mvhd 涵蓋。裁切、變速、重播、dwell、version 1 與其他描述拒收，無 edit 的原 floor／ceil 規則保持。

時間以「前置 movie ticks＋原始 media ticks」的有理數絕對端點一次 floor 到微秒，不能把兩個 floor 結果相加，也不累積逐樣本 rounding。映射後使用既有 Tx3gTrack 的微秒 timebase；原 stts 核對、clear gap、Session／序號與完整文件／同一 pin／RAM Ready 保持，24 小時端點包含前置空白。未選文字軌也須驗證，A/V 原 gate 仍拒收 edit；不新增 decoder、worker、queue、unsafe、媒體／文字寫入或雲端要求。

這是 Phase 08 容器時間解析，沒有啟動播放、動態音訊定位或 Phase 09；一般 GUI 操作／明確選軌／雙列固定金鑰及取消流程不變。合成原文、兩所選服務／三品質 mock 與 hidden native 可驗證到本機 Ready，真實影片與雲端仍留待結案。設計與官方資料見 [ADR 0063](adr/0063-full-track-tx3g-edit-delay.md)。

## 第三十一批：單一正常速度文字 edit 區間裁切

2026-10-16 文字 v0 edit 可宣告非負 media_time 與單一正 duration 區間，完整位於 mdhd 內才接受，可選前置 empty。使用共同有理數單位取得樣本／edit 半開交集、原點扣除及 delay，一次 floor 成呈現時間；可精確處理 active end 在分數 media tick 的情況。重疊 cue 保留完整原文並縮短時間，完全在外側者移除，clear gap 不延長；無 edit 舊時間保持。

先按原始 media timebase 完整讀取／驗證選定軌，包括裁切範圍外的 UTF／wire／sample／text／cue 與序號／取消。僅新增最多8192筆數值對照，在唯一本文 Vec 原地過濾／改時間，完整呈現驗證後才交給既有詞彙／翻譯；沒有複製全文或另一時間軸。只含空窗的 edit 不準備成空白 Ready，而是 InvalidSubtitle，沒有 ASR／服務 fallback。取消／同一 pin／Session 與全軌 metadata／range 驗證沿用。

目前仍限一筆 rate=1 active（可有 leading empty）、version 0；v1／多 active／A/V edit／文字 ctts／modifier、變速／dwell／repeat 仍拒收。這是 Phase08 靜態容器解析，不新增使用者剪輯、媒體寫入、播放或 Phase09 對齊。操作與固定金鑰規則不變，詳見 [ADR 0064](adr/0064-single-interval-tx3g-text-crop.md)。

## 第三十二批：文字軌 version 1 edit list

2026-10-16 tx3g 的 elst 新增 version 1：每 entry 的 duration／signed media_time 以 64 bits 讀取，rate 保持 32 bits；v0 解碼沿用。先核對零 flags、1 或 2 筆、精確表格長度，再按版本讀取，不猜版本或截斷高位。只允許前置 empty 的 -1，active 仍須非負、rate=1、正 duration，完整位於 mdhd 內。delay＋active 使用 checked_add，tkhd／mvhd、24 小時及原有有理數裁切規則保持。

只擴充 elst，mdhd／tkhd／mvhd 仍限 v0；有效 media_time 可高於 i32::MAX，但範圍及總長不能繞過原有 u32 標頭。多 active、變速／dwell／repeat、A/V edit、文字 ctts／modifier 及未知版本仍拒收。完整本文、唯一文字 Vec、數值對照上限、Session／取消／pin、GUI 選軌、中文本機或所選服務／品質與固定金鑰均沿用，沒有新增播放、產品寫入或 Phase 09。見 [ADR 0065](adr/0065-version-one-tx3g-edit-list.md)。

## 第三十三批：有界全文脈絡摘要

2026-10-16 原全文詞彙分析每批同時產生必要的 context_summary，下一批傳入 prior_summary，整份成功後以獨立 PreparedTextAnalysis 封存詞彙與摘要；不把語意塞進詞彙表。摘要限 4096 UTF-8 bytes，綁定 Session／generation、五語、品質與完整段數；缺少、空白、非法控制字元、過長或身分不符都明確失敗，不省略摘要後發布 Ready。臺灣本地化與取消沿用既有 CPU lane／deadline，沒有新增雲端要求或第二個時間軸。

所有自動來源字幕／ASR 翻譯的 Draft／Review 共用最終不可變摘要 Arc，維持各品質原有 pass 與鄰近脈絡策略。摘要是有損模型推論，可能錯誤，只作理解提示，不作識別／版本／仲裁／alignment 證據，也不補寫原文沒有的情節。中文本機路徑及明確手動 library 詞彙入口不自動做摘要。GUI 目的地揭露補上全文脈絡摘要，固定遮蔽金鑰不變；沒有內容保存、temp／cache／export 或 Phase 09。見 [ADR 0066](adr/0066-bounded-prepared-document-context.md)。

## 第三十四批：有界 HTTP(S) 媒體 range 核心

2026-10-16 新增獨立 RemoteHttpMedia Rust library：明確 remote MediaInput／本次 identity／取消 scope，首個一-byte GET 驗證強 ETag、total 與 range，之後同一 client／URL 以 If-Match 讀取精確 byte ranges。只接受 206、明確 Content-Length、單 range 與 identity 編碼；版本／長度改變、截斷、重新導向或忽略 range 都明確失敗，不下載整份或改連其他位置。HTTPS 驗證保持，沒有原始 URL／錯誤或媒體記錄。

限制來源四 GiB、每段最多 64 KiB、65536 份要求、兩個 owner；全程序最多四份讀取中／下游保留的 blocks，payload 至多 256 KiB，切換／退休來源不釋放仍被保留的 block 配額。無等待佇列，過載明確失敗；單要求三十秒，取消及未 poll／執行中 Future Drop 退休來源，舊 range 的新存取不可用。沒有新 CPU pool／執行緒、媒體寫入、temp／cache／export 或第二時間軸。

本批是實際網路讀取 library，尚未接入 Prepared producer／decoder、Ready 或一般 URL GUI；原正式路徑仍回 UnsupportedUrl，不以 range 成功宣稱影片已可準備或播放。完整設計、範圍、生命週期及後續接線見 [ADR 0067](adr/0067-bounded-session-http-media-ranges.md)。Phase 09 尚未具備條件，固定金鑰安排保持。

## 第三十五批：遠端內容格式前置檢查

2026-10-16 新增 probe_remote_audio library，以私有 Read／Seek adapter 將同一 RemoteHttpMedia 接入既有 WAVE／MP3／AAC-ADTS／有限 AAC-LC MP4 內容 gate，不依副檔名或 MIME 猜測格式。每次最多 64 KiB、不預讀或保留 byte cache，成功轉回原 owner，保留 identity、強 ETag／total、client 及已前進的要求序號。大讀取分段，seek／EOF 與錯誤處理有界；版本改變、網路或過載錯誤不被連帶取消蓋掉。

格式解析使用原 Prepared CPU lane，網路等待前釋放 CPU，資料到達後重新 try_acquire；過載不排隊。只有已持有遠端 owner 才能排程 blocking job，全程序最多兩份，沒有新 CPU pool。取消等待實際退出；Future Drop 也取消，已開始工作持有來源／配額直到清理。MP4 moov 沿用一 MiB 上限，其暫時 metadata 不計入 range payload 的 256 KiB 上限，隨 parser 釋放。

這是格式前置檢查；WAVE 只通過既有十二-byte 標頭 gate，其他合成結構也不代表 playable codec。沒有 native decoder、PCM、ASR、Ready、播放或 URL GUI 接線，一般介面仍拒收 URL。Phase 09 尚未具備條件，金鑰安排不變。設計見 [ADR 0068](adr/0068-bounded-remote-content-preflight.md)。

## 第三十六批：有界遠端 PCM16 WAVE 解碼

2026-10-16 新增 RemoteWaveDecoder／RemoteWaveBlock async library，接收同一 ProbedRemoteAudio，實際拉取未壓縮 PCM16 WAVE，沿用 DecodedAudioBlock／Gate、Session／generation、完整框格與累積取樣時間；不是只有格式探測。只接受 canonical 十六-byte fmt、單／雙聲道、8–192 kHz、一份非空 data 與有界 JUNK。完整 RIFF、長度／alignment／byte rate、padding、128 chunks／一 MiB JUNK／二十四小時及剩餘 HTTP 要求預算在 PCM 前核對，其他格式明確拒收。

每塊最多64 KiB；全程序最多四份在途／下游保留 PCM，至多256 KiB，獨立於原 HTTP range payload 配額。舊來源退休不釋放仍被保留的 PCM 許可，慢消費端在新 I/O 前失敗，不建音訊快取。網路等待不占 CPU；轉換使用原 CPU lane，每4096 samples 檢查取消／yield。錯誤及未 poll／執行中 Future Drop 都取消 scope，舊 block 的新借用不可用，沒有部分 block、補靜音或假 EOF。

本批以 Rust 解出固定 PCM16，不改本機 MF／COM／壓縮 decoder。遠端分段／ASR／RAM Ready、壓縮格式的 native HTTP／COM 橋接及 URL GUI 仍待接線；一般介面仍拒收 URL，不以 decoder library 宣稱網站、影片、雲端或播放定位已驗收。Phase 09 尚未具備條件，金鑰維持結案固定介面手動輸入。設計及格式來源見 [ADR 0069](adr/0069-bounded-http-pcm16-wave-decoder.md)。

## 第三十七批：遠端 WAVE Prepared 分段與辨識

2026-10-16 新增 recognize_remote_wave library，接收同一 RemoteWaveDecoder 與呼叫端明確選定的辨識器，先驗證能力再拉取音訊。沿用 PreparedAudioProcessor／PreparedSegmentBuilder／PreparedRecognitionSession 與原文合併，完成單聲道16／24 kHz、十秒正文／半秒左右脈絡及完整 EOF 尾段；沒有第二套 DSP、ASR 或時間軸。

網路等待不持 CPU；一份原始 PCM 與 DSP／分段狀態移至既有 CPU lane 上的有界 blocking job，一來源最多一份工作。每 pull 最多兩份分段，三／四份待處理分段先完成辨識再拉取後續媒體，服務等待不預讀來源。開始前或執行中取消／Drop 退休 scope，排隊／執行中 job 的 CPU／PCM 許可保留到真正回收；過載明確失敗。辨識的並行上限、逾時、前文、Session／generation 與亂序合併沿用原契約。

只有完整媒體 EOF、DSP 尾段與所有服務要求成功，才有 RecognizedRemoteWave。原 EOF decoder／HTTP owner 與原文綁在一起，最後服務等待與結果保留期間不重開 URL，結果 Drop 才釋放 owner 並取消 scope；原文僅提供核對取消後的借用。來源變更保持 typed error，不因連帶取消而變成一般 Cancelled，不交付已完成批次的部分文件。全部內容僅在 RAM；無產品音訊／文字寫入、temp／cache／export 或金鑰取用。

本批只接原文辨識，遠端全文分析／翻譯／RAM Ready、URL GUI 與壓縮格式仍待接續；一般準備視窗仍明確拒收 URL。自有合成 PCM、實際 loopback HTTP 與明確辨識 mock 不代表真實雲端品質、網站／影片相容性或播放定位已驗收。Phase 09 尚未具備條件，金鑰維持結案固定遮蔽介面手動輸入。見 [ADR 0070](adr/0070-demand-driven-remote-wave-prepared-recognition.md)。

## 第三十八批：遠端 WAVE 全文處理與 RAM Ready

2026-10-16 增加明確的 PreparedTask::run_with_remote_wave_recognizer 程式庫入口，串接同一 HTTP owner、內容前置檢查、PCM16 WAVE 解碼與既有辨識，再進入原有全文摘要／詞彙、Fast／Standard／High 翻譯、臺灣本地化與完整文件驗證。本機與遠端共用 finish_recognized，沒有另建翻譯演算法或時間軸；只有目前 Session／sequence 的完整 completion 才能進入本次 RAM Ready，仍不自動播放。

PreparedDocument 私有來源擁有者明確區分本機唯讀檔案與遠端 EOF decoder。新增 crate-private 原文／來源移交，取消先核對，來源保留跨越全部分析、翻譯、最後本地化、completion 尚未 apply 與 Ready；不重開 URL、不建本機媒體暫存。原 HTTP client／ETag／total／sequence 與配額保持，來源變更及其他遠端 typed error 經 MediaError::Remote 保留，連帶子 scope 取消不覆蓋原原因。Session 停止／替換、future Drop、失敗或丟棄結果會退休來源，沒有部分文件。

只接受 remote 且 AsrRequired 的規劃；有選定文字的 URL 組合回 UnsupportedUrl，同順位先回 AmbiguousSubtitle，本機輸入回 InvalidInput，全部不連線或發服務要求。舊詞彙與辨識能力在 HTTP 前驗證，不忽略可靠字幕或把壞文字改成音訊上傳。全文分析成功才可 Draft／Review，無摘要或服務錯誤不自動降級。所有內容僅 RAM，沿用原 CPU／owner／PCM／文字／並行／逾時上限，服務等待不占 CPU 或預讀音訊。

一般 --ui 尚未接此入口，仍拒收 URL；本批不含 URL 視窗、URL 搭配現成字幕、其他遠端編碼或實際媒體／雲端／播放定位驗收。正式金鑰維持結案固定遮蔽介面手動輸入；Phase 09 尚未具備條件，達標時會提醒。見 [ADR 0071](adr/0071-remote-wave-session-analysis-translation-ready.md)。

## 第三十九批：一般 HTTP(S) WAVE 準備介面

2026-10-17 將既有遠端 WAVE 到 Ready 接入一般 --ui 準備視窗。媒體欄同時接受本機完整路徑與明確 http://／https:// 網址（不分大小寫），沒有副檔名或網站猜測；URL 語法與容量仍完整驗證，不 trim／修復。字幕欄必須真正空白且不選內嵌軌，再明確選五語與服務；Auto 與未選服務在金鑰取用前拒收。固定遮蔽 OpenAI／Gemini 欄及取用後清空兩欄／undo 保持，正式金鑰仍留至結案手動輸入。

沿用同一 controller、單一背景 worker／runtime、有界 completion、所選服務 factory 與一次金鑰解析。metadata 規劃先保留字幕歧義錯誤；URL 加任何選定字幕在 Session／worker／服務與安全憑證前拒收，不把無效／空白字幕改成上傳音訊。本機 Recognition 仍走原流程，遠端明確呼叫同一 remote PreparedTask，使用16 kHz、全文摘要／詞彙／三品質翻譯／臺灣本地化及 RAM Ready，不重寫管線。

固定可見揭露先說明按準備會連線媒體網址，再列所選 AI 端點及短片段音訊、辨識原文、摘要／詞彙、翻譯前後文；不將私有 URL／query 複製至目的地、狀態、通知或 AI 要求。輸入／選服務／更新揭露不連線。網址停用 MP4 探索按鈕，直接／過期命令也不啟動掃描；來源變更退休探索 metadata／自動 ID，保留手動字幕選擇。取消、關閉、Ready 鎖定、清除網址與 undo、舊 completion 隔離及不自動播放沿用。

本批只開放既有 canonical PCM16 WAVE 子集合，沒有一般網站、登入／重新導向、其他遠端 codec 或 URL 搭配可靠字幕。錯誤保留 typed cause，沒有新 pool、套件、音訊／文字落盤、temp／cache／export、遙測或第二份時間軸。真實媒體／語意／雲端、五語 Auto 及播放仍待接續；Phase 09 尚未具備條件，達標時會提醒。見 [ADR 0072](adr/0072-explicit-http-wave-prepared-ui.md) 與 [固定金鑰介面](api-key-ui.md#https-wave-本次準備)。

## 第四十批：HTTP(S) WAVE 搭配本機外掛字幕

2026-10-18 將一般網址準備擴大到明確選定的本機 SRT／WebVTT。沿用八份候選上限、語言順位及同順位歧義；所有遠端內嵌選取仍在金鑰／factory／worker前拒收，不忽略選擇而改做 ASR。字幕路徑需通過原有本機語法及讀取防護；選定文字無法讀取／解析就失敗，不改傳音訊。音訊 Auto 不影響已明確宣告的字幕語言。

新增明確 remote-wave-subtitle task 入口，共用本機外掛讀取／解析及全文處理。先在原有有界 worker 解析選定字幕，成功後才讀取同一 HTTP owner 的來源／WAVE 標頭；沒有拉取 PCM、DSP 或辨識要求。中文三類在本機轉為臺灣用語，不建服務或取金鑰；來源字幕只建立所選分析／翻譯服務，沿用全文摘要／詞彙、三品質及本次 RAM Ready。來源 owner 跨服務等待、completion及Ready保留，停止／Drop清理、舊結果隔離及配額不變。

固定提示依外掛中文、外掛來源語言、無字幕音訊與不支援內嵌分別揭露用途；網址與query不進AI要求或目的地／狀態／通知，來源字幕不送媒體音訊。固定兩列遮蔽金鑰及所選取用／清空、Ready鎖定／取消／清除、不自動播放保持，遠端探索仍停用。使用者的字幕關聯是聲明，不是媒體版本或播放定位證據；Ready也不凍結稍後可能改變的遠端內容。詳見 [ADR 0073](adr/0073-remote-wave-local-sidecar-ready.md)。

## 第四十一批：獨立 HTTPS 外掛字幕讀取

2026-10-18 新增明確 HTTPS SRT／WebVTT 的有界讀取及完整文件解析程式庫，保留宣告語言、原始時間與 Existing 身分。輸入先核對真實 Session／generation／sequence，URL 語法沿用防護，不能用假的 Live capture stamp 或識別資料呼叫舊 lookup；Live 期限與容量保持。

單次 GET、不重新導向／重試／scheme fallback或使用代理；明確 MIME／UTF-8／identity、宣告長度與實際 body核對，支援有界未知長度及chunked。網路原始文件最多八MiB；全程序最多兩份要求／本文／結果持有者，過載不排隊。網路等待不持CPU，解析共用原有CPU lane與8192 cues／四MiB本文限制，原始body於解析後釋放。

未poll／網路等待／解析排隊或已開始的Future Drop與父取消都退休子scope；持有者保留配額至實際退出。結果取用前核對取消，沒有Clone／Debug／Serialize或內容寫檔，錯誤不含私有URL／query。404、授權限制、限流、重新導向、逾時、壞字幕及超量都明確失敗，不改做ASR。詳見 [ADR 0074](adr/0074-bounded-prepared-https-subtitle-reader.md)。

這是原文讀取library，尚未接入PreparedRequest／Session／Ready／一般GUI，也不讀金鑰、建立辨識或翻譯服務。現有一般視窗仍只接受本機外掛字幕，字幕網址仍拒收；不以library成功宣稱已可使用網路字幕準備／播放或完成臺灣本地化。後續仍需來源規劃、用途揭露與同Session完整準備接線，Phase09尚未具備條件。

## 第四十二批：HTTPS 字幕 Session／RAM Ready

2026-10-18 新增明確HTTPS字幕候選，與本機外掛及內嵌軌共用八份總量與語言順位，同順位先回歧義。新Task明確入口只讀取勝出網址；未選網址不連線，本機或內嵌勝出沿用既有文字路徑。舊Task／一般controller拒收任何網址字幕候選，保留先前無額外目的地的契約，且在對應I/O／factory／金鑰前拒收；一般GUI尚未開放字幕網址。

遠端文字勝出先持有本機媒體pin或同一遠端WAVE標頭owner，再以同Session／generation／sequence讀取單份字幕，不做PCM／DSP／ASR。來源缺translator、舊詞彙及遠端媒體帶任一內嵌選取在preflight失敗，選定字幕錯誤不退回音訊。中文本機轉為臺灣用語，五語共用原有全文摘要／詞彙、三品質與一致性翻譯，不建立額外client、供應商或時間軸。

原文移交給LoadedDocument後仍保留HTTP字幕lease，跨分析／翻譯、未apply completion與Ready持續占用全程序兩份本文配額中的一份；body先釋放、lease最後釋放，中途失敗亦保持順序。網路／服務等待不持CPU，取消／Drop／舊結果隔離及完整成功才Ready保持。平坦遠端字幕typed failure保留404／429／redirect等原因，不因子scope清理蓋成Cancelled。詳細理由見 [ADR 0075](adr/0075-prepared-https-subtitle-session-ready.md)。

這是明確程式庫Task入口，尚未將一般控制器／視窗接到新增字幕目的地。Ready不是媒體配對、仲裁或播放定位證據，不自動播放；正式金鑰仍留至結案固定介面手動輸入，Phase09尚未具備條件。

## 第四十三批：一般視窗的 HTTPS 字幕準備

2026-10-18 新增兩個明確來源／格式選項，使用既有字幕欄輸入HTTPS SRT或HTTPS WebVTT的完整網址。本機外掛及內嵌模式不因網址文字自行轉成網路輸入；HTTPS模式即使空白仍視為明確字幕，格式／語言／必要服務及網址語法在取密碼前核對，失敗不改做ASR。媒體可為本機路徑或原有限直接HTTP(S) PCM16 WAVE。

`into_https_subtitle_request`只做被動驗證，新的`PreparedBackend::start_with_https_subtitles`預設明確拒收，不委派舊入口；controller實作共用單一worker／runtime／completion與Session。舊start仍拒任何遠端字幕候選，新入口至少需要一份，保留歧義／遠端內嵌等preflight與上一批Task的完整處理、本文／媒體持有、取消及Ready規則。

固定用途區在準備前說明字幕全文連線、本機媒體唯讀或遠端WAVE標頭；中文在本機本地化，五語只將原文、全文摘要、專名／詞彙及翻譯前後文送往所選AI端點，不傳媒體音訊或私有網址。中文不讀金鑰或安全憑證；五語僅建立所選Translation服務，不檢查辨識能力，音訊Auto不影響明確字幕。雙列固定遮蔽金鑰、只取所選列一次、兩欄／undo清空及本次RAM保持。

HTTPS字幕模式停用探索且直接命令也防護，避免自動切換內嵌模式取代網址；其他本機模式仍可探索。格式切換保留手動輸入，Busy／Stopping／Ready鎖定、停止回收、舊completion隔離、清除網址／undo、通知一次及不自動播放沿用。既有29個控制項與可滾動內容保留，固定用途區按實際原生文字量測配置高度。詳細決策見 [ADR 0076](adr/0076-explicit-prepared-https-subtitle-ui.md)。

本批不新增遠端格式或內嵌、一般網站／登入／重新導向、內容辨識／仲裁或播放定位。金鑰仍留至結案固定遮蔽介面手動輸入；真實媒體／雲端／語意／多螢幕／播放仍待驗收。本批完整工程驗證已通過，已具備開始Phase09對齊核心的工程條件；本批不自行啟動下一階段。

## 後續範圍

Phase 08 的核心工程已有對應實作，完整產品驗收尚未結案。WAVE 準備／DSP、原生 MPEG-1 MP3 解碼、有界時鐘與分段快照／原生辨識合併、離線 ASR 排程、同一媒體 pin 的 ASR Ready、OpenAI／Gemini 正式 HTTP 協定及一般 GUI 接線已提供。限五語 Auto、MP3 的正式 VBR／真實檔案相容性、其他 AAC 時鐘／影片與一般容器相容性、更多內嵌字幕格式／實際檔案驗收、更多 HTTP(S) 媒體格式、遠端內嵌、全文摘要的實際語意／指涉與跨句切段品質、播放與實際音訊定位仍待接續。

HTTP(S) 已提供有界 range、內容格式前置檢查、有限 PCM16 WAVE 解碼與 Prepared DSP／分段／辨識、全文分析／翻譯／RAM Ready，並接入一般準備視窗及明確本機 SRT／WebVTT；中文本地化或來源字幕全文翻譯不額外做 ASR。HTTPS字幕網址已有獨立有界讀取／原文解析library及明確Task，第四十三批再以新的明確backend入口、來源／格式選項及用途揭露接入一般視窗；原有start／本機模式仍拒收字幕網址。遠端內嵌及其他遠端編碼仍待提供，不以WAVE或字幕library／mock成功宣稱一般網站或遠端影片已驗收。一般無字幕 Prepared 辨識已支援 WAVE／MPEG-1 MP3／AAC-ADTS，以及有限的 AAC-LC M4A／MP4（可含一軌 H.264 Baseline／Main／High）；ADTS 限 48 kHz，M4A／MP4 支援 44.1／48 kHz，皆限單／雙聲道；合成／mock 的完整 Prepared 成功不能取代真實辨識品質或播放時間軸驗證。

Ready 只代表本次完整文件已準備；外掛字幕關聯仍是使用者聲明，辨識來源則保留實際讀取媒體的 pin，兩者都尚未產生播放定位或仲裁證據。一般 --ui 同時保留 Live 與本次 Prepared 準備入口，Phase 07 本次指定 HTTPS 字幕入口保持可用。Phase 09 尚未啟動；本批完整工程驗證已通過，可提醒使用者開始Phase09對齊核心的規劃與實作，不代表現有相容性／品質限制已消除。實際媒體、Windows 通知顯示、多螢幕／DPI、播放器同步與真實雲端仍須個別驗收；有效 OpenAI／Gemini 金鑰維持結案固定介面由使用者手動輸入。

## 下一階段工程條件

2026-10-18 第四十三批重新依 [SPEC 第82節](../SPEC.md) 的十項工程目標核對，不再只沿用歷史「Phase09尚未具備條件」結論：

| Phase 08 目標 | 目前對應範圍 |
| --- | --- |
| Local file | 一般視窗的本機字幕、WAVE／MP3／AAC／有限M4A與MP4準備 |
| HTTP(S) URL | 直接PCM16 WAVE及明確HTTPS字幕；不代表一般網站或任意編碼 |
| Embedded subtitle | 明確text／tx3g選軌、有限時間表與全文處理 |
| ASR fallback | 無明確字幕時辨識；選定字幕錯誤不改傳音訊 |
| Parallel translation | 原有有界並行、全文摘要／詞彙與順序還原 |
| Consistency pass | Standard／High的Draft後全文Review |
| Completion notification | 原生一次完成通知與失敗狀態；真人通知顯示仍待驗收 |
| Cancellation | Stop／Drop、實際worker回收及舊Session隔離 |
| Temp cleanup | 目前產品路徑不產生temp，適用性為N/A；沒有虛構清理驗收 |
| No persistent cache | 本次RAM owner，無音訊／字幕／翻譯持久快取 |

這些工程基礎已有對應實作；本批格式、Clippy、Debug／Release建置與完整回歸已實際通過，可提醒使用者開始Phase09的對齊核心規劃與實作。這不是Phase08全面產品驗收結案，亦未自行啟動下一階段。

限五語Auto仍是未完成的全產品要求，與更多格式／一般muxer、實際語意及使用者明確延至結案的金鑰／雲端／真人DPI驗收分別保留。SPEC允許不支援的URL回報Prepared不可用再由使用者改選Live，沒有要求第一版接受所有網站與編碼；fuzzy matching、目前位置、seek、offset與drift則是Phase09本身要完成的工作，不能因其尚未實作而反向阻擋該階段開始。

## 首批驗證

2026-10-11 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：733 library、12 設定、4 探索、10 啟動，共 759 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

新增 48 項一般測試：16 項 Session／選取／準備工作、14 項 Prepared parser、16 項本機輸入／有界讀取、2 項 Windows 磁碟分類／指標輸入。Prepared 聚焦 36 項及 media::input 聚焦 16 項通過後執行全套回歸。測試涵蓋真正本機唯讀來源與 SRT／WebVTT、臺灣用語、原始時間、1,100 段文件、五語／中文聲明、字幕優先與歧義、Live 期限不變、8192／四 MiB／八 MiB 邊界、非法語法／時間／UTF-8、取消／成長探測、舊 completion、新 ID、停止／Drop、過載、worker panic 與復原。

已開始 worker 的 Future Drop 使用明確 barrier fixture，實際驗證取消後仍持有容量、第二份要求被拒、worker 退出才釋放及後續真正讀取成功，沒有靠 CPU 快慢或 sleep 猜測。Windows 測試實際驗證目前暫存磁碟查詢、原檔 handle 拒絕 writer／刪除，以及自有 junction 父路徑拒收；fixture 移除 junction 本身後確認目標 marker 未改。映射 SMB 未實際掛載／測試，其遠端類型拒收由純分類測試驗證。

首次編譯補上未使用 import 及新 native module 註冊，未放寬 lint。README 與 SPEC 的階段描述同步更新；Prepared 解析共用語法，但獨立生命週期與預算，完整回歸確認沒有放寬既有 Live 行為。新增 unsafe 只有隔離的 GetDriveTypeW 呼叫，附四個 UTF-16 單元、NUL、借用指標及同步 lifetime 的 SAFETY 說明；沒有新增相依套件或 ignored。

完整回歸在正常 Windows 登入工作階段執行，WinCred 僅建立及刪除唯一合成 `SubtitleNow/Test/...` 項目，既有原生 fixture 只使用本程式隱藏視窗及隔離子程序。新檔案測試只使用 tempfile 自有合成資料，包含不代表影音格式的 opaque bytes；沒有讀取使用者媒體、正式金鑰、擷取音訊或連接實際服務。產品本機路徑沒有建立 temp；測試 fixture 由 Rust RAII 清理，不視為未實作媒體工具的 temp 清理驗收。

以上不代表影音 codec、嵌入字幕、實際 URL、Prepared ASR／全文翻譯、GUI／通知、播放同步、實機／雲端或 Phase 08 全階段已驗收。

## 第二批驗證

2026-10-11 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：765 library、12 設定、4 探索、10 啟動，共 791 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 32 項一般測試：15 項全文翻譯引擎、9 項 PreparedSession 翻譯整合、8 項共用契約／HTTP adapters。先完成 15 項 engine 聚焦，再以 `cargo test --locked --offline prepared -- --nocapture` 通過 68 項相關測試，最後完成全套回歸。首次編譯修正測試對未提供 Clone 的脈絡型別做直接複製，改為測試自行明確複製；沒有增加內容文件的 Clone 或放寬 lint。

引擎使用受控 gate，實際讓同批後段先完成，確認最大 active requests、下一批未提前排程及結果還原順序；檢查跨批次前文、後文、Session glossary、全 Draft 後才 Review，以及校訂的鄰近草稿與已完成譯文。五語核對 Session、序號及媒體時間；1,100 cues 與超過 Live 三秒的回應實際通過。取消、Future Drop、拒絕／錯誤、同步／非同步／能力查詢 panic、空白、非法控制字元、單句超限、翻譯膨脹超過四 MiB、服務取消後仍回成功及要求逾時都不返回部分文件。final-only emitter 實際接受超過 Live 更新佇列數量的部分回應，沒有額外排隊或產生第二個時間軸。

Session 整合真正讀取合成 sidecar，驗證五語結果依原時間合併並做臺灣片語本地化；Standard 回應經校訂才成為 Ready。原生中文排第一時，即使供應商能力方法會 panic 或傳入舊 glossary，也不觸碰服務。跨 Session glossary 在發送要求前被拒；憑證錯誤不交付 Ready，下一輪可以復原。pending provider 期間 CPU 許可確實已釋放且原媒體仍被唯讀 pin；Future Drop 後核對 token 取消、owner Cancelled、媒體 handle 釋放及可重新開啟。新工作取消舊要求，舊已完成文件不能覆蓋新 Ready。最終合併再次拒絕數量不符、控制字元，以及 `內存` 對應「記憶體」造成的本地化長度膨脹。

OpenAI／Gemini adapters 使用真正的本機 HTTP／SSE fixture 驗證 Prepared 脈絡、草稿傳遞、固定指示與本文分離、取消及許可回收。20 ms 合成 client 預設與 60 ms fixture 回應實際驗證 Prepared timeout override；另外檢查要求 builder 是三十秒，Live 要求保留 client 預設。最大 High 脈絡、64 項詞彙及兩層 JSON 跳脫資料確認位於新的四 MiB 上限；Live 請求不加入 PreparedContext 並維持原有容量。

本批沒有新增相依套件、unsafe、ignored、產品磁碟寫入或新暫存媒體。全套回歸仍只使用既有自有 WinCred／隱藏視窗／隔離子程序及合成檔案 fixture，沒有讀取使用者媒體、正式金鑰或呼叫雲端。上述是離線行為與本機 adapter 接線驗證，不能作為真實五語翻譯品質、雲端 token／配額、Prepared ASR、播放器同步或 Phase 08 全階段驗收。下一批仍需完成 Prepared GUI／通知、媒體處理及播放接線；Phase 09 尚未具備啟動條件。

## 第三批驗證

2026-10-12 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：805 library、12 設定、4 探索、10 啟動，共 831 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

新增 40 項一般測試：8 項 UI 輸入／狀態契約、11 項正式 controller 與工作持有、20 項隱藏原生準備視窗、1 項主視窗入口／modal 啟動保護。`cargo test --locked --offline prepared -- --nocapture` 的 108 項相關測試通過，再執行完整回歸。首次編譯修正 native MapWindowPoints import 與測試 IsWindow 的 Option 參數；Clippy 指出的測試分支合併已修正，沒有放寬警告或新增 ignored。

正式 controller 使用真正合成本機媒體／sidecar 與 mock 翻譯服務，驗證中文不建立任何供應商／不讀金鑰、五語原文須明確所選服務、Standard 實際校訂、序號／時間、factory 失敗／panic 復原、pending 要求停止及 Drop 回收、已完成但未 apply 的舊文件、Busy 到實際 reap、取消發生在 factory 期間及不可用 URL／ASR。UI 路徑驗證沒有讀檔，拒絕空白、相對、URL、特殊 Windows 路徑與超限，不讓私人路徑／本文出現在錯誤狀態。

原生控制測試涵蓋兩個固定遮蔽 key、只移動所選 key、中文忽略無效或超長的 key、所有語言／格式／品質、實際長度核對而非截斷、原生內容與 undo 清除、處理與 Ready 鎖定、驗證錯誤在 poll 後持續、重複啟動／清除通知拒收、通知每輪一次及失敗保留 Ready。主視窗驗證 Live busy 時不可進入 Prepared、modal owner 停用時不可用訊息啟動 Live／另一份 Prepared，以及新增按鈕沒有覆蓋搜尋區。

真正的 GUI 整合 fixture 使用正式 PreparedController、合成媒體 bytes 與簡中 SRT，透過原生「準備字幕」進入一段 Ready，確認金鑰欄清除、通知 mock 一次、原媒體唯讀 handle 仍持有；「清除」後可再次以 writer 開啟，沒有新增檔案或自動播放。沒有把 mock Ready 當作正式後端結果。其餘 mock backend 測試以控制的狀態轉移核對取消、關閉與回收；真正 modal pump 接收 WM_QUIT 37 後等待兩輪停止再保留退出碼，並驗證父 HWND 銷毀後仍輪詢至回收、不再操作失效原生控制。

DPI 驗證包括明確 96／120／144／192 的純幾何（100／125／150／200%）、1920×1040 工作區限制與 200%／1000 px client 的內容範圍；另在隱藏原生視窗核對推薦尺寸、固定 footer／操作按鈕與 420 px 視窗的捲動。這不是實際切換不同 DPI 螢幕、字型視覺或播放器矩陣驗收。通知測試檢查固定文字、NUL 結尾 buffer、一次性與失敗處理，使用注入的 sink，未操作真正通知區或確認 popup 顯示。

新增 unsafe 僅在 Win32 準備視窗、捲動容器與 Shell 通知邊界，對穩定 Box、UI 執行緒、NUL buffer、native copied call、stock icon、handle 及銷毀順序說明 SAFETY；沒有新增套件或產品暫存檔。完整回歸使用正常 Windows 登入環境，WinCred 仍只操作唯一合成測試項目。沒有讀取使用者媒體、正式金鑰、擷取音訊或呼叫雲端。GUI／通知接線已完成，但媒體解碼、內嵌字幕、URL／ASR、播放及實際音訊定位仍待完成；Phase 09 尚未具備啟動條件。

## 第四批驗證

2026-10-12 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：859 library、12 設定、4 探索、10 啟動，共 885 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 54 項一般測試：16 項 PCM／格式／時間 gate、17 項有界 worker／取消／結果提交、11 項唯讀 COM IStream、10 項原生 Media Foundation 與 worker 整合。先完成 `cargo test --locked --offline media -- --nocapture` 的 78 項相關測試，補上大型 native buffer 與 ReadSample flags 後，`cargo test --locked --offline win32::media_decode -- --nocapture` 的 10 項通過；最後以正常 Windows 登入工作階段完成上述全套回歸。

真正的 Windows 19045 MF 入口使用自行產生的 PCM16 RIFF/WAVE，逐 sample 核對 44.1／48 kHz、單／雙聲道、原生時間及不完整末塊，確認 EOF 不重送、原檔內容不變且沒有其他檔案。另驗證 decoder 持有同一唯讀檔案 pin，writer／刪除被拒，Drop 後 writer 可重新開啟；中途取消拒收後續 PCM，新工作從序號一重新解碼。破損 WAVE、未知檔案、播放清單、URL、零 Session 與預先取消皆不假回成功；內容保護的 HRESULT 分類只測錯誤映射，沒有使用實際受保護媒體或繞過保護。

大型分片 fixture 直接建立真正 MF memory buffer，注入明確的合成 pending sample；三聲道／44.1 kHz 強制六十四 KiB 對齊至完整 frame，逐塊核對 PCM、序號、取整時間與最後 137 frames，即使已標記 END 也先排空 pending 才回 EOF。這不是三聲道檔案 codec 相容性驗收。ReadSample flags 的獨立測試涵蓋 END 同時有 sample、END 無 sample、空 tick，以及 EOF 同時有 error／格式或串流變更的拒收；沒有聲稱實際 WAVE handler 必定產生這些 flags 組合。

IStream 實際驗證一次有界讀取、真正 EOF／讀取計數、無效指標／過大要求、seek 邊界、拒絕修改／複製輸出、無私人名稱的 Stat、獨立 clone 游標、最後參照釋放 pin、取消及 mutex poison／ABI panic。四個合成 MTA worker 使用 AgileReference，各進行 128 次獨立 seek／read 核對；不將裸 COM pointer 直接跨執行緒傳送。

worker 使用獨立有界 lane 與受控 barrier，驗證未 poll Future Drop 不建立 decoder、已開始工作停止後仍占用容量、第二份要求立即拒收、真正退出後才釋放且可再次成功。另測 factory／consumer 失敗、panic／回復、EOF 時取消、舊 Session、格式／序號錯誤、二十四小時邊界及 PCM 交付前後取消；decoder 建立與銷毀同在非 caller 的工作執行緒。正式入口整合測試逐塊只借用 PCM，成功摘要保留正確 Session／格式／frame 數，檔案 pin 在完成後釋放。使用正式共享 lane 的 controller／GUI／decode fixtures 以測試專用 mutex 序列化，避免互相製造偶發過載；過載行為由獨立 lane 測試，不放寬產品容量。

首次 Clippy 指出固定大小 `chunks_exact` 與測試手動計算 slice bytes，已改用 `as_chunks::<2>()` 與 `size_of_val` 後重新通過，沒有放寬 lint 或增加 ignored。新增 unsafe 隔離於 Windows IStream／MF 模組，對 COM thread／startup 配對、caller 指標、buffer 長度、Lock／Unlock 與原生資源釋放順序附 SAFETY。只開啟既有 windows crate 的 MediaFoundation feature，沒有新增 crate 或安裝元件。

本批沒有產品音訊寫入、暫存媒體、快取、上傳或播放。測試暫存檔只含自有合成輸入並由 RAII 清理；既有全套測試仍只操作唯一合成 WinCred、自有隱藏視窗與隔離子程序。這些結果不代表其他 WAVE 編碼、MP3／AAC／影片、內嵌字幕、URL、Prepared ASR、真正媒體／雲端品質或播放定位已通過。Phase 08 尚未完成，Phase 09 尚未具備啟動條件；有效金鑰仍留到結案固定介面手動輸入。

## 第五批驗證

2026-10-12 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：917 library、12 設定、4 探索、10 啟動，共 943 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 58 項一般測試：18 項純 DSP、20 項 Prepared 前處理／媒體時間、8 項 EOF-aware worker、8 項前處理 worker 整合、4 項真正 WAVE 解碼／前處理。先以 `cargo test --locked --offline media -- --nocapture` 通過 137 項相關測試；另補三十秒原生 WAVE 時間檢查，`cargo test --locked --offline actual_long_wave_preprocessing -- --nocapture` 通過後，重新完成上述最終檢查。中途的 942 項全套結果由最後 943 項取代，沒有把未執行項目列為通過。

純 DSP 核對 8／16／24／44.1／48／96／192 kHz 與 191,999 Hz 奇數取樣率，轉成兩種輸出均符合精確 floor frame 數。相同音訊以 1、137、959、960 及最大區塊等外部分片，濾波狀態與 PCM 結果一致；另核對 DC／靜音／正負極值、同率 bypass、128-frame lookahead、首尾 extension、零／短輸入、重複 finish、位置與計數溢位、固定表格／ring、取消及 callback 錯誤。有限頻譜測試的 1 kHz pass RMS 位於 5,600–5,720，輸入 peak 為 8,000；指定降採樣 stop tones 的 RMS 小於 8，包含 192→16 kHz。8→16／24 kHz 的指定 7 kHz image amplitude 小於 16。這些測試點不是所有頻率、音樂或語音品質的保證，也不是 Phase 10 效能驗收。

前處理測試核對一至八聲道等權平均、反相抵消、極值與小數平均，輸出為原生媒體起點與完整 Session／序號的二十毫秒區塊。44.1 kHz 等非整數比例在不同 decoded block 分片下產生相同資料與時間；EOF 尾段完整交付且不超出區段 frame clock。真正的 gap 先排空前區段，再重建濾波器，不跨缺口混音，只在新區段第一份輸出標示 discontinuity；逐塊看似合法但累積超過一微秒的漂移被拒收。舊 Session／generation、格式／序號／重疊錯誤、二十四小時與數量／時間溢位、兩取消 scope、sink 失敗、空輸入與不足一個 target frame 都不返回部分成功摘要。完成／失敗／取消立即釋放 run 的 PCM 與 FIR，無法續用。

EOF-aware worker 的受控 fixture 驗證非空有效 EOF 才 finish 一次；空 EOF、晚到無效輸入或 decoder 失敗不 finish。Finish 取消、typed error 或 panic 均拒絕提交，容量可供下一份工作使用；owner 在 finish gate 期間 Drop 後仍保持容量，真正退出才釋放，gate 有有限等待及 Drop 回復路徑。前處理 worker 使用合成 non-Send decoder，核對 EOF 前不交付短尾、callback 與原生物件都在同一工作執行緒、失敗不 flush、取消後成功 callback 也拒收，以及未 poll 的公開 Future 不讀取缺少的來源。

真正的本機 Media Foundation 測試使用合成 PCM16 WAVE，核對 44.1／48 kHz、單聲道及雙聲道反相輸入，兩種目標 rate 的混音結果、媒體時間、序號、二十毫秒上限及 EOF frame 數；原檔不變、沒有額外檔案，完成後唯讀 pin 釋放。取消於第一個 output callback 後，只觀察一次輸出；等待該合成 worker 實際釋放 pin，新的準備從第一塊與零媒體時間開始。192 kHz、單 frame 原生 WAVE 回報 InsufficientAudio，沒有補出資料。新增三十秒／44.1 kHz、超過一 MiB 的合成輸入，完整核對跨多個原生 sample 與 FIR history wrap 的累積時間、DC、frame 數與沒有缺口；沒有把單次 native sample 上限誤用成整個檔案大小限制。

本批沒有新增 crate、unsafe、安裝、產品暫存檔、磁碟音訊輸出或雲端要求。完整回歸仍在正常 Windows 登入環境執行，只使用自有合成 WinCred、隱藏視窗、隔離子程序與 RAII 合成輸入。跨元件唯讀審查核對 EOF、取消、固定 ring／phase table、媒體時間與許可持有，未放寬 lint、容量或舊公開 callback 契約。

前處理成功不是辨識、翻譯或字幕 Ready。GUI／Prepared ASR、其他影音格式、URL、全文辨識結果合併／脈絡分析、播放與實際定位仍待接續；沒有讀取使用者媒體、有效金鑰或呼叫雲端。Phase 08 尚未完成，Phase 09 尚未具備啟動條件；真實金鑰維持結案固定遮蔽介面手動輸入。

## 第六批驗證

2026-10-12 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：1017 library、12 設定、4 探索、10 啟動，共 1043 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 100 項一般測試：18 項離線辨識契約、31 項 owned PCM 分段、31 項原生時間合併、18 項有界辨識 runner、2 項跨元件 RAM 流程。最後執行 `cargo test --locked --offline prepared --quiet` 的 255 項相關測試通過，再完成上述全套檢查；沒有放寬 lint 或新增 ignored。

分段核對兩種 rate、不同輸入分片、十秒正文及真實 overlap、短 EOF／完整正文加短尾、真正 gap、不足／錯誤資料、時鐘漂移、二十四小時／計數溢位、取消與未排空 ready 的明確過載。130 秒合成 RAM 音訊逐步交付，檢查 working buffer／ready 容量及正文順序，沒有整份媒體 PCM 容器。PCM／context 採 boxed slices；超額 batch 在建立 Future 前丟棄，合法 batch／回應移除多餘 Vec reserved capacity，容量反例確認所有權邊界不以 len 冒充 allocation 上限。

契約與 merger 核對完整 Session／generation／sequence／run、五語與 Auto 能力、正長度及原生 cue 範圍、正文跨界拒收、前 run 脈絡隔離、倒序完成、全文 cues／UTF-8 bytes 上限、靜音與混語、無效回應清理。切新 run 與整份 finish 必須 audio_end_us 等於 body_end_us，測試拒絕遺漏右 context 的假末段，接受合法 EOF 尾段及真正 gap；不裁句、移動時間或交付部分 transcript。

Runner 使用明確 mock，驗證一至四個要求能力、實際並行上限、前批已提交脈絡、完整輸入註冊後才啟動服務、typed provider error／panic、獨立六十秒期限設定及受控短期限逾時。已成功保留 source 本文後，下一批未 poll 的 Future Drop 立即取消及清除；進行中 Drop／父取消、要求自取消後回成功及父取消後晚成功均拒收，要求 guard／取消 token 回收，沒有孤兒工作。

公共 API 整合測試將 owned DecodedAudioBlock 交付分段器，使用 21 秒合成音訊、真實 gap 後一秒與合法 EOF 尾段形成四份 chunk；mock 依 2、3、4、1 完成，並行最高二個，最後還原完整原生時間及正文順序，沒有重複 cue。另一項十二秒輸入在辨識開始後取消，確認要求退出、剩餘 EOF 尾段及 transcript 不再交付。這些測試不經過舊借用 callback，不讀檔、不呼叫真正服務，也不產生 Ready。

本批沒有新增 crate、unsafe、安裝、產品暫存檔、音訊／本文落盤、快取、上傳或播放。完整回歸在正常 Windows 登入環境執行；既有測試只使用自有合成 WinCred、隱藏視窗、隔離子程序與 RAII 合成輸入。本批合成辨識結果不代表正式音訊品質、檔案 ASR 或雲端服務已驗收。

下一個接線範圍仍在 Phase 08：私有依需求拉取的 native producer、正式 ASR adapter，以及辨識後翻譯／Ready／GUI 組合。一般 GUI 的 ASR 仍明確不可用；Phase 09 尚未具備啟動條件。有效金鑰維持結案時由使用者在固定遮蔽介面手動輸入。

## 第七批驗證

2026-10-12 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：1056 library、12 設定、4 探索、10 啟動，共 1082 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 39 項一般測試：12 項 owned DSP、13 項私有 producer、12 項公開本機 ASR 入口、2 項舊正式 decoder owner 生命週期。先通過 `cargo test --locked --offline prepared --quiet` 的 291 項相關測試；補上零輸出首 run 與 panic 清理容量 probe 後，`cargo test --locked --offline prepared_producer --quiet` 的 13 項通過，再完成上述全套檢查。Clippy 發現測試的多餘 i32 cast，已移除並重新通過，沒有放寬警告或新增 ignored。

Owned DSP 在兩種目標 rate 與既有 public borrowed 路徑逐值核對，透過 allocation pointer 確認真正移交而非複製；測試 EOF／gap、取消前後、callback typed error、gate、順序與有界分段接線。原 public borrowed consumer 不得保留 PCM 或等待網路的契約保持不變。

Producer 使用非 Send 的合成 decoder、隔離 CPU／owner Semaphore 與有限 gate，核對建立、next、Drop 同在一個非 caller 工作執行緒。未 poll 不呼叫 factory；CPU 或 owner 飽和立即拒收，沒有待跑 queue。Idle 期間 CPU 可供其他工作、native owner 仍占用、pull count 不再增加，外部取消可喚醒 actor。同樣測試真 EOF 及正文尾段、gap 的最多兩份回覆、晚取消、factory／native next 已開始時 Drop，兩份容量都持到真正退出；不把 abort 當作完成。Panic 的 Drop probe 記錄原生銷毀時 CPU／owner available permits 仍為零；typed error／panic 不假回 EOF，下一份 actor 可以重新成功。

零輸出首 run 的反例使用 192 kHz、單 frame，真 gap 後才有足夠音訊；16／24 kHz 都接受第一份實際 mono 音訊、保留一秒真起點、正文沒有 gap 標誌或合成靜音，EOF 與許可回收通過。私有正規化不改 public DSP 的原有 gap semantics，也不放寬後續 mono gate。

公開入口驗證預先取消、零身分、unsupported rate／語言／Auto／取消／時間／並行能力及 capability panic 都先於 missing-file I/O，不啟動辨識。未 poll Future Drop 取消本次 scope，錯誤顯示只有安全 domain 文字，不含私人路徑或本文。

兩項 Windows 整合 fixture 經過真正的本機 Media Foundation 及私有 actor，輸入只為自行產生的 PCM16 WAVE，辨識為明確 mock。二十一秒／16 kHz 單聲道逐 sample 核對送至 recognizer 的 PCM、三份正文及 EOF 尾段、Session／原生時間／順序、ASR provenance、許可回收與沒有額外檔案；回傳獨立 transcript，不產生 Ready。六十五秒 WAVE 在第一批最多四份且尚未 EOF 時停於 pending recognizer，確認共享 CPU 全部可取得、native owner 仍持有、唯讀 pin 拒絕 writer；取消後所有要求 token／lease 釋放，actor 在 CPU 全部被測試占用時仍能完成閒置 RAII 清理，原檔可重新由 writer 開啟。此驗證不代表真正語音辨識或雲端品質通過。

跨元件唯讀審查核對 CPU／owner 分離、channel／batch 容量、同執行緒清理、EOF Join、取消／Drop、panic 及零輸出首 run，兩項發現修正後無剩餘必修問題。完整回歸仍在正常 Windows 登入環境，只使用自有合成 WinCred、隱藏視窗、隔離子程序及 RAII 合成輸入；沒有讀取使用者媒體、正式金鑰、擷取音訊或連接真正服務。

本批沒有新增 crate、unsafe、安裝、產品暫存檔、音訊／本文落盤、快取、播放或 GUI ASR。正式 ASR adapter、辨識後全文脈絡／詞彙／翻譯與 Ready／GUI、其他容器、URL 及播放定位仍待完成。Phase 08 尚未結案，Phase 09 尚未具備啟動條件；有效金鑰維持結案固定遮蔽介面由使用者手動輸入。

## 第八批驗證

2026-10-12 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：1092 library、12 設定、4 探索、10 啟動，共 1118 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 36 項一般測試：11 項 ASR 結構驗證、9 項共用翻譯引擎、16 項公開 owned 翻譯／本地化結果。最新 `cargo test --locked --offline prepared --quiet` 的 328 項相關測試通過後完成上述全套檢查；第一次編譯修正 mock 的原生時間 Option 欄位，並同步既有 timeout 測試的私有 view 參數，沒有放寬 lint 或增加 ignored。

ASR validator 核對零身分／generation、非空及 8192 cues、單句與全文 UTF-8 容量、ASR 控制字元、正長度、checked 結尾、二十四小時、有序及不重疊。測試包括精確上限與超限、非 ASCII byte 計數、overflow／倒序、原 owner pointer 與 getter 不變；不以結構檢查冒充 active Session 證據。Caller 提供的 glossary 必須與完整來源身分一致，同 SID 但不同 generation 及不同 SID 都在服務要求前拒收。

共用引擎測試透過真正 PreparedTranscriptMerger 建立五語合成 source，核對原生時間與 Final request；原 ASR 不轉為 ExistingText，不生成 publication content ID。Fast／Standard／High 的一／三／八句前後文、Fast 單 Draft 與 Standard／High 全文 Draft 後 Review、鄰近草稿及跨批已提交脈絡實際生效。受控倒序完成最高二個要求，整批完成後才排下一批，結果依原索引還原；provider error／panic、要求自取消及父取消後假成功、pending Cancel／Drop、單句／全文超量與預先取消都不返回部分文本。既有外掛字幕及中文優先路徑由全套回歸維持。

公開 wrapper 核對原文 Vec／Box allocation pointer 隨 ownership 移交而保留，原 SID／generation／語言／時間／句數不變、固定 zh-TW、品質策略與同 Session glossary，以及真正臺灣片語轉換。結果封閉持有原 ASR 與 boxed 譯文，不具 Clone／Debug／Serialize、媒體 pin、Ready 或播放證據。網路等待時共享 CPU 容量可全部取得；最後本地化遇 CPU 飽和明確回報 Overloaded，沒有額外待跑 queue。未 poll Future Drop 不呼叫 provider；進行中 Drop／取消、晚到成功、空白、控制字元、錯誤及超量結果均取消 scope 與要求，沒有部分結果。

本地化容量反例不僅核對 raw output：`內存` 重複 1365 次的 8190 bytes 原文會轉為 12285 bytes，單句被拒；518 筆各 raw 5400 bytes／localized 8100 bytes，原始譯文總量小於四 MiB、本地化後超過四 MiB，同樣拒收。受控已開始 finalizer 使用有限 gate 及真正 owned 原文／譯文，Future Drop 後確認 root 已取消但 CPU 許可仍占用；釋放 gate 後真正 finish 返回 Cancelled，worker 清理並釋放容量才接受下一份工作，不把 abort 當作退出。等待皆有上限與 Drop 釋放保護。

跨元件唯讀審查核對 borrowed view、原文 ownership、完整 glossary 身分、取消優先、worker 許可／清理順序及原始／本地化兩份容量，無剩餘必修問題。字典本地化仍在共用 CPU worker，沒有藉本批改動舊字幕的重疊或控制字元語意。完整回歸在正常 Windows 登入環境，僅使用既有自有合成 WinCred、隱藏視窗、隔離子程序、RAII 合成輸入與明確 mock／本機 transport fixture；沒有正式金鑰、真實辨識／翻譯服務或使用者音訊。

本批沒有新增 crate、unsafe、安裝、磁碟音訊／本文、產品暫存檔、快取或匯出。自動全文分析／詞彙建立、正式 ASR adapter、同一媒體 handle 的 ASR Ready／GUI、其他影音格式、URL 及播放定位仍待接續，不能以 library／mock 品質作為產品驗收。Phase 08 尚未完成，Phase 09 尚未具備啟動條件；有效金鑰維持結案固定遮蔽介面手動輸入。

## 第九批驗證

2026-10-12 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline prepared --quiet`：345 項相關測試通過。
- `cargo test --locked --offline --quiet`：1112 library、12 設定、4 探索、10 啟動，共 1138 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 20 項一般測試：6 項媒體 pin／native／producer／ASR 生命週期、14 項 Session ASR Ready 整合。首次新增完整測試的編譯檢查發現 assert_eq! 要求未提供 Debug 的來源語言型別，改用等值判斷後重跑；沒有新增 Debug、放寬 lint 或增加 ignored。全部正式檢查均在最終程式碼上通過。

真正的合成 PCM WAVE 經本機 MF、DSP、分段、mock recognizer、共用翻譯與臺灣本地化進入 Ready，保留 Session／generation、真正媒體時間、原文、語言、順序與品質。既有字幕及辨識原文以不同借用來源型別交付；中文、來源語言字幕、損壞的最高優先字幕及歧義選擇都不觸碰 ASR，沒有偷偷降級。過期 SID／generation 詞彙在 recognizer 能力檢查與媒體 I/O 前拒收；有效本次詞彙交付翻譯及 High 校訂。

同一 file object 的 handle 複本與原 pin 各自保留 Windows writer／delete 拒收。真正 EOF 後 CPU／native-owner 許可已復原，原 pin 仍在最後 ASR、翻譯、尚未 apply 的 completion 及 Ready 期間禁止寫入；取消、Future Drop、服務錯誤與文件清除後可重新以 writer 開啟。合成 native destructor panic 導致未交付的 EOF／join 失敗時，也拒收結果、釋放 pin 並恢復容量，不以 abort 當作清理證明。原媒體 bytes 保持不變，產品沒有產生額外內容檔。

Session 測試核對未 poll Drop、ASR 等待中停止、翻譯 Future Drop、明確 Speech／Translation／unsupported 錯誤及無部分 Ready。子 scope 失敗不改寫父取消狀態，apply 保留真正 Failed 原因；使用者停止仍連動取消要求。新 Session 的 Ready 不受舊 pending 或已成功 completion 影響；被拒的舊結果釋放自己的 pin，不能解鎖或清空新文件。

跨元件唯讀審查核對一次開檔、共用 file cursor 限制、COM／MF 同執行緒銷毀、EOF 真正 join、取消深度、Ready provenance、容量及文件的 GUI／library 區分，無剩餘必修問題。完整回歸在正常 Windows 登入環境，使用既有自有合成 WinCred、隱藏視窗、隔離子程序及 RAII 合成輸入；沒有正式金鑰、使用者音訊或真正雲端服務。

本批沒有新增 crate、unsafe、安裝、產品暫存媒體、音訊／本文寫檔、快取、匯出或自動播放。正式 ASR adapter、GUI ASR、自動全文分析／詞彙建立、其他容器、內嵌字幕、URL 及播放定位仍待接續，mock 品質不能代替正式驗收。Phase 08 尚未完成，Phase 09 尚未具備啟動條件；有效金鑰維持結案固定遮蔽介面手動輸入。

## 第十批驗證

2026-10-12 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline prepared_speech --quiet`：53 項相關測試通過。
- `cargo test --locked --offline --quiet`：1147 library、12 設定、4 探索、10 啟動，共 1173 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 35 項一般測試：15 項 HTTP／Session 整合、20 項協定編碼／原生時間解析與 worker 生命週期。所有指令在最終程式碼上通過，沒有放寬 lint、改為單執行緒測試或新增 ignored。

編碼測試核對兩種取樣率、五語提示、真正 mono PCM16 WAVE、十一秒容量、UTF-8 提示尾段及 multipart 碰撞。OpenAI 回應必須具有正式 segment 欄位、語言及時間，本文與片段一致；Gemini 必須有 completed model_output、text 與原生 word_info，包含與缺少 UTF-8 byte 索引的回應都精確核對本文。測試涵蓋真空白輸出、缺少輸出拒收、時間文法／四捨五入、錯語言、重複欄位／索引、倒序、控制字元、未標註本文、跨正文及容量；完整 context 先驗證再排除，不裁時間或把未知資料當靜音。

真正 loopback HTTP 測試核對兩種認證標頭與 RAM 包裝、固定正式 HTTPS 主機及 client 拒絕 HTTP、未 poll／預先取消／Auto 不送出要求、HTTP 狀態分類及不跨服務 fallback。限兩次重試、相同 payload、Retry-After 長值／HTTP-date／非法及溢位值、JSON MIME、未完成回應、Content-Length／chunked 上限、pending socket 在取消／Drop／整份逾時後退出，以及額滿立即 Overloaded 均受控；網路等待可取得全部共享 CPU 許可。

內部 HTTP 子取消 scope 的修正保留真正解析／網路錯誤，不讓清理把它們蓋成 Cancelled，失敗／逾時／未 poll Drop 也不改寫 caller 的父取消狀態。完整回歸曾揭露測試在取消 Future 後立即檢查容量，早於已開始 encoder 的真正清理；已改為最多五秒取得全部 request permits，再檢查恢復及沒有送出 HTTP。這個等待仍會因洩漏／未退出而失敗。獨立 barrier 測試另外確認已開始的 CPU／HTTP lease 在 Future Drop 後仍持有，直到 worker 真正退出才釋放，晚取消與 panic 不交付成功；唯讀複查無剩餘必修問題。

Windows 合成 WAVE 經真正 Media Foundation、DSP、分段、兩種正式 HTTP adapter 的本機回應、明確 mock 翻譯及臺灣本地化進入 Session Ready。保留原生時間／來源身分／品質及同一媒體 pin，停止後原檔可寫、bytes 不變且沒有額外內容檔。損壞 JSON 的同一完整流程則回報 Speech InvalidData，釋放 pin，不生成部分 Ready。這些測試不代表真正辨識準確率、雲端品質或模型取樣率相容性已驗收。

本批沒有新增 crate、unsafe、安裝、產品暫存媒體、音訊／本文保存、內容快取、匯出或自動播放。完整回歸在正常 Windows 登入環境，只使用自有合成 WinCred、隱藏視窗、隔離子程序、RAII 合成輸入及本機 HTTP；沒有正式金鑰、使用者媒體或真正雲端要求。官方協定及限制記錄於 [ADR 0045](adr/0045-bounded-prepared-speech-http-providers.md)。

正式 Prepared 辨識 HTTP library 已完成；限五語 Auto、一般 GUI ASR、自動全文分析／詞彙建立、其他容器／內嵌字幕、URL 及播放定位仍待接續。Phase 08 尚未完成，Phase 09 尚未具備啟動條件；有效金鑰及真實服務測試維持結案時由使用者在固定遮蔽介面手動輸入。

## 第十一批驗證

2026-10-13 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline prepared --quiet`：407 項相關測試通過。
- `cargo test --locked --offline --quiet`：1174 library、12 設定、4 探索、10 啟動，共 1200 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 27 項一般測試：15 項 app factory／Controller、10 項原生準備視窗、2 項 UI 輸入邊界。格式、Clippy、聚焦／完整回歸及兩種建置在最終程式碼上通過，沒有放寬警告、改用單執行緒測試或新增 ignored。另先執行 all-targets／all-features 的離線 cargo check，接線編譯通過。

UI 邊界核對完全空字串才生成 AsrRequired，獨立於字幕格式／語言；空白字元仍為 InvalidInput，不以 trim 導致媒體上傳。原生視窗驗證 Auto 在任何 key 讀取或 backend start 前拒收，即使兩欄是非法／超限合成 key 也只回報 Auto 原因並清除 undo。明確五語與兩種服務皆正確交付；既有字幕忽略音訊 Auto／無選項，字幕路徑變更立即更新停用欄位及目的地，沒有自行啟動工作。

原生測試核對字幕／ASR 各自的用途揭露、所選 key 及雙列清除、另一列錯誤不補救／不影響、空所選 key 的安全憑證要求、missing provider、忙碌／Ready 鎖定及關閉後回收。文字反例確認明確音訊語言或字幕的 UnsupportedLanguage 不改稱 Auto 前置拒收；正式非同步 Failed 保留 typed 原因。目的地高度由原生 EDIT 折行、實際字型與 GDI metrics 決定，隱藏視窗在 320／480／640 px 寬及一般／加倍字型核對完整高度；96／120／144／192 DPI 的純 geometry、焦點及捲動範圍維持可操作。這不是實際多螢幕／200% monitor、鍵盤或視覺驗收。

三項正式 factory 建構測試只使用合成 CredentialStore／ApiKey，不發要求；核對兩種 provider 的辨識與翻譯共用一次讀取、明確輸入 key 跳過 store、missing credential 不重試／不查另一服務。私有 kind／services variant 必須一致，不符回報 WorkerFailed，不能偷偷改跑另一條 pipeline。中文字幕不建任何服務，來源字幕只建翻譯；Auto／missing provider 在 worker、Session 身分、factory 與 missing-file I/O 前被拒收。

十二項 Controller 整合測試以自行產生的 PCM WAVE 經真正 MF／DSP／分段與明確 mock 語音／翻譯服務，逐一核對兩種 provider、五語、16 kHz、原生時間、辨識來源及 Standard 的 Draft／Review；譯文使用臺灣詞語。Ready 保留原檔 pin，服務物件已回收、沒有額外內容檔；清除後檔案可寫。Pending 最後 ASR 及翻譯期間保持 pin，網路等待時 CPU／native-owner 許可已回復；stop／Controller Drop 取消並等待真實 worker/runtime 退出，回收後才可重啟。已停止服務的晚成功不進翻譯、buffered completion 不交付舊 Ready，新 Session 身分獨立。能力不符、typed ASR／翻譯／setup 錯誤及錯 variant 均不交付部分文件；URL 仍回報不可用，不呼叫辨識或翻譯。

跨元件唯讀複查核對公開 backend 邊界、優先路徑、金鑰隔離、native callback、動態 footer、GDI 字型恢復／DC 釋放及取消／媒體 pin，無剩餘必修問題。新增原生呼叫隔離在用途文字量測，附同 UI 執行緒、owned EDIT、借用 font／DC 及 RAII 的 SAFETY 說明。沒有新增 crate、安裝、產品暫存媒體、音訊／本文保存、內容快取、匯出或自動播放。

完整回歸在正常 Windows 登入環境，僅使用既有自有合成 WinCred、隱藏視窗、隔離子程序、RAII 合成檔及本機 HTTP；沒有正式金鑰、使用者媒體或真正雲端要求。一般 Prepared GUI ASR 接線已完成，真實五語辨識／翻譯品質、Auto、其他容器／內嵌字幕、URL、全文專名／詞彙及播放定位仍待接續。Phase 08 尚未結案，Phase 09 尚未具備啟動條件；有效金鑰維持結案固定遮蔽介面由使用者手動輸入。設計見 [ADR 0046](adr/0046-explicit-prepared-asr-gui.md)。

## 第十二批驗證

2026-10-13 實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline local_audio_format --quiet`：16 項格式門檻測試通過。
- `cargo test --locked --offline mp3 --quiet`：6 項原生 MP3 測試通過。
- `cargo test --locked --offline prepared --quiet`：409 項相關測試通過。
- `cargo test --locked --offline --quiet`：1198 library、12 設定、4 探索、10 啟動，共 1224 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 24 項一般測試：16 項有界內容門檻、6 項實際 MP3／公開解碼／時鐘拒收、2 項 Controller 的零要求失敗及字幕優先。全套回歸、格式、Clippy 與 Debug／Release 在最終程式碼通過，沒有放寬 lint、時間精度、改用單執行緒或新增 ignored。

內容門檻核對 WAVE、三種 MPEG-1 rate／mono／stereo／CRC、padding／VBR header、兩個完整 frame、rate／channels 一致、ID3 三版本／合法 footer、reserved flags、synchsafe／容量／truncation、free format／MPEG-2／2.5／其他 layer／AAC／MP4／playlist／junk 拒收。真正一 MiB tag 只被 seek 越過，最多讀 31 bytes、六次 seek，不保留 metadata。短讀可完成，EOF／declared length 都檢查，預先取消零 I/O、seek／read 中取消勝過成功，I/O error 與 32 次 Interrupted 停止有明確原因。

六項 native 測試使用 Windows encoder／sink 在 RAM 產生真正合成 440 Hz MP3，來源只為自有 PCM，測試檔採 RAII 且故意使用 .bin 副檔名。核對三種 rate 與兩種 channels 的原生格式、非空 frames／有界 block、RMS／tone 頻率、時間／gap、EOF、writer／delete pin 拒收、decoder 銷毀後原 pin 仍保持及最後釋放。Raw／ID3v2.2／2.3／2.4／footer 都經真正 MF 讀取；取消後新 run 可讀，短檔／tag 超量不建立 decoder 或額外內容檔。公開 decode_local_audio 完成後 CPU／native-owner 許可回收、原檔可寫、bytes 不變且沒有產品輸出。

首次測試修正 test-only COM error 型別及 MP3 sink 固定串流設定；沒有加外部 encoder 或假二進位樣本。測試另觀察到 native decoder delay 缺口，以及 44.1 kHz 壓縮 frame 的 100 ns 累積取整；完整壓縮 frame 的時間與 native end 以可計算的量測容差核對，不把來源 PCM 長度當作檔案時間。先前的兩秒 DSP 測試真正回報 InvalidAudio，顯示完整 MP3 Prepared 接線尚不正確；本批改為在音訊前拒收的明確支援邊界，沒有忽略該限制或放寬既有 DSP。兩種目標 rate 均回 UnsupportedAudioClock，borrowed consumer 零呼叫，native-only 解碼仍保留實際時間／缺口。

Controller 對兩種所選 provider 核對：無字幕 MP3 在 recognizer／translator 的要求前失敗，零辨識／翻譯呼叫、無部分 Ready、服務 owner 與媒體 pin 釋放，原檔及目錄不變。MP3 加明確中文字幕仍經原有本機本地化進入 Ready，沒有建立 client 或讀金鑰，清除後解除 pin；沒有因音訊限制破壞字幕優先。

自行複查 CPU／owner 許可、native／sample-clock factory 分流、取消／RAII、同一檔案及來源身分、trusted MIME 與 test-only writable stream 的邊界。平行代理本批因用量限制未能執行，沒有將其當作已完成的審查。產品新增 MIME 呼叫沿用隔離 native block 與 SAFETY；encoder 的 unsafe／Lock／Unlock／Shutdown 只在 cfg(test)，實際建置不含編碼或輸出入口。沒有新增 crate、安裝、音訊／本文保存、快取、匯出或自動播放。

完整回歸仍在正常 Windows 登入環境，只使用既有自有合成 WinCred、隱藏視窗、隔離子程序、RAII 合成資料與 loopback HTTP；沒有正式金鑰、使用者音訊或雲端要求。原生 MPEG-1 MP3 library 已提供；MP3 取樣時鐘映射、實際 VBR／使用者檔案相容性、其他容器、Auto、全文詞彙及播放定位仍待完成。一般 GUI 無字幕辨識維持 WAVE，Phase 08 尚未結案，Phase 09 尚未具備啟動條件；真實金鑰維持結案固定介面手動輸入。設計與官方來源見 [ADR 0047](adr/0047-bounded-mp3-native-decoding-and-clock-gate.md)。

## 第十三批驗證

2026-10-13 在最終程式碼實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：1230 library、12 設定、4 探索、10 啟動，共 1256 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

本批新增 32 項一般測試：29 項純 RAM 時鐘／窗口、3 項真正 MP3／DSP／生命週期。迭代時的 decoded_clock 與 mp3_clock 聚焦測試分別通過 29／3 項；加入結果來源身分後再次完整回歸，最終全部通過。沒有放寬 lint、取整容差、改用單執行緒或新增 ignored；Release 建置實際完成。

純測試以獨立整數公式核對 2048 個 44.1 kHz／1152-frame 封包，原生每封包先取整為 100 ns 後累積，sample clock 則一次由累積 frame 計算。超過五十微秒的偏移後接兩微秒真缺口，仍保留缺口而不將 origin 重設回 native time。涵蓋 mono／stereo allocation 與 PCM 原樣移交、原有正負一微秒邊界、真缺口／重疊／倒退、Session 與 generation 分別拒收、重複／跳序／格式變更、二十四小時及 frame／run 溢位、owner／supplied 取消與 terminal 清理。

窗口核對完整多 span coverage、左右半開端點、整數有理內插與取整後空區間拒收；不能跨 run／缺口、取用未觀測或已 prune 的音訊。十二秒及 512 spans 的精確上限各自驗證，超限不默默淘汰仍需要的資料；prune 保留 straddling span、回收容量並保留順序狀態。混合不同 normalizer 的原生時間、缺口及累積相位均拒收，錯身分／資料／取消後清除 evidence，換新 token 不能復活。

三項 native 測試使用 Windows 在 RAM 產生的自有合成 MP3。兩秒資料涵蓋 32／44.1／48 kHz、mono／stereo，分別經映射交付未修改精度的 16／24 kHz DSP；核對同一 allocation pointer／checksum／frames、每個原生端點及來源序號、每 run 精確輸出 floor(frames * output_rate / input_rate)、缺口、二十毫秒 block、tone RMS 與 pin。十三秒 44.1 kHz stereo 在兩種目標 rate 核對 rolling prune、保留長 run 取整偏移、實際 EOF 短尾不外推及舊證據拒收。取消後銷毀 native decoder 並解除檔案 pin，新 Session 從獨立身分／序號／run 開始；原檔 bytes 不變且沒有額外內容檔。

獨立唯讀複查找到一個不需改 private fields 的公開 API 反例：兩個相同 Session 的 normalizer 在 44.1 kHz 使用不同 run origin／frame 相位，微秒端點看似相接仍會多出一微秒。已讓 span 保留 source frame 起迄及 run origin，窗口核對完整累積相位及接續，公開反例測試正確拒收。映射結果也使用私有欄位與 getters，保留完整來源身分、native run 及第一／最後 decoded block 序號；來源序號不冒充 ASR chunk。末端取消檢查與 metadata 清理亦已複查，沒有剩餘必修問題。

本批沒有新增 crate、unsafe、安裝、產品 thread／queue、網路要求、音訊／本文保存、快取、匯出或自動播放。test-only encoder 的有界上限改為最大 rate 的十六秒及 320 KiB 壓縮 bytes，以驗證超過十二秒窗口；仍逐小段產生 PCM，產品建置不含編碼入口。完整回歸只使用既有自有合成 WinCred、隱藏視窗、隔離子程序、RAII 合成來源與 loopback HTTP，沒有正式金鑰、使用者音訊或真正雲端。

有界時鐘映射核心已完成；一般 MP3 Prepared／ASR 仍明確拒收，不以 library／合成成功宣稱 GUI Ready。下一批須將不可變、有界 map 綁定每份 chunk／context，再轉換辨識時間並驗證結果合併，不能讓四份十秒 batch 等結果回來才查 rolling window。WAVE 與字幕優先維持可用；Phase 08 尚未結案、Phase 09 尚未具備啟動條件。真實相容性、VBR、雲端及播放仍待驗收，金鑰維持結案固定介面由使用者手動輸入。設計見 [ADR 0048](adr/0048-bounded-decoded-sample-to-native-clock-map.md)。

## 第十四批驗證

2026-10-13 在最終實作實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`
- `cargo test --locked --offline --quiet`：1276 library、12 設定、4 探索、10 啟動，共 1302 項一般測試；既有 15 項 ignored 不變。
- `cargo build --locked --offline`
- `cargo build --locked --offline --release`

新增 46 項一般測試：18 項不可變快照／來源接續、26 項 mapped chunk／merger／辨識、1 項實際 DSP 零輸出 run、1 項真正 MP3 分段到辨識文件。迭代的 clock 聚焦 88 項、最終 clock_tests 聚焦 30 項及零輸出 DSP 聚焦 1 項皆通過；最後交付取消檢查與格式修正後完整回歸再次通過，沒有隱藏警告、放寬容差、改用單執行緒或新增 ignored。初次編譯修正測試的 default 取消建構，新增測試未使用的局部變數已移除，沒有放寬 lint。

快照測試核對完整原始 source spans 與精確可用 context 範圍、半開端點、來源 run 推導、禁止跨 gap／外推、Session／generation、取整後空區間、512 spans／十二秒容量、兩種準備 scope 取消。窗口完全 prune／Drop 或之後錯誤不影響已移交證據，fractional frame 相位不會因裁 context 而改算；相鄰快照核對共用 rows、source origin／frame／sequence／format，不接受不同 normalizer 的不相容資料。

mapped chunk 核對完整 audio／正文、PCM owner、重綁及直接／mapped 混用拒收。四份十秒要求在四十秒來源窗口回收後仍持有自己的 maps；明確 mock 以 semaphore 讓最早要求最後返回，服務只看 sample-clock 時間，定稿按原序使用原生時間。正文外、跨界及左脈絡文字不得變成自己擁有的 cue；轉換保留觀測差異，任何原生一微秒重疊均清除全文，不裁句或移動時間。

EOF 短正文擁有前段右脈絡時，audio end 可以相同，甚至完整 audio context 也可以相同；合法增加左脈絡依 source row 交集核對，仍不允許 audio end 倒退或跳過正文。source run 與 chunk run 分開，純來源 proof 可略過零輸出 IDs；另一個實際 192 kHz DSP fixture 在 16／24 kHz 均產生來源 run 2／4、chunk run 1／2，零輸出 run 沒有假音訊或假分段，最後仍合併正確原生時間。

取消核對綁定後、pending、buffered silent response、已提交非空／全篇空結果與最後交付；較早獨立 creation scope 在兩段都提交後取消，也會撤回整份文件及 context。scope guards 精確上限八個，不同 scope 超限明確 Overloaded 並清除本文，正常 scope 重用二十段仍只保留一個。全文 validate 前後都核對取消，不讓最後驗證期間的撤回交付成功；沒有保留全部舊 maps 或無界取消樹。

真正 native fixture 以 Windows RAM encoder 產生自有十三秒 44.1 kHz stereo MP3，經原生 decoder、未放寬的 DSP、原有 segmentation、owned snapshot 與明確 mock PreparedRecognitionSession。兩種目標 rate 都完成三份有真缺口／左右 overlap／EOF 的分段。窗口／DSP／segment builder／normalizer／decoder 在辨識前銷毀，快照仍能完成；以原始 decoder timestamp／真實 frames 的獨立 u128 公式核對每個最終端點，確認原生 end 與 sample end 的長 run 差異有保留。原 file pin 在辨識期間仍拒絕 writer，釋放後可寫，bytes 不變且目錄沒有額外內容檔。成功的 provider request child 清理不取消準備 scope；這不是正式服務準確率、Ready 或播放證據。

獨立唯讀複查協助補齊靜音取消優先順序、較早已提交 creation scope、EOF 共用端點及增加左脈絡的契約；已加入回歸及有界實作。最後核對所有 scope guards、完整 span 交集、同一 Arc binding、原生正文／cue 順序與全文驗證後取消檢查，無剩餘必修問題。共用 row 比對使用有界線性索引；還原每個 cue 的查詢仍由最多 512 spans 約束，不宣稱已達性能 SLA。

本批沒有新增 crate、unsafe、安裝、產品 worker／queue／mutex、網路要求、音訊／本文保存、內容快取、匯出或自動播放。完整回歸只使用既有自有合成 WinCred、隱藏視窗、隔離子程序、RAII 合成來源及 loopback HTTP；沒有正式金鑰、使用者音訊或真正雲端。

分段映射及原生定稿合併 library 已完成，一般 native producer factory／GUI 與 MP3 gate 維持原有行為。下一批接入同一原生 owner 的快照移交，驗證大 block／長缺口／窗口容量與取消，再決定開放 MP3；WAVE／字幕優先保持可用，不能以本批合成／mock 成功宣稱產品 MP3 Ready、真實服務或播放相容性。Phase 08 尚未結案，Phase 09 尚未具備啟動條件；金鑰維持結案固定遮蔽介面由使用者手動輸入。設計見 [ADR 0049](adr/0049-chunk-owned-clock-evidence-and-native-finals.md)。

## 第十五批驗證

2026-10-14 在最終 Rust 程式碼實際執行：

- `cargo fmt --check`：通過。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：通過，無警告。
- `cargo test --locked --offline prepared_producer --quiet`：22 項 producer 聚焦測試通過。
- `cargo test --locked --offline mp3 --quiet`：14 項 MP3 聚焦測試通過。
- `cargo test --locked --offline --quiet`：1303 項 library、12 項設定、4 項探索失敗、10 項啟動測試通過，共 1329 項一般測試；既有 15 項 ignored 沒有增加。
- `cargo build --locked --offline`：Debug 通過。
- `cargo build --locked --offline --release`：Release 通過，實際完成時間為 1 分 01 秒。

本批一般測試淨增 27 項：16 項 pending／retained clock、8 項 mapped native actor、1 項真正 MP3 的正式 actor／公開辨識入口、3 項 Prepared Ready／失敗／取消測試（取代 1 項舊壓縮時鐘拒收測試）。初次編譯修正測試的取消 token 建構及使用現有 Tokio current-thread runtime；編譯期間的未完成測試警告已消除，最終 Clippy 與全套回歸無警告，沒有放寬 lint、精度、改用單執行緒測試或新增 ignored。全套 library 實際耗時 135.03 秒；不是延遲 SLA 或產品效能保證。

純 RAM actor 在兩種目標 rate 核對 7 個最大 64 KiB／8 kHz mono block，共 28.672 秒，包含 12.288 秒 staged 範圍與 20.480→20.500 秒正文窗口邊界。完整來源 span 保留，native 一微秒封包重疊／累積相位仍映射到原生定稿；沒有靠裁切端點避免容量限制。另核對超過十二秒真缺口的舊正文／尾段排出、首段／中途多個零輸出 run、零輸出尾 run 的 EOF、44.1 kHz 逐 packet 取整、513 rows 明確 Overloaded、取消後不再拉取，以及建立／使用／銷毀固定在同一 native 執行緒。服務等待時 CPU 可用，native owner 僅在真正清理後釋放。

新增 clock 測試核對 pending span 與 append 共用完整來源 validator、跨大 gap 的各 run 快照、十二秒／512 rows 上限、單調 watermark、保留跨界原始 row 的舊前綴拒收、取消及 terminal 清理。最多 512 stored rows 加一個 pending span 的查詢不新增 metadata buffer；完整分段快照仍最多 512 rows，不能外推或跨真缺口。

真正 Windows encoder 在 RAM 產生自有 32／44.1／48 kHz、mono／stereo CBR MP3，故意使用 .bin 檔名；每種原生格式都經正式 actor／公開 recognize_local_media 的 16／24 kHz 處理，44.1 kHz stereo 使用十三秒音訊，其餘兩秒。以原始 decoder timestamp／真實 frames 的獨立端點公式核對最終 cues，並確認 EOF 後 CPU／native 許可回收、原檔 bytes 不變及沒有額外內容檔。

Prepared Ready 整合使用自有十三秒 44.1 kHz stereo MP3、明確選定的 OpenAI／Gemini mock、真正原生 producer 與原有全文翻譯流程。GUI Controller 的正式 backend 走 16 kHz，另一個 24 kHz 由同一 PreparedSession API 驗證；沒有為測試虛構 GUI rate 選項。獨立 oracle 直接從原生 decoder 時間及 frame 計算 source run／sample phase，未呼叫產品 normalizer 或 map。核對原生 cues、zh-TW 本地化、Standard draft／review、Ready 原檔 pin 拒絕 writer／delete、清除後釋放；網路／憑證／翻譯失敗無 Ready，最後 ASR 等待時停止與 Drop 都取消並等待資源清理。MP3 既有中文字幕優先仍不建立雲端 client 或讀金鑰。

獨立唯讀檢查核對完整來源相位、左側跨界 span、窗口／回覆／ASR 容量、零輸出 EOF、取消、同執行緒 RAII 與原 pin 移交，沒有剩餘必修問題。沒有新增 crate、unsafe、安裝、OS worker／queue／global mutex、產品媒體寫入、暫存檔、快取、匯出或自動播放。全套回歸仍只用自有合成 WinCred、隱藏視窗、隔離子程序、RAII 合成媒體與 loopback HTTP；沒有正式金鑰、使用者音訊或真實雲端要求。

一般 Prepared／GUI 的 MPEG-1 MP3 映射接線已完成；公開 borrowed DSP 仍拒收壓縮封包時鐘，WAVE 精度與字幕優先保留。實際 VBR／使用者檔案相容性、雲端辨識品質及播放器／播放定位仍待驗收。Phase 08 尚未結案，Phase 09 尚未具備啟動條件；固定遮蔽金鑰與結案手動輸入維持原有安排。設計見 [ADR 0050](adr/0050-native-prepared-clock-handoff.md)。

## 第十六批驗證

2026-10-14 在最終 Rust 程式碼實際執行：

- `cargo fmt --check`：通過。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：通過，沒有放寬 lint 或隱藏警告。
- `cargo test --locked --offline app::prepared --quiet`：34 項 controller 聚焦測試通過。
- `cargo test --locked --offline --quiet`：1325 項 library、12 項設定、4 項探索失敗、10 項啟動測試通過，共 1351 項一般測試；既有 15 項 ignored 不變。最終 library 實際耗時 147.02 秒，不作為延遲 SLA。
- `cargo build --locked --offline`：Debug 通過。
- `cargo build --locked --offline --release`：Release 通過，實際完成時間為 1 分 07 秒。

本批淨增 22 項測試：10 項詞彙核心、8 項正式 HTTP adapter 的合成／本機協定測試、3 項來源字幕 controller 與 1 項 ASR controller 整合。全文最大 8192 cues／四 MiB 逐窗口完整涵蓋，另核對 64 cue／64 KiB 不同切割邊界、空候選、手動詞彙優先、重複／衝突譯名、第 65 筆不同詞彙明確 Overloaded、非法 UTF-8 範圍、錯 Session／sequence、panic、typed error、timeout、取消及 Future Drop。

真正 TCP loopback 核對兩個服務的結構化 JSON／分段 SSE、原文與指令分開、未知／重複 JSON 欄位、原文不存在、錯 MIME、拒絕／未完成狀態、有界回應及最大雙層 escaping payload。日文來源的 UTF-8 範圍由本機推導；原始 JSON 不先做本地化，避免破壞資料。共用兩份許可、同一速率 gate、瞬時錯誤重試、憑證不重試、等待與開啟的回應取消都驗證。Live 文字上限保持原有方式。

Controller 使用真正本機 sidecar／合成 WAVE／MP3 路徑及明確 mock，核對分析在 Draft 前、所有 Draft／Review 共用本地化詞彙，辨識來源保留原生時間／來源種類；分析失敗不翻譯、不 Ready，停止不接受遲到內容，清理後原媒體 pin 可釋放／重啟。獨立取消測試占用測試 runtime 的 blocking worker，使真實詞彙合併排入 CPU lane 後再撤回要求，確認不交付詞彙、runtime 真正回收 queued closure 及許可；沒有假 timer、產品測試 hook 或提前放掉已開始 worker 的許可。

本批程式碼另檢查取消 guards、CPU 許可、完整 Session／sequence、超限與秘密／內容生命週期。沒有新增 crate、unsafe、產品媒體寫入、字幕／詞彙快取、匯出或自動播放；測試僅自有合成資料、測試憑證、隱藏視窗、隔離程序及本機 loopback，沒有有效金鑰、使用者音訊或真實雲端要求。

自動全文詞彙整理與正式 GUI 接線完成，雲端 schema 接受程度、真實五語語意／譯名品質與費用仍須結案手動驗收。原文存在不等於譯名正確；不同生成譯名與超過 64 筆是目前明確拒收限制。完整主題／語意脈絡、其他輸入、Auto、跨句品質及播放／音訊定位仍待接續，Phase 09 尚未具備啟動條件。設計與官方協定資料見 [ADR 0051](adr/0051-grounded-prepared-session-glossary.md)。

## 第十七批驗證

2026-10-14 在最終 Rust 程式碼實際執行：

- `cargo fmt --check`：通過。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：通過，沒有放寬 lint 或隱藏警告。
- `cargo test --locked --offline --quiet`：1336 項 library、12 項設定、4 項探索失敗、10 項啟動測試通過，共 1362 項一般測試；既有 15 項 ignored 不變。library 實際耗時 123.90 秒，不作為延遲 SLA。
- `cargo build --locked --offline`：Debug 通過。
- `cargo build --locked --offline --release`：Release 通過，實際完成時間為 38.65 秒。

本批淨增 11 項測試：4 項 ADTS 內容門檻、1 項密集 clock snapshot、3 項 native AAC 及 3 項 controller 整合。另將原有 513-row 過載 fixture 改為使用正式 row cap 加一，仍驗證明確拒收、取消、worker／native 許可及同執行緒資源清理，不增加 ignored 或改用單執行緒測試。

門檻測試核對兩個完整相鄰 frame、CRC header 大小、可變 frame 長度、率／聲道一致、MPEG-2／其他 profile／PCE／多 raw block／空 payload／截斷／前置 junk 拒收；最大合法 frame 下實際只讀二十 bytes，成功 reset cursor，一 byte 短讀、取消與 I/O 錯誤亦受控。這些合成 headers 只證明門檻，不宣稱有效 AAC payload 或 checksum 驗證。

真正 Windows AAC 編碼在 RAM 使用可寫／可 Seek 的 stream。測試揭露 HGLOBAL 的 MF capabilities 為 readable／seekable，因而改用 SHCreateMemStream；Sink Writer 零起算 index 與 sink identifier 分開。48 kHz stereo 真正解碼核對 PCM16、bounded block、440 Hz 合成 tone 的頻率／RMS、EOF、原生時間範圍、原檔 bytes 不變及 pin 拒絕 writer／delete，釋放後恢復可寫。其他 44.1 kHz／mono 組合以實際合成 AAC 驗證前置 UnsupportedAudioClock，沒有 PCM／ASR／glossary／translation 要求或部分 Ready。

十三秒 48 kHz stereo 合成檔包含超過 512 個壓縮 AAC packets；真正 MF native 觀測會合併封包，沒有假設一 packet 一 row。兩個所選服務的明確 mock 分別經 GUI controller 的 16 kHz 及同一 Session API 的 24 kHz，完整走 native actor、映射、ASR、全文詞彙、Standard Draft／Review 到 RAM Ready。原生時間以只使用真正 decoder timestamp／frames 的獨立 oracle 核對，不呼叫產品 normalizer／map；停止、服務錯誤、中文 sidecar 優先、原檔 pin 與資源回收均驗證。另以 560 個 1024-frame／48 kHz 純 RAM spans 核對快照完整保留、640-row 有界預算、原生端點及十二秒上限。

44.1 kHz mono 的實際失敗為 block 3 的 start 975235 µs、previous end 975239 µs，差四微秒。沒有裁音訊、重設時間、補靜音或放寬一微秒檢查；其他 AAC 組合保持前置時鐘門檻。介面及文件列出 48 kHz 雙聲道限制，不能把本批視為全部 AAC／M4A／MP4 相容性。

程式碼另檢查 native getters 的同執行緒 COM 生命週期、取消與 subtype／PCM 協商、codec 前置拒收及固定 MIME；新內容門檻無 unsafe。沒有新增 crate、安裝、OS worker、產品音訊／文字寫入、temp／cache、匯出、自動播放或跨供應商 fallback。全套仍僅自有合成資料、測試憑證、隱藏視窗、隔離程序與 loopback；沒有正式金鑰、使用者音訊或真實雲端要求。

真實 AAC 檔案、其他 encoder／時鐘組合、MP4／其他容器、URL、Auto、全文語意與播放／實際音訊定位仍待接續。Phase 08 尚未結案，Phase 09 尚未具備啟動條件；有效金鑰留至結案固定遮蔽介面手動輸入。設計與官方資料見 [ADR 0052](adr/0052-bounded-adts-aac-and-native-clock-gate.md)。

## 第十八批驗證

2026-10-14 在最終 Rust 程式碼實際執行並通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：沒有放寬 lint 或隱藏警告。
- `cargo test --locked --offline --quiet`：1339 項 library、12 項設定、4 項探索、10 項啟動，共 1365 項一般測試通過；既有 15 項 ignored 不變。library 實際耗時 147.06 秒，不作為延遲 SLA。
- `cargo build --locked --offline`：Debug 通過。
- `cargo build --locked --offline --release`：Release 通過，實際耗時 38.51 秒。

本批淨增 3 項測試：48 kHz 單聲道的真正 native tone／時間／pin、四種 AAC 率／聲道的直接 IMF sample time／duration 觀測、48 kHz 單聲道的完整 Prepared Ready。ADTS 聚焦 15 項通過後，再由全套回歸涵蓋最後增補的 worker／native 許可及 Ready pin 清理斷言；沒有新增 ignored。

十三秒的四種合成 AAC 直接讀取皆到真正 EOF，ReadSample timestamp 與 sample time 相同；48 kHz 單／雙聲道的最大相鄰 native duration 重疊為零，44.1 kHz 單／雙聲道為 43 個 100 ns ticks。此觀測不經產品 normalizer 或 map，也不假設一 AAC packet 一 native sample；44.1 kHz production gate 維持，僅開放已通過原有 PCM/frame 精度的 48 kHz 單聲道。直接 native duration 並不能消除已觀測到的重疊。

48 kHz 單聲道的十三秒合成檔分別經 OpenAI／Gemini 明確 mock、controller 的 16 kHz 與同一 Session API 的 24 kHz，完整到 ASR、全文詞彙、Standard Draft／Review 及 RAM Ready。每個 cue endpoint 由只用真正 decoder timestamp／frames 的獨立 oracle 核對；Ready 原檔 pin 拒絕 writer／delete，停止撤回文件並恢復可寫，原 bytes 不變、worker／native 許可完整恢復。原有停止、服務錯誤、borrowed DSP 拒收與中文字幕優先測試擴大至單／雙聲道；44.1 kHz 則仍在 PCM／speech／詞彙／translation 要求前拒收。

正式修改僅 native format 聲道門檻及 GUI 提示；新 unsafe 僅測試內同執行緒 RAM reader／sample 查詢，附 SAFETY 說明，COM／MF／sample 依 RAII 釋放。正式 capture、時鐘精度、bounded 容量、取消與 Session 規則沒有變更；沒有新增 crate、安裝、產品媒體／文字寫入、temp／cache、匯出、自動播放或跨服務 fallback。全套仍僅自有合成資料、測試憑證、隱藏視窗、隔離程序及 loopback，不使用正式金鑰、使用者音訊或真實雲端。

本批完成不代表任意 AAC 編碼器、使用者檔案、其他 AAC 率／包裝、影片容器或雲端品質已驗收。URL、Auto、全文語意脈絡及播放／實際音訊定位仍待接續，Phase 08 尚未結案，Phase 09 尚未具備啟動條件；有效金鑰留至結案固定遮蔽介面手動輸入。決策與官方資料見 [ADR 0052](adr/0052-bounded-adts-aac-and-native-clock-gate.md)。

## 第十九批驗證

2026-10-14 在本機 Windows `10.0.19045.0` 實際執行並通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：12.85 秒，沒有放寬 lint 或隱藏警告。
- `cargo test --locked --offline --quiet`：1354 項 library、12 項設定、4 項探索、10 項啟動，共 1380 項一般測試通過；既有 15 項 ignored 不變。library 實際耗時 176.19 秒，不作為延遲 SLA。
- `cargo build --locked --offline`：Debug 通過。
- `cargo build --locked --offline --release`：Release 通過，38.20 秒；已確認兩個執行檔存在。

本批淨增 15 項測試：10 項純 MP4 容器門檻、2 項原生 AAC／MP4、3 項 Prepared controller 整合。MP4 聚焦 16 項通過後，最後補上的真正重複音軌反例由全套回歸涵蓋。Clippy 初次指出固定長度 chunks_exact，已改成 as_chunks 後重新通過；沒有 lint suppression、新 ignored 或改用單執行緒模式。

門檻測試涵蓋單檔內引用、external url／ES URL／dependency 拒收、加密／非 LC／PCE／率或聲道描述不符、真正雙軌、重複 box、未知 extension、fragmented、全部 truncation 位置、u32／u64 長度及 table overflow、moov／box 容量、每份 sample 範圍與 sample count 一致性。另驗證 constant-size／多 chunk／多 run／co64、moov 在 mdat 前、頂層 extended size、逐 byte mutation 不 panic，以及一 byte 短讀、I/O 錯誤與 metadata 讀取中取消。純 header／table fixture 不被當成有效 AAC payload。

真正 Windows AAC／MP4 RAM encoder 產生單／雙聲道 48 kHz 音訊，以錯誤副檔名驗證內容判別；經正式 decoder 核對 PCM 格式、bounded block、440 Hz tone、RMS、EOF 時間及原有一微秒 gate。Ready 之前與之後的同一原檔 pin、writer／delete 拒絕、停止／取消釋放及 bytes 不變皆有核對。44.1 kHz MP4 的本批 clock gate 與 borrowed DSP 拒收也確認完整釋放。

十三秒 mono／stereo 檔分別經 OpenAI／Gemini 明確 mock、一般 controller 的 16 kHz 及同一 Session API 的 24 kHz，完整走 ASR、全文詞彙、Standard Draft／Review 及 RAM Ready。每份 cue 的 native endpoint 由真正 decoder timestamp／frames 的獨立 oracle 核對；worker／native owner 許可、停止撤回 Ready、服務錯誤及中文 sidecar 優先保持正確。外部引用和未開放時鐘都沒有 speech／詞彙／翻譯要求。

本批另檢查 metadata 配置上限、整數／slice 邊界、逐筆取消、同執行緒 COM／固定 stack blob 及媒體 pin 生命週期。正式 capture、pipeline 容量、Session 隔離與時鐘精度保持；沒有新增 crate、安裝、產品媒體／本文寫入、temp／cache、匯出、自動播放或跨服務 fallback。全套只使用合成資料、明確 mock、既有自有測試憑證／隱藏視窗／隔離程序及 loopback，沒有有效金鑰、使用者音訊或真實雲端要求。

本批只提供有限的單音軌音訊容器：影片、多音軌、fragmented、edit list、封面或其他非空 metadata 仍拒收。真實檔案相容性、44.1 kHz MP4、其他輸入、URL、Auto、全文語意及播放／實際定位仍待接續。Phase 08 尚未結案，Phase 09 尚未具備條件；有效金鑰維持結案固定遮蔽介面手動輸入。設計與官方來源見 [ADR 0053](adr/0053-self-contained-mp4-aac-prepared-input.md)。

## 第二十批驗證

2026-10-14 在本機 Windows `10.0.19045.0` 最終 Rust 程式碼實際執行並通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：18.59 秒，無 lint suppression 或隱藏警告。
- `cargo test --locked --offline --quiet`：1357 項 library、12 項設定、4 項探索、10 項啟動，共 1383 項一般測試通過；既有 15 項 ignored 不變。library 耗時 237.04 秒，不作為即時延遲 SLA。
- `cargo build --locked --offline`：Debug 通過。
- `cargo build --locked --offline --release`：Release 通過，56.77 秒；已確認兩個執行檔存在。

本批淨增 3 項測試：十二種組合的直接 native timing 觀測、44.1 kHz mono／stereo native tone／gate／pin、44.1 kHz 完整 Prepared Ready。MP4 聚焦 19 項通過；一般 GUI 的最後格式提示由全套回歸涵蓋。既有取消／borrowed DSP 拒收、停止／服務錯誤／中文字幕優先及外部引用反例擴大至兩種率與聲道，沒有新增 ignored 或改用單執行緒測試。

直接 timing 觀測要求最多 1024 次讀取內真正 EOF，ReadSample 與 sample time 相同、格式／flags／native sample 容量／frame alignment 正確。分別比較原生 duration 與 PCM frames 推導端點，不以產品 map 驗證自己；此機 44.1 kHz native duration 最大重疊一個 100 ns tick，frame-derived 端點最大重疊零，48 kHz 兩者皆零。正式測試只要求不超過既有一微秒，不要求其他 Windows 版本重現固定數字。完整十二種本機數值見 [ADR 0053](adr/0053-self-contained-mp4-aac-prepared-input.md#第二十批441-khz-原生時間證據)。

原生 decoder 的實際 bounded block 逐筆通過原有 DecodedAudioGate，核對 tone 頻率／RMS、EOF、原檔 byte 不變及 writer／delete pin。十三秒加短尾音訊經 OpenAI／Gemini 明確 mock、controller 的 16 kHz 與同一 Session API 的 24 kHz，完成 ASR、詞彙、Standard Draft／Review 到 RAM Ready；所有 cue endpoint 以獨立 decoder timestamp／frames oracle 核對。停止撤回文件、取消 token、錯誤清理、原檔恢復可寫與 worker／native 許可回收都有驗證。外部引用在兩種率／聲道都無 speech／詞彙／翻譯要求；ADTS 44.1 kHz 前置拒收由既有全套測試確認。

靜態審查另核對容器選擇與 private rate gate、固定 ASC／PCM 格式、同執行緒 COM、取消／Session 及原檔 pin。正式改動只有容器限定的 rate gate、註解及格式提示；沒有放寬時鐘、新增 crate／OS worker／產品媒體或本文寫入、temp／cache、匯出、自動播放或跨服務 fallback。新增 unsafe 僅測試內直接 RAM reader／sample 查詢，附 SAFETY 並以 RAII 釋放。

全套只使用自有合成資料、明確 mock、既有測試憑證／隱藏視窗／隔離程序及 loopback；沒有有效金鑰、使用者音訊或真實雲端要求。M4A／MP4 目前仍是有限的單音軌 AAC-LC 容器；影片、封面等未支援結構、一般檔案相容性、URL、Auto、全文語意及播放／實際定位仍待接續。Phase 08 尚未結案，Phase 09 尚未具備條件；有效金鑰維持結案固定遮蔽介面手動輸入。

## 第二十一批驗證

2026-10-14，在本機 Windows `10.0.19045.0` 最終 Rust 程式碼實際執行並通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：16.32 秒，沒有 lint suppression 或隱藏警告。
- `cargo test --locked --offline --quiet`：1370 項 library、12 項設定、4 項探索、10 項啟動，共 1396 項一般測試通過；既有 15 項 ignored 不變。library 耗時 234.98 秒，不作為即時延遲 SLA。
- `cargo build --locked --offline`：Debug 通過。
- `cargo build --locked --offline --release`：Release 通過，33.76 秒；已確認兩個執行檔存在。

本批淨增 13 項測試：9 項純 metadata 型別／容量／畸形／取消、1 項容器音訊引用不可被標籤掩蓋、1 項真正 decoder PCM／時間比較、2 項 Prepared 整合。metadata 聚焦 10 項、MP4 聚焦 32 項通過；最後增補的 provider 要求內容檢查由全套回歸涵蓋。沒有新增 ignored 或改用單執行緒模式。

純測試核對十種 UTF-8 文字 tag、URL／path 形狀文字、不解碼的 JPEG／PNG 開頭、固定 trkn／disk／tempo／compilation record。另核對錯版本／type／encoding／封面不符、未知／external reference／freeform／lyrics／nested atoms、duplicate／multiple values、文字 64 KiB、封面 512 KiB、整體 768 KiB 邊界、所有截斷位置、長度溢位、逐 byte mutation 不 panic，以及預先取消連空資料也拒收。原有 moov 一 MiB 與 sample table／external audio reference 測試仍有效。

真正 native encoder fixture 只在 RAM 替換最後的 udta，加入自有文字／數字與 genuine 一像素 PNG；既有 mdat／AAC／sample tables／offsets 不變。兩秒加短尾、四種率／聲道各開原檔與 tagged 檔，逐筆比較所有 PCM、sequence、native start／end／EOF，全部相同且通過原有 gate。兩個有界 blocks 即完成比較，沒有整份 PCM 副本；原檔 bytes／writer／delete pin 及 RAII 清理有核對。

十三秒加短尾的 tagged M4A 經兩個所選服務 mock、16／24 kHz、ASR／全文詞彙／Standard Draft／Review 到 RAM Ready，所有 cue endpoint 由獨立 native oracle 核對。詞彙、source、stable／prepared context、draft／neighboring drafts、committed prefix 與 glossary 都檢查不可含標題／作者／專輯／URL fixture 標記；沒有真正雲端傳輸。停止／服務錯誤／中文字幕優先、token 取消、Ready 撤回、媒體 pin 與 worker／native 許可回收仍正確。

新增產品模組完全 safe Rust，只借用有界 slices，逐筆及重工作前後檢查取消；不回傳 metadata model，不解析 URL 或解碼圖片，不改時鐘／取樣資料。沒有新增 crate、安裝、產品 unsafe／OS worker、媒體／標籤／封面寫檔、temp／cache、匯出、自動播放或跨服務 fallback。全套仍只操作自有合成資料、既有測試憑證／隱藏視窗／隔離程序及 loopback，不讀取有效金鑰、使用者音訊或真實雲端。

目前只接受明確的單值標籤子集合；未知／多值／超量 metadata、影片、多音軌、fragmented／edit list 仍拒收。完整圖片有效性、一般 tag writer／使用者檔案相容性、URL、Auto、全文語意與播放／實際定位仍待驗證或實作。Phase 08 尚未結案，Phase 09 尚未具備條件；有效金鑰留至結案固定介面手動輸入。來源與限制見 [ADR 0054](adr/0054-bounded-mp4-descriptive-tags.md)。

## 第二十二批驗證

最終 Rust 程式碼已實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：18.20 秒，沒有隱藏警告或新增 lint suppression。
- `cargo test --locked --offline --quiet`：1385 項 library、12 項設定、4 項探索、10 項啟動，共 1411 項一般測試通過；既有 15 項 ignored 不變。library 耗時 353.23 秒，不作為字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.98 秒。
- `cargo build --locked --offline --release`：Release 通過，44.06 秒；兩個執行檔存在。

2026-10-14，本機 Windows `10.0.19045.0`，MP4 聚焦 `cargo test --locked --offline mp4 -- --nocapture` 的 47 項測試通過，187.36 秒。較久的個別測試提示來自既有測試共享原生資源鎖及完整 Ready 流程，不作為即時延遲量測，沒有用單執行緒模式取代並行 harness。

本批淨增 15 項測試：8 項容器／軌道／表格，1 項 AVC 描述，3 項原生解碼／取消／時間，3 項 Prepared Ready／取消錯誤／無服務要求。純結構 fixtures 明確不含可播放 codec payload，只證明門檻；真正音訊與 Ready 使用原生 RAM H.264 Baseline／AAC encoder。

每軌引用、重複 ID／handler、無音軌、額外軌道、protected／Main profile、SPS／PPS framing／界限、尺寸、stss 索引及 audio ctts count 均有反例。交錯 chunks 可通過，cross-track alias 與 mdat escape 拒收；每個截斷位置、逐 byte mutation 不 panic、metadata 讀取中取消及 AVC 容量／尾端未知資料拒收。預處理、辨識及翻譯沒有收到無效視訊軌內容。

原生 fixture 在 RAM 編碼 128×96、每秒一張固定 NV12 合成影格與 440 Hz tone；獨立原生 H.264 stream 型別／尺寸及壓縮 samples 數量／EOF 有核對。產品 decoder 只選音訊，兩秒加尾段四種率／聲道核對 gate、64 KiB block、frequency／RMS、真正 EOF、writer／delete pin、原 bytes 不變；取消及公開 borrowed clock 拒收也覆蓋四種組合。

直接 IMF timestamp／duration／PCM frames 的十二種影音 MP4 組合（44.1／48 kHz、單／雙聲道、一秒／兩秒加 137 frames／十三秒加 311 frames）全部真正 EOF，native duration 最大重疊 1 hns（0.1 微秒），frame-derived 端點無重疊；與本機同長度 audio-only 觀測相同。仍使用既有一微秒 gate，沒有用 stts／ctts 覆蓋原生時間、補靜音、裁 codec padding 或修正樣本。

十三秒 genuine 影音檔的四種率／聲道，經兩個所選服務的 mock、16／24 kHz、ASR／全文詞彙／Standard Draft／Review 到 RAM Ready；每個 cue endpoint 以獨立 native oracle 核對。另含 tagged／PNG 影音組合，服務要求無任何描述標記；停止／Network 錯誤／中文字幕優先、token 取消、Ready 撤回、pin 和 worker／native 許可清理皆核對。無效 video reference／profile／offset 在任何 speech／詞彙／翻譯要求前失敗，要求數都為零。

新增產品程式為 safe Rust、有界借用的 metadata／chunk cursor。兩軌合併沒有每個 sample range 的 allocation；沒有新增 crate、安裝、產品 unsafe／encoder／renderer／worker、媒體保存／temp／cache、匯出或自動播放。所有測試只使用自有合成資料及明確 mocks，不讀取有效金鑰、使用者媒體或正式雲端。

僅支援上述 Baseline＋AAC 有限子集合；其他 profile／codec、多音軌、fragmented／edit list、一般影片／muxer／真實檔案相容性仍待驗收。內嵌字幕、URL、Auto、全文語意及播放／實際定位待接續。Phase 09 尚未具備啟動條件；金鑰留至結案固定介面手動輸入。設計見 [ADR 0055](adr/0055-bounded-h264-aac-mp4-prepared-input.md)。

## 第二十三批驗證

最終 Rust 程式碼已實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：31.75 秒，沒有新增 lint suppression 或隱藏警告。測試 helper 的固定切塊寫法依 lint 改為 `as_chunks`，最終全套涵蓋修正。
- `cargo test --locked --offline --quiet`：1398 項 library、12 項設定、4 項探索、10 項啟動，共 1424 項一般測試通過；既有 15 項 ignored 不變。library 耗時 395.32 秒，不作為字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.88 秒。
- `cargo build --locked --offline --release`：Release 通過，58.61 秒；兩個執行檔存在。

2026-10-14，本機 Windows `10.0.19045.0`，`cargo test --locked --offline mp4 -- --nocapture` 的 60 項聚焦測試通過，258.54 秒；保持正常並行 harness。較久的測試訊息含既有 native 資源鎖等候與完整 Prepared 生命週期，不作為字幕延遲 SLA。

本批淨增 13 項測試：4 項 AVC／SPS、2 項組合時間表、4 項原生 fixture／tone／pin／取消／直接時間、3 項 Prepared 整合。純描述測試核對 Main／High、可選 extension、High 4:2:0／8-bit、SPS ID 0／31／超限、過長 prefix、emulation prevention 有效／截斷／非法下一 byte、未知 profile、SPS mismatch、所有截斷位置／mutation 不 panic 與預先取消。表格核對 signed／unsigned records、版本／flags／count／size，仍拒收 external reference／mdat escape／空 sample，音訊 version 1 保持拒收。

真正 RAM fixture 用 typed profile 配置 Windows encoder，輸出 avcC profile 必須相符；獨立 raw H.264 型別／尺寸、非空壓縮 sample 數及真正 EOF 也核對。Main／High 各四種率／聲道的 AAC 音訊 tone／RMS、64 KiB block、一微秒 gate、真正 EOF、原檔 bytes、writer／delete pin、取消與公開 borrowed clock 拒收都通過。未新增產品視訊 decoder／renderer 或 encoder。

Main／High 各十二種率／聲道／長度（一秒、兩秒加 137 frames、十三秒加 311 frames）直接查詢 IMF timestamp／duration／PCM frames。24 種組合全部真正 EOF；44.1 kHz native duration 最大重疊 1 hns（0.1 微秒）、48 kHz 沒有重疊，frame-derived 端點無重疊，與本機 audio-only／Baseline 觀測相同。沒有用 stts／ctts 修改 IMF 時間、補靜音、裁 codec padding 或修正樣本。

Main 44.1 kHz mono 與 High 48 kHz stereo 的十三秒加尾段、文字標籤與 genuine PNG 經兩個所選服務 mock／16／24 kHz、ASR／全文詞彙／Standard Draft／Review 到完整 RAM Ready；每個 cue endpoint 與獨立 native oracle 相同，要求不得含 metadata 標記。Main 48 kHz mono 與 High 44.1 kHz stereo 另核對 Network 錯誤、pending 取消、token、中文字幕優先與資源清理。兩種 profiles 的無效 video reference／profile／offset 都在任何 speech／詞彙／翻譯要求前拒收，要求數為零。

產品新增 safe Rust 的借用 bit reader 與視訊 table 門檻；只有 header 四欄位解析、每欄 fixed prefix 上限及逐 row／SPS 取消檢查，沒有新增 RBSP／視訊 payload allocation。既有 moov 一 MiB、avcC 64 KiB、sample rows、cross-track ranges、唯讀 pin、bounded worker／ASR／PCM 及 Session 契約保持。沒有新增 crate、安裝、產品 unsafe／OS worker、媒體保存／temp／cache、匯出、自動播放或服務 fallback。測試只用自有合成來源與明確 mocks，沒有有效金鑰、使用者媒體或正式雲端。

本批只有有限的 ordinary H.264 profiles／AAC-LC 子集合。ctts 是結構門檻，原生 test encoder 使用預設零 B frames，沒有宣稱真實 B-frame／其他 muxer／一般影片或播放相容性；High10／422／444／其他 profile、其他深度／chroma／SPS-ext、edit list、fragmented、多音軌仍拒收。URL、Auto、內嵌字幕、全文語意及播放／實際定位待接續，Phase 09 尚未具備條件；金鑰維持結案固定介面手動輸入。決策與主要來源見 [ADR 0056](adr/0056-ordinary-h264-profiles-and-composition-tables.md)。

## 第二十四批驗證

最終 Rust 程式碼已實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：31.16 秒，沒有隱藏警告或新增 lint suppression。
- `cargo test --locked --offline --quiet`：1407 項 library、12 項設定、4 項探索、10 項啟動，共 1433 項一般測試通過；既有 15 項 ignored 不變。library 耗時 431.97 秒，不作為字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，1.00 秒。
- `cargo build --locked --offline --release`：Release 通過，Cargo 顯示 1 分 21 秒；兩個執行檔存在。

2026-10-14，本機 Windows `10.0.19045.0`，`cargo test --locked --offline b_frames -- --nocapture` 的 7 項聚焦測試通過，55.47 秒；`b_slice_tests` 的 2 項純證據測試通過。測試模組依 Clippy 要求移到檔案末尾後，再執行最終全套回歸；沒有用 lint suppression 或單執行緒模式掩蓋問題。

本批淨增 9 項測試：2 項 Annex B／slice_type／prefix 證據，4 項原生 B-frame／tone／pin／取消／直接 AAC 時間，3 項完整 Prepared 整合。B-frame fixture 是真正 Windows MFT 編碼；不能只靠 maximum count=2 的設定或改寫 ctts 宣稱成功。先核對原生 H.264、固定尺寸、非空 samples、完整影格數及 EOF，再解析非 IDR NAL 的實際 B slice（type 1／6），並要求 presentation timestamps 倒序。

Main／High、44.1／48 kHz、單／雙聲道八種組合的六秒加 137 frames，均有七張影格、兩張 B frames、兩次時間倒序。四秒與十三秒加 311 frames 的案例分別觀測到一／四張 B frames，且有對應時間倒序；此處是固定 fixture 的實際數字，不要求其他 Windows／encoder 重現相同 GOP。三／四 bytes start code、I／P／B 的兩種 slice_type 範圍、SPS／無 packet boundary、截斷／過長 prefix／forbidden NAL 的純反例另有核對；畸形 fixture 必須失敗，不作假成功。

Main／High 各十二種 profile／率／聲道／長度（四秒、六秒加 137 frames、十三秒加 311 frames）直接查詢 IMF AAC timestamp／duration／PCM frames，24 種組合全部 EOF。44.1 kHz native duration 最大重疊 1 hns（0.1 微秒）、48 kHz 無重疊，frame-derived 端點均無重疊；原有一微秒 gate 保持。十三秒加尾段的原生音訊端點與前批相同輸入長度的非 B-frame 觀測相符，沒有以視訊 ctts 重設音訊、補靜音或裁 codec padding。

八種組合另外驗證 tone／RMS、64 KiB block、原 bytes 不變、writer／delete pin、取消與公開 borrowed clock 拒收。Main 44.1 kHz mono／High 48 kHz stereo 的十三秒加尾段、合成文字與 genuine PNG，經兩個所選服務 mock、16／24 kHz、ASR／全文詞彙／Standard Draft／Review 到 RAM Ready；cue endpoints 與獨立 native oracle 相同，要求不得含 metadata。Main 48 kHz mono／High 44.1 kHz stereo 核對 Network 錯誤、pending 取消與 token、中文字幕優先及資源清理；兩種 profiles 的 video ctts 版本／count、self-contained reference 反例均在任何 speech／詞彙／翻譯要求前失敗，要求數為零。

新增原生 unsafe 限 cfg(test) 的 MFT 型別／數值 VARIANT 操作及借用鎖定的壓縮 buffer，均註明同一 MTA、指標／長度、呼叫生命週期與 RAII。原 encoder helper 仍限十六秒／320 KiB，raw reads 限 64，slice prefix 限五個零／48 macroblocks；只借用當前有界 native buffer，沒有整份 RBSP／完整影片／解碼影像副本或 raw payload logging。新增 windows 既有 OLE binding feature 不新增套件／版本或系統安裝。

正式 parser／decoder／時鐘／ASR／Ready／UI 保持；沒有新增產品 encoder／renderer／worker、媒體保存／cache／匯出、自動播放或服務 fallback。全套仍只用自有合成資料、測試憑證、隱藏視窗、隔離程序與 loopback，沒有有效金鑰、使用者影音或正式雲端。

這些證據只涵蓋本機固定 B-frame 合成案例；其他 muxer／設定／GOP／fps／尺寸／一般影片、播放器同步與雲端品質仍待驗收。URL、Auto、內嵌字幕、全文語意及播放／實際定位待接續，Phase 09 尚未具備條件；有效金鑰維持結案固定遮蔽介面手動輸入。設計與主要來源見 [ADR 0057](adr/0057-genuine-b-frame-mp4-audio-evidence.md)。

## 第二十五批驗證

2026-10-14，本機 Windows `10.0.19045.0`，最終 Rust 程式碼實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：27.56 秒，未隱藏警告。
- `cargo test --locked --offline --quiet`：1429 項 library、12 項設定、4 項探索、10 項啟動，共 1455 項一般測試通過；既有 15 項 ignored 不變。library 441.91 秒，不作為字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，1.06 秒。
- `cargo build --locked --offline --release`：Release 通過，38.69 秒。

本批新增 22 項純樣本／共用文字階段測試。`cargo test --locked --offline embedded_tx3g -- --nocapture` 的 22 項聚焦通過，1.15 秒；最後固定切片寫法修正後，由上述完整回歸再次涵蓋全部 22 項。沒有新增 ignored 或 lint suppression。

涵蓋 Unicode／emoji、UTF-8／UTF-16 BE／LE／BOM、明確八種文字語言聲明、clear samples／gaps、newline／literal markup、完整長度與所有 sample 截斷位置、非法 UTF-8／surrogate／控制字元／BOM-only、modifier 拒收、非零建構條件、無本文軌道、舊 Session／generation、連續 sample sequence、時間重疊／overflow／微秒折疊／24 小時端點、取消與失敗後釋放及無部分文件。6000 份 90 kHz 樣本的 absolute endpoint 與各 cue 接續核對，不累積 duration 截斷；單份 raw／UTF-16-to-UTF-8 膨脹、16384 次 clearing work、8192 cues／四 MiB decoded text／八 MiB wire budget 分別測試。

輸出使用既有 ExistingText 結構與 structural validation。合成簡體本文實際經既有 OpenCC 臺灣片語轉為「軟體、記憶體與滑鼠」；原文及原 ticks 換算的時間保留。五語明確聲明共用既有 Standard Draft／Review mock 引擎，核對本次 identity、固定 zh-TW、前／後句 context、review draft、結果順序與 original timestamps。這是資料契約與演算法驗證，不是辨識／翻譯品質或完整 Prepared Ready 驗收。

核心新增沒有 unsafe、thread／task／queue、credentials／network、檔案 I/O／產品寫入、保存／匯出或內容 cache。sync sample work 要由未來容器 reader 在 Prepared 有界 CPU lane 執行；單份資料與整份文件皆有明確上限，任一失敗釋放文字。完整回歸沿用既有自有合成媒體、唯一測試憑證、隱藏視窗、隔離程序與 loopback，未使用使用者媒體、正式金鑰或真實雲端。原有 MP4 音訊門檻、capture、decoder、時鐘、Session／取消／Ready 與一般 GUI 流程保持。

尚未接入真實 MP4 軌道讀取、tx3g sample description／display semantics、tables／references／edit list／composition offset、跨軌 byte 範圍／同一 pin、GUI 選軌／語言／字幕優先；一般 --ui 仍不讀內嵌字幕。來源 content ID 僅是本次區域識別，不能充當版本／alignment 或仲裁證據。樣式／karaoke／links 等 modifier 與其他字幕 codec 尚不支援；真實檔案、URL、Auto、完整語意、播放定位與雲端品質仍待接續。Phase 09 尚未具備條件；有效金鑰維持結案固定遮蔽介面手動輸入。決策及格式主要來源見 [ADR 0058](adr/0058-bounded-tx3g-plain-text-samples.md)。

## 第二十六批驗證

2026-10-15，本機 Windows `10.0.19045.0`，最終 Rust 程式碼已實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：24.36 秒，沒有新增 lint suppression。
- `cargo test --locked --offline --quiet`：1453 項 library、12 項設定、4 項探索、10 項啟動，共 1479 項一般測試通過；既有 15 項 ignored 不變。library 394.19 秒，不作為字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.80 秒。
- `cargo build --locked --offline --release`：Release 通過，55.66 秒。

本批新增 24 項一般測試：17 項容器／文字 samples 與 7 項 worker／pin／取消。`cargo test --locked --offline mp4_subtitle -- --nocapture` 的 24 項聚焦通過，0.04 秒；後續把兩個共用 production lane 的測試改為同步 fixture，持有全域序列化 guard 再由 runtime.block_on 執行，避免 async function 持同步鎖跨 await。其餘 controlled tests 使用自有注入的 semaphore，沒有全域鎖跨 await 或 lint 忽略。最終完整回歸重新包含全部 24 項；synthetic worker panic 只用來核對 WorkerFailed／容量回收，沒有假成功或新增 ignored。

合成 MP4 結構包含真正的 UTF-8／UTF-16 BE／LE tx3g bytes，以及由實際 byte lengths 建立的 sample offsets。音訊／視訊 payload 明確只為 gate fixture，不能作 playable AAC／H.264 或 muxer 相容性證據。兩／三軌、constant／variable sample sizes、stco／co64、多 chunk／未引用 gap、clear samples／原始時間、extended top box、明確八種文字語言／und 與衝突拒收都測試。跨音訊／視訊／文字 byte alias、out-of-mdat、錯誤 counts／sizes／duration、外部 reference、edts／ctts／senc、非法 description／font／default ID／UTF-8、完整輸入截斷及零本文文件均拒收。

movie rounding 以 90 kHz text timescale 對 1000 Hz movie timescale 的 floor／ceil 測試，保留原 ticks 對應的 7777／12222／11111 微秒，沒有採 movie duration 改寫字幕；不符 rounding 的值拒收。raw sample 超限與超過 16384 份的 stsz 在完整 sample 讀取前 TooLarge；不足兩 bytes 為 InvalidSubtitle。舊文字核心原有八 MiB wire／四 MiB decoded text／8192 cues 與取消／Session 測試由全套一併驗證。

Counting reader 核對無效範圍時沒有讀取任何完整 text sample；既有共用 loader 的最後一 byte mdat 可讀性探測另有明確保留，不把它宣稱為完全未碰 payload。讀取中取消及第二份畸形清除 sample 都不返回第一份 cue 或部分文件。描述 ftab 的大小／唯一 ID／count／default font、截斷及尾資料有個別核對，不載入媒體字型。

Windows entry test 真正開啟自有 temp 合成檔，來源副檔名是 data 仍按內容解析，文字結果與 identity 正確；保留 pin 時 writer／delete 拒絕，原 bytes 不變，Drop 後可寫入／移除。Controlled barrier 在 blocking worker 取得同一類 pin 後中止 Future，核對取消 token、writer／delete 仍拒絕、CPU lane 飽和，放行到真正退出才可取得許可／移除檔案／再次成功；未使用 sleep 推測。未 poll Drop、取消後仍回成功、無效 identity、過載、panic／typed error 及復原都有結果。remote input 明確 UnsupportedUrl，不建立服務或 fallback。

沒有新增 unsafe、相依套件、系統元件、產品媒體／字幕寫入／temp／cache／export、雲端／credential lookup、音訊或視訊 decoder。只共用既有 bounded CPU lane／local file pin／MP4 metadata gate，新增 async owner 不在 capture／UI thread 做讀取。完整回歸沿用既有自有合成媒體、唯一測試憑證、隱藏視窗、隔離程序及 loopback，未讀取使用者媒體、正式金鑰或真實服務。沒有 .git，因此未建立 commit，也未初始化或丟棄任何原有檔案。

目前只有指定單一 text／tx3g track 的 Rust 呼叫介面；一般 GUI／PreparedRequest 的選軌、語言、優先選用、全文翻譯與 Ready 尚未接上此 reader，原音訊 probe 仍拒收含文字軌的媒體。更多 handler／codec／多字幕軌、動態文字／modifier、UTF-16 font names、edit list／composition、真實 muxer／使用者影片／播放與雲端品質仍未驗收；區域 content ID 不作版本／alignment 或來源仲裁證據。Phase 09 尚未具備條件；有效金鑰維持結案固定遮蔽介面手動輸入。決策及主要格式依據見 [ADR 0059](adr/0059-selected-pinned-mp4-tx3g-reader.md)。

## 第二十七批驗證

2026-10-15，本機 Windows `10.0.19045.0`，最終 Rust 程式碼實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：最後一次 10.97 秒，無新增 lint suppression。
- `cargo test --locked --offline --quiet`：1473 項 library、12 項設定、4 項探索、10 項啟動，共 1499 項一般測試通過；既有 15 項 ignored 不變。library 447.31 秒，不作為字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.97 秒。
- `cargo build --locked --offline --release`：Release 通過，49.61 秒。

本批新增 20 項一般測試：8 項 Session／規劃／pin，5 項 controller／所選服務，2 項 UI draft，5 項 hidden native dialog。`cargo test --locked --offline embedded_prepared -- --nocapture` 的 20 項聚焦通過，4.74 秒；其後完整回歸涵蓋新增與既有全部行為，沒有新增 ignored 或假 cloud success。

規劃對四種語言順位的全部 embedded／sidecar 組合核對，equal rank 明確 AmbiguousSubtitle，text_language 同樣回報錯誤。八份總預算包含 embedded，重複內嵌設定拒收。三種中文真正讀取合成 MP4 原文並轉為「軟體與記憶體／滑鼠與伺服器」，同一個一份 CPU 許可的 Session 可到 Ready；原文、0／700000／1100000／1000000 微秒的 cue start／duration、identity、byte length 與原檔 bytes 保留。傳入會 panic 的 Translator 或 SpeechRecognizer 不被呼叫，即使音訊 Auto 仍走已選文字，沒有 ASR／decoder。

來源字幕以五語 × 三種品質 × 兩個所選服務，共三十組 explicit mock controller 驗證。每組只建一個 Translation service／一次 synthetic key，先全文 analysis，再使用本次已本地化「極光引擎」詞彙；Fast 兩個 cue 各一次 Draft，Standard／High 另有 Review，原文與時間、結果順序及品質參數一致。這是正式 controller／契約與 mock 引擎的整合證據，不代表三十組真實雲端品質或實際帳號配額已驗收。

pending analysis 期間同一 media pin 擋 writer／delete，CPU 許可已全部歸還；Stop 後 token Cancelled、沒有 translation／Ready，worker reaped 後可寫原檔並重新準備中文。來源錯軌／畸形容器不產生 glossary／translation 要求，Network error 保留型別、不跨服務 fallback；沒有服務的來源字幕在建立 worker／factory 前拒收。舊完成文件不能覆蓋新 Ready，單 lane 飽和回報 Overloaded 並可復原；錯軌不退回較低順位 sidecar 或 ASR。所有原文／結果／pin 只在本次 RAM，沒有新增產品輸出／temp。

UI draft 以完整 ASCII u32 decimal 驗證軌 ID，涵蓋空字串／零／正負號／空白／換行／全形數字／小數／溢位／過長／路徑字串拒收，最大 u32 與十位前導零的有效非零 ID 通過；明確八種文字語言保留，本機路徑 guard 保持。native format combo 從三個增加為四個選項，空白時仍能選模式；embedded 模式空白 ID 只產生輸入錯誤，不顯示音訊傳送或 AUTO_NOTICE，不啟動 backend，音訊語言停用。切回外掛空白模式才恢復既有 ASR disclosure。

hidden native tests 驗證八種文字語言／三品質、中文不讀無效 password edits、兩個所選來源服務僅移交正確 synthetic key、另一列超限不被讀取、兩列清空及 undo 規則沿用；來源文字 disclosure 含所選 endpoint 及不傳媒體音訊。真實 PreparedController 的中文 hidden dialog 實際完成 manual track 3 → Ready 共兩段 → clear，原 pin 保持到清除，通知 fixture 只觸發一次、不播放；錯軌 4 明確 Failed，釋放 pin 後修正回 3 能再次 Ready／清除／關閉。通知是明確 mock，未驗收真實 Windows toast、使用者視覺操作或新增螢幕配置。

正式程式沒有新增 unsafe、crate／系統安裝、產品錄製／媒體或字幕寫入／cache／export、credential logging、第二個 worker pool／STT／翻譯時間軸或自動播放。新增 native test unsafe 僅同一執行緒自有且存活 child HWND 的 scalar enabled-state 查詢，附 SAFETY，沒有 pointer／buffer 或 ownership 逸出；Win32 模組沒有新增正式 unsafe 呼叫。整份回歸只使用自有合成檔案／音訊、唯一測試憑證、隱藏視窗、隔離程序與 loopback，未讀取正式金鑰或使用者媒體、未呼叫真實服務。來源容器的音訊／視訊 bytes 在新增 fixture 中明確為結構 gate 資料，不作 playable codec／muxer 證據。

一般 --ui 已接通明確指定的單一 text／tx3g 軌；需要使用者已知軌 ID，尚無探索清單／自動語言或版本判定。前批無 edit list／文字 ctts／modifier／多字幕軌等門檻未放寬，普通 MP4 音訊 ASR gate 仍拒收含文字軌的輸入。Ready 僅為完整文件準備，content ID 不作 alignment／arbiter／版本證據。自動探索、一般 muxer／實際影片、更多描述、URL／Auto、完整語意、播放／音訊定位與真實雲端仍待接續；Phase 09 尚未具備條件。有效金鑰維持結案固定遮蔽介面手動輸入。設計及操作見 [ADR 0060](adr/0060-explicit-embedded-prepared-ready-and-ui.md)。

## 第二十八批驗證

2026-10-15 至 2026-10-16，本機 Windows `10.0.19045.0`，最終 Rust 程式碼實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：14.52 秒，沒有新增 lint suppression。
- `cargo test --locked --offline --quiet`：1486 項 library、12 項設定、4 項探索、10 項啟動，共 1512 項一般測試通過；既有 15 項 ignored 不變。library 460.16 秒，不當作字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.85 秒。
- `cargo build --locked --offline --release`：Release 通過，45.91 秒。

本批新增 13 項一般測試：4 項容器探索、5 項 owner、1 項 async 邊界、3 項 hidden native GUI；並把探索按鈕納入原有 native DPI／client bounds 回歸。首次聚焦 `mp4_discovery` 的 11 項通過，後續補上 CPU 飽和／未 poll Drop、原生來源變更取消後，以完整回歸驗證最終 13 項與全部既有測試，沒有新增 ignored。

容器以 zho／und／五語共七種語言提示驗證：zho 僅中文，不推定臺灣繁中或繁簡；mul／spa／非法 packed 語言不支援。無效 sample count／size／time／ID／offset／description 等不返回 candidate，全部輸入截斷與預先取消拒收。Counting reader 確認探索沒有讀完整 mdat／文字 samples；共用 loader 的一 byte EOF liveness 另有明確保留。損壞文字 sample 長度但結構有效的 fixture 可探索成功，後續抽取仍明確失敗，證明未把 metadata success 當成正文或 Ready。

owner 以 controlled channels 驗證 pending 取消、故意在取消後回傳成功、已緩衝成功退休、退出前拒絕重啟、重啟的新 identity、typed error／panic 復原與 Drop 取消／join。正式 discovery entry 真正讀取自有合成檔、結果只含 ID／提示，退出後 writer 可重新開啟，原 bytes 不變且沒有產品輸出檔；remote input 明確 UnsupportedUrl，沒有建立網路服務。未 poll Future Drop 取消，佔滿正式共用 CPU lane 後 discovery 在開檔前回 Overloaded。

hidden native 測試完整完成本機探索→填 ID／內嵌模式→要求中文明確宣告→真正中文 PreparedController→Ready 共兩段→清除／釋放 pin。探索不讀兩列 synthetic password edits，開始即清空及清除 undo；不觸發 backend 或 Ready 通知，不自動準備／播放。五語提示填入對應選項、服務仍由使用者選擇，沒有 API 要求；來源改動清除自動 ID／語言。程式化來源改動在首次 poll 前取消 pending／buffered 成功，舊 ID 不回填，新來源失敗保留型別；錯誤、清除及工作中關閉後可移除 fixture。通知仍使用明確 mock，不宣稱真實 toast／使用者視覺操作已驗收。

正式新 worker／parser 沒有新增 unsafe、unwrap／expect、相依套件或安裝。Win32 新增 unsafe 僅以 scalar 固定 index 更新同一 UI 執行緒自有 combo，附 SAFETY；沒有 pointer／buffer 或所有權逸出。全批只使用既有共用 CPU semaphore／取消及 pin 邊界，沒有第二個 CPU pool、ASR／翻譯時間軸、正式金鑰讀取、產品媒體／文字寫入、temp／cache／export。完整回歸沿用自有合成媒體、唯一測試憑證、隱藏視窗、隔離程序及 loopback，未讀使用者媒體或正式金鑰、未呼叫真實雲端。本機無 .git，未建立 commit 或初始化 repository。

現在一般 GUI 可探索已支援的單一 tx3g 軌，不必先從外部取得 ID；仍不是多文字軌清單或一般 MP4 相容性保證。探索來源 pin 在退出釋放，準備時重新開檔並驗證，不能把這次探索當成之後版本或 alignment 證據。原 AAC-LC／可選 H.264、無 edit list／文字 ctts／modifier 等門檻保持；fixture 的 codec bytes 是結構資料，不代表 playable codec／muxer。多文字軌／其他格式、真實影片、URL／Auto、完整語意、播放定位與真實雲端仍待接續，Phase 09 尚未具備條件；有效金鑰維持結案固定遮蔽介面手動輸入。見 [ADR 0061](adr/0061-bounded-local-mp4-subtitle-discovery.md)。

## 第二十九批驗證

2026-10-16，本機 Windows `10.0.19045.0`，最終 Rust 程式碼實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：12.31 秒，無新增 lint suppression。
- `cargo test --locked --offline --quiet`：1496 項 library、12 項設定、4 項探索、10 項啟動，共 1522 項一般測試通過；既有 15 項 ignored 不變。library 474.79 秒，不當作字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，1.25 秒。
- `cargo build --locked --offline --release`：Release 通過，1 分 02 秒。

本批新增 10 項一般測試：5 項多軌容器、1 項 discovery owner、1 項所選服務 controller、3 項 hidden native GUI；原 native DPI／client bounds 同時納入探索按鈕及新 combo。`cargo test --locked --offline mp4_multitrack -- --nocapture` 的 10 項聚焦通過，1.34 秒，後續完整回歸涵蓋最終新增與既有全部行為，沒有新 ignored 或假 cloud success。

八種明確文字語言的合成容器有八條不同本文／ID 的 tx3g 軌；探索按容器順序返回固定 catalog，三種中文都只提示中文、不猜繁簡或地區。每條指定抽取都只返回自己的兩段原文、identity 及 0／1100000 微秒 start、700000／1000000 微秒 duration；clear sample 的空窗保持。第九軌 TooLarge，無文字軌拒收，單一結果介面遇多軌 AmbiguousSubtitle；原 MP4 音訊 ASR gate 仍拒收文字軌。沒有新增同時準備八份文字或自動語言順位選用。

未選軌的重複 ID／無效 sample count／offset／duration、文字軌之間的 byte alias 均拒收整份探索與抽取；所有輸入截斷、預先取消不返回部分 catalog。Counting reader 核對探索不讀完整本文，仍保留共用 loader 的一 byte mdat EOF liveness；未選軌本文故意損壞時，探索與完好選定軌抽取可成功，選到損壞軌才失敗，不會讀取或混入未選本文。結構成功不當作所有本文有效或 Ready 證據。

正式 owner 實際讀取自有八軌 fixture，返回八個選項、明示必須選擇，退出後 pin 釋放且原 bytes 不變。controller 使用兩個所選服務的明確 mock 分別驗證來源軌／第二條中文：只保留指定原文／時間，來源只建立一次所選翻譯服務及一次分析；中文傳入會 panic 的 factory 仍能在本機到 Ready「字幕軌 4 軟體與記憶體」，不建服務或取用 key。Ready 擋 writer，stop 釋放 pin，原檔不變。這是本機契約／mock 證據，不代表實際雲端品質。

hidden native 以真實中文 PreparedController 完成雙軌探索→未選時 ID 空白且不能準備→選第二軌→明確中文宣告→Ready 兩段→清除／釋放 pin；未選不改為 ASR，探索不觸發 Ready 通知。八軌清單逐項核對 ID／五語提示／中文需宣告，改語言保留同來源 catalog，手動 ID 退回提示列、選回提示列清空 ID，來源變更清掉舊清單／ID／語言；失敗重探不保留部分或舊選項。清除及關閉後目錄沒有產品輸出，fixture 可移除。兩列 synthetic password edits 的清空／undo、舊 Session／取消／CPU 飽和／pin／原有單軌與 DPI bounds 都由完整回歸驗證，通知仍是明確 mock，未驗收真實 toast／使用者視覺操作。

正式新 parser／catalog 沒有 unsafe、unwrap／expect、套件或安裝；Win32 新操作僅是同一 UI 執行緒自有 combo 的 scalar index、reset 與同步複製有界 label，附 SAFETY，沒有 pointer payload 或 COM／buffer 逸出。仍使用原 CPU lane／bounded terminal reply／唯一 identity／取消及 reaping，沒有第二個 worker pool、STT／翻譯時間軸、錄製、產品媒體／文字寫入、temp／cache／export。全套只使用自有合成媒體、唯一測試憑證、隱藏視窗、隔離程序與 loopback，未使用正式金鑰、使用者影片或真實雲端。本機無 .git，未建立 commit、初始化或丟棄既有工作。

現在一般 GUI 可探索最多八條既有格式的 tx3g 並明確選一軌，仍需單 AAC-LC、可選 H.264，沒有 edit list／文字 ctts／modifier／其他 handler 或語言。codec bytes 為結構 fixture，不能據此宣稱一般 muxer／真實影片已驗收；未選本文不驗收，metadata 不作版本／alignment 證據。更多格式、實際檔案、URL／Auto、完整語意、播放定位及真實雲端仍待接續；Phase 09 尚未具備條件，符合條件時會提醒。有效金鑰維持結案固定遮蔽介面手動輸入。設計及操作見 [ADR 0062](adr/0062-bounded-mp4-multiple-subtitle-tracks.md)。

## 第三十批驗證

2026-10-16，本機 Windows `10.0.19045.0`，最終 Rust 程式碼實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：34.55 秒，無新增 lint suppression。
- `cargo test --locked --offline --quiet`：1515 項 library、12 項設定、4 項探索、10 項啟動，共 1541 項一般測試通過；既有 15 項 ignored 不變。library 461.04 秒，不當作字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.84 秒。
- `cargo build --locked --offline --release`：Release 通過，51.44 秒。

本批新增 19 項一般測試：8 項純 edit helper、6 項容器、3 項 controller、2 項 hidden native GUI。`cargo test --locked --offline text_edit -- --nocapture` 的 19 項聚焦通過，1.41 秒，後續完整回歸涵蓋新增與全部既有行為，沒有新增 ignored。獨立唯讀審查另確認所有文字軌驗證、A/V 原 gate、時間換算、clear／序號／容量／取消及 pin 邊界，未發現阻擋問題；實跑證據以 root 執行結果為準。

helper 核對 version 0／零 flags／精確 1 或 2 筆、完整 rate=1、media_time=0、可選 leading -1；未知／duplicate／extra box、截斷／尾資料、零長度／開放長度、裁切、變速／dwell／重播與 version 1 拒收。active 長度必須精確涵蓋 mdhd，不可整除也明確 UnsupportedMedia；tkhd 總長與 mvhd 覆蓋須相符。無 edit 的 90 kHz→1000 Hz floor／ceil 舊規則保持。包含 delay 的 24 小時端點、樣本越界／overflow／小於一微秒的零長度、無效 scale 與取消均有測試。

時間相位以 1/3 秒延遲加 2/90000 秒媒體端點核對為 333355 微秒；分別 floor 會早一微秒。6000 份連續小樣本核對每段相接、沒有累積 duration rounding。實際文字容器另以 movie scale 3000／text scale 9000 驗證第一段 start=333、duration=77778，第二段 start=122555、duration=111111 微秒，證明呈現端點只做一次合併取整。

單軌 identity 與 1／500／2500 ms 前置空白均保持兩段原文、原 400 ms clear gap、Session 與序號；兩文字軌可有不同延遲且只抽取選定軌。未選軌壞 edit 仍拒收整份探索／抽取，A/V edit 保持拒收，全部檔案截斷／取消沒有部分結果。原 stts duration 仍依媒體 ticks 完整核對；映射僅用在交給既有 builder 的呈現時間，不更改音訊時鐘或啟動播放定位。

三種中文透過真正 PreparedController 到 Ready，原文 start 從 0／1100000 移到 500000／1600000 微秒，duration 保持 700000／1000000，譯文為「軟體與記憶體／滑鼠與伺服器」；會 panic 的 factory 未被呼叫，沒有 credential lookup。兩個所選服務 × 三種品質以明確 mock 完成同一全文分析／翻譯，Fast 兩次 Draft、Standard／High 四次 Draft／Review，原始延後時間保持。非法 crop 在分析或翻譯前失敗，不改走 ASR 或其他服務；原檔 pin 在 Ready 擋 writer／delete，stop／clear 後釋放，bytes 不變且沒有產品輸出。

hidden native 用真實中文 controller 完成延遲字幕探索→仍須明確中文宣告→Ready 兩段→清除／關閉；通知只是一份明確 mock，並未測真實 Windows toast。來源文字另以兩服務 mock backend 核對所選 endpoint／key、只傳文字不傳媒體音訊、探索不啟動準備／通知，以及雙列清空／undo／取消回收。沒有使用正式金鑰、使用者媒體或真實雲端，也不把合成 AAC／H.264 結構 bytes 當成 playable codec／muxer 或視覺操作相容性證據。

正式新增程式沒有 unsafe、unwrap／expect、套件或安裝，不新增 pool／task／queue／鎖／COM、媒體或字幕保存／temp／cache／export。共用 CPU lane、全軌 byte range、既有 sample／wire／text／cue 上限、唯一 Session、取消與同一 pin 保持；完整回歸沿用自有合成媒體、唯一測試憑證、隱藏視窗、隔離程序及 loopback。本機無 .git，沒有建立 commit、初始化或丟棄既有工作。

目前支援完整文字軌的有限 v0 edit／前置空白，不能據此宣稱一般 edit list 或實際影片已驗收。裁切／version 1／A/V edit／文字 ctts／modifier、其他格式、URL／Auto、全文語意、播放定位與雲端品質仍待接續；Phase 09 尚未具備條件，符合時會提醒。有效金鑰維持結案固定遮蔽介面手動輸入。設計與格式依據見 [ADR 0063](adr/0063-full-track-tx3g-edit-delay.md)。

## 第三十一批驗證

2026-10-16，本機 Windows `10.0.19045.0`，最終 Rust 程式碼實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：16.85 秒，無新增 lint suppression。
- `cargo test --locked --offline --quiet`：1527 項 library、12 項設定、4 項探索、10 項啟動，共 1553 項一般測試通過；既有 15 項 ignored 不變。library 445.00 秒，不當作字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.84 秒。
- `cargo build --locked --offline --release`：Release 通過，55.04 秒。

本批新增 12 項一般測試：2 項 edit helper、6 項容器、3 項 controller、1 項 hidden native GUI。`cargo test --locked --offline text_ -- --nocapture` 的 176 項聚焦測試通過，3.79 秒；完整回歸涵蓋最終新增及所有既有行為，沒有新增 ignored。

裁切採半開區間與精確有理數交集：media 200–1600 ms、前置空白 500 ms 時，兩段呈現時間為 500000／500000、1400000／500000 微秒，完整文字及中間 400 ms clear gap 保留。精確落在樣本邊界時不重複 cue；全部位於外側則移除，只剩 clear gap 明確 InvalidSubtitle。movie scale 7／text scale 9000、media_start 100、前置與 active 各 1 movie tick 的案例核對 start／duration 為 142857／66666 與 253968／31746 微秒，沒有分開取整或累積 duration 誤差。負 media_time、延伸超出來源及既有未支援 edit 型態仍拒收。

所有選定軌的原始樣本先完整通過 Tx3gTrack；裁切前後被排除的本文故意損壞仍拒收，不讓裁切繞過既有 sample／wire／text／cue 容量。只配置最多 8192 筆數值對照，原地過濾唯一文字文件；沒有第二份全文。多文字軌各自裁切不混入未選本文，未選軌無效範圍仍拒收整份。全檔截斷及讀取被裁掉樣本時的取消不返回部分結果，原始媒體時間與無 edit 舊規則保持。

真正 PreparedController 驗證三種中文本機到 Ready、不建立服務或讀取憑證；兩個所選服務 × 三種品質的明確 mock 只收到可見的最後一段原文與 250000／500000 微秒時間，一次全文分析，Fast 一次 Draft、Standard／High 各一次 Draft／Review。空窗在分析／翻譯前失敗，不改用 ASR。Ready 保持唯讀 pin，stop 後釋放，原 bytes 不變。hidden native 使用真正中文 controller 完成選軌／Ready／清除，再以空窗案例確認沒有第二次 Ready 通知；兩列 synthetic password edits 清除，fixture 可移除且沒有產品輸出。通知仍是明確 mock，不宣稱真實 toast 或使用者視覺操作已驗收。

正式新增程式沒有 unsafe、unwrap／expect、相依套件或安裝，沒有新增 pool／task／queue／鎖／COM、錄製、產品文字或媒體寫入、temp／cache／export。既有 CPU lane、全部軌 byte ranges、唯一 Session、取消及同一來源 pin 保持；完整回歸只使用自有合成資料、既有唯一測試憑證、隱藏視窗、隔離程序與 loopback，沒有使用正式金鑰、使用者影片或真實雲端。本機仍無 .git，沒有初始化或建立 commit。

目前僅支援文字 v0 單一正常速度區間及可選前置空白；這是容器時間解讀，不是剪輯或播放功能。version 1、多 active edits、A/V edit、文字 ctts／modifier、其他格式、一般 muxer／真實影片、URL／Auto、完整語意、播放定位與真實雲端仍待接續。Phase 09 尚未具備啟動條件，符合時會提醒；有效金鑰維持結案固定遮蔽介面手動輸入。設計與主要格式來源見 [ADR 0064](adr/0064-single-interval-tx3g-text-crop.md)。

## 第三十二批驗證

2026-10-16，本機 Windows `10.0.19045.0`，最終 Rust 程式碼實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：16.95 秒，沒有新增 lint suppression。
- `cargo test --locked --offline --quiet`：1539 項 library、12 項設定、4 項探索、10 項啟動，共 1565 項一般測試通過；既有 15 項 ignored 不變。library 447.90 秒，不當作字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.84 秒。
- `cargo build --locked --offline --release`：Release 通過，42.62 秒。

本批新增 12 項一般測試：3 項 edit helper、5 項容器、3 項 controller、1 項 hidden native GUI。`cargo test --locked --offline mp4_text_v1 -- --nocapture` 的 12 項聚焦通過，0.81 秒；後續完整回歸涵蓋最終新增及原有全部案例，沒有新增 ignored。原 v0 controller／native 測試保留，v1 共用相同測試流程與各自的真實 parser，沒有複製正式準備實作。

v0／v1 成對核對完整字幕、前置 0／1／250／2500 ms、頭尾裁切、精確邊界、1 ms 短區間及只含 clear gap；原文、identity、區域 content ID、時間與段數一致，空窗仍 InvalidSubtitle。分數 movie／media phase 使用原有一次有理數取整。正式容器以 mdhd scale=1000000、duration=2201000400、active media_time=2200000400 驗證高於 i32::MAX 的合法正起點，最後一段呈現 start=250000／duration=500000 微秒；相同 bit pattern 放入 v0 則為負值並拒收。這不代表允許超出 v0 標頭可表示範圍。

純 helper 核對超過 u32 的 duration／delay／media_start 不截斷，checked 總長溢位與超過 24 小時拒收。容器另以高 32 bits 非零但低位看似有效的 media_time／duration／empty delay 核對整份拒收；signed 最小值、-2、錯誤 -1 編碼、未知版本／flags／count、欄位寬度混用及 v1 mdhd／tkhd／mvhd 仍拒收。v0／v1 混合多軌各自映射，未選軌壞 edit 仍拒收整份；A/V edit gate 未放寬。全部檔案截斷、被裁掉本文損壞及讀取被裁掉樣本時取消，都不產生部分文件。

真正 PreparedController 驗證三種中文本機 Ready、不建服務或讀取憑證；兩所選服務 × 三品質的明確 mock 僅分析／翻譯可見原文與原時間，空窗在服務要求前失敗。Ready 持有原檔 pin、stop 釋放且 bytes 不變。hidden native 真正中文 controller 完成探索／明確語言／Ready／清除，再驗證空窗失敗沒有第二次 Ready 通知，雙列 synthetic password edits 清空、fixture 可移除且無產品輸出。通知是明確 mock，未驗收真實 toast、使用者視覺操作或雲端品質。

正式修改僅新增固定寬度數值解碼與驗證，沒有 unsafe、unwrap／expect、新套件或安裝，沒有新增 worker／pool／queue／鎖／COM、錄製、產品媒體或文字寫入、temp／cache／export。CPU lane、全部軌範圍、唯一文字文件、容量、Session／取消與 pin 沿用。完整回歸只使用自有合成媒體、既有唯一測試憑證、隱藏視窗、隔離程序及 loopback；未使用正式金鑰、使用者影片或真實雲端。本機仍無 .git，沒有初始化、建立 commit 或丟棄既有工作。

目前只擴充文字 elst v1；mdhd／tkhd／mvhd 仍限 v0，多 active edits、A/V edit、文字 ctts／modifier、其他格式／一般 muxer／真實影片、URL／Auto、完整語意及播放定位仍待接續。Phase 09 尚未具備啟動條件，符合時會提醒；有效金鑰維持結案固定遮蔽介面手動輸入。設計與主要來源見 [ADR 0065](adr/0065-version-one-tx3g-edit-list.md)。

## 第三十三批驗證

2026-10-16，本機 Windows `10.0.19045.0`，最終 Rust 程式碼實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：15.04 秒，無新增 lint suppression。
- `cargo test --locked --offline --quiet`：1547 項 library、12 項設定、4 項探索、10 項啟動，共 1573 項一般測試通過；既有 15 項 ignored 不變。library 458.46 秒，不當作字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.87 秒。
- `cargo build --locked --offline --release`：Release 通過，44.54 秒。

新增 8 項一般測試：4 項摘要核心／翻譯、2 項分析 HTTP、1 項翻譯 HTTP、1 項 controller。`cargo test --locked --offline prepared_context -- --nocapture` 聚焦共 12 項通過，0.42 秒；後續補強 UTF-8／最大 escaped request 與原生揭露斷言，最終以上完整回歸驗證全部修改。沒有新增 ignored 或假 cloud success。

首次完整回歸發現 ASR 測試 mock 的新斷言誤把明確的 `run_with_recognizer` 手動 library 入口也當作自動分析，對空摘要 unwrap 失敗，造成共用測試鎖 PoisonError 連鎖。修正測試分別核對：已做自動分析時必須帶摘要，手動入口必須沒有憑空摘要；正式自動路徑仍要求分析完整成功，沒有放寬失敗判定或吞掉 poisoned lock。原先第一個 `synthesized_mp3_reaches_pinned_ready_with_native_endpoints_for_each_provider_and_rate` 單獨重跑通過，16.31 秒，之後重新執行格式／Clippy 與整份回歸並全部通過。

129 段合成原文分成三批，逐批核對前一份摘要已轉為臺灣用語，三種品質都在所有分析完成後才翻譯。所有 Draft／Review 的 context 指標相同，最後摘要為「軟體主題 129」，涵蓋段數 129；Fast 129 次 Draft，Standard／High 各 258 次 Draft／Review，輸出順序及原時間沿用。分析套件及要求退出後 Weak 無法升級，確認沒有保留摘要 owner。既有最大四 MiB／8192 段案例仍遍歷 128 批，最終 context 的完整段數相符。

後批空摘要不進入翻譯；Session／generation、語言、品質或整份段數不符在供應商呼叫前拒收。4096 ASCII bytes 接受、4097 bytes 或 1366 個三-byte 字元拒收，空白、NUL 與非法控制字元拒收，prior summary 也有相同邊界。兩服務的真正 TCP loopback 核對 prior_summary 與系統指令隔離、required schema、分片 SSE、摘要缺漏／重複欄位與 byte 上限；翻譯 body 核對 document_summary 是資料且不進系統指令，舊身分／語言／品質或 cue 超出範圍拒收。最大原文／詞彙加上 4096-byte escaped prior summary 仍在原有 512 KiB request 上限內，Live／Prepared 的原 stream 與錯誤測試維持。

真正 controller 以兩所選服務的明確 mock 驗證無效摘要回報 Translation(InvalidData)、沒有 Ready／翻譯或 ASR fallback，原檔 pin 釋放且 bytes 不變。既有五語、三品質、內嵌裁切／多軌與 ASR controller mock 都核對自動翻譯確實收到摘要；原中文不建服務、手動入口不自動分析的邊界保持。hidden native 的來源字幕／ASR 揭露包含「全文脈絡摘要」，沿用目的地、雙列遮蔽金鑰／清空 undo、DPI／client bounds、取消及關閉回收；沒有宣稱真實 toast 或使用者視覺操作已驗收。

新增正式模組與接線沒有 unsafe、unwrap／expect、新套件或安裝；沒有新增 AI 要求、worker／pool／queue／鎖、第二個時間軸、內容保存、temp／cache／export 或 telemetry。摘要本地化使用原 CPU lane，網路等待釋放 CPU 許可，HTTP 共用原 client／並行與速率限制。完整回歸只使用自有合成資料、既有唯一測試憑證、隱藏視窗、隔離程序與 loopback；沒有正式金鑰、使用者媒體或真實雲端。本機仍無 .git，未初始化、建立 commit 或丟棄既有工作。

有界全文摘要已接通，仍是可能遺漏資訊或出錯的模型提示；主題、人物指涉、跨句語意、真實雲端 schema 接受程度及費用尚待實際驗收。URL 媒體讀取、限五語 Auto、更多真實媒體相容性與播放／實際音訊定位仍待接續；Phase 09 尚未具備啟動條件，符合時會提醒。有效金鑰維持結案固定遮蔽介面手動輸入。設計與主要來源見 [ADR 0066](adr/0066-bounded-prepared-document-context.md)。

## 第三十四批驗證

2026-10-16，本機 Windows `10.0.19045.0`，最終 Rust 程式碼實際通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：13.29 秒，沒有新增 lint suppression。
- `cargo test --locked --offline --quiet`：1563 項 library、12 項設定、4 項探索、10 項啟動，共 1589 項一般測試通過；既有 15 項 ignored 不變。library 453.17 秒，不當作字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.92 秒。
- `cargo build --locked --offline --release`：Release 通過，43.64 秒。

本批新增 16 項一般測試。真正 TCP loopback 驗證一-byte probe、精確 offset／range／If-Match、同一持續連線與 64 KiB 最大 block；必要欄位缺漏／重複、弱 ETag、非法 range、未知或過大 total、multipart、壓縮、chunked、錯誤長度及 body 截斷都拒收。版本改變或 412／416 使舊 range 不可再存取，新 Session 可重新驗證並讀取新版本。重新導向不接觸 Location，200 不退回整份下載；HTTPS 對自有非 TLS 端點只發 handshake，失敗不退回 HTTP，typed error 不包含合成私有 query。這不代表真實網站或憑證相容性已驗收。

取消、未 poll／執行中 Future Drop 與 stalled body deadline 都核對來源退休、連線關閉及 buffer 配額回收；owner Drop 才釋放 owner 容量。四份 range 被慢消費端保留時，第五份在 I/O 前明確 Overloaded；來源退休不釋放保留中的 payload 配額，只有 Drop 各 block 才回復容量，避免用切換來源繞過 256 KiB 上限。舊 generation、錯誤 sequence、超出檔尾與 checked offset 溢位均不發要求。沒有目前 Tokio runtime 時回 RuntimeUnavailable。鎖定的 reqwest 0.12.28 明確設定 retry::never，停用預設協定 NACK 重試；最終完整回歸涵蓋這項設定，沒有只以應用程式未寫 retry loop 推定底層不重試。

新增正式模組沒有 unsafe、recoverable unwrap／expect、新相依或安裝，沒有新 CPU pool／應用程式執行緒、無界佇列、媒體或本文落盤、temp／cache／export、telemetry 或第二份時間軸。只使用既有 reqwest／Tokio，以兩個 owner 及四份在途／保留 buffers 限制資源，網路等待不占 Prepared CPU lane；URL／query、ETag、原始網路錯誤與 bytes 不進 Debug 或日誌。新增網路測試只使用自有 loopback／合成 bytes，完整回歸沿用既有合成媒體、唯一測試憑證、隱藏視窗及隔離程序；沒有正式金鑰、使用者媒體或真實雲端。本機無 .git，未初始化、建立 commit 或丟棄既有工作。

本批交付獨立的實際 HTTP(S) range library，尚未接入 Prepared producer／decoder、Ready 或一般 URL GUI；正式準備路徑仍拒收 URL，不以 library 測試宣稱遠端影片已可準備／播放。後續仍須完成取消與來源一致性橋接、格式探測、解碼／EOF、URL 目的地揭露及真實媒體驗收；五語 Auto、語意與雲端品質、播放／實際音訊定位仍待接續。Phase 09 尚未具備啟動條件，符合時會提醒；有效金鑰維持結案固定遮蔽介面手動輸入。設計與格式依據見 [ADR 0067](adr/0067-bounded-session-http-media-ranges.md)。

## 第三十五批驗證

2026-10-16，本機實際確認 Windows `10.0.19045.0`，最終 Rust 程式碼通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：12.53 秒，沒有新增 lint suppression。
- `cargo test --locked --offline --quiet`：1576 項 library、12 項設定、4 項探索、10 項啟動，共 1602 項一般測試通過；既有 15 項 ignored 不變。library 460.14 秒，不當作字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，1.19 秒。
- `cargo build --locked --offline --release`：Release 通過，46.47 秒。

本批新增 13 項一般測試，`cargo test --locked --offline remote_probe -- --nocapture` 聚焦通過，0.13 秒，最終完整回歸包含全部新案例。第一輪編譯修正測試用 MP4 fixture 的可見範圍，沒有複製正式 parser 或放寬格式判定。

實際 TCP loopback 以錯誤副檔名／沒有格式 MIME 的 URL 執行 WAVE、兩相鄰 MPEG-1 MP3／AAC-LC ADTS 封包及既有 MP4 gate；成功回傳原 identity、長度、格式與同一 owner，next_sequence 持續前進，後續 If-Match 讀取仍驗證原始 bytes。未知／播放清單、截斷 MP3／MP4、音訊參數改變均拒收。合成結構不視為可播放 codec，WAVE 的十二-byte 標頭也不是完整 fmt／data 驗收；沒有建立 decoder、PCM、ASR 或 Ready。

150000-byte adapter 案例核對尾端 seek／EOF／空 read 後回頭讀取：網路範圍恰為 probe 0／1、尾端 149997／3，以及 0／65536、65536／65536、131072／18928；每個傳輸區塊仍受 64 KiB 上限，adapter 沒有 read-ahead cache。非法絕對／負值／溢位 seek 在第二份要求前失敗且後續不能恢復。SourceChanged、412、429 及 body 截斷保留真正 typed error，不因底層同時取消而一律變成 Cancelled。

網路 body 暫停時確實釋放 CPU；另一工作取得該許可後，恢復傳輸的 probe 明確 Overloaded，不等待 CPU。網路取消會關閉連線並等待 worker 真正退出，owner／buffers 回復。以明確 barrier 驗證已啟動 CPU 工作的 Future Drop：舊 range 存取失效，owner／CPU 仍被持有直到工作退出，保留的 range 仍占 buffer 配額直到 Drop；正常取消也不提前回覆已清理。明確注入的 worker panic 轉 WorkerFailed 並回收全部配額，不是真正 parser panic。缺少目前 runtime 在新排程／讀取前回 RuntimeUnavailable，公開入口以原 CPU lane 在 current-thread runtime 完成前置檢查。

首次完整回歸只有既有 `transient_http_failure_retries_boundedly_but_credentials_do_not_retry` 失敗：測試 server 對已由 client 關閉的錯誤回應繼續分片寫 body，Windows 10053 使 server panic，下一次重試因而沒有伺服器接收。單獨重跑十二次通過，仍保留失敗並依完整輸出修正。此重試次數 fixture 改用合法空錯誤本文，成功 SSE 仍分片，server JoinHandle 必須正常成功，不再忽略 panic；修正後該例單獨通過 0.26 秒，再執行格式／Clippy 與最終整份回歸並通過。沒有改動正式 AI client、重試次數、速率限制或憑證錯誤分類，也沒有新增 ignored。

新增正式模組沒有 unsafe、recoverable unwrap／expect、新相依或安裝、COM、永久／temp 媒體或文字寫入、cache／export、telemetry 或第二份時間軸。只在既有 Tokio blocking pool 啟動受遠端 owner 上限限制的工作；CPU 沿用 available_parallelism 的共用 lane，不持 CPU 等網路，沒有每 block 執行緒或無界命令佇列。MP4 暫時 metadata 沿用一 MiB moov 等上限，不留在結果中；它不計入四份 range payload 的 256 KiB 配額。完整回歸只使用自有合成資料、既有唯一測試憑證、隱藏視窗、隔離程序與 loopback，沒有正式金鑰、使用者媒體或真實雲端。本機無 .git，沒有初始化、建立 commit 或丟棄既有工作。

本批完成遠端內容格式前置檢查，尚待原生解碼／EOF、ASR／RAM Ready 與 URL GUI 目的地揭露；一般準備介面仍拒收 URL。五語 Auto、一般真實媒體／語意／雲端品質與播放定位仍待接續。Phase 09 尚未具備條件，符合時會提醒；有效金鑰仍留至結案固定遮蔽介面由使用者手動輸入。設計見 [ADR 0068](adr/0068-bounded-remote-content-preflight.md)。

## 第三十六批驗證

2026-10-16，本機實際確認 Windows `10.0.19045.0`，最終 Rust 程式碼通過：

- `cargo fmt --check`。
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：35.78 秒，沒有新增 lint suppression。
- `cargo test --locked --offline --quiet`：1593 項 library、12 項設定、4 項探索、10 項啟動，共 1619 項一般測試通過；既有 15 項 ignored 不變。library 457.51 秒，不當作字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，1.18 秒。
- `cargo build --locked --offline --release`：Release 通過，45.14 秒。

本批新增17項一般測試。首輪14項聚焦通過0.50秒，補上容量預檢、公開入口與 Windows 原生／DSP 比對後，`cargo test --locked --offline remote_wave -- --nocapture` 的17項通過0.77秒。Clippy 要求固定兩-byte 分組改用 as_chunks，依建議修改且核對 remainder，沒有新增 allow；最終完整回歸涵蓋該修正及所有新舊案例。

實際 TCP loopback 以50003 frames、8／16／44.1／48／192 kHz、單／雙聲道核對每個 PCM16 bit pattern、完整框格、identity／sequence、累積取樣 timestamp、尾段與重複 EOF；Encoded HTTP 與 PCM 序號不混用。完整 RIFF 在 PCM 前驗證，包含奇數 JUNK 的 padding、fmt／data 順序／重複／缺漏、未知 LIST、float／extensible／其他位元深度／多聲道、byte rate／alignment、殘留或截斷內容。128 chunks 與一 MiB JUNK 接受，超過一筆／一 byte 拒收，JUNK 本文沒有被讀取。

只提供44-byte實體標頭的 sparse loopback 案例，以虛擬 Content-Range total 核對二十四小時合法框格數、再多一框格拒收，以及四 GiB 媒體因剩餘 range sequence 不足而在第一塊 PCM 前拒收；所有網路要求均落於標頭。沒有配置、傳輸或宣稱驗收二十四小時／GiB 音訊。已解析的 probe／標頭要求也計入65536份預算，避免到大檔案末尾才失敗。

來源版本改變、412／416、429與截斷 body 保留真正 typed error，不回傳部分 PCM 或將 SourceChanged 蓋成 Cancelled；舊 block 的新借用同時失效。四份 PCM 被保留時，第五次拉取在 I/O 前 Overloaded，decoder Drop 釋放 HTTP owner 但 PCM 許可保持，重開新 generation 也不能繞過，只有 Drop 舊 blocks 才回復。不同 generation 的取消彼此隔離，DecodedAudioGate 拒收另一來源的 block。

以明確網路 body barrier 核對等待時 CPU 配額可由另一工作取得；傳輸恢復而 CPU 已滿時立即 Overloaded，不排隊。網路取消／Future Drop 會關閉連線並回收 wire／PCM 配額；在部分轉換的 yield 點捨棄 Future，也回收 CPU／wire／PCM 且不交付半塊。未 poll 的 open／read Drop 不發新 I/O、撤回舊 block；沒有目前 runtime 時回 RuntimeUnavailable，不連線或 panic。EOF 仍先檢查取消。

公開入口另以 Windows 實際 MF decoder 比對同一份44100 Hz／雙聲道／50003 frames 合成 PCM WAVE 的所有取樣值，再把遠端 block 借給既有 PreparedAudioProcessor，完成16000 Hz mono、18141 output frames、零缺口與真正 EOF flush。原生比對只使用 tempfile 自有合成檔，檔案 bytes 不變且沒有額外輸出，退出由 RAII 清理；正式遠端 decoder 不寫檔。這是 PCM／DSP 證據，不是遠端 ASR、RAM Ready、一般影片或真實雲端品質驗收。

新增正式程式沒有 unsafe、recoverable unwrap／expect、新套件／安裝、COM、執行緒／每 block worker、無界佇列、媒體或本文落盤、temp／cache／export、telemetry 或第二份時間軸。本機 MF／壓縮路徑未修改；CPU 沿用 available_parallelism 的原 lane，HTTP owner／client／ETag／total／取消仍是同一份。PCM payload 最多256 KiB，另有原有 HTTP payload 配額及 transport／metadata 開銷，並非宣稱整個程序只占256 KiB。完整回歸只用自有合成資料、既有唯一測試憑證、隱藏視窗、隔離程序與 loopback；沒有正式金鑰、使用者媒體或真實雲端。本機無 .git，未初始化、建立 commit 或丟棄既有工作。

本批完成有限 PCM16 WAVE 的實際遠端解碼 library；一般準備介面仍拒收 URL，遠端分段／ASR／RAM Ready、壓縮格式的 native HTTP／COM 橋接、URL GUI 目的地揭露及真實媒體仍待接續。五語 Auto、語意／雲端品質與播放定位也未驗收；Phase 09 尚未具備條件，符合時會提醒。金鑰維持結案固定遮蔽介面手動輸入。決策與 Microsoft 格式來源見 [ADR 0069](adr/0069-bounded-http-pcm16-wave-decoder.md)。

## 第三十七批驗證

2026-10-16，Windows 10.0.19045.0，實際執行通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：34.81 秒。
- `cargo test --locked --offline --quiet`：1606 項 library、12 項設定、4 項探索、10 項啟動，共1632項一般測試通過，零失敗；既有15項 ignored不變。library 549.05秒，不作字幕延遲 SLA。
- `cargo build --locked --offline`：Debug 通過，0.97 秒。
- `cargo build --release --locked --offline`：Release 通過，1 分 21 秒。

首輪 `cargo test --locked --offline remote_prepared -- --nocapture` 的11項針對性測試通過，75.81秒。補上公開入口與 EOF 邊界後，本批共新增13項一般測試；多取樣率比對採十一秒加十三個 source frames，另以五十二秒案例驗證跨辨識批次及前文，測試耗時不作字幕延遲或雲端品質 SLA。編譯初輪補上工作結果錯誤型別；Clippy 要求測試 MutexGuard 以區塊界定生命週期，已修正，沒有新增 allow 或停用警告。

實際 loopback HTTP 搭配自有合成 PCM，核對8／16／44.1 kHz輸入、單／雙聲道到16／24 kHz的取樣值、半秒左右脈絡、累積時鐘、十秒正文及真正 EOF 尾段。十秒整段、多一個取樣點、單一取樣點與不足一個輸出 frame 個別核對，不丟尾段、不補靜音或以空文件成功。既有重採樣器是 PCM oracle，辨識器全為明確測試 mock；沒有把合成原文當作真正語音辨識。

五十二秒來源在服務 barrier 等待時沒有後續 HTTP 讀取，CPU／wire／原始 PCM 配額均釋放；保留的分段仍受原 ASR／DSP 記憶體上限約束。後批取得已完成原文脈絡，並行完成順序不同仍依媒體正文排序。最後一份服務等待、成功結果保留及結果 Drop 分別核對 owner 許可與取消後原文存取。

來源在已完成辨識批次後變更仍回 SourceChanged，不因 scope 退休而蓋成 Cancelled，不交付部分文件。憑證失敗、錯 generation、NoSpeech、能力不足／panic、預先取消及 DSP 過載都無成功或隱式 fallback。網路／服務等待的取消及 future Drop 停止要求與預讀；限制 Tokio blocking pool 為一個執行緒並以 barrier 占用它，核對排隊 DSP 在 Drop 後取消、最終回收 CPU／PCM 許可。公開入口另使用原共用 CPU lane，保持原 EOF owner。

本批正式程式無新增 unsafe、recoverable unwrap／expect、套件／安裝、COM、獨立 OS thread、無界音訊佇列、產品媒體／本文寫入、temp／cache／export、telemetry 或第二份時間軸。CPU 沿用 available_parallelism 的既有配額，PCM 與原 HTTP owner／client／ETag／sequence／取消保持同一來源。正式金鑰、使用者音訊與真正雲端服務沒有測試；金鑰維持結案固定介面手動輸入。本機沒有 .git，未建立 commit 或丟棄其他工作。

本批完成遠端 PCM16 WAVE 到原文辨識的程式庫接線；遠端全文摘要／詞彙／翻譯／RAM Ready、URL GUI、壓縮格式、五語 Auto 及真實媒體／雲端／播放驗收仍待接續。Phase 09 尚未具備啟動條件，符合時會提醒。見 [ADR 0070](adr/0070-demand-driven-remote-wave-prepared-recognition.md)。

## 第三十八批驗證

2026-10-16，Windows 10.0.19045.0，實際執行通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：最後一次14.76秒。
- `cargo test --locked --offline --quiet`：1619項 library、12項設定、4項探索、10項啟動，共1645項一般測試通過，零失敗；既有15項 ignored不變。library 459.28秒，不作字幕延遲 SLA。
- `cargo test --locked --offline remote_ready -- --nocapture`：新增13項針對性測試通過，2.25秒。
- `cargo build --locked --offline`：Debug 通過，1.01秒。
- `cargo build --release --locked --offline`：Release 通過，45.35秒。

真實 loopback HTTP 與自有44100 Hz雙聲道合成 PCM 經同一遠端來源到16／24 kHz辨識、全文摘要／詞彙、Fast／Standard／High、臺灣本地化及 PreparedSession Ready。明確 mock 的原文保留真正0至250000微秒範圍及 Recognized provenance，沒有偽裝既有字幕；Fast一份 Draft、Standard／High各含 Draft／Review，共用同一不可變摘要。詞彙及輸出包含軟體／伺服器等本地化驗證。

以實際全程序兩個 HTTP owner 配額核對來源保留：分析與翻譯 barrier 等待、成功 completion 尚未 apply、Ready 各階段均仍占一個 owner；另一份 spare probe 以外不發網路要求，第三份 owner 明確 Overloaded，Stop 後可重新取得配額。服務等待時既有 CPU lane 完整可用；保留來源不占原始 PCM 或建立媒體暫存。

憑證、分析拒絕、翻譯限流、空摘要、錯 generation、來源在已完成辨識批次後變更、HTTP 429／401／302／200不支援range及已探測MP3 codec等錯誤均不發布部分文件或 fallback。SourceChanged 等 typed cause 經 Session 保留，不能因連帶子 scope 取消蓋為 Cancelled；分析失敗沒有 Draft，ASR失敗沒有分析或翻譯。

未 poll Drop／預先取消無 HTTP 或服務工作；舊詞彙／能力不足在網路前拒收。臺灣繁中、其他繁中、簡中及來源字幕的選定組合先明確拒收，同順位先回 AmbiguousSubtitle，不讀不存在的 sidecar、不發辨識要求；本機輸入也不走遠端入口。網路等待的 Stop，以及 ASR、全文分析與翻譯等待的 Stop／Drop 都取消 scope，沒有活動要求或後續讀取；已完成舊文件不能覆蓋新 Session 的 Ready。HTTP owner／CPU 飽和均明確失敗，原配額回收。

編譯初輪修正測試分析器的取消參數，使其符合現有 TranslationCancellation 契約。既有公開 HTTP 連線測試改與其他 production-entry fixtures 共用測試鎖，避免實際兩個 owner 配額在不同測試間競爭；只改測試序列化，不放寬正式限制。沒有停用警告或以調整正式配額通過測試。

正式程式無新增 unsafe、recoverable unwrap／expect、相依套件／安裝、worker pool、無界音訊佇列、產品內容落盤、temp／cache／export、遙測或第二份時間軸；沒有存取正式金鑰、使用者音訊或真實雲端。兩個服務的正式協定維持既有實作，這批只用明確服務 mock。金鑰維持結案固定遮蔽介面手動輸入。本機無 .git，未初始化或建立 commit。

本批完成明確程式庫入口的遠端 WAVE 全文處理及本次 RAM Ready。一般 URL GUI、URL搭配可靠字幕、其他遠端格式、五語Auto及真實媒體／語意／雲端與播放驗收仍待接續；Phase 09 尚未具備條件，達標時會提醒。見 [ADR 0071](adr/0071-remote-wave-session-analysis-translation-ready.md)。

## 第三十九批驗證

2026-10-17，Windows 10.0.19045.0，實際執行通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：最後一次11.40秒。
- `cargo test --locked --offline --quiet`：1637項 library、12項設定、4項探索、10項啟動，共1663項一般測試通過，零失敗；既有15項 ignored不變。library 467.98秒，不作字幕延遲 SLA。
- `cargo test --locked --offline remote_ -- --nocapture`：102項遠端相關測試通過，32.77秒。
- `cargo test --locked --offline win32::prepared_dialog::tests -- --nocapture`：51項原生準備視窗測試通過，22.64秒，包含新遠端揭露及擴充的窄視窗／雙倍字型量測。
- `cargo test --locked --offline ui::prepared::tests --quiet`：16項草稿／狀態驗證通過，0.00秒。
- `cargo build --locked --offline`：Debug 通過，0.82秒。
- `cargo build --release --locked --offline`：Release 通過，56.16秒。

新增4項草稿邊界、9項 controller、6項隱藏原生案例，共19項，取代1項舊 URL 不可用測試，淨增18項。原生字型量測既有案例擴大為本機／網址、兩個服務、原字型／26px字型、320／480／640視窗寬度，不以此宣稱真人多螢幕或雲端驗收。UI草稿首輪發現舊內嵌URL案例仍期待InvalidInput；新入口對此組合回明確UnsupportedUrl，已更新預期且維持拒收與零I/O，沒有刪除容量／路徑防護或停用警告。

真實 loopback HTTP、自有 PCM WAVE與明確 service factory／辨識／分析／翻譯 mock，驗證一般 controller 的兩個所選服務、三品質、唯一金鑰取用、16 kHz與Ready，原文仍保留真正媒體時間及Recognized身分。URL／query不加入辨識／分析／翻譯要求；無額外預讀、內容落盤或播放。無效字幕組合、同順位歧義、Auto、缺服務與能力不足於對應preflight拒收；URL加字幕不建Session／worker／factory或讀安全憑證，保留手動選擇。

HTTP 429保留MediaError::Remote；網路body、辨識與翻譯等待的停止會取消並等待原worker清理，不出現Ready。已完成但未取用的completion在停止後不能套用，重新啟動取得新Session，晚回辨識不會串入新工作。維持單一worker、runtime、bounded completion與原來源／CPU配額，沒有加入另一份pipeline或時間軸。

隱藏原生控制項驗證五語與品質、明確scheme、固定兩列ES_PASSWORD、所選金鑰交接及兩欄／undo清空；過長未選金鑰不被取用。按準備前有媒體連線與所選AI端點／短音訊／原文／摘要／詞彙／翻譯前後文揭露，目的地與狀態不回顯完整私有URL／query。遠端字幕或內嵌模式（含空ID）在讀密碼前拒收；探索按鈕停用、直接命令不啟動掃描；來源修改退休catalog並保留手動字幕。Busy／Ready鎖定、停止／清除、通知一次、URL與undo清理維持。

新正式程式未新增unsafe區塊、recoverable unwrap／expect、相依套件／安裝、pool、無界音訊佇列、媒體／本文落盤、temp／cache／export或telemetry。原生操作沿用既有同執行緒安全邊界；新增測試scalar查詢附SAFETY理由。獨立檢查未發現需修正的來源／金鑰／URL隱私或取消並行問題。沒有讀取正式金鑰、使用者媒體或真實雲端；正式金鑰仍維持結案固定介面手動輸入。本機無.git，未建立commit或丟棄既有工作。

一般URL準備已接線，但只限原有PCM16 WAVE子集合；URL搭配可靠字幕、其他遠端編碼、五語Auto與真實媒體／語意／雲端／播放仍待接續，Phase09尚未具備條件，達標時會提醒。見 [ADR 0072](adr/0072-explicit-http-wave-prepared-ui.md)。

## 第四十批驗證

2026-10-18，Windows 10.0.19045.0，實際執行通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：最後一次13.94秒。
- `cargo test --locked --offline --quiet`：最終單獨重跑1666項library、12項設定、4項探索、10項啟動，共1692項一般測試通過，零失敗；既有15項ignored不變。library 508.72秒，不作字幕延遲SLA。
- `cargo test --locked --offline remote_ -- --nocapture`：131項遠端相關測試通過，35.86秒。
- `cargo test --locked --offline win32::prepared_dialog::tests --quiet`：55項原生準備視窗測試通過，28.16秒。
- `cargo build --locked --offline`：Debug通過，最後一次1.04秒。
- `cargo build --release --locked --offline`：Release通過，1分26秒。

本批新增14項Session、9項controller、2項草稿及4項原生測試，共29項；既有無字幕ASR、內嵌拒收、來源編輯與Ready鎖定案例保留或擴充。兩種字幕格式、三類中文、五語與三品質完整處理均用合成WAVE、真正loopback HTTP及明確AI mock驗證，不以測試文字當真正語音辨識。來源讀取逐一核對只落在WAVE標頭範圍，不拉取任何PCM payload；來源／字幕原文與時間、Existing provenance及臺灣用語保留，沒有製造版本配對或播放定位證據。

最高順位只讀所選外掛；不存在的低順位字幕不影響結果。同順位歧義、遠端任一內嵌選取（包含未勝出軌）、缺服務／translator、舊glossary及錯入口在對應preflight拒收。缺檔、壞格式及超量選定字幕於媒體連線前失敗，不做ASR。中文不建factory、讀金鑰或檢查辨識能力；來源字幕只建立所選Translation服務及一次所選金鑰，音訊Auto不影響字幕聲明；錯factory不能換成Recognition。HTTP 429／401／302、來源變更、錯格式及服務錯誤保持typed cause，不交付部分文件或fallback。

網路body、全文分析、翻譯等待的Stop／Future Drop／預先取消均清理scope與持有者。CPU或兩個HTTP owner飽和明確拒收，沒有排無界工作；同一來源跨服務等待、完成但未apply及Ready持續占配額，停止後可重新取得。已完成舊completion與非合作晚回結果不能跨新Session發布。服務等待不持CPU、不做後續媒體預讀。

隱藏原生測試核對中文不需服務／音訊語言／金鑰，兩個所選AI端點只傳文字用途，五語、SRT／WebVTT、遮蔽欄選定金鑰交接及清空兩欄／undo；未選欄超長不被取用。遠端內嵌（包含空ID）、非法本機字幕路徑、缺語言／格式／必要服務在讀密碼前拒收。遠端探索按鈕與直接命令防護、手動字幕保留、Busy／Ready／Stop／Clear、網址undo與通知一次保持。字型案例擴大為本機／遠端ASR、兩個遠端來源字幕服務及無服務中文，涵蓋320／480／640寬、原字型與26px字型的實際高度及按鈕分隔，不外推真人多螢幕驗收。

首輪測試編譯因TextLanguage無Debug而改用相等布林斷言，不新增正式Debug輸出。原生字型首輪發現中文本機說明僅兩段，寬視窗可完整顯示兩行，既有至少三行預期過嚴；僅該中文案例改為至少兩行，雲端仍至少三行，實際高度、完整容納及不碰按鈕檢查均保留。沒有調整正式布局、容量、timeout、停用警告或刪除防護來通過測試。

首輪完整回歸與Release建置同時進行，既有`b_frames_mp4_stop_error_and_chinese_sidecar_release_resources`在原五秒terminal等候內未退出；其餘181項為共享TEST_OWNERS鎖的PoisonError連帶失敗，該輪為1484通過／182失敗／15 ignored、154.94秒，不計為通過。Release完成後單獨重跑原案例，1項通過、7.47秒（包含兩份合成媒體及多次獨立等候）。並行建置造成資源競爭僅為可能原因，未當成已證實；沒有修改產品、等待期限、鎖語意或忽略測試。

最後在沒有並行建置的情況下，使用同一正式程式與原有測試期限單獨執行完整回歸；1666項library與26項整合測試全部通過，包含先前逾時的本機B-frame案例。這是成功重跑的證據，沒有把首輪失敗或mock品質宣稱為已驗收。

正式程式未新增unsafe、recoverable unwrap／expect、依賴／安裝、pool、無界音訊佇列、媒體／本文落盤、temp／cache／export或遙測。只重用既有字幕讀取／解析與全文處理，來源與本文在本次RAM；未讀取正式金鑰、使用者媒體或真實雲端。金鑰仍留至結案固定遮蔽介面手動輸入；本機無.git，未建立commit或丟棄既有工作。

一般HTTP(S) WAVE已支援明確本機外掛字幕；遠端內嵌／字幕網址、其他遠端編碼、五語音訊Auto與真實媒體／語意／雲端／播放仍待接續，Phase09尚未具備條件，達標時會提醒。見 [ADR 0073](adr/0073-remote-wave-local-sidecar-ready.md)。

## 第四十一批驗證

2026-10-18，Windows 10.0.19045.0，實際執行通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：36.97秒。
- `cargo test --locked --offline --quiet`：1686項library、12項設定、4項探索、10項啟動，共1712項一般測試通過，零失敗；既有15項ignored不變。library 464.14秒，不作字幕延遲SLA；Release完成後才單獨執行完整回歸。
- `cargo test --locked --offline media::remote_subtitle::tests -- --nocapture`：新增20項測試通過，4.84秒。
- `cargo build --locked --offline`：Debug通過，0.96秒。
- `cargo build --release --locked --offline`：Release通過，47.13秒。

本批新增10項輸入／傳輸／全文解析與10項生命周期測試，未改動舊Live、PreparedRequest、本機外掛、Session或一般GUI。loopback真實HTTP核對宣告兩種格式、三類中文及五語的原文、5秒起點／1.25秒長度、Session／generation／sequence與Existing身分；不猜副檔名或語言、不執行本地化或翻譯、不把內部content ID當媒體版本證據。

正規固定長度、chunked與connection-close完整訊息皆可讀取；截斷Content-Length／chunked、錯MIME／字元集／encoding、多重或矛盾標頭、壞UTF-8／時間／markup及空本文均明確失敗。實際八MiB原始上限可解析，宣告或未知長度超量拒收；8192段／四MiB本文正好上限通過，各自超一拒收，不放寬Live預算。401／403／404／429／302等回typed error；HTTPS對只會接收TLS的合成失敗端點只發一次握手，不降級或重試，錯誤沒有私有query。

公開入口另外核對缺runtime與零Session／generation／sequence在網路前拒收；實際全程序CPU已占滿時單次GET後回Overloaded，釋放後可成功，兩份保留結果阻止第三份要求且不連線，Drop後可恢復。其他案例使用獨立注入配額，避免與原生媒體測試爭用正式資源。

未poll即Drop不發I/O或取消父scope；等待body的Drop／父取消與要求期限均關閉socket、清理scope並回收容量。blocking pool僅一執行緒且先占用，以明確同步核對排隊解析未開始就被Drop；已啟動解析則以barrier核對Drop／取消／逾時後CPU及本文許可仍保留，到實際退出才回收。兩份結果、舊Session取消與新generation各自隔離，退休結果原文不可再從API取用。

稽核中將未知長度body改用精確容量預留，避免Vec倍增超出八MiB本文容量；沒有擴大上限。首次fmt修正新模組排序，測試編譯將SendError的unwrap改為布林斷言，保留ResolverCancellation無Debug，沒有新增正式敏感輸出或停用警告。獨立安全／並行稽核未發現其他需修正項目。

新正式程式未新增unsafe、recoverable unwrap／expect、套件／安裝、獨立pool／OS thread、無界音訊佇列、內容落盤、temp／cache／export、telemetry、金鑰取用或AI要求。只有本次遠端字幕原文，無媒體音訊、Ready、時間軸、識別／仲裁或播放；測試只使用合成資料與literal loopback，不讀使用者媒體或真正雲端。本機無.git，未建立commit或丟棄既有工作。

本批為獨立library；一般準備視窗仍拒收字幕網址，後續需Session／來源選取／用途揭露／全文本地化翻譯接線。遠端內嵌、其他編碼、五語音訊Auto與真實媒體／雲端／播放驗收仍待接續，金鑰維持結案固定遮蔽介面手動輸入；Phase09尚未具備條件，符合時會提醒。見 [ADR 0074](adr/0074-bounded-prepared-https-subtitle-reader.md)。

## 第四十二批驗證

2026-10-18，Windows 10.0.19045.0，實際執行通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：最後一次22.95秒。
- `cargo test --locked --offline --quiet`：最終單獨完整回歸1705項library、12項設定、4項探索、10項啟動，共1731項一般測試通過，零失敗；既有15項ignored不變。library 505.58秒，不作字幕延遲SLA；建置完成後才單獨執行。
- `cargo test --locked --offline https -- --nocapture`：21項相關測試通過，1.64秒。
- `cargo test --locked --offline media::planning::remote_tests --quiet`：5項規劃測試通過，0.00秒；與前一篩選有重疊，不重複計入新增數量。
- `cargo test --locked --offline media::remote_subtitle::tests -- --nocapture`：讀取器20項測試通過，最後一次2.93秒。
- `cargo build --locked --offline`：Debug通過，最後一次1.01秒。
- `cargo build --release --locked --offline`：Release重建通過，1分06秒；測試fixture調整後的最後快取檢查0.86秒。

新增5項metadata規劃、12項Session及2項controller防護，共19項測試。規劃涵蓋本機／遠端、內嵌／遠端的八類宣告語言順位矩陣、同順位歧義、原網址索引及三種來源共用八份預算。只有勝出的HTTPS字幕發出GET；本機／內嵌勝出不連線至低順位網址，遠端勝出不讀不存在的低順位本機外掛。

真實loopback字幕／WAVE、自有合成媒體與明確AI mock核對SRT／WebVTT、三類中文、五語與三品質的完整處理。中文字幕不發分析／翻譯要求；五語沿用全文摘要／詞彙、草稿／一致性校訂與臺灣本地化，保留真正字幕時間、原文及Existing身分。遠端媒體只讀既有WAVE標頭，沒有PCM／DSP／ASR；URL／query／媒體路徑不加入服務本文或脈絡，不以mock文字宣稱語音或語意品質已驗收。

同順位歧義、缺translator、舊glossary、遠端媒體帶任一內嵌選取及錯入口於對應preflight拒收。404、壞字幕及服務失敗保留typed cause，不改做ASR或發布部分文件。所有七個既有Task入口與一般controller拒收任何HTTPS字幕候選，包含低順位候選；controller不建立Session／worker／factory或取用金鑰，一般視窗維持原字幕欄契約。

媒體owner在字幕GET前持有，字幕原文與lease共同移交；分析／翻譯等待、未apply completion與Ready都仍占全程序兩份本文配額中的一份。第三份要求明確過載且不連線，釋放後可恢復。未poll Drop、HTTP／分析／翻譯等待的Stop或Future Drop均清理；已開始worker持配額至真正退出，晚回及舊completion不能替換新Session。原文／譯文先釋放、lease後釋放，包含中途錯誤，網路／服務等待不占CPU。

首輪完整回歸為1704項library通過、1項失敗、15項ignored，516.78秒；整合測試因library失敗未執行。既有`raw_document_budget_accepts_exact_limit_and_rejects_declared_or_actual_excess`在第412行回Timeout，原案單獨重跑1項通過、5.33秒。原fixture以換行填滿八MiB，會解析約八百萬個空行並逐行核對取消；全套CPU競爭是可能因素，沒有當成已證實的唯一原因。

該fixture改以每行最多四KiB的純空白填滿同樣八MiB，避免把傳輸容量測試變成大量逐行檢查的五秒效能門檻。真正HTTP、精確上限、宣告／未知長度超一位元組拒收及原斷言保留；五秒fixture期限、正式三十秒期限、八MiB原始容量、8192 cues／四MiB本文、正式parser與取消檢查均未改動。20項reader定向重跑通過，不以此宣稱密集空行的效能已驗收；沒有停用警告或忽略測試。

最後在沒有並行建置的情況下再次執行完整套件，1705項library與26項整合測試全部通過，包含調整後的原容量案例；命令正常結束。這是最終回歸通過的證據，首輪失敗與原案單獨重跑仍保留在本紀錄，不把fixture修正稱為產品容量或取消策略變更。

獨立程式／文件稽核修正Ready及失敗路徑的本文／lease清理順序，並釐清每份lease持一份許可、全程序最多兩份，未發現其他需修正項目。正式程式沒有新增unsafe、recoverable unwrap／expect、依賴／安裝、pool／OS thread、無界音訊佇列、內容落盤、temp／cache／export、遙測或第二份時間軸；未讀取正式金鑰、使用者媒體或真正雲端。本機無.git，未建立commit或丟棄既有工作。

本批完成明確Task的HTTPS字幕Session／全文處理／RAM Ready，一般controller／GUI的字幕網址輸入與用途揭露仍待接續；Ready不代表媒體版本／仲裁／播放定位，也不自動播放。其他遠端格式、遠端內嵌、五語音訊Auto、真實媒體／語意／雲端／播放仍待驗收。金鑰維持結案固定遮蔽介面手動輸入；Phase09尚未具備條件，符合時會提醒。見 [ADR 0075](adr/0075-prepared-https-subtitle-session-ready.md)。

## 第四十三批驗證

2026-10-18，Windows 10.0.19045.0，實際執行通過：

- `cargo fmt --check`
- `cargo clippy --locked --offline --all-targets --all-features -- -D warnings`：42.52秒，零警告。
- `cargo test --locked --offline --quiet`：1727項library、12項設定、4項探索、10項啟動，共1753項一般測試通過，零失敗；既有15項ignored不變。library 492.20秒，不作字幕延遲SLA；Release建置完成後才單獨執行。
- `cargo test --locked --offline https -- --nocapture`：43項相關測試通過，14.81秒，包含本批全部22項新增測試。
- `cargo test --locked --offline win32::prepared_dialog::tests --quiet`：文案修正後64項原生視窗測試通過，54.76秒，包含完整138次字型／寬度量測。
- `cargo build --locked --offline`：Debug通過，0.98秒。
- `cargo build --release --locked --offline`：Release通過，1分03秒。

新增6項UI草稿／backend邊界、7項controller及9項隱藏原生測試，沒有移除或忽略既有測試。明確HTTPS模式與舊本機／內嵌模式分開，八類宣告語言、兩格式、本機／遠端媒體、非空／HTTPS-only／URL容量／本機路徑及default backend fail-closed均核對；舊模式不因字幕欄含網址改成網路工作。

controller以真正literal loopback字幕、合成媒體及明確Factory／分析／翻譯mock驗證三類中文、SRT／WebVTT與兩服務／五語／三品質。中文不建factory或取key，所選來源語言才建一次Translation服務與所選金鑰；Fast只有Draft，Standard／High另有Review，全文摘要／詞彙與本地化、原文時間及Existing身分保留。勝出本機中文忽略低順位網址，勝出遠端中文不讀較低順位本機外掛；音訊Auto不影響文字準備，沒有ASR。

歧義、缺必要服務、遠端媒體帶內嵌或錯入口在對應preflight拒收；新入口無遠端候選不改做辨識，舊start仍先拒任何遠端候選。404、分析／翻譯失敗及錯factory保留typed error或明確WorkerFailed，不發音訊要求。HTTP／分析／翻譯等待的Stop退休scope，工作未退出保持Busy，reap後才重啟；buffered舊completion不能發布新Ready。AI原文與脈絡逐一核對不含私有媒體／字幕URL、query或路徑，沒有使用者內容或真正雲端。

原生控制項驗證新兩模式走明確backend、格式／語言／品質保留、中文不讀兩個故意無效password欄、來源只移交所選遮蔽欄並清空兩欄與undo。空白／HTTP／非法網址、缺語言／服務或舊格式下的網址在取密碼前拒收，不切ASR或抹除手動選擇。用途揭露隨格式、語言、服務及媒體欄事件更新，兩個私有位置不進狀態／用途；HTTPS模式探索按鈕與直接命令均防護，不自動換內嵌來源。Busy／Stopping／Ready鎖定、停止／清除／關閉回收、網址與undo清理及一次通知沿用。

首輪`cargo test --locked --offline win32::prepared_dialog::tests --quiet`為63項通過、1項失敗，47.05秒。既有實際字型量測矩陣擴為23種用途分支×兩字型×320／480／640寬，共138次；新增HTTPS用途在本機媒體／OpenAI／來源字幕、320寬及26px字型時覆蓋按鈕36px（文字bottom 660、button top 624）。這是實際布局錯誤，不以其餘功能測試通過視為版面已驗收。

僅縮短HTTPS固定文案，移除重複敘述並保留字幕全文連線、本機媒體唯讀或遠端WAVE標頭、實際AI端點、同一所選服務與金鑰、四類文字用途、不傳音訊／來源網址、本次記憶體及不存檔／播放。新模式的字面期待更新為同等必要資訊，另核對不保存／播放；原有其他分支、138次矩陣、26px字型、寬度、最少段落、實際完整高度及不碰按鈕斷言均保留，沒有調整DIP／固定band或擴大捲動架構。最後64項原生回歸全部通過；這不代表真人多螢幕／DPI已驗收。

最終在沒有並行建置的情況下執行完整套件，1727項library與26項整合測試全部通過，包含新入口、既有legacy拒收及修正後的字型／窄視窗案例；命令正常結束。首輪63／64的版面失敗與修正原因保留，沒有以功能mock成功代替版面驗證或真實服務驗收。

正式程式沒有新增unsafe、recoverable unwrap／expect、依賴／安裝、pool、無界音訊佇列、內容落盤、temp／cache／export、遙測或第二份時間軸；沿用原本單一worker／runtime／completion、CPU／媒體／字幕／要求配額與真正退出才回收規則。獨立唯讀稽核未發現新的來源／金鑰／URL隱私、取消或Session隔離問題；本機無.git，未建立commit或丟棄既有工作。

本批提供一般準備視窗的明確HTTPS字幕模式，遠端媒體仍限既有PCM16 WAVE，不提供一般網站／登入／重新導向／遠端內嵌。真實使用者媒體、五語語意與雲端品質、真人多螢幕／DPI與播放仍待個別驗收；有效金鑰維持結案固定遮蔽介面手動輸入。本批完整工程驗證已通過，已具備開始Phase09對齊核心的工程條件；本批不自行啟動。見 [ADR 0076](adr/0076-explicit-prepared-https-subtitle-ui.md)。
