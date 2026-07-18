# Response shapes — the integrator contract

For programs that call the gateway's tools directly (web apps, bots, other
servers) rather than through an AI assistant. Everything here was captured
against a live Foundry v13 world; shapes are per-tool stable — breaking changes
will be called out in release notes.

## 1 · The envelope

Every tool call returns standard MCP content. **The first `text` item is a
JSON document** — parse it, that's the actual result:

```jsonc
// tools/call result
{ "content": [ { "type": "text", "text": "{\"connected\":true,…}" } ] }
```

- **Errors** are in-band: `"isError": true` and the text is a plain
  `"Error: <message>"` string (not JSON). JSON-RPC-level errors only occur for
  unknown tools or malformed requests.
- **`structuredContent`** (MCP 2025-06-18) carries the same JSON as an object when
  the result *is* an object — prefer it when your client supports it; the text
  item stays for older clients. Arrays are text-only, by spec.
- **`client_capture` is the one exception**: `content[0]` is
  `{ "type": "image", "data": "<base64>", "mimeType": "image/webp" }`, and
  `content[1]` is a text caption `{"scene", "width", "height"}`.

## 2 · Reads: `get_<collection>` / `get_<singular>`

`get_actors`, `get_journals`, `get_scenes`, … return a **JSON array of source
documents**, filtered/projected server-side:

```jsonc
[ { "_id": "mn8njVooeYGbNpeO", "name": "Pahas'Tis", "type": "character", … } ]
```

- `requested_fields` projects, but `_id` and `name` are **always** included.
- `where` supports dotted paths and `__in` / `__contains` / `__ne` / `__exists`.
- `max_length` truncates by dropping **trailing documents** (never mid-document),
  so what you receive always parses.
- Singular forms (`get_actor` …) return **one document object**; a miss is an
  in-band error, not `null`.

> ⚠️ **Source ≠ displayed.** World documents store *source* data — on many
> systems (starwarsffg included) displayed stats live in `prepareData` and can
> read as `0` in the source. For what the player actually sees, use
> `client_get_derived` (companion) or the system tools (`roll_actor_skill`
> derives pools server-side).

## 3 · Events: `get_events`

```jsonc
{
  "count": 40,
  "events": [ { "seq": 1091, "event": "modifyDocument", "args": [ … ] } ],
  "lastSeq": 1106
}
```

Poll incrementally: pass `since_seq = lastSeq` from the previous call. The
buffer is a ring (300 entries) — poll at least every few minutes during busy
sessions or you may miss events. `args` are the raw Foundry broadcast payloads.

## 4 · Companion tools: `client_*`

The text JSON is **whatever the browser-side handler returned**, verbatim.
Representative shapes:

```jsonc
// client_status
{ "module": "foundry-mcp-gateway-companion", "version": "0.4.1",
  "responder": "Gamemaster", "system": "starwarsffg",
  "dependencies": { "diceSoNice": true, "sequencer": true, … } }

// client_get_state
{ "activeUsers": [ { "id", "name", "isGM", "character", "viewedScene" } ],
  "currentScene": "tyWwYkaCY3CcBydH" }

// client_get_derived
{ "uuid", "name", "type", "system": { …prepared values… },
  "effects": [ { "_id", "name", "disabled", "applied", "changes" } ],
  "items": [ … ] }                    // only if items:true

// client_ask
{ "user": "Nima", "userId": "…", "answered": true, "answer": "Oui" }
// or { "user", "answered": false, "reason": "timeout" | "dismissed" }

// client_scene_report (excerpt)
{ "scene": {"_id","name"}, "grid": {"size","distance","units"},
  "tokens": [ { "_id","name","actor","disposition","hidden","visible",
                "x","y","col","row","elevation" } ],
  "doors": [ {"_id","type","state"} ], "lights": [...], "templates": [...],
  "controlled": [...], "targeted": [...] }
```

`client_ask` is the only tool whose answer comes from the *targeted* client
(validated live: the reply carries that player's `user`/`userId`). Keep its
`timeout_seconds` under ~45: most MCP clients abandon a call after ~60 s. The
answer is not lost — it still arrives in `get_events` on the companion channel.

If no companion is installed/awake, `client_*` tools fail in-band after their
timeout with an explicit message. Note: the GM's **browser tab must be alive**
— browsers freeze background tabs, and a frozen tab never answers.

## 5 · Health & admin

```jsonc
// ping (cheap, no world dump)
{ "connected": true, "generation": 13, "hostname": "…", "userId": "…",
  "eventSeq": 0, "server": { "active": true, "world": "star-wars",
  "version": "13.351", "system": "starwarsffg", "users": 2, "uptime": 1725494 } }

// admin_status (works even with the world down)
{ "adminPasswordConfigured": true, "status": { …/api/status verbatim… } }

// admin_check_package (works with the world up)
{ "package": "campaign-codex", "type": "module",
  "installed": "5.7.5", "remote": "5.7.5", "updateAvailable": false }

// admin_update_package
{ "package", "type", "from", "to", "installedNow", "updated": true,
  "attempts": 1, "note": "vérifié installé (manifest statique)" }
```

## 6 · Read-only deployments

A gateway started with `FOUNDRY_READONLY=1` lists **only** read-only tools and
answers any write with an in-band error. Point untrusted or player-facing
integrations at such an instance: `tools/list` is your source of truth for what
is callable, and `annotations.readOnlyHint` marks each tool.

## 7 · Conventions worth relying on

- Timestamps are Foundry's (`modifiedTime` in ms); ids are Foundry 16-char ids.
- Booleans are real booleans, numbers real numbers — nothing is stringified
  except the whole envelope text.
- Read-only tools carry `annotations.readOnlyHint: true` in `tools/list` —
  safe to auto-approve; only `delete_document` / `delete_compendium` are
  flagged destructive.
- When in doubt about a shape: call the tool once and inspect — shapes don't
  vary by input beyond documented optional fields.
