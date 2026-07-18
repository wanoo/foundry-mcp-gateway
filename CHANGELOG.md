# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

**Response shapes** — the contract documented in
[docs/integrators.md](docs/integrators.md) — only change on a **major** version.

## [1.1.0] — 2026-07-19

### Added

- **Multi-world**: several instances are now served **simultaneously**, one
  socket each, opened on demand. Every tool accepts an `instance` argument;
  `choose_foundry_instance` only moves the default and no longer tears the
  connection down. Validated live with two concurrent connections under two
  different accounts.
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
