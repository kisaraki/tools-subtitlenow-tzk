//! Phase 1 diagnostic text view. Does not own native interfaces or a processing pipeline.

use std::io::{self, Write};

use crate::audio::sessions::{DiscoveryUpdate, ProcessScope, SessionState};

pub const HELP: &str = "SubtitleNow — Windows 程序音訊與串流辨識\n\n  --ui                 開啟互動字幕視窗；選來源、服務、語言及品質後手動開始\n  --list-sources       列出目前音訊工作階段後結束\n  --watch-sources      持續更新來源與音量；按 Ctrl+C 停止\n  --capture-pid PID    僅擷取指定程序及其子程序，不使用雲端\n  --stt-pid PID [--provider openai|gemini] [--language auto|en|ja|fr|de|ko] [--interactive-key]\n                      將指定來源送至所選服務，顯示 Partial／Final；按 Ctrl+C 停止\n  --translate-pid PID [--provider openai|gemini] [--language auto|en|ja|fr|de|ko]\n                      [--quality fast|standard|high] [--interactive-key]\n                      以單一時間軸顯示原文及臺灣繁中翻譯\n  --translate-window-pid PID [--provider openai|gemini] [--language auto|en|ja|fr|de|ko]\n                      [--quality fast|standard|high] [--interactive-key]\n                      以字幕視窗翻譯指定來源；選單停止或關閉視窗會回收後端\n  --set-stt-key openai|gemini    隱藏輸入金鑰並儲存至 Windows 認證管理員\n  --check-stt-key openai|gemini  檢查金鑰是否可讀取，不連線\n  --help              顯示說明\n\n省略辨識供應商／語言時使用設定；未選供應商時回報錯誤。--interactive-key 在開始時隱藏提示輸入金鑰，僅供本次使用、不寫入認證管理員。API 金鑰不能作為命令列參數或放入設定。\n使用 --list-sources 選擇獨立程序 PID；擷取能力依所選來源的 Windows API 結果判定。\n音訊僅在 RAM 處理、不儲存。--stt-pid／--translate-pid 使用雲端且只接受互動式終端機，不提供逐字稿匯出。\nAuto 僅接受五語；未取得語言回報的 Partial 會暫緩顯示。切換來源請停止後重新指定 PID。\n未指定參數時僅檢查設定，不擷取、不連線。翻譯品質預設使用設定（Standard）。--ui 唯讀探索來源，不自動擷取或連雲端；可停止後重新選取及開始。--translate-window-pid 顯示來源提示後才開始連線；視窗模式不輸出逐字稿到終端機。真實服務測試留至結案。\n";

/// Never emit application-controlled terminal escape/control characters.
fn clean(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn text(value: &Option<String>) -> String {
    value
        .as_deref()
        .map(clean)
        .unwrap_or_else(|| "無法取得".into())
}

pub fn write_update(output: &mut impl Write, update: &DiscoveryUpdate) -> io::Result<()> {
    writeln!(
        output,
        "\n音訊來源更新 #{}（探索 {}，{:.1} 秒）",
        update.sequence,
        update.run_id,
        update.elapsed.as_secs_f64()
    )?;
    let report = match &update.result {
        Ok(report) => report,
        Err(error) => {
            writeln!(output, "{error}；持續探索時會在下一次更新重試。")?;
            return output.flush();
        }
    };
    writeln!(
        output,
        "播放裝置：{}；工作階段：{}；尚未擷取音訊",
        report.device_count,
        report.sources.len()
    )?;
    if report.is_partial() {
        writeln!(
            output,
            "部分資料無法取得：裝置 {}、工作階段 {}、視窗 {}、通知遺失 {}、達數量上限 {}",
            report.unavailable_devices,
            report.skipped_sessions,
            report.windows_unavailable,
            report.notifications_lost,
            report.limit_reached
        )?;
    }
    if report.sources.is_empty() {
        writeln!(output, "目前沒有可顯示的音訊工作階段。")?;
    }
    for source in &report.sources {
        let scope = match source.process_scope {
            ProcessScope::SingleProcess => "一般程式",
            ProcessScope::MultipleProcesses => "跨程序（PID 為初始建立者）",
            ProcessScope::SystemSounds => "系統音效（不是可選程式）",
        };
        let state = match source.state {
            SessionState::Active => "串流執行中",
            SessionState::Inactive => "閒置",
        };
        let signal = match source.signal_present() {
            Some(true) => "有訊號",
            Some(false) => "無訊號",
            None => "未知",
        };
        let peak = source
            .peak
            .map(|peak| format!("{:.1}%", peak * 100.0))
            .unwrap_or_else(|| "無法取得".into());
        let mute = match source.muted {
            Some(true) => "是",
            Some(false) => "否",
            None => "未知",
        };
        writeln!(
            output,
            "{} | PID {} | {} | {} | {} | 峰值 {} | 靜音 {}",
            text(&source.application_name),
            source.process_id,
            scope,
            state,
            signal,
            peak,
            mute
        )?;
        writeln!(
            output,
            "  裝置：{}；視窗：{}；圖示：{}",
            text(&source.endpoint_name),
            text(&source.window_title),
            if source.icon.is_some() {
                "已取得"
            } else {
                "無法取得"
            }
        )?;
    }
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_terminal_control_characters() {
        assert_eq!(clean("視窗\x1b[2J\n\r\t\0標題"), "視窗 [2J    標題");
    }
}
