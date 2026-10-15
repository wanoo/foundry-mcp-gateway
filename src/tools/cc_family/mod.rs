//! Suite wgtnGM (« famille Campaign Codex ») — un sous-module par addon :
//!   · Campaign Codex   (cc_*)  fiches = JournalEntry + flags["campaign-codex"]
//!   · Asset Librarian  (al_*)  tags flags["asset-librarian"] + navigateur
//!   · Mini Calendar    (mc_*)  temps (core.time), notes, météo
//!   · mini_suite               Countdown Dice, Light Switch, Mini Player…
//! Chaque sous-module déclare ses outils, ses commandes compagnon et ses
//! outils en lecture seule ; ce fichier ne fait que les assembler.

use anyhow::{Result, bail};
use serde_json::Value;

use super::ToolDef;
use super::companion::Delivery;
use crate::mcp::McpState;

pub mod asset_librarian;
pub mod campaign_codex;
pub mod mini_calendar;
pub mod mini_suite;


static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(|| {
    campaign_codex::definitions()
        .iter()
        .chain(asset_librarian::definitions())
        .chain(mini_calendar::definitions())
        .chain(mini_suite::definitions())
        .cloned()
        .collect()
});

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

/// Commandes compagnon de toute la suite (fusionnées dans COMPANION_COMMANDS).
pub fn commands() -> impl Iterator<Item = &'static (&'static str, &'static str, Delivery)> {
    campaign_codex::COMMANDS
        .iter()
        .chain(asset_librarian::COMMANDS)
        .chain(mini_calendar::COMMANDS)
        .chain(mini_suite::COMMANDS)
}

pub fn is_read_only(name: &str) -> bool {
    [
        campaign_codex::READ_ONLY,
        asset_librarian::READ_ONLY,
        mini_calendar::READ_ONLY,
        mini_suite::READ_ONLY,
    ]
    .iter()
    .any(|l| l.contains(&name))
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    if campaign_codex::handles(name) {
        campaign_codex::run(state, name, args).await
    } else if asset_librarian::handles(name) {
        asset_librarian::run(state, name, args).await
    } else if mini_calendar::handles(name) {
        mini_calendar::run(state, name, args).await
    } else if mini_suite::handles(name) {
        mini_suite::run(state, name, args).await
    } else {
        bail!("Unknown tool: {name}")
    }
}
