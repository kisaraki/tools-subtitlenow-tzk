# ADR 0058：有界 tx3g 純文字樣本核心

日期：2026-10-14。狀態：Phase 08 第二十五批採用；容器與 GUI 接線尚未完成。

2026-10-15 後續：第二十六批已提供明確單軌的唯讀 MP4 reader，見 [ADR 0059](0059-selected-pinned-mp4-tx3g-reader.md)。本核心保持；GUI／Ready、更多軌道／edit／composition 與實際 muxer 仍未完成，以下為當批決策與限制。

## 問題與決策

Prepared 應優先使用既有字幕，MP4 音訊解碼則沒有提供內嵌文字。先把 tx3g sample 本文解析獨立於音訊 decoder、容器 I/O、provider 及 UI，避免為抽取字幕新增錄製／暫存或第二個翻譯時間軸。此批是可執行、可測試的文字樣本核心，未當作完整軌道或使用者檔案支援。

`Tx3gTrack::new` 要求非零 Session／generation／track ID／timescale，以及明確 `TextLanguage`。ISO 語言或文字字形不能自動證明 native zh-TW。`push` 只借用樣本 bytes、使用獨立 stamp：原始 ticks、Session 及從 1 開始的連續 sample sequence。完成文件的區域 content ID 不含檔名／URL，不能充當 media pin、版本／edit match 或 alignment 證據。

接受兩 bytes 大端本文 byte length、UTF-8（可有 UTF-8 BOM）、BOM UTF-16 BE／LE；不採 lossy replacement。依長度取得全文，任何尾資料暫時 UnsupportedSubtitle，包含已知 style／karaoke／link modifier。這是本專案的明確子集合限制，並非宣稱完整 3GPP decoder。零長度字串只清除該 sample 區間；BOM-only／空白不偽裝清除。非法控制字元拒收，Unicode line／paragraph separator 轉為 newline，markup 留作純文字。

絕對 start／end ticks 各用 u128 換算微秒，再相減，不逐段累積截斷 duration。所有樣本（包括清除）皆核對順序、正 duration、非重疊及 24 小時端點；不填 gap、延長文字或改寫音訊時鐘。任一樣本錯誤釋放累積 cues，後續 push／finish 保留失敗；取消與跨 Session 同樣不交付部分文件。

## 邊界與生命週期

- raw sample 最多 8194 bytes；decoded UTF-8 cue 最多 8192 bytes，UTF-16 膨脹另核對。
- raw track 八 MiB；decoded document 四 MiB、8192 cues；包含 empty clearing samples 最多 16384 次 push。
- 只保留 RAM／本次 Session 的原始本文與媒體時間；無 Clone／Debug／Serialize／export／disk I/O。
- 同步解析在 Prepared 有界 CPU lane 執行；無新 thread、queue、remote request、unsafe 或相依套件。
- `finish` 共用 ExistingText structural validation；原中文走既有 Taiwan conversion、來源語言走既有全文 glossary／translation stages。核心不自行選服務或建金鑰 client。

## 驗證與未完成範圍

合成樣本驗證多語／emoji 與雙 byte order、長度／truncation、surrogate／UTF-8 errors、clearing／gaps／newlines、modifier 拒收、精確容量、時間 overflow／重疊／endpoint rounding、取消／舊 Session／sequence 及錯誤後無部分文件。輸出與既有 Taiwan localization、五語 Standard Draft／Review 的 context／原始 timing 相容性亦測試；這些 fixture 不是實際 MP4，也不代表 muxer 相容性。

尚需容器 reader 核對 tx3g sample description／font／display semantics、整份 track tables、檔內 data reference、mdat 範圍、跨軌 byte overlap、edit list／composition offset，再從同一 pin 讀取樣本。不能以本文核心接受直接放寬原有 MP4 音訊門檻。GUI 選軌／語言／優先計畫與完整 Ready／取消接線仍待接續；無自動播放或 Phase 09。真實服務／有效金鑰維持結案固定遮蔽介面手動輸入。

驗證命令及結果見 [Phase 8](../phase-8.md)。

## 格式依據

[ETSI TS 126 245 V10.0.0，5.1／5.2／5.9／5.11／5.16／5.17](https://www.etsi.org/deliver/etsi_ts/126200_126299/126245/10.00.00_60/ts_126245v100000p.pdf)：Unicode／BOM、byte count 與 character offset、mdhd 語言、hard wrap、tx3g sample entry、sample 本文與 modifier 結構。本專案選擇在尚未支援 modifiers 時拒收，而非略過擴充。

[Apple Subtitle sample data](https://developer.apple.com/documentation/quicktime-file-format/subtitle_sample_data)：16-bit byte length、UTF-8／BOM UTF-16（BE／LE）與尾端 sample extensions。[QuickTime 6.3 3GPP 支援](https://developer.apple.com/library/archive/documentation/QuickTime/QT6_3/Chap1/QT6WhatsNew.html) 說明 tx3g 與 UTF-8／UTF-16。
