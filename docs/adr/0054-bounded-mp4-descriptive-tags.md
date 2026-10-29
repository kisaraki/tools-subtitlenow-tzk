# ADR 0054 — Bounded MP4 Descriptive Tags

2026-10-14，Phase 08 第二十一批，Windows 19045 基準。

## 決策與原因

前批 M4A／MP4 門檻只接受空的 ilst，導致含普通標題或封面的 AAC-LC 音軌也被拒收。新增有界描述資料門檻，接受少數明確型別的文字、封面及數值欄位，讓同一已支援音訊可繼續準備。維持一個音軌、檔內音訊引用、AAC-LC 44.1／48 kHz 單／雙聲道，以及原有 native clock／Ready 管線。

檢查位於 `moov/udta/meta`、handler 必須是 mdir，ilst item 只能包含一個 data。四 bytes type／四 bytes locale 後為本次的值；不產生可回傳的 metadata model。所有 slices 都借用前批已限制一 MiB 的 moov；門檻完成後原 allocation 釋放。標籤／封面不寫入 timeline、詞彙表、翻譯脈絡、日誌、要求或快取。值內的 URL 只當文字，沒有 URL 解析、抓取或外部圖片來源。

## 明確的接受範圍

文字項目限 ©nam、©ART、aART、©alb、©gen、©wrt、©day、©cmt、©too、cprt，type 必須是 1，payload 是有效 UTF-8、每筆最多 64 KiB。可接受空文字；不顯示或拿來辨識內容，因此這不是臺灣本地化或 metadata resolver 功能。

covr 限 type 13／14，payload 最多 512 KiB，分別核對 JPEG／PNG 開頭。只有格式形狀檢查，不解碼圖片、不配置像素、不取出尺寸、不驗證完整圖片或 CRC，也不顯示／保存封面。純 header fixture 的成功不能宣稱該圖片可開啟。

數值限 trkn 的八 bytes、disk 的六 bytes tuple，reserved bytes 必須零，已知 total 時 index 不得大於 total；tmpo 是 type 21 的兩 bytes 非負整數，cpil 是 type 21 的一 byte 0／1。這些是明確支援的常見記錄形式，不推論所有 tag writer 的變體相容。

ilst 合計最多 768 KiB，另受整份 moov 一 MiB 及既有每層最多 128 boxes 影響。已知 tag 不重複、每個 item 只容納一個 data；同標籤的多語／多張封面目前拒收。超量、錯 type／版本、UTF-16、畸形 UTF-8、截斷、長度溢位、未知項目、freeform、非空 Xtra、mdta／keys／iloc／dref、lyrics 等未支援結構明確拒收。沒有因為容器出現已知 tag 就跳過音訊 data reference、ES descriptor、sample table 或 chunk byte 範圍檢查。

驗證每個 item 前後檢查取消，空列表及大資料前亦檢查；UTF-8 單筆 CPU 工作以 64 KiB 限制，仍位於原有有界 native worker，不進入 capture 或 UI。沒有新增 worker、queue、crate、產品 unsafe、檔案工具或圖片 decoder。

## 真正音訊與私密內容邊界

測試只在 cfg(test) helper，對原生 RAM encoder 的 genuine MP4 修改最後的 moov/udta，mdat、sample tables、壓縮 AAC 及 offsets 全部保留。標籤是自有合成內容；PNG 是自製一像素資料，含 CRC-covered chunks 與 zlib stored block。這是測試 fixture，不是產品 tagging／圖片產生或媒體匯出 API。

真正 44.1／48 kHz mono／stereo 逐 block 比較未加標籤及加標籤兩個 reader，所有 PCM samples、sequence、native start／end 和真正 EOF 完全相同；DecodedAudioGate、唯讀 pin、原 bytes 不變及資源回收另有驗證。比較只保留目前兩個有界 blocks，沒有完整 PCM 副本。

十三秒加短尾的有標籤／PNG M4A 經兩個所選服務的明確 mock、16／24 kHz 前處理、ASR、詞彙、Standard Draft／Review 到 RAM Ready。詞彙及翻譯 mock 檢查所有 source／draft／context／glossary，不得含 fixture 的標題、作者、專輯或 URL 標記；辨識契約只接收真正 decoded PCM。此項測試只證明服務介面邊界，沒有真實金鑰或雲端要求。停止／錯誤／中文字幕優先及原檔 pin 沿用。

## 仍未提供

影片、多音軌、fragmented、edit list、未知標籤、超量／多張封面、一般 metadata 變體、URL 實際媒體讀取、限五語 Auto、完整語意與播放／實際定位仍待接續。這不是一般使用者檔案／其他編碼器或真實雲端品質驗收。Phase 08 尚未結案，Phase 09 尚未具備條件；有效金鑰維持結案固定遮蔽介面手動輸入。

## 主要來源

[Apple Well-known types](https://developer.apple.com/documentation/quicktime-file-format/well-known_types) 定義 UTF-8、JPEG、PNG 及有號整數的 type；[Value atom](https://developer.apple.com/documentation/quicktime-file-format/value_atom) 說明四 bytes type／locale 及多值形式。本產品目前接受單值，沒有聲稱完整 QuickTime metadata 支援。

[GPAC 的專案文件](https://github.com/gpac/gpac/wiki/mp4box-other-opts) 列出常見 M4A tag 名稱，以及 track／disk、tempo、compilation 的意義；格式型別與 bounds 仍由本次產品門檻核對。沒有安裝或呼叫 GPAC，也沒有下載外部媒體。

後續第二十二批的有限 H.264 Baseline／AAC 影片輸入見 [ADR 0055](0055-bounded-h264-aac-mp4-prepared-input.md)；本文件記錄第二十一批的單音軌決策與當時限制。
