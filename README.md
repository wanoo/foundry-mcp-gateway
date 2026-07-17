# foundry-mcp-gateway

**🇫🇷 [Version française](README.fr.md)**

An independent **MCP** (Model Context Protocol) server for **Foundry VTT**, written
in Rust. One single binary that logs into your Foundry world **like a regular
player** (native socket client — no module to install in Foundry, no browser to
keep open) and exposes it to any MCP client: Claude Code, Claude Desktop, or
anything speaking MCP over streamable HTTP.

Let your AI assistant read your journals, roll dice, run combats, manage your
compendia, prep your sessions — 24/7, as long as your world is up.

## Highlights

- **Zero footprint on Foundry** — connects through the same socket protocol the
  official client uses. Works on Foundry **v13 and v14** (session binding is
  auto-detected), including instances served under a route prefix
  (`my-host.com/my-world`).
- **One tiny binary** — Rust, a few MB of RAM. Runs happily on the smallest
  cloud instances.
- **Fast** — per-collection reads (never a full world dump except `get_world`),
  server-side query pushdown, database-index listings: ~7,000 journals listed
  in ~0.3 s.
- **78 tools** — generic document CRUD, session-time GM tools (show journals to
  players, combats, playlists, tokens…), plus **game-system modules**
  (Star Wars FFG, D&D 5e, Daggerheart) that anyone can extend.
- **A good MCP citizen** — tool annotations (read-only/destructive), paginated
  resources, prompts, subscriptions, SSE notifications.
- **Self-healing** — infinite reconnection with backoff; survives world restarts
  and even a v13→v14 server upgrade mid-flight.

## Installation (step by step, no wizardry required)

### 1. Create a dedicated Foundry user for the bot

In Foundry, as gamemaster: **Configure Players → Create Additional User**. Name it
e.g. `MCP-Bot`, give it the **Gamemaster** role (or less if you want a restricted
bot) and a **password**.

You now need that user's **16-character `_id`** — the easiest way once the server
runs is the `show_credentials`/`get_users` tools, but for the first setup: open
your world's login page, pick MCP-Bot in the dropdown, and look at the page
source — or run this once from any machine:

```sh
curl -s https://YOUR-HOST/join | grep -o '{"name":"MCP-Bot"[^}]*'
# → ..."_id":"AbCdEfGh12345678"...
```

### 2. Configure the three environment variables

```sh
# The secret that protects your endpoint (URL path: /mcp-<secret>)
export MCP_SECRET="a-long-random-string"

# Who to log in as, where. hostname MAY include a route prefix.
export FOUNDRY_CREDENTIALS_JSON='[{
  "_id": "my-world",
  "hostname": "my-host.com/my-world",
  "userid": "AbCdEfGh12345678",
  "password": "the-bot-password"
}]'

# Optional (default 8080)
export PORT=8080
```

Several worlds/instances? Put several objects in the array and switch at runtime
with the `choose_foundry_instance` tool.

### 3. Run the server

**Locally / any server with Rust installed:**

```sh
cargo run --release        # binary: target/release/foundry-mcp
```

**Clever Cloud (5 commands):**

```sh
clever create --type rust foundry-mcp-gateway
clever env set MCP_SECRET "a-long-random-string"
clever env set FOUNDRY_CREDENTIALS_JSON '[{"_id":"…","hostname":"…","userid":"…","password":"…"}]'
clever env set CC_RUST_BIN foundry-mcp
clever deploy
```

**Anywhere else:** build once (`cargo build --release`), copy the single
`foundry-mcp` binary, set the env vars, run it behind HTTPS.

Check it's alive: `curl https://YOUR-DEPLOYMENT/health` → `ok`.
The world must be **launched** (login page visible) for the bot to connect;
if the world is down the server waits and reconnects automatically.

### 4. Connect your MCP client

```sh
# Claude Code
claude mcp add foundry --transport http https://YOUR-DEPLOYMENT/mcp-<secret>
```

Claude Desktop: *Settings → Connectors → Add custom connector* with the same URL
(the secret lives in the URL because Desktop cannot send custom headers).

## The 78 tools

### Generic (66) — work with any game system

