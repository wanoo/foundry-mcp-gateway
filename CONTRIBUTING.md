# Contributing to foundry-mcp-gateway

Two extension points cover almost everything: **game systems** and **addon
integrations**. Both are plugins — the core never needs touching.

*(🇫🇷 : les contributions sont bienvenues en français — code et docs ; les
descriptions d'outils restent en anglais, c'est ce que lisent les modèles.)*

## Dev setup

```sh
git clone https://github.com/wanoo/foundry-mcp-gateway && cd foundry-mcp-gateway
cargo test                       # 120 unit tests, no Foundry needed
./scripts/check-docs.sh          # docs must match the code (see below)
foundry-mcp --dump-tools | jq    # the full tool catalogue, no connection needed
```

Run it against a world:

```sh
eval "$(cargo run -q -- --generate-mcp-key | grep MCP_ROOT_PRIVATE_KEY)"
export MCP_ROOT_PRIVATE_KEY
TOKEN=$(cargo run -q -- --mint-token dev | grep -v '^#' | tr -d '\n')
PORT=8940 \
FOUNDRY_CREDENTIALS_JSON='[{"_id":"dev","hostname":"…","userid":"…","password":"…"}]' \
cargo run
# then: claude mcp add foundry-dev --transport http http://localhost:8940/mcp-$TOKEN
```

### A world to test against

**Don't develop against a campaign you care about.** Point the gateway at a
throwaway world instead — `"hostname": "localhost:30000"` works as-is, plain
HTTP and non-standard ports included, so a Foundry on your own machine or LAN
needs no extra setup.

**No Foundry licence?** You can still contribute. What works with no world at
all:

| You can | How |
|---|---|
| Add a game system or a tool | `cargo test` — dice engines take an injectable RNG, derived-stat logic is pure |
| Inspect the tool catalogue | `foundry-mcp --dump-tools \| jq` |
| Check registry consistency | `cargo test chaque_outil_a_un_seul_proprietaire` |
| Check the docs | `./scripts/check-docs.sh` |

What you genuinely can't do offline is confirm a *third-party module's* runtime
API behaves as you assumed. Say so in your PR — "not tested against a real
world" is an accepted answer, and it tells the reviewer exactly what to run.

### Docs are checked, not trusted

`./scripts/check-docs.sh` compares each language's README and user guides against
what the binary actually exposes: total, base, read-only and companion tool
counts, unit-test count, tools that exist but aren't documented, and tools
documented but non-existent. Each translation must cover the complete catalogue,
including singular reads. Keep the [English](docs/capabilities.md) and
[French](docs/capabilities.fr.md) catalogues aligned when adding tools. The check
runs in CI independently of the developer's runtime environment. This exists
because those numbers drifted three times — the READMEs once announced 131
*and* 134 tools on the same page.

## 🎮 Adding a game system

Everything about a system lives in **one file**: `src/systems/<system_id>.rs`
(use the exact Foundry system id, e.g. `pf2e`).

```rust
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![("pf2e_roll_check", "Roll a check … ", json!({ /* JSON-schema */ }))]
}
pub fn handles(name: &str) -> bool { name.starts_with("pf2e_") }
pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> { … }
```

1. **Prefix every tool name** with your system id.
2. Register the module in `src/systems/mod.rs` (`all_modules()`).
3. **Dice engines take an injectable RNG closure** so unit tests are
   deterministic — see `swffg_dice.rs`.
4. If your system's *stored* documents differ from what players *see*
   (stats at 0, modifiers in items/talents), either derive server-side
   (see `swffg_derived.rs`, the reference implementation) or lean on the
   companion's `client_get_derived`, which works for every system.
5. `src/systems/README.md` has the full walkthrough.

## 🧩 Adding an addon integration

Ask first: **does this need a browser?** If the addon's data lives in documents
(flags, settings, journal content), a plain server-side tool in `src/tools/`
is enough — no companion code. Tagger and status counters needed nothing but
`where` filters on flags, for instance.

If it needs the addon's client API, you write a **pair**:

| Side | File | What |
|---|---|---|
| Server (Rust) | `src/tools/<theme>.rs` | Tool definition + `call_companion(state, "cmd", args, targets, timeout)` |
| Companion (JS) | `scripts/addons/<theme>.mjs` in [the companion repo](https://github.com/wanoo/foundry-mcp-gateway-companion) | `export const X_HANDLERS = { async cmd(args) { … } }`, merged in `main.mjs` |


Add the pairing to `COMPANION_COMMANDS` in `src/tools/companion.rs` and call `call_companion(state, cmd_for(name), …)` rather than a literal — that table is the single source of truth, exported to `docs/companion-commands.json` and checked against the companion's handlers by its `check-parity.sh`.
**Pick the command's delivery category** (in the companion's `main.mjs`):

- **scene** — every targeted client runs it, one ack (notifications, camera, sounds);
- **addressed** — the *targeted* client runs it **and answers** (`client_ask`);
- **unique** — only the elected GM responder runs it (default: API calls, rolls).

Rules of the road:

- Degrade gracefully: `if (!game.modules.get("x")?.active) throw new Error("x module not active")`.
- **Probe the addon's real API in a live world before writing** (`client_run_script`
  is perfect for this). We dropped FXMaster's scene filters because v13 exposes no
  hook for them — better no tool than a guessed one.
- Names: `<addon>_*` for server-side tools, `client_<addon>_*` for delegated ones.
- Read-only tools: add them to the `annotations()` list in `src/tools/mod.rs`.

## 📤 Pull requests

- **Commits in English, [Conventional Commits](https://www.conventionalcommits.org/)**:
  `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `ci:`. Scope welcome
  (`feat(systems): add pf2e`). Older commits are French and free-form — that
  changed with the public release, don't imitate them.
- **Versions track the companion module** on major.minor: 1.5.x on both sides.
  A protocol change (a new `client_*` tool, a changed payload) bumps the minor on
  *both* repositories. A fix confined to one half is a patch release there alone —
  `client_status` only flags a drift when major.minor differ.
- One logical change per PR. The template asks how you tested it — answer
  honestly, "not tested against a real world" is a valid answer that saves the
  reviewer time.
- CI must be green: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, and `./scripts/check-docs.sh`.

## ✅ Definition of done

- [ ] `cargo test` green (add tests for any pure logic).
- [ ] Tool descriptions in **English**, written for a model: say *when* to use the
      tool, not just what it does.
- [ ] **Tested as far as you can.** With a licence: validate against a real
      world and note the Foundry + system/addon versions in your PR. Without
      one: say "not tested against a real world" — that is an accepted answer.
      What is never accepted is silence about it. Anything touching a
      third-party module's runtime API needs one live run before it *merges*,
      by you or by a reviewer; that's a merge gate, not a PR gate.
- [ ] README tables updated (EN **and** FR) + the total tool count.
- [ ] `./scripts/check-docs.sh` green (it will tell you which number to fix).
- [ ] Companion changes: bump `module.json` + `VERSION` in `main.mjs`; releases
      are automated — pushing a new version number to `main` publishes it.

## 🌍 Translations

The server is language-neutral. GM-facing strings live in the companion's
`lang/*.json` — new locales welcome. For translated *compendium content*,
`client_babele` already speaks [Babele](https://foundryvtt.com/packages/babele).
