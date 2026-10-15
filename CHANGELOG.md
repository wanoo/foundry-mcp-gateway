# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

**Response shapes** — the contract documented in
[docs/integrators.md](docs/integrators.md) — only change on a **major** version.

## [1.7.0] — 2026-10-16

*The wgtnGM suite moved a lot, and so did Foundry. This release catches up with
both: Campaign Codex 6.9, Mini Calendar 10, Asset Librarian 1.4 and four new
Mini modules, all tested end to end on a live Foundry 14.369 world — 365 checks
across the four test batteries.*

### Added

- **Campaign Codex quests** (`cc_quest_update`, `cc_objective`). Status,
  pinning, urgency, giver, related sheets, rewards, check-ins and the nested
  objective tree, with `updatedAt` and the `linkedQuests` back-links kept
  current, and an optional chat notice. The post-session routine — "the party
  finished this, learned that" — no longer means editing raw flags.
- **What players see, and what a sheet says** (`cc_set_visibility`,
  `cc_set_content`): sheet visibility, GM-only tabs and hidden links; the
  Information, GM Notes and custom-tab prose in Campaign Codex 6 managed pages.
- **Shops** (`client_cc_shop_add_item`, `cc_shop_set_item`). Items are added
  through Campaign Codex itself so the price snapshot is right; quantities,
  custom prices, infinite stock, visibility and markup are then set on the
  server.
- **Web Hub Sync, read and exclude only** (`cc_webhub_status`,
  `cc_webhub_exclude`). Web Hub Sync uploads the whole codex — GM notes
  included — to an external service. These tools read which sheets are synced
  and exclude sensitive ones; nothing here can start a sync.
- `client_cc_open_quest_board`, `client_cc_open_sheet`, and
  `client_cc_migrate_content`, which repairs sheets created by older versions
  of this gateway.
- **Calendar notes, read and written the way Mini Calendar stores them**
  (`mc_add_note`, `mc_update_note`, `mc_delete_note`): day page `YYYY-MM-DD`,
  notes in flags, page HTML regenerated, so an open calendar shows them at once.
- **Weather and calendar** (`mc_get_weather`, `mc_weather_types`,
  `mc_calendar_info`, `client_mc_set_weather`): the stored forecast in °C and
  °F, the usable weather types, the calendar's months, seasons, moons and sun,
  and forcing a day's weather. Today's date is computed with Mini Calendar's own
  arithmetic, not Foundry's.
- **Live date** (`client_mc_get_date`, `client_mc_set_date`): the localized
  date with season, dawn/dusk and moon phases, and jumping to any date — "you
  drop out of hyperspace three days later, at dawn".
- **Asset Librarian** `al_list_tags`, `al_image_tags` and `client_al_search`:
  the tag registry and groups, the tags given to image files, and asset search
  by tags across the world and compendia.
- **Mini Countdown Dice** (`cd_list`, `cd_set`): threat clocks and countdowns
  every player sees, auto-counting on turn or round. They write the module's
  world state directly — its own remove asks for a confirmation and cannot be
  scripted.
- **Mini Light Switch** (`ls_lights`, `ls_toggle`): switch a scene's lights by
  id or Tagger tag and set their animated graphic. They write
  `AmbientLight.hidden`, so they work whether or not the module is active.
- **Mini Player** (`mp_list_presets`, `mp_scene_audio`,
  `client_mp_play_preset`, `client_mp_play_tags`): start a saved ambience or
  music by tags, and set the audio a scene plays when it is activated.

### Changed

- **Campaign Codex sheets are hidden from players by default.**
  `cc_create_sheet` made every sheet readable by all players unless `gm_only`
  was passed, while Campaign Codex itself creates them hidden — an easy way to
  leak a secret. Pass `player_visible: true` to share one; `gm_only` still
  works.
- `cc_link` now writes both sides by default (`bidirectional: true`), and infers
  the relation from the two sheet types when `relation` is omitted.
- The wgtnGM tools live in one source file per addon
  (`src/tools/cc_family/`), each declaring its own companion commands.

### Fixed

- **`cc_create_sheet` produced pre-6.x sheets**: no sheet class, no managed
  pages, `data.widgets`, associates on quests. They opened with the plain
  journal sheet, and Campaign Codex later duplicated their text when migrating
  them. Sheets now match what Campaign Codex 6.9 creates.
