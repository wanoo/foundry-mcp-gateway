# Security Policy

## Reporting a vulnerability

**Please don't open a public issue.** Use GitHub's private vulnerability
reporting:

👉 **[Report a vulnerability](https://github.com/wanoo/foundry-mcp-gateway/security/advisories/new)**

It reaches the maintainer privately, with no email address exposed to anyone.
Useful things to include: affected version (`foundry-mcp --version`), how you
deploy it, reproduction steps, and what an attacker gains.

| | |
|---|---|
| Acknowledgement | within 5 days |
| Assessment | within 10 days |
| Fix or mitigation plan | communicated once assessed |

Coordinated disclosure: please allow 90 days before going public. You'll be
credited in the advisory unless you'd rather not be.

## Supported versions

The latest minor release is supported. Given the project's age and pace, older
minors don't receive backports — upgrading is the fix.

| Version | Supported |
|---|---|
| 1.x (latest minor) | ✅ |
| older | ❌ |

## What this software actually holds — please read

This is not an ordinary web service, and reports are more useful if you know
what's at stake:

- **It stores Foundry credentials.** `FOUNDRY_CREDENTIALS_JSON` contains the
  password of a Gamemaster account. Anyone who reads that environment owns the
  world.
- **The MCP endpoint's only protection is a biscuit token in the URL path**
  (`/mcp-<token>`). That's a deliberate trade-off: MCP clients like Claude
  Desktop cannot send custom headers. Unlike the shared secret it replaced, a
  token expires, can be scoped read-only, and can be revoked on its own — but
  it is still a bearer credential. It means the token travels in URLs and
  may land in proxy logs or browser history. **Always put HTTPS in front**, and
  treat the URL as a password.
- **Full-access tokens can exercise the bot's document rights and permitted
  actions in the connected GM browser.** Companion code runs with that
  browser's GM rights, subject to its script and macro settings. Setting
  `FOUNDRY_ADMIN_PASSWORD` also enables server/package administration.
  A read-only token is independently restricted at dispatch.
- **`client_run_script` executes arbitrary JavaScript** in the GM's browser. It
  is off by default and must be enabled explicitly in the companion module's
  settings.
- **Code can hide in data.** Monk's Active Tiles actions, core Region
  `executeScript`/`executeMacro` behaviors, Mass Edit preset scripts and Item
  Piles macros all run JavaScript from ordinary documents. Since 1.8 every
  write is inspected and such content is refused unless the GM set the
  companion's matching setting to « auto »; the gateway cannot change those
  settings, and the companion reverts changes made by clients that do not run
  it. A bypass of this guard is in scope for a report.
- **`FOUNDRY_READONLY=1`** exists for untrusted or player-facing integrations:
  it exposes only read-only tools and refuses writes at dispatch. The 1.8
  catalogue has 76 with administration enabled, or 74 without it, before
  game-system filtering. Read-only access may still reveal GM-only content.

Especially in scope for a report: credential leakage, authentication bypass on
the HTTP transport, reaching tools without a valid token, escaping the read-only
mode, and anything letting a *player* drive Gamemaster-level actions.

Out of scope: an operator who deliberately exposes the endpoint without HTTPS,
or hands the URL to someone untrusted. That's the documented threat model, not
a vulnerability.
