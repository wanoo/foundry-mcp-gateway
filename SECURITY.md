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
- **The MCP endpoint's only protection is a secret in the URL path**
  (`/mcp-<secret>`). That's a deliberate trade-off: MCP clients like Claude
  Desktop cannot send custom headers. It means the secret travels in URLs and
  may land in proxy logs or browser history. **Always put HTTPS in front**, and
  treat the URL as a password.
- **Anyone reaching that endpoint acts as a Gamemaster**: read and write every
  document, run macros, and — with `FOUNDRY_ADMIN_PASSWORD` set — shut worlds
  down, install packages, restore backups.
- **`client_run_script` executes arbitrary JavaScript** in the GM's browser. It
  is off by default and must be enabled explicitly in the companion module's
  settings.
- **`FOUNDRY_READONLY=1`** exists for untrusted or player-facing integrations:
  it exposes only the 50 read-only tools and refuses writes at dispatch.

Especially in scope for a report: credential leakage, authentication bypass on
the HTTP transport, reaching tools without the secret, escaping the read-only
mode, and anything letting a *player* drive Gamemaster-level actions.

Out of scope: an operator who deliberately exposes the endpoint without HTTPS,
or hands the URL to someone untrusted. That's the documented threat model, not
a vulnerability.
