//! Modules système : outils spécifiques à un jeu (starwarsffg, dnd5e,
//! daggerheart…), chargés selon FOUNDRY_SYSTEMS (défaut : tous). Convention :
//! outils préfixés par l'id du système (swffg garde ses noms historiques).

pub mod daggerheart;
pub mod dnd5e;
pub mod swffg;
pub mod swffg_derived;
pub mod swffg_dice;

use anyhow::Result;
use serde_json::Value;

use crate::mcp::McpState;

pub struct SystemModule {
    pub id: &'static str,
    pub definitions: fn() -> &'static [crate::tools::ToolDef],
    pub handles: fn(&str) -> bool,
}

pub fn all_modules() -> Vec<SystemModule> {
    vec![
        SystemModule {
            id: "starwarsffg",
            definitions: swffg::definitions,
            handles: swffg::handles,
        },
        SystemModule {
            id: "dnd5e",
            definitions: dnd5e::definitions,
            handles: dnd5e::handles,
        },
        SystemModule {
            id: "daggerheart",
            definitions: daggerheart::definitions,
            handles: daggerheart::handles,
        },
    ]
}

/// Résolus une seule fois : `FOUNDRY_SYSTEMS` est fixé au démarrage, et le
/// relire à chaque dispatch coûtait un accès à l'environnement par appel.
static LOADED: std::sync::LazyLock<Vec<SystemModule>> = std::sync::LazyLock::new(resolve_modules);

pub fn loaded_modules() -> &'static [SystemModule] {
    &LOADED
}

fn resolve_modules() -> Vec<SystemModule> {
    match std::env::var("FOUNDRY_SYSTEMS") {
        Err(_) => all_modules(),
        Ok(wanted) => {
            let ids: Vec<&str> = wanted
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            all_modules()
                .into_iter()
                .filter(|m| ids.contains(&m.id))
                .collect()
        }
    }
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    if swffg::handles(name) {
        swffg::run(state, name, args).await
    } else if dnd5e::handles(name) {
        dnd5e::run(state, name, args).await
    } else if daggerheart::handles(name) {
        daggerheart::run(state, name, args).await
    } else {
        anyhow::bail!("Unknown tool: {name}")
    }
}