- **`cc_link` wrote one side of most links**, and `associates` for everything.
  Campaign Codex maintains back-links only in the browser that made the change,
  and the gateway has none — so a location never listed the NPC linked to it.
  Every relation (locations, shops, regions and sub-regions, factions and
  groups, members, journals, scenes, actors) is now written on both sides, with
  unlinking, type checks and cycle guards.
- `cc_get_sheet` read `data.description`, empty on 6.x sheets; it now reads the
  managed pages. `cc_list_sheets` counted four of the fourteen relation fields.
- `client_cc_open_toc` closed the Hub when it was already open (the API
  toggles, and the table of contents became the Hub in 6.6). It now opens it,
  or brings it forward with the requested view and tags.
- `client_cc_convert` reported success when Campaign Codex refused the
  conversion; it returns the created sheets or an error.
- **`al_tag` wrote filter tags as a string**, but Asset Librarian 1.4 expects an
  array: tagged documents did not show in its filters. It now writes the
  normalized array, can add or remove single tags, and registers new tags in the
  tag cloud. `al_find` missed array tags and was case-sensitive; matching now
  runs in the gateway and can scan every collection. `client_al_open` dropped
  tags, search and folder.
- `mc_list_notes` returned the rendered page HTML — no time, duration,
  visibility, recurrence or links. It now reads the notes' flags and filters by
  date.
- `client_mc_set_time` only knew dawn and dusk of *today*, so "dawn" could move
  time backwards. It takes day offsets, explicit hours, noon/midnight and
  `next`. `client_mc_open` closed an open calendar; it now only opens unless
  `toggle` is set.
- **`draw_from_table` ignored linked tables and Mini Rolltable quantities**, so
  a loot table answered "Gems" instead of two gem draws. It follows linked
  tables up to 5 levels deep, rolls the quantity formulas, and posts a nested
  chat card. Document results whose description is empty fall back to their
  name.
- **Writes are broadcast again.** Every create, update and delete went out with
  `broadcast: false`, and Foundry (v13 as well as v14) honours that flag: the
  document was saved, but no connected client heard about it. A Gamemaster saw
  an actor the gateway had just created only after a reload, and the companion
  answered `Document not found` for it. The flag is gone; Foundry's default
  broadcasts.
- **Scenes created on v14 have a level.** v14 scenes need at least one level,
  and the default one is added by the browser in `Scene#_preCreate` — code a
  socket client never runs. A scene from `create_document` or `copy_documents`
  was saved without one and the canvas crashed opening it
  (`_determineInitialLevel`). The gateway now adds the default level the way
  the browser does, carrying over the old background fields.
- **`ChatMessage` gets an author.** v14 refuses a message whose `author` is
  null; `create_document` now fills in the bot's user.
- **Admin tools on v14.** Build 368 answers a bare 400 to any POST without an
  `Origin` header, and v14 moved two routes: world creation now lives on
  `/create`, and shutdown is `POST /setup {action: "worldShutdown"}`.
  `admin_create_world`, `admin_shutdown_world` and `admin_edit_world` failed on
  v14; they now send `Origin`, try the v14 route first and fall back to v13's.
  Shutdown is confirmed on `/api/status` instead of trusting a 200, which both
  routes return even when nothing happened. Uploads send `Origin` too.
- **`manage_combat set_initiative` finds combatants by name.** Combatants
  created from scene tokens store no name; the lookup now resolves it from the
  token, as `status` already did.

## [1.6.0] — 2026-10-15

*The release that closes the doors the companion left open, and stops trusting
a shared secret to guard a whole world.*

### Added

- **Signed companion commands.** Foundry relays module socket channels to every
  connected client and never says who sent a message, so any player could emit
  `run_macro` from their browser console and have the Gamemaster execute it —
  bypassing the remote-script setting entirely. Commands now carry an ECDSA
  P-256 signature the companion verifies with `crypto.subtle`: no dependency,
  no build step, and no WASM in a Foundry module. `--generate-key`,
  `--print-public-key`, and a `companion_trust_key` tool that installs the
  public half itself rather than making you copy a long hex string.
- **Biscuit tokens replace `MCP_SECRET`.** A shared secret had no scope, no
  expiry and no revocation: handing it to an integrator handed over all 135
  tools with Gamemaster rights, forever. Tokens expire, can be minted
  read-only, can be attenuated further offline without the private key, and can
  be revoked one at a time. `--generate-mcp-key`, `--mint-token`, and
  `MCP_REVOKED_TOKENS`.

### Fixed

