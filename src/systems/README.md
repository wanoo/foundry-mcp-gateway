# Game-system modules — add support for YOUR system

The server core is **system-agnostic**: documents, scenes, tokens, compendia,
playlists, combat, events… work on any Foundry world. Everything specific to a
game system (dice mechanics, sheet data paths, chat-card formats) lives here,
one module per system, so adding dnd5e-style support for another game never
touches the core.

## Layout

```
src/systems/
  mod.rs             registry (add your module to all_modules()) + dispatch
  swffg.rs           Star Wars FFG — the reference implementation (7 tools)
  swffg_dice.rs      narrative dice engine (official faces, injectable rng)
  swffg_derived.rs   sheet-derivation engine (stored values + attribute mods)
  dnd5e.rs           D&D 5e (SRD d20 engine, checks/saves/skills, stats)
  daggerheart.rs     Daggerheart (Duality Dice per the SRD, traits, resources)
```

## Writing a module

1. Create `src/systems/<system_id>.rs` (the id is Foundry's `game.system.id`,
   e.g. `"pf2e"`) exposing:

   ```rust
   pub fn definitions() -> Vec<(&'static str, &'static str, serde_json::Value)>
   // (tool_name, description, json_schema) — PREFIX tool names with your
   // system id ("pf2e_roll_check") so several modules coexist without
   // collisions. (swffg keeps its historical unprefixed names.)

   pub fn handles(name: &str) -> bool          // usually: definitions() lookup

   pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value>
   ```

2. Register it in `mod.rs`: add a `SystemModule` entry to `all_modules()` and a
   branch in `run()`.

3. **Verify your data paths against a REAL world before hardcoding them.**
   Document shapes differ between system versions — note the version you
   validated against in the module header. Beware: some systems (e.g.
   starwarsffg) display DERIVED values that are NOT stored in the source
   document; if so you need a derivation pass like `swffg_derived.rs`.

4. Dice engines must take an **injectable rng** (`impl FnMut() -> f64`) so unit
   tests are deterministic — and keep the rng OUT of `.await` regions
   (`ThreadRng` is not `Send`; scope it in a block).

5. Useful helpers from `crate::tools`: `str_arg`, `text_response`, `post_chat`
   (ChatMessage with the bot as author — required since Foundry v13),
   `roll_table_draws`; from the client (`state.foundry`): `find_document`,
   `get_documents` (fresh per-collection reads with `where` filters),
   `modify_document` (dotted-key updates, embedded docs via `parentUuid`).

6. `cargo test`, then open a PR with: the module, its tests, and a note about
   which system version you validated against.

## Loading

All bundled modules load by default. Deployments can restrict with the
`FOUNDRY_SYSTEMS` env var (comma-separated ids; empty string disables all):

```sh
FOUNDRY_SYSTEMS=starwarsffg
```
