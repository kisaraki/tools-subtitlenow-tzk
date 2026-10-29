# ADR 0063：完整 tx3g 軌的有限 edit list 延遲

日期：2026-10-16。狀態：Phase 08 第三十批採用。

後續：第三十一批擴大單一正常速度文字 edit 的頭尾區間裁切，見 [ADR 0064](0064-single-interval-tx3g-text-crop.md)；下列完整軌道限制記錄當批行為。

## 問題與決策

前批已能探索並選擇多條 tx3g，但任何 edts 都拒收，包括只宣告字幕延後開始的容器。本批在文字軌接受有限 edts／elst，將靜態容器時間解釋成 Prepared 文件時間；不建立播放、動態對齊、seek／drift 或 Phase 09 功能。

子集合為 version 0、flags 全零、精確長度的 elst；只允許一筆完整正常 edit，或一筆前置 empty edit 加完整正常 edit。正常項 media_time=0、rate=1、duration>0；前置空白 media_time=-1、rate=1、duration>0。只允許完整原文，不做頭尾裁切、重播、dwell、變速、未知版本、開放長度或不明 atom。

elst duration 用 movie timescale、media_time 用文字 media timescale，不能直接相加。此語意依 [Apple edit list](https://developer.apple.com/documentation/quicktime-file-format/edit_list_atom/edit_list_table) 與 [edit list 範例](https://developer.apple.com/documentation/quicktime-file-format/playing_with_edit_lists)；[W3C ISO BMFF note](https://www.w3.org/TR/mse-byte-stream-format-isobmff/#iso-init-segments) 另有單一 rate=1 的時間映射敘述，但它的 MSE／fragmented 要求不等同本機容器驗收範圍。

版本 0 每筆 12 bytes 的欄位布局亦核對 [GPAC 官方 elst reader](https://github.com/gpac/gpac/blob/master/src/isomedia/box_code_base.c#L1456)；本機 rate=1 檢查值 `0x00010000` 可與 [GPAC 的正常／empty edit 寫入](https://github.com/gpac/gpac/blob/master/src/isomedia/isom_write.c#L2901) 交叉核對。這些是格式依據，並未加入 GPAC 相依或宣稱其 muxer 已實測。

## 完整長度與時間換算

有 edit 時，完整 mdhd duration 乘 movie timescale 必須能整除文字 timescale，active duration 必須完全相等。tkhd duration 必須等於前置空白與 active duration 總和，mvhd 必須涵蓋。若僅以 floor／ceil 接受 active duration，可能真的裁去或延長最後一份文字，故這批明確拒收不能精確表示的完整長度。無 edts 仍保留既有 tkhd floor／ceil 規則，不改原始文字 ticks。

有 edit 的呈現端點以 checked u128 有理數一次計算：

```text
floor((delay_movie_ticks * text_timescale + media_tick * movie_timescale)
      * 1_000_000 / (movie_timescale * text_timescale))
```

每份樣本的絕對起訖各自換算，duration 是兩者差，避免累積逐段取整誤差。也不能把 floor(delay_us) 加到 floor(media_us)，兩個小數相位可能相差一微秒。映射後交給既有 Tx3gTrack，使用 1_000_000 timebase；無 edit 繼續原 timebase。原 Timing 仍按 stts 核對全部媒體 ticks／duration，空字串清除、序號、Session 與全文驗證不變。包含前置空白的呈現端點也受原 24 小時上限限制。

## 邊界與所有權

每一條文字軌，包括未選軌，都先驗證 edit；任一錯誤不返回部分 catalog 或 Ready。A/V 仍送進原 validate_track，繼續拒絕 edit list，不偷偷改動原生音訊時鐘。八條文字軌／總十軌、moov、sample／wire／text／cue、cross-track byte range 及原檔 pin 規則保持。

helper 只在既有 Prepared CPU lane 做有界純計算，不新增 worker、queue、鎖、COM、unsafe 或相依套件。取消檢查沿用 read／track／sample 邊界，無 provider、金鑰讀取、音訊解碼、媒體／文字寫入、temp／cache／export。映射後的完整 ExistingText 沿用同一詞彙／品質翻譯／本地化及 RAM Ready，原始呈現時間與同一檔案 pin 保留到清除。

## 驗證與限制

helper 測試涵蓋無 edit 舊 rounding、identity／前置空白、不同時間尺度的合併端點相位、長段不累積 drift、24 小時、樣本超界、錯誤結構／rate／版本及取消。容器 fixture 核對原 clear gap、兩文字軌不同 delay、未選壞 edit、完整檔案截斷、A/V 仍拒收與超限；小數端點範例確實能揭露分別 floor 早一微秒的錯誤。

正式 controller 核對三種中文不建服務、兩個所選服務／三種品質的明確 mock，以及 Ready 呈現時間／pin；非法 crop 不分析、不翻譯、不 ASR fallback。hidden native fixture 完成探索→中文宣告→真正本機 Ready→清除，來源文字另驗證雙服務用途揭露及 synthetic key 隔離。結果見 [Phase 8](../phase-8.md)。

這些是合成容器／實際文字 bytes／mock 服務證據，codec bytes 不是一般影片相容性證明。仍不接受一般裁切／version 1／A/V edit／composition／tx3g modifier；真實 muxer／影片、其他格式、URL／Auto、完整語意、播放定位與真實雲端仍待接續。Phase 09 尚未具備條件；有效金鑰仍留至結案固定遮蔽介面手動輸入。
