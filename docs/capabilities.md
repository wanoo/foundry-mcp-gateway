# Capabilities and integrations

[Français](capabilities.fr.md) · [Home](../README.md) · [Installation](installation.md) · [Advanced guide](advanced.md)

The 1.7 catalogue contains **166 tools**: 158 without an admin password, plus
8 administration tools unlocked by `FOUNDRY_ADMIN_PASSWORD`. There are
69 read-only tools and 46 companion tools (included in the total).
A read-only token or `FOUNDRY_READONLY=1` reduces the exposed catalogue;
`FOUNDRY_SYSTEMS` can narrow it further.

Core document tools work across game systems. Names beginning with `client_`
require the Companion and a connected, awake GM browser. Other integrations
require the matching Foundry system or module unless stated otherwise.
Optional module dependencies are listed below; missing modules return an error.
Use `foundry-mcp --dump-tools` for the schemas exposed by your deployment.

## 📖 Prep — build and query your world

| Tools | What for |
|---|---|
| `get_actors` `get_items` `get_journals` `get_scenes` `get_tables` `get_macros` `get_playlists` `get_cards` `get_combats` `get_messages` `get_folders` `get_users` `get_settings` (+ singular forms) | Read **any** collection — filters with dotted paths & operators (`__in`, `__contains`, `__ne`, `__exists`), field projection, pagination. |
| `search_journals` · `export_journals` | Full-text search · Markdown export of your whole lore |
| `create_document` `modify_document` `delete_document` | Write anything: journals, actors, scenes… embedded docs via `parent_uuid`, compendia via `pack` |
| `list_compendium_packs` `get_pack_documents` `import_from_compendium` `create_compendium` `delete_compendium` | Full compendium workflow |
| `browse_files` `create_directory` `upload_file` | File storage — upload maps & art by URL or base64 |
| `get_world` `ping` `get_setting` `set_setting` `show_credentials` `choose_foundry_instance` | World metadata, health, settings, multi-world |
| `companion_trust_key` | Install this gateway's signing key in the world, so the companion accepts its commands |

## 🎬 Run the session — the GM's remote control

| Tools | What for |
|---|---|
| `show_journal_to_players` · `share_image` | “Everyone, look at this” — handouts, art reveals |
| `activate_scene` `get_current_scene` `pull_users_to_scene` | Scene changes, drag the party along |
| `list_tokens` `place_token` `move_token` `update_token` | Token control on the live scene |
| `toggle_actor_condition` (27 core statuses) · `apply_critical_injury` | Conditions & crits |
| `manage_combat` (create / initiative / turns / end) · `get_combat` | Full combat lifecycle |
| `control_playlist` · `draw_from_table` | Music control · roll on your d100 tables |
| `toggle_pause` · `wait_for_message` · `get_events` | Pause · wait for a player's chat reply · live event feed (writes, chat, combat) |
| `list_actor_ownership` `set_actor_ownership` · `grant_xp` | Permissions & rewards |

## 👁️ Perception — the AI *sees* your table *(companion)*

