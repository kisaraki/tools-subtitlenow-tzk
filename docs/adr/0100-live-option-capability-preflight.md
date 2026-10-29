# ADR 0100：Live 語言選項能力與啟動前檢查

日期：2026-10-25。狀態：採用，Phase 11 人工介面修正。

## 原因

使用者確認目前 Live Translate 暫停／恢復、最後一行、自動跟隨、人工回看及停止保留均正常，但選「自動偵測」會得到檢查來源或服務的錯誤。既有模型驗證已拒絕 Native Live Translate／GPT 的 Auto，UI 卻提供相同 Auto 選項並直到背景啟動才回報；狀態框的概括錯誤容易被誤認為來源或金鑰失效。

目前應用程式的五語 Auto 只接於 Gemini Transcribe Live。GPT 不提供可依既有契約核對的偵測結果；Native Live Translate 的原文與中文流沒有可靠的共同段落關聯，不能把廣泛語言偵測能力冒充本程式已驗證的五語 Auto。此限制是目前整合界線，不是宣稱 Gemini Live Translate API 本身不會偵測語言，也不在本次新增另一條辨識管線或模型 fallback。

## 決定

以 LiveSpeechModel 的 supports_auto_detection 表達同一既有能力，validate_live 繼續沿用原規則。Gemini Transcribe Live 的語言索引零顯示「自動偵測」；其餘兩模型顯示「請手選語言」。保留六個索引、已選的英／日／法／德／韓及零值偏好，切換模型只更新第一項文字，不暗選英文或改服務。儲存偏好不代改選項，舊 Native＋Auto 偏好仍載入為待手選狀態。

ControlPanel.request 先核對模型、來源語言與品質，才允許 shell 讀取 key 或建立 controller。開始按鈕使用同一 request 判斷，無效組合停用；直接 posted command 仍受同一檢查，不依賴按鈕狀態。Gemini 無音訊診斷亦在讀取 Gemini 欄位前檢查實際所選 Gemini 模型／語言，初始化只使用 Standard，保留既有診斷目的。

不繁忙時，主頁把明確選項提示放在狀態最前方，並說明若要 Auto 可手動選 Gemini Transcribe Live；不把不支援組合呈現成一次新的擷取或服務故障。繁忙時仍顯示實際工作階段，選擇器保持鎖定，不以編輯中的選項覆蓋活動來源。固定雙列金鑰儲存與本機檢查不依賴來源語言，能力檢查不讀、清空或搬移金鑰。

## 界線與驗證

本次沒有新增 Auto 辨識實作、網路要求、模型、權限、設定 schema、fallback 或依賴；不修改既有 capture／時鐘／佇列／Session／翻譯／字幕流程。六個語言索引保持，原品質策略與能力限制保持。

隱藏原生視窗測試涵蓋三模型、五種明選語言、Auto 與模型切換、來源索引及韓文選取保留、存量 Native＋Auto 偏好、主頁提示、開始停用、直接開始／診斷繞過阻擋，以及無後端呼叫且合成金鑰不被讀取消耗。既有 OpenAI 啟動 fixture 改成符合已存在後端契約的明確英文，金鑰鎖定與定位資源斷言不減少。

執行結果及首次 fixture 失敗如實記入 [Phase 11 診斷](../phase-11-gemini-diagnostics.md)。本批選項與 Win32 操作以相關模組、Phase 11 定向及格式／Clippy／Debug／Release 驗證；未修改的原生媒體完整矩陣沿用前批紀錄，不冒稱新完整測試。真實五語 Auto 仍需使用者手動選 Gemini Transcribe Live 測試，不把模型設定成功當作雲端正例。
