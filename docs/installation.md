# Install Gateway and Companion

[Français](installation.fr.md) · [Home](../README.md) · [Capabilities](capabilities.md) · [Advanced guide](advanced.md)

This is the recommended setup: the published Gateway Docker image plus the
Companion module in Foundry. Docker images support **amd64 and arm64**.
Native binaries and source builds are covered in the [advanced guide](advanced.md).

These commands use a POSIX shell (macOS, Linux or WSL). Keep the same terminal
open through the setup, and replace the example values with your own.

## 1. Check the prerequisites

You need Docker, a licensed Foundry installation with a **launched world**,
Gamemaster access, and an AI client supporting MCP over **Streamable HTTP**.
The Gateway does not include an AI model or a Foundry licence.

The Gateway must be able to reach Foundry, and your AI client must be able to
reach the Gateway. These are two separate connections:

| Foundry location | Example `hostname` as seen from the Gateway container |
|---|---|
| On the Docker host | `http://host.docker.internal:30000` |
| Another machine on your LAN | `http://192.168.1.50:30000` |
| Internet host, with an optional route prefix | `https://foundry.example.com/my-world` |

Inside Docker, `localhost` is the container itself. On Linux, the run command
below adds `host.docker.internal` explicitly; Docker Desktop provides it too.
For another Foundry container, use its service name on a shared Docker network
and an explicit `http://` scheme.

A local AI client can use a Gateway on `http://127.0.0.1:8080`. A remotely hosted
client needs a Gateway URL it can reach over HTTPS; the loopback-only example
below is not accessible from a hosted connector. See [hosting](advanced.md#hosting).
For the Companion, open Foundry over HTTPS or on browser localhost: signature
verification uses the browser's Web Crypto API, which needs a secure context.

## 2. Create a dedicated Foundry user

In your world, open **Configure Players → Create Additional User**. Create
`MCP-Bot`, give it the **Gamemaster** role and set a password. The Gateway uses
this account; keep your own GM account for the browser.

As a GM, open the browser console (F12 → Console) and run:

```js
game.users.getName("MCP-Bot").id
```

Copy the resulting user ID. In the configuration below, `_id` is your own
instance label (`my-world`), while `userid` is this Foundry user ID. A lower
role is possible, but Foundry will refuse operations outside that role's rights.

## 3. Generate the two keys and a client token

Select the published image:

```sh
export GATEWAY_IMAGE=wanoo/foundry-mcp-gateway:latest
docker pull "$GATEWAY_IMAGE"
docker run --rm "$GATEWAY_IMAGE" foundry-mcp --generate-mcp-key
```

The command prints an MCP root key pair. Copy its **private** value into your
terminal environment, without the `MCP_ROOT_PRIVATE_KEY=` prefix:

```sh
export MCP_ROOT_PRIVATE_KEY='REPLACE_WITH_MCP_ROOT_PRIVATE_KEY'
```

Mint a token for your AI client:

```sh
docker run --rm -e MCP_ROOT_PRIVATE_KEY "$GATEWAY_IMAGE" \
  foundry-mcp --mint-token assistant --days 365
```

Save the token and its revocation ID privately. The token goes in the connection
URL in step 5; the revocation ID lets you invalidate only this token later.
Use `--readonly` when minting a token that should only read your world.

Generate the **separate** key used to sign commands for the Companion:

```sh
docker run --rm "$GATEWAY_IMAGE" foundry-mcp --generate-key
```

Copy the printed private value into the environment:

```sh
export COMPANION_SIGNING_KEY='REPLACE_WITH_COMPANION_SIGNING_KEY'
```

Keep both private keys across restarts. The Companion signing key must stay
stable; otherwise the world's trusted public key no longer matches. Save
credentials in your own secret store, outside version control. The two key
pairs serve different connections and cannot be substituted for each other.

## 4. Start the Gateway

Set your Foundry credentials. This is a JSON array, without comments:

```sh
export FOUNDRY_CREDENTIALS_JSON='[
  {
    "_id": "my-world",
    "hostname": "http://host.docker.internal:30000",
    "userid": "AbCdEfGh12345678",
    "password": "REPLACE_WITH_BOT_PASSWORD"
  }
]'
```

Use a JSON serializer if a password contains characters needing escaping.
Start the container, passing the three environment variables you just set:

```sh
docker run -d --name foundry-mcp \
  --restart unless-stopped \
  --add-host=host.docker.internal:host-gateway \
  -p 127.0.0.1:8080:8080 \
  -e MCP_ROOT_PRIVATE_KEY \
  -e COMPANION_SIGNING_KEY \
  -e FOUNDRY_CREDENTIALS_JSON \
  "$GATEWAY_IMAGE"

curl --fail http://127.0.0.1:8080/health
```

The health endpoint confirms the HTTP server is running. It does **not** prove
that the Foundry login succeeded; `ping` checks that in step 7. Use
`docker logs foundry-mcp` to inspect connection errors. Administrative package
and server tools stay unavailable unless you add `FOUNDRY_ADMIN_PASSWORD`.

## 5. Connect your AI client

Add a Streamable HTTP MCP server with this URL, replacing `<token>` with the
actual token from step 3:

```text
http://127.0.0.1:8080/mcp-<token>
```

For Claude Code running on the same machine:

```sh
claude mcp add foundry --transport http 'http://127.0.0.1:8080/mcp-<token>'
```

For a remotely hosted connector or another client, supply your **reachable HTTPS
Gateway URL**, for example `https://gateway.example.com/mcp-<token>`, in its MCP
connection settings. Interface labels vary by client. The whole URL is a bearer
credential; do not post it in issues, screenshots or shared logs.

## 6. Install and trust the Companion

In Foundry's **Add-on Modules → Install Module → Manifest URL**, paste:

```text
https://github.com/wanoo/foundry-mcp-gateway-companion/releases/latest/download/module.json
```

Enable **Foundry MCP Companion** in your world's module settings. Keep a GM
browser connected and its tab awake. Both components must share a major.minor
version (for example 1.7.x); their patch numbers can differ. The `latest` links
install published versions, which may lag this source tree's upcoming release.

Ask your assistant:

> “Run companion_trust_key for my Foundry instance to install this Gateway's public key.”

**Reload all GM browsers afterwards.** Each browser reads the trusted key when
the module starts. With multiple worlds, pass the intended `instance` label.
The tool refuses to overwrite a different existing key; investigate that
mismatch before deliberately replacing it.

You can instead print the configured public key and paste it into
**Configure Settings → Foundry MCP Companion → Gateway public key**:

```sh
docker exec foundry-mcp foundry-mcp --print-public-key
```

Reload GM browsers after this manual setup too. Leave signature checking on
**Guarded**. Remote JavaScript is disabled by default; remote script macros
ask the GM first. No extra module is needed for the Companion's basic features.
FXMaster, Sequencer, Dice So Nice and other integrations are optional.

## 7. Verify and prepare something

Ask your assistant to run:

1. `ping` — confirms the Gateway is connected to your launched world.
2. `client_status` — confirms a Companion responder, its version and available integrations.
3. `client_scene_report` — with a scene open, checks a signed browser command.

Then try:

> “List my campaign journals and propose three preparation tasks for next session. Don't change anything yet.”

For a token minted with `--readonly`, install the trusted key using a full-access
token or the manual public-key method before switching to the read-only client.
If a check fails, follow [troubleshooting](advanced.md#troubleshooting).