| Category | Tools | Notes |
|---|---|---|
| **Read** | `get_actors`/`get_actor`, `get_items`/`get_item`, `get_journals`/`get_journal`, `get_scenes`/`get_scene`, `get_folders`, `get_users`, `get_macros`, `get_cards`, `get_playlists`, `get_tables`, `get_combats`, `get_messages`, `get_settings` (+ singular forms) | `where` filters with dotted paths & operators (`__in`, `__contains`, `__ne`, `__exists`), field projection, `offset`/`limit`/`max_length`, automatic DB-index for light listings |
| **Write** | `create_document`, `modify_document`, `delete_document` | embedded documents via `parent_uuid`, compendia via `pack`, `keep_id` |
| **Compendia** | `list_compendium_packs`, `get_pack_documents`, `import_from_compendium`, `create_compendium`, `delete_compendium` | pack reads use the DB index too |
| **Files** | `browse_files`, `create_directory`, `upload_file` (URL or base64) | |
| **Session (GM)** | `show_journal_to_players`, `share_image`, `toggle_pause`, `activate_scene`, `get_current_scene`, `pull_users_to_scene`, `list_tokens`, `place_token`, `move_token`, `update_token`, `toggle_actor_condition` (27 core statuses), `manage_combat` (create/initiative/turns/status/end), `control_playlist`, `draw_from_table` (d100 crit tables & co) | |
| **Campaign Codex** | `cc_list_sheets`, `cc_get_sheet`, `cc_create_sheet`, `cc_link` (bidirectional) | for the [Campaign Codex](https://foundryvtt.com/packages/campaign-codex) module |
| **Events** | `get_events` (incremental polling), `wait_for_message` (blocking wait for another client's chat message) | |
| **Misc** | `ping` (cheap health), `get_world`, `search_journals` (full text), `export_journals` (Markdown), `set_setting`, `list_actor_ownership`, `set_actor_ownership`, `show_credentials`, `choose_foundry_instance` | |

### Game-system modules (12)

| System | Tools |
|---|---|
| **Star Wars FFG** (`starwarsffg`) | `roll_actor_skill` (the real dice pool **derived from the sheet** — stored values + species/equipment/learned-talent modifiers), `roll_ffg_pool` (server-side narrative dice, official faces), `request_player_roll` (chat button that opens the pre-filled roll dialog), `adjust_actor_stats` (wounds/strain/credits/XP/obligation/duty/morality + vehicles hull/system strain), `adjust_destiny` (destiny pool), `grant_xp`, `apply_critical_injury` (+10 per existing injury, attaches the compendium item) |
| **D&D 5e** (`dnd5e`) | `dnd5e_roll_check` (ability/skill/save, modifiers derived from the sheet, advantage/disadvantage, DC, nat 20/1), `dnd5e_adjust_stats` (hp clamped to max, temp hp, xp, exhaustion, currency) |
| **Daggerheart** (`daggerheart`) | `dh_roll_duality` (2d12 Hope/Fear, doubles = critical, ±d6 advantage), `dh_roll_actor_trait`, `dh_adjust_stats` (hit points/stress/hope, clamped) |

All modules are loaded by default; restrict with `FOUNDRY_SYSTEMS=starwarsffg,dnd5e`.

### MCP capabilities beyond tools

- **Resources**: browse actors (JSON) and journals (HTML, Campaign Codex data
  attached) with cursor pagination — pin them into your client's context.
- **Prompts**: `session-recap`, `world-overview`, `prep-checklist` — GM templates
  filled with live world data.
- **Subscriptions & notifications**: subscribe to a document URI and receive
  `resources/updated`; every Foundry broadcast is relayed as a logging
  notification on the SSE stream.
- **Annotations**: read-only tools are flagged so clients can auto-approve them;
  only the two `delete_*` tools are marked destructive.

## Client-side actions (optional companion)

The socket protocol can only touch *documents*. To reach the browser-only client
API — run macros, roll with the real system engine + Dice So Nice 3D, pan/ping
cameras, play stingers, drive the Campaign Codex API, read client telemetry —
install the optional **[foundry-mcp-companion](https://github.com/wanoo/foundry-mcp-companion)**
module. It adds 14 `client_*` tools (`client_run_macro`, `client_pan_camera`,
`client_roll_pool_native`, `client_cc_convert`, `client_get_state`…). Without it,
the `client_*` tools simply time out with a clear message; everything else works
unchanged. See that module's README for the full list.

## Contributing a game system

The core is 100 % system-agnostic; everything game-specific lives in
`src/systems/`, one file per system. Adding yours:

1. Create `src/systems/<system_id>.rs` exposing three functions:
   `definitions()` (tool name/description/JSON-schema triples — prefix tool
   names with your system id), `handles(name)`, and `async run(state, name, args)`.
2. Register it in `src/systems/mod.rs` (`all_modules()`).
3. **Verify your data paths against a real world** before hardcoding them
   (document shapes differ between system versions — note the version you
   validated against). Dice engines should take an injectable `rng` closure so
   tests are deterministic.
4. `cargo test` + open a PR. See `src/systems/README.md` for the full guide;
   `swffg.rs` is the reference implementation (including a sheet-derivation
   engine for systems whose source documents don't store displayed values).

## License

MIT.
