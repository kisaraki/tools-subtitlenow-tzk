# ADR 0044 — 同一媒體 handle 的辨識來源 Prepared Ready

日期：2026-10-12  
狀態：採用於 Phase 08 第九批；正式 ASR adapter、GUI ASR 與播放定位仍待接續。

## 問題

辨識結果與全文翻譯已能保留來源身分及原生媒體時間，但 native decoder 在 EOF 銷毀時也釋放原媒體 handle。若完成翻譯後再依路徑開檔，可能得到另一份已被替換的檔案，無法證明 Ready 對應先前實際辨識的媒體。把 ASR 包裝成 ExistingSubtitle 也會錯置來源證據。

## 決策

native worker 只開啟一次通過既有本機路徑與一般檔案檢查的唯讀媒體。保留原 OpenedMedia，透過 File::try_clone 複製同一檔案物件的 handle 給 IStream；不再次開啟 pathname。原 owner 只提供大小與生命週期，不對共用檔案位置進行讀取或 seek；IStream 保留原有 logical cursor 與同步鎖定。複製 handle 共用檔案位置的語意依據 [Rust File::try_clone](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_clone) 與 [Microsoft DuplicateHandle](https://learn.microsoft.com/en-us/windows/win32/api/handleapi/nf-handleapi-duplicatehandle)。Windows 開檔的 SHARE_READ 持續禁止 writer、刪除與替換，直到所有 handle 釋放。

私有 producer 只有 EOF 回覆可以移交媒體 pin，並在同一 worker 銷毀 decoder、MF 與 COM。caller 等待真正 worker join 後才接收 EOF 成果。native-owner 許可至實際銷毀後釋放；檔案 pin 繼續存在於最後一批 ASR、全文翻譯與 Ready。服務等待不持 CPU 許可，亦不因單純持有唯讀檔案而保留 native-owner 許可。

新增私有辨識成果 owner，將媒體 pin 與完整 PreparedTranscript 一起交付 Session。原公開 recognize_local_media 仍只交付 transcript，在成功返回前釋放 pin；獨立 translate_recognized_transcript 也維持不持媒體的語意。不能用這兩個獨立入口的結果重新開檔冒充具媒體連續性的 Ready。

PreparedTask 提供 caller 明確交付 recognizer 與 translator 的入口。先使用原有字幕優先序；有選定外掛字幕時沿用原路徑，不呼叫 recognizer，也不因字幕損壞而偷偷改用 ASR。無字幕時先核對本次 glossary 身分，再辨識同一媒體及翻譯。完整成功、取消檢查及 Session apply 的身分／sequence 核對通過後才成為 Ready。

PreparedDocument 使用明確的既有字幕／辨識來源 body，原文以安全借用型別區分 ExistingText 與 PreparedTranscript。共用 Ready owner 提供譯文與句數，保留 ASR 的五語、原文、媒體時間、順序及實際翻譯品質，不建立假 publication、content ID、字幕配對或播放定位證據。所有本文只在本次 RAM，沒有 Clone、Debug、Serialize 或匯出。

停止、新要求、Session／Task／Future Drop、辨識或翻譯失敗都撤回本次工作；舊 completion 不能覆蓋新 Ready。已開始同步 worker 仍合作取消並在實際退出時清理，abort 不是資源已釋放的證明。錯誤只公開固定 typed domain message，不包含路徑、本文、PCM 或憑證。

## 結果與限制

library Session 已能透過 caller 提供的真實能力契約進行無字幕 ASR fallback 並交付保持同一媒體 pin 的 Ready；測試使用明確 mock provider 與合成 WAVE，不能代替正式辨識品質驗收。一般 GUI 尚未建立正式 Prepared ASR client，無字幕仍回報不可用，既有固定金鑰介面的用途不變。

Ready 表示完整文件已準備，不代表目前播放位置已驗證，也不自動播放。自動全文脈絡／專名分析及 glossary 建立、其他影音容器、內嵌字幕、URL 與播放定位仍待完成。Phase 09 尚未具備啟動條件；有效 OpenAI／Gemini 金鑰與雲端驗收維持結案固定遮蔽介面手動輸入。

未新增 crate、unsafe、元件安裝、產品暫存媒體、持久化內容、快取或匯出。本批實際驗證另記於 [Phase 8](../phase-8.md)，不以此決策預先宣稱測試通過。
