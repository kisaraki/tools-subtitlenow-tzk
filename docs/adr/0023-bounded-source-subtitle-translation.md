# ADR 0023 — 有界來源字幕翻譯沿用既有服務與前文契約

日期：2026-10-10。狀態：Phase 7 第二批採用。

已取得來源字幕不等於整份字幕都應立刻上傳或已與播放位置相符。Phase 7 需要在 Live 延遲限制內準備候選，也必須保留 Phase 8 整份媒體處理與 Phase 9 進階定位的邊界。因此 SourceTextTranslator 要求 caller 明確指定一至四個 cue 的範圍，不推測時間偏移、不從零開始處理整份文件，也不建立第二條 TranscriptTimeline。

翻譯沿用 provider-neutral Translator、QualityPolicy、SessionGlossary 及固定 zh-TW 指示。Fast／Standard／High 使用一／三／八句前文，High 另以草稿校訂；已知完整片段無須 ASR debounce。片段逐一處理，只有整個範圍成功翻譯及本地化後才提交前文。相鄰性與前文原始文字／時間核對防止跳段及修訂後混用脈絡；內容／來源／語言改變清除詞彙，所有內容仍只在工作階段 RAM。

候選準備不應讓服務的串流部分輸出塞滿無人消費的更新佇列，也不應把未定位的候選當成 Live ASR 字幕。TranslationEmitter 因此提供 crate 內部 final_only 模式：保留驗證但不建立更新訊息，候選只使用完整回傳。正式 Live emitter 仍使用原有有界佇列。原文與語言保留在輸出，最終譯文透過既有 CPU lane 做臺灣詞彙轉換及容量檢查。

每個管理物件一次只執行一個要求；整個視窗受原查詢 freshness 與三秒總翻譯期限共同限制。期限包含等候、Draft、Review 及 CPU 後處理。取消或失敗不交付部分候選；同步或非同步 panic 轉為型別錯誤，Drop 取消要求並回收 CPU worker。四句是工作量上限，不是吞吐保證；caller 可依已驗證播放位置選取更小範圍。

僅有各 coordinator 自己的並行上限不足以保護共用服務。因此既有 HttpTranslator 另在 client 內持有兩個共用 permit，包含重試及 rate gate 的整段生命週期，等待可取消。來源字幕入口只借用已選定並公開目的地的 client，不讀取金鑰、不自動切換供應商、不新增端點。準備成功仍不是採用授權；後續唯一 source arbiter 必須再驗證 Session、freshness、信心與播放定位。
