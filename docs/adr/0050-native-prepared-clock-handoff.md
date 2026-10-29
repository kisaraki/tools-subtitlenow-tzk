# ADR 0050：原生 Prepared actor 的分段時鐘移交

日期：2026-10-14

狀態：採用，Phase 08 第十五批

## 問題

MPEG-1 MP3 的原生封包時間含 decoder delay 缺口及逐 frame 取整。ADR 0048／0049 已提供取樣時鐘、不可變分段映射與原生定稿合併；正式 producer 尚須在有界解碼與分段之間安全移交證據。若先將完整 block 加入十二秒窗口，最大 64 KiB／8 kHz mono block 或長缺口會在舊分段尚未排出時超限；裁切跨界 span 則會破壞原始取樣相位。

## 決定

沿用同一個依需求拉取的 native actor、CPU lane、native owner、同執行緒 COM／MF 資源及原媒體 pin。只以內容 probe 判定 MP3 需要映射；WAVE 保留直接取樣時鐘及既有一微秒精度。一般 Prepared ASR 使用 Native 解碼，公開 borrowed DSP 仍只接受 SampleAccurate，不改變其禁止保留 PCM 的契約。

每次只暫存一個已核對完整來源身分、序號、run、格式及 frame 相位的原始 span 與其至多 64 KiB PCM。分段從 rolling rows 加這一筆 pending span 取得完整 audio／context 的不可變快照，再交付原有至多兩份分段回覆。快照仍限十二秒／512 rows，服務只接收取樣時鐘與音訊，定稿先驗證正文再依同一快照還原原生時間。

完成本次 DSP 推進與快照移交後，以 segment buffer 的左脈絡起點及 DSP 尚未交付的輸出起點之最小值回收 rows。私有 append_retained 以單調 watermark 限制可查詢範圍至十二秒，保留最多 512 rows 及一筆跨左界的完整原始 span。被保留 span 的較早前綴不能再查詢；不裁切端點、不插入靜音、不重設 origin。原始單一 block 最長 4.096 秒，因此實體端點範圍額外至多一個有界 block。公開窗口 append 的完整端點十二秒檢查保持原有方式。

真正 source gap 在新證據加入前先排完舊 DSP 尾段並關閉舊 segment run。原有輸入 gates 保留，下一個 push 仍必須證明真缺口；零輸出 source run 不產生假的 ASR run 或音訊。先前已有正文但最後 source run 沒有 target frame 時，EOF 可完成先前文件；全檔沒有輸出則仍明確失敗。

## 界線與結果

一般 Prepared／GUI 可使用內容核對通過的本機 MPEG-1 MP3；字幕仍優先。任何容量、取消、時間或來源錯誤均終止當次準備，沒有部分 Ready。成功 EOF 必須等待 native owner 真正退出，原媒體 pin 保留至本次 Ready 清除；工作 idle 等候辨識時不占 CPU 許可。

不增加產品媒體寫入、暫存檔、快取、匯出、播放、不同服務 fallback 或新執行緒模型。測試只使用 RAM 產生的自有合成 MP3、RAII 測試輸入及明確 mock，不能代表真實 VBR／使用者檔案、雲端準確率或播放定位。Auto、其他容器、URL、全文詞彙及播放仍屬 Phase 08 待辦，Phase 09 尚未具備條件；真實金鑰留到結案固定遮蔽介面手動輸入。

實際驗證指令與結果見 [Phase 8](../phase-8.md)。
