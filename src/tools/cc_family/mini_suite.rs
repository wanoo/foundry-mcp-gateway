//! Petits modules wgtnGM : Mini Countdown Dice, Mini Light Switch, Mini Player,
//! Mini Rolltable (1.7).

use anyhow::{Result, bail};
use serde_json::Value;

use crate::mcp::McpState;
use crate::tools::ToolDef;
use crate::tools::companion::Delivery;

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

/// Outils `client_*` de ce module → commande compagnon (fusionnés dans
/// `companion::COMPANION_COMMANDS`).
pub const COMMANDS: &[(&str, &str, Delivery)] = &[];

/// Outils sans effet de bord (annotations + mode FOUNDRY_READONLY).
pub const READ_ONLY: &[&str] = &[];

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    vec![
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let _ = (state, args);
    bail!("Unknown tool: {name}")
}
