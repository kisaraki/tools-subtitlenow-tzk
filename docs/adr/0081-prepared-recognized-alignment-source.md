# ADR 0081：直接借用 Prepared 辨識原文作定位候選

日期：2026-10-23。狀態：Phase 09 第五批採用。

## 決策與原因

Ready 原文保留 Existing／Recognized 型別，第三批定位入口只接受 Existing。沒有現成字幕、經 ASR 完成準備的媒體因此無法使用相同定位流程。本批擴充原有 `PreparedSession::fuzzy_locator` 與 controller 委派入口，直接借用目前已 apply 為 Ready 的五語 Recognized 原文；不把它轉為 ExistingText、虛構 content_id 或來源字幕出版物。

私有 `subtitle::alignment_text::AlignmentText` 分別借用 ExistingText／PreparedTranscript，提供唯讀 cue、語言、來源種類及結構驗證。所有本文保留原 allocation，不重建整份 cue Vec，不複製原文或譯文。Existing 仍使用原有來源／metadata／文字／時間契約；Recognized 重查 sealed transcript 與目前文件 Session／generation，沿用 8192 cues／四 MiB、每 cue 8192 bytes、非空與控制字元、順序、正原生時間範圍及媒體最長時間限制。

## 共同核心與來源身分

公開同步 `locate_fuzzy_cue` 的 ExistingText 契約保持；Ready worker 經私有借用 view 使用同一搜尋核心。Final ASR 的來源 Session／generation／時間窗口、明確五語語言、全文唯一性、900／50／850 門檻、256 scalar、DP 與原始期限全部維持。Recognized 的每個 cue 都參與全文歧義檢查，不只搜尋猜測播放區間，也不跳過過長 cue。

候選新增 `FuzzyTextOrigin::{ExistingText, PreparedRecognition}` 及唯讀 getter，來源種類不會因成功匹配而遺失。這仍是 FuzzyCueProposal，沒有轉成 ExactCueMatch／CandidateEvidence 的通道，也不證明辨識正確、同一媒體／發行版本、offset 或目前播放位置。原 ExactCueVerifier、擷取時鐘與來源仲裁不變。

## 生命週期與限制

沿用 [ADR 0079](0079-revocable-prepared-ready-alignment-text.md) 的弱文字 owner 與獨立媒體 owner：locator／未 poll Future 不持強本文參考；取得原有 CPU 名額後 job 才升級文字，沒有媒體 pin／decoder 交給背景搜尋。Stop／替換／Session Drop 取消舊發布，執行中工作只保留原文／譯文 owner 至真正清理；Ready 媒體資源可以先釋放。

沿用 [ADR 0080](0080-latest-only-fuzzy-proposal-session.md) 的最新候選管理，不新增 STT／翻譯管線、服務呼叫、worker pool、runtime、OS thread、佇列或保存功能。三秒工作、原窗口五秒、每 locator 一項、共享 CPU 名額及四祖先取消上限保持。未 apply completion 仍回 NotReady；中文 Existing 仍回 UnsupportedEvidence，不使用臺灣譯文作原文。

本批沒有一般 Live／GUI 自動接線、中文配對、跨 cue 合句、多觀測可靠錨點、播放定位、seek／pause／restart 或 offset／drift；Ready 不自動播放。有效金鑰與真實雲端／私有媒體仍留到結案固定遮蔽介面手動驗收，Phase 10 尚未具備啟動條件。

## 驗證

合成 sealed ASR 經真正 merger 驗證五語 Unicode 編輯、來源 allocation、獨立 Session、非零原生媒體時間、全文重複、8192 cues、後段超量、取消／期限／DP 預算。真正合成 WAVE 經 mock 辨識／翻譯到 Ready，再經實際 worker／最新管理器核對來源種類、未 apply 拒收、停止／Drop／替換／發布撤回、媒體 pin、零額外服務要求，以及足夠長的中文譯文也不能冒充原文。

這些是本機合成及 mock 的工程證據，不是實際媒體辨識品質、遠端服務或播放相容性驗收。完整命令及結果見 [Phase 9](../phase-9.md)。
