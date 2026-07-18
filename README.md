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
- **119 tools** — generic document CRUD, session-time GM tools (show journals to
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

## The 119 tools

### Generic (66) — work with any game system

| Category | Tools | Notes |
|---|---|---|
| **Read** | `get_actors`/`get_actor`, `get_items`/`get_item`, `get_journals`/`get_journal`, `get_scenes`/`get_scene`, `get_folders`, `get_users`, `get_macros`, `get_cards`, `get_playlists`, `get_tables`, `get_combats`, `get_messages`, `get_settings` (+ singular forms) | `where` filters with dotted paths & operators (`__in`, `__contains`, `__ne`, `__exists`), field projection, `offset`/`limit`/`max_length`, automatic DB-index for light listings |
| **Write** | `create_document`, `modify_document`, `delete_document` | embedded documents via `parent_uuid`, compendia via `pack`, `keep_id` |
| **Compendia** | `list_compendium_packs`, `get_pack_documents`, `import_from_compendium`, `create_compendium`, `delete_compendium` | pack reads use the DB index too |
| **Files** | `browse_files`, `create_directory`, `upload_file` (URL or base64) | |
| **Session (GM)** | `show_journal_to_players`, `share_image`, `toggle_pause`, `activate_scene`, `get_current_scene`, `pull_users_to_scene`, `list_tokens`, `place_token`, `move_token`, `update_token`, `toggle_actor_condition` (27 core statuses), `manage_combat` (create/initiative/turns/status/end), `control_playlist`, `draw_from_table` (d100 crit tables & co) | |
| **CC-family addons** | Campaign Codex, Asset Librarian, Mini Calendar — see the dedicated section below | [wgtnGM](https://campaigncodex.wgtngm.com/) suite |
| **Events** | `get_events` (incremental polling), `wait_for_message` (blocking wait for another client's chat message) | |
| **Misc** | `ping` (cheap health), `get_world`, `search_journals` (full text), `export_journals` (Markdown), `set_setting`, `list_actor_ownership`, `set_actor_ownership`, `show_credentials`, `choose_foundry_instance` | |

### Game-system modules (12)

| System | Tools |
|---|---|
| **Star Wars FFG** (`starwarsffg`) | `roll_actor_skill` (the real dice pool **derived from the sheet** — stored values + species/equipment/learned-talent modifiers), `roll_ffg_pool` (server-side narrative dice, official faces), `request_player_roll` (chat button that opens the pre-filled roll dialog), `adjust_actor_stats` (wounds/strain/credits/XP/obligation/duty/morality + vehicles hull/system strain), `adjust_destiny` (destiny pool), `grant_xp`, `apply_critical_injury` (+10 per existing injury, attaches the compendium item) |
| **D&D 5e** (`dnd5e`) | `dnd5e_roll_check` (ability/skill/save, modifiers derived from the sheet, advantage/disadvantage, DC, nat 20/1), `dnd5e_adjust_stats` (hp clamped to max, temp hp, xp, exhaustion, currency) |
| **Daggerheart** (`daggerheart`) | `dh_roll_duality` (2d12 Hope/Fear, doubles = critical, ±d6 advantage), `dh_roll_actor_trait`, `dh_adjust_stats` (hit points/stress/hope, clamped) |

All modules are loaded by default; restrict with `FOUNDRY_SYSTEMS=starwarsffg,dnd5e`.

### CC-family addons — the [wgtnGM](https://campaigncodex.wgtngm.com/) suite

Grouped tools for the Campaign Codex family of modules. Server-side tools work
on documents; the `client_*` ones need the optional companion module.

| Addon | Tools |
|---|---|
| **[Campaign Codex](https://foundryvtt.com/packages/campaign-codex)** | `cc_list_sheets`, `cc_get_sheet`, `cc_create_sheet`, `cc_link` (bidirectional) · companion: `client_cc_convert` (journal → CC sheet, bulk migration), `client_cc_export_obsidian`, `client_cc_open_toc` |
| **Asset Librarian** | `al_tag` / `al_find` (read & write `flags.asset-librarian` tags on documents) · companion: `client_al_open` (open the filtered asset browser) |
| **Mini Calendar** | `mc_get_time` / `mc_set_time` (world time via `core.time`), `mc_list_notes` (the calendar note journals) · companion: `client_mc_set_time` (incl. dawn/dusk), `client_mc_open` |

### Other addon integrations

| Addon | Tools |
|---|---|
| **[Monk's Active Tile Triggers](https://foundryvtt.com/packages/monks-active-tiles)** | `mat_list` (trigger-tiles of a scene) · companion: `client_mat_trigger` (fire a tile's action chain — teleport, scene change, macros…) |
| **[Sequencer](https://foundryvtt.com/packages/sequencer)** | companion: `client_play_effect` (at a token/point), `client_seq_between` (effect from one token to another — attacks/projectiles), `client_seq_sound` |

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

## Client-side actions (optional companion) — 26 tools

The socket protocol can only touch *documents*. To reach the browser-only client
API, install the optional **[foundry-mcp-gateway-companion](https://github.com/wanoo/foundry-mcp-gateway-companion)**
module. It adds the `client_*` tools. Without it they simply time out with a
clear message; everything else works unchanged.

| Tool | What it does (client-side) |
|---|---|
| `client_status` | Is the companion installed/active? Returns its version, the responding GM, and which optional deps (Dice So Nice, Campaign Codex, Sequencer) are available |
| `client_run_macro` | Run any Foundry macro by _id or name on the GM client (returns its value) — the universal key to anything scriptable |
| `client_run_script` | Run arbitrary async JS on the GM client (⚠️ off by default; the GM must enable it in the module settings) |
| `client_roll_pool_native` | *starwarsffg*: roll a pool with the real FFG engine — native chat card + **Dice So Nice** 3D dice on the table |
| `client_pan_camera` | Pan/zoom the targeted clients' cameras — "everyone look here" (x/y or a token _id) |
| `client_ping` | Ping a point on the map for the targeted clients |
| `client_play_sound` | One-shot sound (a dramatic stinger) on the targeted clients, no playlist needed |
| `client_notify` | UI notification (info/warn/error) on the targeted clients |
| `client_show_document` | Open a document sheet (by uuid) on the targeted clients |
| `client_play_effect` | A [Sequencer](https://foundryvtt.com/packages/sequencer) visual effect at a token or point (if installed) |
| `client_get_state` | Telemetry: active users, each one's viewed scene & character. Live selections/targets stream via `get_events` |

Scene actions (`client_pan_camera`, `client_ping`, `client_play_sound`,
`client_notify`, `client_show_document`) accept a `targets` argument
(`all` / `gm` / `players` / list of user _ids). Optional integrations degrade
gracefully: `client_roll_pool_native` needs starwarsffg, `client_play_effect`
needs Sequencer, `client_cc_*` need Campaign Codex.

### Perception, table & ambience (15 more, companion ≥ 0.4.0)

The newer companion tools, grouped by what they unlock:

| Category | Tool | What it does (client-side) |
|---|---|---|
| **Perception** | `client_get_derived` | THE reliable sheet read: the PREPARED values (after `prepareData` + active effects) — source documents often store 0 where the player sees the real stat |
| | `client_enrich` | Enriched HTML of a document or journal pages: `@UUID` links resolved, inline rolls evaluated, GM secrets |
| | `client_search` | Name search across every world collection via the client index (returns uuids) |
| | `client_capture` | Screenshot of the GM's canvas view, returned as a real MCP image — the AI literally sees the table |
| | `client_scene_report` | Playable scene state: tokens with grid coords, disposition and REAL visibility, doors open/closed, lights, templates, selection, targets |
| | `client_babele` | [Babele](https://foundryvtt.com/packages/babele): the TRANSLATED view of compendia — reverse search by displayed OR source name (find « Force Lightning » from « Éclair de Force »), translated index, translated document(s) |
| **Table** | `client_ask` | Ask a player a question in a real dialog on THEIR screen and get their answer back |
| | `client_select` / `client_target` | Real selection / crosshair targets on the GM's canvas — show what you're talking about |
| | `client_fog` | Reset the explored fog of war on the active scene |
| **Ambience** | `client_weather` / `client_weather_types` | [FXMaster](https://foundryvtt.com/packages/fxmaster): scene weather particles (rain, fog, embers, bats…) |
| | `client_token_fx` / `client_token_fx_presets` | [Token Magic FX](https://foundryvtt.com/packages/tokenmagic): filter presets on tokens (glow, fire, shadow…) |
| | `client_effect_catalog` | Sequencer: search the installed effect database (JB2A & co) to find a valid path before playing an effect |

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
