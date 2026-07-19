# What and why

<!-- What changes, and what problem it solves. Link the issue: Closes #123 -->

## How it was tested

<!-- Be specific and honest. "Unit tests only, not tried against a live world"
     is a perfectly good answer — it tells the reviewer where to look.
     If you did test live: which Foundry version, which game system? -->

- Foundry version:
- Game system / addon:

## Checklist

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
- [ ] `./scripts/check-docs.sh` (it tells you which number is off)
- [ ] `CHANGELOG.md` updated
- [ ] New tools: description written **for a model** — says *when* to use it, not just what it does
- [ ] New tools: added to both READMEs (EN + FR); read-only ones listed in `is_read_only()`

## Does this change a documented response shape?

<!-- The shapes in docs/integrators.md are a contract: other programs parse
     them. Changing one is a MAJOR version bump, so say so here. -->

- [ ] No
- [ ] Yes — `docs/integrators.md` updated and this needs a major version

## Companion module

<!-- Server and companion share a version number. A new client_* tool needs a
     handler in the companion repo — link that PR here. -->
