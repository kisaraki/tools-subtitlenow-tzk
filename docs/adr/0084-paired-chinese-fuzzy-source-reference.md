# ADR 0084：中文字幕以同版本來源參考軌提供模糊候選

日期：2026-10-23。狀態：Phase 09 第八批採用。

## 決策與原因

中文不能與五語 ASR 直接比較，也不能拿 Live 翻譯冒充原文。本批提供同步 CPU 入口 `locate_paired_fuzzy_cue` 與背景擁有者 `FuzzyCueLocator::new_paired`，只接受明確傳入的 ExistingTextBundle：中文主軌加上供應端宣告同版本的五語 SourceTextReference。不猜測／生成參考軌，不發出網路或 AI 要求，不把相同 content_id 當作同版本證明。

SourceTextReference 新增 `validate_document(primary, identity)`，共用原有中文類型、來源語言、Session／generation、來源種類、content_id、非空／合法／相同 edition ID、兩份文件結構／時間／文字檢查。既有 query 驗證另保留 query.source／content_id 與主軌相符的檢查，再套用相同文件規則，不製造 capture query 或 stamp。供應端的同版本宣告是結構契約，不是實際版本辨識或播放證據。

## 原文搜尋與中文對應

只將來源參考軌交給原有全文模糊核心，沿用相同五語 Final ASR、單段／相鄰雙段、字界／Unicode、900／50／850、256／513 scalar、DP 與歧義規則。中文本文完全不參與文字比較，不複製原文或中文本文。遠處參考軌重複仍阻止唯一性，即使中文主軌僅有一個時間對應也不能補救詞彙歧義。

原文唯一勝出後，為每個命中的參考 cue 掃描完整中文主軌，只接受唯一、起點與長度精確相同的 cue。差一微秒也回 TimingMismatch，不取整、偏移、補時或模糊時間；同時間有多個中文 cue 回 NoUniqueMatch。雙段須分別一對一對應且中文索引相鄰，不支援一對多／多對一／中間插入 cue 或猜測分句。

候選新增 `FuzzyTextOrigin::PairedSourceReference` 與 `paired_primary_cue_range()`；`cue_range()`／`cue_index()` 永遠是實際搜尋的來源參考軌索引，中文索引另存，不假設兩軌相同。media range 因逐 cue 精確對應而與中文原始時間一致，capture range 仍來自 ASR。原有單軌／PreparedRecognition 的新 getter 回 None。多觀測 metadata 同時保留兩軌索引，修訂比較與向前不重疊檢查均涵蓋中文範圍，仍只回時間一致性提示。

## 資源、期限與生命週期

雙軌合計最多 8192 cues／四 MiB，每 cue 最多 8192 bytes。constructor 只檢查長度與主軌身分，兩軌全文與同版本宣告驗證在原共享 CPU lane 內執行。原始參考 cue 超過 256 scalar 仍使整次失敗；中文不套用原文字元限制，但須遵守原文件 byte／控制字元等契約。映射最多兩次完整主軌數值掃描，每 cue 檢查取消／期限，超限／失敗不交付部分候選。

配對驗證、原文搜尋與中文映射共用最初截止時間，不在第二階段重設三秒或原窗口五秒期限。背景管理的 offer 期限、每 locator 一項、既有共享 CPU 名額、過載不排隊、最新輸入／結果、明確取消／Future Drop 清理均沿用。不新增 worker、runtime、OS thread、queue、取消祖先、依賴、unsafe、I/O、金鑰取用或內容保存。

這是明確 owned bundle 的 library 入口，Arc 移入同一份不可變 RAM 文件；constructor 不複製本文。Stop／owner Drop 撤回工作及已交付候選，已交付 metadata 不持有 bundle。未 poll Future 與執行中工作遵循原 owned locator 生命週期。尚未提供遠端 ExistingTextLease 自動橋接或 Prepared 中文參考軌 Ready；外部來源／文件撤回、替換或 epoch 變動時，caller 必須 Stop 並重建定位器，不可假設本入口自動訂閱外部 lease。

## 範圍與驗證

沒有將模糊候選轉成 ExactCueMatch／CandidateEvidence、可靠錨點、目前播放位置或字幕採用；精確驗證器、仲裁、擷取時鐘及 TranscriptTimeline 不改接。簡中／其他繁中主軌仍須原有臺灣本地化流程才可顯示，本入口不輸出中文本文、不省略本地化。一般 Live／GUI、Ready 中文參考軌橋接、可靠錨點、位置／seek／pause／restart、offset／drift 仍待接續，Phase 10 尚未具備條件。

合成測試涵蓋五語與三種中文分類、兩軌不同索引、雙段映射、中文不能冒充原文、缺參考／錯來源／版本／身分／語言、精確時間與重複／不連續對應、全文重複、原 allocation、雙軌總預算、後段原文超量、取消／期限／DP 及既有 query 契約。實際 worker／管理器核對雙軌借用、三個獨立觀測、來源種類、容量／typed error、Stop／Drop 撤回及父取消隔離。

測試只用合成 RAM 文件，不是實際供應端版本品質、雲端辨識或播放相容性驗收。正式金鑰維持結案固定遮蔽介面手動輸入。完整實際結果見 [Phase 9](../phase-9.md)。
