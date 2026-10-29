//! Shared value types; these preferences do not claim an implemented pipeline or UI.

use serde::{Deserialize, Serialize};

/// The translation target is fixed and cannot be overridden in configuration.
pub const TARGET_LANGUAGE: &str = "zh-TW";

/// Auto detection, once implemented, must select only among these five languages.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum SourceLanguage {
    #[default]
    Auto,
    English,
    Japanese,
    French,
    German,
    Korean,
}

/// Translation planning uses distinct debounce, context, and review policies.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum TranslationQuality {
    Fast,
    #[default]
    Standard,
    High,
}

/// Phase 5 renderers must share one transcript timeline.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum DisplayMode {
    #[default]
    SideChat,
    BottomSubtitle,
    Dual,
}
