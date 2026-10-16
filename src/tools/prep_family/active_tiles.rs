//! Monk's Active Tiles — création et édition de tuiles-actions (mat_*)

use anyhow::{Result, bail};
use serde_json::Value;

use crate::mcp::McpState;
use crate::tools::ToolDef;
use crate::tools::companion::Delivery;

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

pub const COMMANDS: &[(&str, &str, Delivery)] = &[];

pub const READ_ONLY: &[&str] = &[];

#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    vec![]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

pub async fn run(_state: &McpState, name: &str, _args: &Value) -> Result<Value> {
    bail!("Unknown tool: {name}")
}