| Tool | What for |
|---|---|
| 📸 `client_capture` | Screenshot of the GM's view, returned as a real image — the AI literally sees the map |
| 🗺️ `client_scene_report` | Machine-readable scene: tokens with grid coords, disposition, **real** visibility, doors, lights, templates |
| 📊 `client_get_derived` | The **prepared** values of any sheet (post-`prepareData` + active effects) — source docs often store 0 where players see the real stat |
| 🔗 `client_enrich` | Enriched HTML: `@UUID` links resolved, inline rolls evaluated, GM secrets only with explicit opt-in; replies use a shared socket channel |
| 🔎 `client_search` | Name search across every collection via the client index |
| 🌍 `client_babele` | [Babele](https://foundryvtt.com/packages/babele) translations: reverse search by **displayed** name (find “Force Lightning” from “Éclair de Force”), translated indexes & documents |

## 🗣️ Interact — a dialogue with your players *(companion)*

| Tool | What for |
|---|---|
| ❓ `client_ask` | Pose a question in a real dialog **on a player's screen** and get their answer back |
| 🎲 `client_roll_formula` | **Any formula, any system** through Foundry's real `Roll` engine — native chat card, Dice So Nice, actor roll data, per-die results |
| 📣 `client_notify` · 🔔 `client_ping` · 🎥 `client_pan_camera` | Notifications, map pings, “everyone look here” camera moves — all targetable (`gm` / `players` / user ids) |
| 📜 `client_show_document` | Open a sheet on the targeted clients |
| 🎯 `client_select` / `client_target` · 🌫️ `client_fog` | Real selection & crosshair targets · reset explored fog |
| 📡 `client_get_state` · `client_status` | Who's connected, viewing what · companion health & detected deps |
| 🚀 `client_run_macro` · `client_run_script` | Run any macro (the universal key) · arbitrary JS (**off by default**, GM opt-in) |

## 🌦️ Atmosphere — stagecraft *(companion)*

| Tool | What for |
|---|---|
| 🌧️ `client_weather` / `client_weather_types` | [FXMaster](https://foundryvtt.com/packages/fxmaster) particles: rain, fog, embers, snow, bats… |
| ✨ `client_play_effect` · `client_seq_between` · `client_seq_sound` | [Sequencer](https://foundryvtt.com/packages/sequencer) effects at a point, **between tokens** (projectiles!), sounds |
| 🎇 `client_token_fx` / `client_token_fx_presets` | [Token Magic FX](https://foundryvtt.com/packages/tokenmagic): glow, fire, shadow… 70 presets |
| 🗂️ `client_effect_catalog` | Search the installed effect database (JB2A & co) for valid paths |
| 🔊 `client_play_sound` | One-shot sound sting, no playlist needed |
| 🎞️ `mat_list` · `client_mat_trigger` | [Monk's Active Tiles](https://foundryvtt.com/packages/monks-active-tiles): list & fire trigger tiles (teleports, scene changes, chains) |

## 🎲 Dice & game systems

Game-specific logic lives in pluggable modules ([add yours!](../CONTRIBUTING.md)):

| System | Tools |
|---|---|
| **Star Wars FFG** | `roll_actor_skill` (pool **derived from the sheet**: species, talents, gear), `roll_ffg_pool`, `request_player_roll` (chat button opens their roll dialog), `adjust_actor_stats`, `adjust_destiny`, `grant_xp`, `apply_critical_injury` · companion: `client_roll_pool_native` (**real FFG engine + Dice So Nice 3D dice**) |
| **D&D 5e** | `dnd5e_roll_check` (sheet-derived modifiers, advantage, DC, nat 20/1), `dnd5e_adjust_stats` |
| **Daggerheart** | `dh_roll_duality` (Hope/Fear 2d12), `dh_roll_actor_trait`, `dh_adjust_stats` |

All modules load by default; restrict with `FOUNDRY_SYSTEMS=starwarsffg,dnd5e`.

## 🗃️ Campaign management — the [wgtnGM](https://campaigncodex.wgtngm.com/) suite

| Addon | Tools | What for |
|---|---|---|
| **[Campaign Codex](https://foundryvtt.com/packages/campaign-codex)** 6.9 | `cc_list_sheets` `cc_get_sheet` `cc_create_sheet` `cc_link` | Sheets in the 6.x format (managed Information / GM Notes pages, hidden from players unless `player_visible`), links written on **both** sides for every relation, with unlinking and cycle guards |
| | `cc_quest_update` / `cc_objective` | Quests: status, pin, urgency, giver, related sheets, rewards, check-ins, optional chat notice — and the nested objective tree |
| | `cc_set_visibility` / `cc_set_content` | What players see (sheet, per-tab GM-only, hidden links) and the prose (Information, GM Notes, custom tabs) |
| | `cc_shop_set_item` · companion: `client_cc_shop_add_item` | Shop inventory: add items through Campaign Codex, then quantity, custom price, infinite stock, visibility, markup |
| | `cc_webhub_status` / `cc_webhub_exclude` | Web Hub Sync: read which sheets are synced, exclude sensitive ones — a flag only, **never** triggers a sync |
| | companion: `client_cc_convert` `client_cc_open_toc` `client_cc_open_quest_board` `client_cc_open_sheet` `client_cc_migrate_content` `client_cc_export_obsidian` | Convert journals into sheets, open the Hub / quest board / a sheet without toggling them shut, repair old sheets, Obsidian export |
| **Asset Librarian** 1.4 | `al_tag` `al_find` `al_list_tags` `al_image_tags` · companion: `client_al_search` `client_al_open` | Tag documents (category + filter tags, as arrays), find them by tag, list the tag registry and groups, read image tags, search assets and open the browser pre-filtered |
| **Mini Calendar** 10 | `mc_get_time` `mc_set_time` `mc_list_notes` `mc_add_note` `mc_update_note` `mc_delete_note` | World time, and calendar notes read and written the way the module stores them |
| | `mc_get_weather` `mc_weather_types` `mc_calendar_info` · companion: `client_mc_set_weather` | Forecast (°C and °F), usable weather types, calendar setup (months, seasons, moons, sun); force the weather of a day |
| | companion: `client_mc_get_date` `client_mc_set_date` `client_mc_set_time` `client_mc_open` | Live date (season, moon phases), jump to a date or to dawn / noon / dusk / midnight, *next* or in N days |
| **Mini Countdown Dice** | `cd_list` / `cd_set` | Threat clocks and countdowns shown to everyone: create, set, ±delta, auto-count on turn or round, show/hide, lock to a scene |
| **Mini Light Switch** | `ls_lights` / `ls_toggle` | List a scene's lights, switch them on/off by id or Tagger tag, set the animated graphic — works even without the module |
| **Mini Player** | `mp_list_presets` `mp_scene_audio` · companion: `client_mp_play_preset` `client_mp_play_tags` | Start a saved ambience or music by tags; set the audio a scene plays when activated |
| **Mini Rolltable** | `draw_from_table` | Quantities (`1d4+1` goblins) and linked tables are honoured, up to 5 levels deep |

## 🛠️ Administration — the world itself

`manage_modules`, `manage_users` and `admin_edit_world` are available without the admin password. The other admin tools appear only when
`FOUNDRY_ADMIN_PASSWORD` is set.

| Tool | What for |
|---|---|
| 🩺 `admin_status` | `/api/status` — works even with the world down |
| ✏️ `admin_edit_world` | World title, description, background image, next-session date — while it runs |
| 🧩 `manage_modules` | Installed vs enabled (with versions) · enable/disable |
| 👥 `manage_users` | Player / GM / assistant-GM accounts: list, create, change roles, assign characters (passwords stay in Foundry) |
| 💾 `admin_list_backups` / `admin_backup_world` | Foundry's own backups — and `admin_update_package` takes one automatically first |
| ⏻ `admin_shutdown_world` / `admin_launch_world` | Stop & start worlds (the bot reconnects by itself) |
| ⬆️ `admin_check_package` / `admin_update_package` | Update modules, **systems**, worlds — check → install → verify, refuses while a world runs |

## Individual documents

The collection reads above also have singular forms: `get_actor`, `get_item`,
`get_journal`, `get_scene`, `get_table`, `get_macro`, `get_playlist`, `get_card`,
`get_combat`, `get_message`, `get_folder`, `get_user` and `get_setting`.

## Transfer and provision worlds

| Tool | Purpose |
|---|---|
| `copy_documents` | Transfer selected documents and their folder tree between instances; preview with `dry_run` |
| `copy_assets` | Copy missing files between instance storage, or overwrite existing ones explicitly; preview with `dry_run` |
| `admin_install_package` | Install a system, module or world on the destination server; requires the admin password |
| `admin_create_world` | Create a destination world; requires the admin password |

See the [advanced guide](advanced.md#multiple-worlds-and-transfers) for ordering,
identifier preservation and the limits of a content transfer.

## Beyond tool calls

MCP resources expose actors and journals with cursor pagination. Built-in prompts
`session-recap`, `world-overview` and `prep-checklist` bring live world data into
preparation. Resource subscriptions and SSE relay changes to connected clients.
The transport negotiates MCP 2025-06-18 or 2025-03-26 over Streamable HTTP.

Building your own client? The [integrator contract](integrators.md) documents
response shapes. Want another system or module? [Contributions are welcome](../CONTRIBUTING.md).