- **`POST /join` on Foundry v14 build 368.** That build rejects the lowercase
  `userid` field (401 `JOIN.ErrorUserDoesNotExist`) and answers a generic 400
  when the request has no `Origin` header. The join payload now carries both
  `userid` (v13) and `userId` (v14), and the request sends `Origin`.
  Thanks @Gremlim (#11).
- **Three ways to crash the gateway from a tool call.** An unbounded dice pool
  made `format_pool` allocate gigabytes and `abort()` the process — taking every
  MCP session with it. A RollTable with the formula `1d0` panicked on a division
  by zero. An unbounded `timeout_seconds` overflowed `Instant + Duration`. All
  three are bounded, and refused with an error that names the cause.
- **Writes could be silently duplicated.** On reconnection the pending acks were
  dropped but the outgoing frame queue was not, so a `create_document` reported
  as failed still reached Foundry seconds later — and the agent, told it had
  failed, retried.
- **MCP sessions were never reclaimed.** Only an explicit `DELETE` removed one,
  so a client that disconnected abruptly leaked a receiver for the life of the
  process.
- **A burst of events made the event bridge drop out.** `Lagged` — the normal
  outcome of a full broadcast buffer — was treated as a closed channel, so the
  bridge slept and resubscribed exactly when traffic was heaviest.
- **`upload_file` would fetch any URL**, including cloud instance metadata.
  Link-local addresses are now refused everywhere; private ranges are allowed
  only when the gateway is hosted alongside a private network — which it detects
  from your own instances, so a Raspberry Pi or homelab setup keeps working.
- **`FOUNDRY_READONLY` refused eight tools that read nothing**, making the mode
  stricter than documented.
- **A tool could be claimed by two modules, or by none.** `handles` was based on
  name prefixes and only worked because of the order of the dispatch chain; each
  module now lists its own names, and a test asserts exactly one owner per tool.
- `/api/status` was hard-coded to HTTPS, so `ping` failed on every plain-HTTP
  local Foundry.

### Changed

- Tool definitions are built once instead of on every dispatch, and
  `truncate_by_bytes` no longer re-serialises the whole array per dropped
  document.
- `CONTRIBUTING.md` no longer promises a test environment the repository does
  not ship, and its definition of done no longer requires something a
  contributor without a Foundry licence cannot produce.
- The `docker` and `crates` release jobs are skipped outside the upstream
  repository, so a fork's release pipeline is no longer red by construction.
- The gateway↔companion pairing lives in one table (`COMPANION_COMMANDS`),
  exported to `docs/companion-commands.json` and checked against the
  companion's handlers, so the two halves can no longer drift in silence.

## [1.5.0] — 2026-07-19

*First release aimed at people other than its author.*

### Added

- **`--dump-tools`, `--version`, `--help`** — the full tool catalogue as JSON
  without connecting to Foundry. It also became the source of truth for
  `scripts/check-docs.sh`, which fails CI when the READMEs' numbers drift from
  the code. They had drifted three times; the READMEs once announced 131 *and*
  134 tools on the same page.
- **Version drift detection** — the gateway and the companion module now share
  one version number, and `client_status` reports `versionDrift` naming the half
  that's behind, instead of leaving you to compare by eye.
- **Automated publishing** — bump the version, push, and the workflow tags,
  builds a multi-arch Docker image, cuts the release with binaries, and
  publishes to crates.io. Nothing else to run.
- **Collaboration kit** — Contributor Covenant 3.0, a security policy stating
  the real threat model, issue forms, PR template, dependabot, CODEOWNERS.

### Fixed

- **The quick start's first step was impossible.** It told readers to find the
  bot's `_id` in `view-source:/join`; in Foundry v13 that page is a 2 KB
  client-rendered shell, verified on both a live world and a fresh container.
  Rewritten around a browser-console one-liner.
- Tool counts now stated honestly: 134 with `FOUNDRY_ADMIN_PASSWORD`, 126
  without, 50 read-only — the number depends on configuration, which the docs
  used to hide.

## [1.4.0] — 2026-07-19

*Cross-server migration validated end to end against a real second server
(Foundry 13.351 in Docker): system installed, world created and launched,
player accounts recreated, 48 folders + 112 journals + 20 actors + 12 scenes +
2 tables + 25 macros copied, assets uploaded across servers, `_id`s preserved.*

### Added

- **Plain-HTTP and custom-port instances** — a `hostname` may now carry an
  explicit scheme (`http://foundry.lan:30000`), and localhost targets default to
  HTTP. Local/dev Foundry containers were previously unreachable: every URL was
  hardcoded to `https://`/`wss://`.

### Fixed

- **`copy_documents` rejected `journals`** — its own description advertised the
  plural, but the code only accepted the internal `journal`. Both work now, as
  everywhere else.
- **`admin_create_world` and `admin_install_package`** are no longer untested:
  both drove the migration above.

## [1.3.0] — 2026-07-19

### Added

- **`admin_create_world`** and **`admin_install_package`** — the last two links
  of a cross-server migration: stand up an empty world on the target and bring
  it to module/system parity before pouring content in. Together with
  `copy_documents`, `copy_assets` and `manage_users`, a world can be recreated
  on a fresh server end to end. ⚠️ Both are implemented from the setup protocol
  but **not yet exercised against a live server** — creating worlds and
  installing packages on someone's instance isn't something to test unprompted.

## [1.2.0] — 2026-07-19

### Added

- **Multi-world**: several instances are now served **simultaneously**, one
  socket each, opened on demand. Every tool accepts an `instance` argument;
  `choose_foundry_instance` only moves the default and no longer tears the
  connection down. Validated live with two concurrent connections under two
  different accounts.
- **`copy_assets`** — carry the files (maps, tokens, art, audio) along with the
  documents: walks the source storage, recreates the tree on the target and
  uploads what's missing, streaming through the gateway so the two servers never
  need to reach each other. This is what makes a **cross-server** clone actually
  work, since document image paths alone would dangle.
- **`copy_documents`** — move content between instances: `where`/`ids` selection,
  `_id`s preserved so `@UUID` links survive, folders recreated, `dry_run`,
  and `overwrite` that **updates** the target's existing twin instead of
  duplicating it.

## [1.0.0] — 2026-07-18

*130 tools · 50 of them read-only · validated against a live Foundry v13.351 world.*

First stable release. The server is feature-complete for its scope, in
production, and its response shapes are now a versioned contract.

### Added

- **Read-only mode** — `FOUNDRY_READONLY=1` exposes only the 50 read-only tools
  and refuses writes at dispatch (defence in depth). Lets you plug untrusted or
  player-facing integrations into the same world.
- **User management** — `manage_users`: list accounts, create/update/delete
  them, set roles (player / trusted / assistant / gamemaster), assign
  characters. Passwords are deliberately **not** handled: the GM sets them in
  Foundry; accounts are created password-less.
- **Backups** — `admin_list_backups`, `admin_backup_world`, and an automatic
  backup before `admin_update_package` (disable with `backup: false`; a failed
  backup aborts the update). Validated live: a 150 MB world archived in seconds.
- **Generic dice** — `client_roll_formula`: any formula through Foundry's real
  `Roll` engine, on any system, with the native chat card, Dice So Nice, actor
  roll data, GM whisper, and per-die results.
- **MCP 2025-06-18** — protocol version negotiation (2025-03-26 clients still
  get 2025-03-26) and `structuredContent` on every object-shaped result.
- **Docker** — multi-stage `Dockerfile` (119 MB image, `/health` healthcheck).
- **Integrator docs** — `docs/integrators.md`: exact response shapes.

### Changed

- `admin_check_package` now works **while the world is running** (it reads the
  remote manifest at its declared URL).
- `admin_update_package` retries up to **5 times** on failure and verifies
  against the package's static manifest.

### Fixed

- **Installed version always `null`** — Foundry's `checkPackage` only returns
  the *remote* manifest, so updates believed everything was up to date. The
  installed version is now read from the package's static manifest.
- **`toggle_pause` reported success blindly** — the `pause` event carries no ack,
  so a refusal (insufficient rights) still looked like success. It now waits for
  the bot's role first and fails loudly on a non-GM account. (Foundry rebroadcasts
  `pause` to everyone *except* the sender, so waiting for a confirmation is a
  false negative — verified.) Found by testing a restricted-role bot.
- **`manage_modules` was annotated read-only** although it writes
  `core.moduleConfiguration` — clients could auto-approve it.

## [0.x] — 2026-07-16 → 2026-07-17 (pre-release)

- Independent **Rust rewrite**: single binary = native Foundry socket client +
  streamable-HTTP MCP transport, replacing the TypeScript connector (100 %
  parity validated by a 45/45 battery against a live v13 world).
- 126 tools: generic document CRUD, GM session tooling, game-system modules
  (Star Wars FFG with sheet derivation, D&D 5e, Daggerheart), Campaign Codex
  family, perception / interaction / atmosphere via the companion module, world
  administration.
- Foundry v13 & v14 (session binding auto-detected), route prefixes, per-collection
  reads with query pushdown and DB-index listings, infinite reconnection.
