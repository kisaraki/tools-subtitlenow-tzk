//! Bounded app-lifetime RAM display history with UTF-16 scroll/selection anchors.
//! Historical rows never reenter recognition, translation or source arbitration.
use super::presentation::Presentation;
use crate::{ai::speech::StreamIdentity, subtitle::timeline::Stability};

struct RowRange {
    identity: StreamIdentity,
    sequence: u64,
    utterance: u64,
    start: u32,
    body_start: u32,
    end: u32,
}
pub(crate) struct SideHistory {
    pub text: String,
    epoch: Option<u64>,
    rows: Vec<RowRange>,
}
impl SideHistory {
    pub fn from_view(view: &Presentation) -> Self {
        let mut text = String::new();
        let mut rows = Vec::new();
        let mut offset = 0u32;
        // SideChat must include the latest paragraph, including untranslated tails.
        // BottomSubtitle independently retains its last readable Chinese caption.
        for row in view.rows() {
            let timestamp = row
                .timestamp_us
                .map(|us| {
                    format!(
                        "{:02}:{:02}.{:03}",
                        us / 60_000_000,
                        us / 1_000_000 % 60,
                        us / 1000 % 1000
                    )
                })
                .unwrap_or_else(|| "時間未提供".into());
            let stability = match row.stability {
                Stability::Tentative => "暫定",
                Stability::Stable => "穩定",
                Stability::Final => "完成",
            };
            let historical = if view.is_historical(row) {
                " · 先前字幕"
            } else {
                ""
            };
            let caption_only = if row.source.is_empty() && !row.translation.is_empty() {
                " · 僅譯文"
            } else {
                ""
            };
            let header = format!("[{timestamp} · {stability}{historical}{caption_only}]\r\n");
            let body_start = offset + header.encode_utf16().count() as u32;
            let item = format!("{header}{}\r\n{}\r\n\r\n", row.source, row.translation);
            // Each field is bounded by MAX_TRANSCRIPT_BYTES, with at most 64 rows;
            // the resulting UTF-16 offsets are far below u32::MAX.
            let end = offset + item.encode_utf16().count() as u32;
            rows.push(RowRange {
                identity: row.identity,
                sequence: row.sequence,
                utterance: row.utterance,
                start: offset,
                body_start,
                end,
            });
            text.push_str(&item);
            offset = end;
        }
        Self {
            text,
            epoch: Some(view.clear_epoch()),
            rows,
        }
    }
    pub fn placeholder(text: String) -> Self {
        Self {
            text,
            epoch: None,
            rows: Vec::new(),
        }
    }
    /// Map only within the same Session/generation and a retained subtitle item.
    /// Mutable tails clamp within that item; removed rows do not select other text.
    pub fn retained_position(&self, next: &Self, position: u32) -> Option<u32> {
        if self.epoch.is_none() || self.epoch != next.epoch {
            return None;
        }
        let old = self
            .rows
            .iter()
            .find(|row| row.start <= position && position < row.end)
            .or_else(|| self.rows.last().filter(|row| row.end == position))?;
        let new = next.rows.iter().find(|row| {
            row.identity == old.identity
                && row.sequence == old.sequence
                && row.utterance == old.utterance
        })?;
        if position >= old.body_start {
            Some(new.body_start + (position - old.body_start).min(new.end - new.body_start))
        } else {
            Some(new.start + (position - old.start).min(new.body_start - new.start))
        }
    }
    pub fn scroll_position(&self, next: &Self, position: u32) -> u32 {
        self.retained_position(next, position).unwrap_or_else(|| {
            // Retired/filtered anchors start at the earliest still available row.
            // Status/header scrollback keeps its absolute character offset.
            if self.epoch == next.epoch && self.rows.first().is_some_and(|row| position < row.start)
            {
                position.min(next.rows.first().map_or(0, |row| row.start))
            } else {
                next.rows.first().map_or(0, |row| row.start)
            }
        })
    }
}
