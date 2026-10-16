//! Préparation de scène et de butin — un sous-module par addon :
//!   · Monk's Active Tiles (mat_*) tuiles-actions = Tile + flags["monks-active-tiles"]
//!   · Item Piles        (ip_*)  piles, coffres, marchands (via le compagnon)
//!   · Mass Edit         (me_*)  presets à poser sur une scène (via le compagnon)
//! Chaque sous-module déclare ses outils, ses commandes compagnon et ses
//! outils en lecture seule ; ce fichier ne fait que les assembler.

use anyhow::{Result, bail};
use serde_json::Value;

use super::ToolDef;
use super::companion::Delivery;
use crate::mcp::McpState;

pub mod active_tiles;
pub mod item_piles;
pub mod mass_edit;

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(|| {
    active_tiles::definitions()
        .iter()
        .chain(item_piles::definitions())
        .chain(mass_edit::definitions())
        .cloned()
        .collect()
});

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

/// Commandes compagnon de toute la suite (fusionnées dans COMPANION_COMMANDS).
pub fn commands() -> impl Iterator<Item = &'static (&'static str, &'static str, Delivery)> {
    active_tiles::COMMANDS
        .iter()
        .chain(item_piles::COMMANDS)
        .chain(mass_edit::COMMANDS)
}

pub fn is_read_only(name: &str) -> bool {
    [
        active_tiles::READ_ONLY,
        item_piles::READ_ONLY,
        mass_edit::READ_ONLY,
    ]
    .iter()
    .any(|l| l.contains(&name))
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    if active_tiles::handles(name) {
        active_tiles::run(state, name, args).await
    } else if item_piles::handles(name) {
        item_piles::run(state, name, args).await
    } else if mass_edit::handles(name) {
        mass_edit::run(state, name, args).await
    } else {
        bail!("Unknown tool: {name}")
    }
}
