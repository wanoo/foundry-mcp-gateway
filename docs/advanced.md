# Advanced configuration and troubleshooting

[Français](advanced.fr.md) · [Home](../README.md) · [Installation](installation.md) · [Capabilities](capabilities.md)

Start with the [Docker installation guide](installation.md) for key generation,
Foundry credentials and Companion trust. This guide covers other deployments,
access controls and ongoing operation.

## Native installation and source builds

Download a binary for your platform from the [published releases](https://github.com/wanoo/foundry-mcp-gateway/releases).
The release assets include SHA-256 files. Extract the archive and put
`foundry-mcp` on your PATH. Check the asset list for available platforms.

With `cargo-binstall` installed, you can use `cargo binstall foundry-mcp-gateway`.
To compile the published crate, use `cargo install foundry-mcp-gateway`.
Source builds require Rust **1.85 or newer**:

```sh
git clone https://github.com/wanoo/foundry-mcp-gateway.git
cd foundry-mcp-gateway
cargo build --release
./target/release/foundry-mcp --version
```

The native executable provides the same setup commands:

```sh
foundry-mcp --generate-mcp-key
foundry-mcp --generate-key
```

Save their private values as `MCP_ROOT_PRIVATE_KEY` and `COMPANION_SIGNING_KEY`,
then export them in your shell. Set `FOUNDRY_CREDENTIALS_JSON` as in the
[installation guide](installation.md#4-start-the-gateway), using
`http://localhost:30000` when the native Gateway and Foundry share a machine.
Mint a client token with `foundry-mcp --mint-token assistant --days 365`, then
start `foundry-mcp` (or `./target/release/foundry-mcp` from the source checkout).
For a source build, use that binary path for all setup commands too.

## Docker Compose

Use the environment variables from the installation guide. Save this as
`compose.yaml`; `${NAME:?message}` makes Compose refuse an unset or empty value.

```yaml
services:
  foundry-mcp:
    image: wanoo/foundry-mcp-gateway:latest
    ports:
      - "127.0.0.1:8080:8080"
    extra_hosts:
      - "host.docker.internal:host-gateway"
    environment:
      MCP_ROOT_PRIVATE_KEY: ${MCP_ROOT_PRIVATE_KEY:?Set the MCP root private key}
      COMPANION_SIGNING_KEY: ${COMPANION_SIGNING_KEY:?Set the Companion signing key}
      FOUNDRY_CREDENTIALS_JSON: ${FOUNDRY_CREDENTIALS_JSON:?Set the Foundry credentials}
    restart: unless-stopped
```

Run `docker compose config --quiet` to validate, then `docker compose up -d`.
The image has a built-in `/health` check. Keep keys stable in your secret store;
no container volume is needed for the keys when they are supplied through the
environment. For repeatable deployments, use a published version tag instead
of `latest` once the matching release exists.

## Environment reference

| Variable | Default / purpose |
|---|---|
| `MCP_ROOT_PRIVATE_KEY` | MCP root private key; required to mint tokens; can also verify them |
| `MCP_ROOT_PUBLIC_KEY` | Alternative verification-only root key; takes precedence if both root variables are set |
| `MCP_REVOKED_TOKENS` | Empty; comma-separated revocation IDs to reject |
| `COMPANION_SIGNING_KEY` | A temporary key per start if absent; set a stable private key for Companion trust |
| `FOUNDRY_CREDENTIALS_JSON` | Required JSON array of `{ "_id", "hostname", "userid", "password" }` entries |
| `FOUNDRY_ADMIN_PASSWORD` | Unset; adds 8 tools for Foundry server/package administration |
| `FOUNDRY_READONLY` | Off; `1`, `true` or `yes` exposes and permits only read-only tools |
| `FOUNDRY_SYSTEMS` | All built-in systems; comma-separated IDs such as `starwarsffg,dnd5e` |
| `FOUNDRY_ALLOW_INTERNAL_FETCH` | Auto-detected from configured instances; `1` allows or `0` denies private-network fetches by `upload_file`; link-local destinations remain refused |
| `PORT` | `8080`; HTTP listener port |
| `RUST_LOG` | `info`; logging filter |

A `hostname` without a scheme normally uses HTTPS. Loopback and
`host.docker.internal` default to HTTP; use `http://` explicitly for other LAN
hosts. Route prefixes such as `https://foundry.example.com/my-world` are supported.
The instance `_id` is a Gateway label, not Foundry's world or user ID.

## Hosting

The Gateway can run alongside Foundry, on your LAN or on a remote host. A remote
Gateway needs network access to Foundry, usually a public HTTPS address or a
private network connection. Your MCP client independently needs access to the
Gateway endpoint.

For an Internet-facing Gateway, put an HTTPS reverse proxy in front and allow
Streamable HTTP POST, GET/SSE and DELETE through to the same `/mcp-<token>` path.
Preserve the `mcp-session-id` header and long-lived SSE responses. Avoid recording
credential-bearing paths in access logs. The loopback binding in the installation
guide works with a proxy on the host; adapt networking for a containerized proxy.

### Clever Cloud

From a checkout with the desired version, with the Clever CLI configured and the
setup variables exported:

```sh
clever create --type rust foundry-mcp-gateway
clever env set MCP_ROOT_PRIVATE_KEY "$MCP_ROOT_PRIVATE_KEY"
clever env set COMPANION_SIGNING_KEY "$COMPANION_SIGNING_KEY"
clever env set FOUNDRY_CREDENTIALS_JSON "$FOUNDRY_CREDENTIALS_JSON"
clever env set CC_RUST_BIN foundry-mcp
clever deploy
```

Use the application's reachable HTTPS URL in your MCP client. Complete the same
Companion trust setup as for Docker. Hosting plans and billing depend on your
provider; this project does not promise free hosting.

## Access and Companion trust

### Client access with Biscuit

Biscuit authenticates and authorizes **AI client → Gateway**. Mint a separate
token per client; default lifetime is **365 days**:

```sh
foundry-mcp --mint-token assistant --days 365
foundry-mcp --mint-token lore-reader --readonly --days 90
```

The command prints the token and its revocation ID. For Docker, prefix the same
arguments with `docker exec foundry-mcp foundry-mcp` instead of `foundry-mcp`.
Keep the token-bearing URL private. To revoke one token, add its revocation ID
to `MCP_REVOKED_TOKENS` and restart/recreate the Gateway with that environment.
Retain other IDs already on your list. Revoking an ancestor also invalidates
tokens attenuated from it.

A root private key is needed for minting. A running Gateway that only verifies
tokens can use `MCP_ROOT_PUBLIC_KEY` instead. Biscuit supports further offline
attenuation without the private key; the built-in minting CLI offers full access
or read-only access, not per-tool or per-world scopes. A token otherwise reaches
the configured instances with the bot accounts' rights.

A read-only token filters the catalogue and refuses write calls even if the
server normally permits them. `FOUNDRY_READONLY=1` applies that restriction to
all clients. The full 1.7 catalogue has 69 read-only tools with administration
enabled, or 67 without it, before any system filtering. A read-only token cannot
install the Companion trust key; use a full-access client or paste the public key
manually first. Read-only access can still reveal your campaign's GM information.

### Signed commands to the Companion

ECDSA P-256 authenticates **Gateway → Companion**. It uses
`COMPANION_SIGNING_KEY`, separate from the Biscuit root key. Generate it once
with `--generate-key`, install its public half using `companion_trust_key`, then
**reload GM browsers**. Alternatively paste the output of `--print-public-key`
into the Companion's Gateway public key setting and reload.

If an existing different key is found, the tool refuses to overwrite it.
`force: true` intentionally replaces that trust; use it only after identifying
which Gateway should control the world. A browser keeps its startup key until
reload and warns if the world setting changes during the session.

| Companion setting | Default | Behaviour |
|---|---|---|
| Command signature checking | Guarded | Without a trusted key, refuses code-executing commands; with a key, requires valid signatures for all commands |
| Gateway public key | Empty | Public half of the stable Companion signing key |
| Remote script execution | Disabled | `client_run_script` can instead ask first or run automatically if the GM opts in |
| Remote script-macro execution | Ask me first | Separate policy for remotely invoked script macros; does not intercept macros run manually in Foundry |
| Relay client telemetry | On, per client | Relays selections, targets and scene state; each client can disable it |

Strict signature mode requires authenticated commands even without a key set.
Off mode disables checking. Keep Guarded for the recommended setup. Code approval
is separate from command authentication; prompt wording does not enforce approval.
Signature verification needs Web Crypto in a browser secure context (HTTPS or
localhost). Signatures do not encrypt replies: Foundry's module socket channel
is shared. In particular, `client_enrich` excludes GM secret blocks by default;
requesting them explicitly sends them over that shared channel.

### Code hidden in data

Some modules run JavaScript stored in ordinary documents: Monk's Active Tiles
actions (`runcode`, and every value field it evaluates), core Region behaviors
`executeScript`/`executeMacro`, Mass Edit preset spawn scripts and Item Piles
pile macros. The Gateway is a GM account, so Foundry would accept such writes;
since 1.8 the Gateway inspects **every** write, nested scenes included, and
refuses code-capable content unless the GM set the matching Companion setting
to **Automatic**. Monk's Active Tiles actions pass when they are on a known-safe
list and their values are numbers, dice, `+ 3`/`= 5` style operations or
identifiers. The Gateway can never change the Companion's script settings
itself, and the Companion reverts any such change made by a client that does
not run the module. Triggering an existing tile or spawning a preset that
contains code goes through the same Companion policy and confirmation dialog
as `client_run_script`.

## Upgrading

### From 1.6 to 1.7

Update both components to 1.7.x when the releases are available. Keep the existing
Biscuit root key, client tokens and stable `COMPANION_SIGNING_KEY`; there is no
new key migration for 1.7. Recreate the Gateway container after pulling the chosen
image, update the Foundry module, reload GM browsers and reconnect the MCP client
to refresh its tool cache. Run `ping` and `client_status` afterwards.

If you never configured a stable signing key in 1.6, complete the trust setup
before relying on Companion commands. Campaign Codex sheets created by older
Gateway versions can be repaired through `client_cc_migrate_content`; run it
only for sheets needing migration, after a backup. New Campaign Codex sheets
are hidden from players by default; explicitly share them when appropriate.

### From 1.5 or earlier

`MCP_SECRET` is no longer supported. Generate an MCP root key with
`--generate-mcp-key`, mint a token with `--mint-token`, replace the old environment
setting, and update your client connection to `/mcp-<token>`. Separately generate
a stable Companion key with `--generate-key`, configure `COMPANION_SIGNING_KEY`,
update the Companion, install trust and reload the GM browsers. Review the
separate settings for remote scripts and script macros.

Both components track **major.minor** together; patch numbers may differ.
`client_status` identifies a minor-version mismatch. The
[Gateway](../CHANGELOG.md) and
[Companion](https://github.com/wanoo/foundry-mcp-gateway-companion/blob/main/CHANGELOG.md)
changelogs contain the full changes.

## Multiple worlds and transfers

Put multiple credential objects in `FOUNDRY_CREDENTIALS_JSON`. Each entry points
to a launched world, with its own instance label; the Gateway opens connections
on demand. A single Foundry server normally runs one world at a time, so two
simultaneously reachable worlds generally need separate Foundry processes.

Tools acting on a world accept an optional `instance` (`copy_documents` takes `from`/`to`). `show_credentials` lists configured
instances without passwords; `choose_foundry_instance` changes the default.
Install Companion trust separately in each world you want to drive.

To preview `copy_documents`, pass these tool arguments:

```json
{
  "from": "sandbox",
  "to": "live",
  "collection": "journals",
  "ids": ["AbCdEfGh12345678"],
  "dry_run": true
}
```

Review the preview, then rerun with `dry_run: false` for the actual transfer.
By default, IDs are preserved, folders recreated and existing documents skipped.
`overwrite: true` updates existing destination documents. Copy linked documents
too if you need their `@UUID` references to resolve.

For different servers, use `copy_assets` to transfer referenced images/audio
as well. It copies missing paths; existing files are skipped unless `overwrite`
is enabled. It does not compare file contents to detect changes. Both tools
have safety limits; check their schemas and results for larger transfers.

For a content migration, provision systems/modules with `admin_install_package`
and create the destination with `admin_create_world`, then launch it and configure
its credentials. Transfer folders, journals/items/tables, actors and scenes in
that order, followed by assets and user setup through `manage_users`.
Only the copy tools offer the `dry_run` preview described here. This is a content
transfer, not a complete Foundry backup: package configuration, game-specific
references and user passwords need separate attention. Verify the target world
before using it for a session.

## Troubleshooting

| Symptom | Check / next step |
|---|---|
| `/health` does not answer | Container/service logs, port mapping and proxy routing |
| `/health` works but `ping` is disconnected | Launched world, host reachable from inside the container, bot ID/password; the Gateway retries connections |
| Local Foundry cannot be reached from Docker | Use `host.docker.internal` plus the Linux host mapping, or a shared Docker network; container `localhost` points at itself |
| MCP URL returns 404 | Exact `/mcp-<token>` path, token expiry, root key and revocation list; authentication failures intentionally return 404 |
| All Companion tools time out | Module enabled, matching major.minor, an awake GM browser, and a trusted key matching the running Gateway |
| Signature rejected after restart | Persist `COMPANION_SIGNING_KEY`; do not regenerate it on every start; install the intended public key and reload |
| Signature check fails on a LAN HTTP page | Open Foundry over HTTPS or browser localhost so Web Crypto is available |
| A script or macro is refused | Verify trust, then its separate script/script-macro policy; confirm the request in the GM browser if that mode is enabled |
| A module-specific tool fails | Check the required module is installed, enabled and compatible; use `client_status` for detected dependencies |
| New tools are missing after an update | Reconnect your MCP client (Claude Code: `/mcp`), check token scope, `FOUNDRY_READONLY`, `FOUNDRY_SYSTEMS` and the actually installed version |
| Server administration tools are missing | Set `FOUNDRY_ADMIN_PASSWORD` if needed; 8 tools depend on it, while `admin_edit_world` does not |
| A browser action returns an error | Check the GM console; keep the tab awake and click once to unlock browser audio for sounds |

For setup help, use [Discussions](https://github.com/wanoo/foundry-mcp-gateway/discussions).
For a bug, include versions, deployment type and reproduction steps, with tokens,
passwords and private keys removed. Follow the [security policy](../SECURITY.md)
for private vulnerability reports.
