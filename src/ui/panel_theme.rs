//! A display preference only; independent of subtitle rendering and providers.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum PanelTheme {
    #[default]
    Light,
    Dark,
}
