#!/usr/bin/env bash
# Vérifie que les chiffres annoncés dans la doc correspondent au code.
#
# Pourquoi : les 134 outils ne sont pas comptables à la main (26 sont générés en
# boucle sur COLLECTIONS), et la doc a déjà dérivé trois fois — README annonçant
# à la fois 131 et 134, CONTRIBUTING 22 tests pour 23. Ce script est la source
# de vérité ; il tourne en CI et en local (`./scripts/check-docs.sh`).
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
note() { printf '  %-46s %s\n' "$1" "$2"; }
bad() { note "$1" "✗ $2"; fail=1; }
ok() { note "$1" "✓ $2"; }

echo "== docs vs code =="

BIN=target/release/foundry-mcp
[ -x "$BIN" ] || cargo build --release --quiet

# The full catalogue assumes the admin tools are unlocked (8 more).
total=$(FOUNDRY_ADMIN_PASSWORD=x "$BIN" --dump-tools | jq 'length')
base=$("$BIN" --dump-tools | jq 'length')
readonly_count=$(FOUNDRY_ADMIN_PASSWORD=x FOUNDRY_READONLY=1 "$BIN" --dump-tools | jq 'length')

# --- total tool count, as claimed by both READMEs
for f in README.md README.fr.md; do
  # Uniquement la forme en gras (**134 tools**) : c'est l'annonce officielle,
  # les autres nombres de la page parlent d'autre chose (lecture seule, etc.).
  claimed=$(grep -ohE '\*\*[0-9]{2,4} (tools|outils)\*\*' "$f" | grep -ohE '[0-9]{2,4}' | sort -u)
  count=$(echo "$claimed" | grep -c . || true)
  if [ "$count" -ne 1 ]; then
    bad "$f: tool count" "conflicting or missing values -> $(echo "$claimed" | tr '\n' ' ')"
  elif [ "$claimed" != "$total" ]; then
    bad "$f: tool count" "claims $claimed, the binary exposes $total"
  else
    ok "$f: tool count" "$total"
  fi
done

# --- read-only tools
for f in README.md README.fr.md; do
  if grep -qE "\b$readonly_count (read-only tools|outils de lecture)" "$f"; then
    ok "$f: read-only tools" "$readonly_count"
  else
    bad "$f: read-only tools" "expected $readonly_count, not found as written"
  fi
done

# --- unit-test count, tel qu'annoncé dans CONTRIBUTING
tests=$(cargo test --release 2>/dev/null | grep -oE '^test result: ok\. [0-9]+' | grep -oE '[0-9]+$' | paste -sd+ - | bc)
claimed_tests=$(grep -oE '[0-9]+ unit tests' CONTRIBUTING.md | grep -oE '^[0-9]+' | head -1)
if [ "$claimed_tests" = "$tests" ]; then
  ok "CONTRIBUTING.md: unit tests" "$tests"
else
  bad "CONTRIBUTING.md: unit tests" "claims ${claimed_tests:-nothing}, there are $tests"
fi

# --- every tool must be documented…
# Exception: singular reads (get_actor, get_item…) are covered collectively
# by "(+ singular forms)" in the table, not one by one.
singular='^get_(actor|item|folder|user|scene|journal|macro|card|playlist|table|message|combat|setting)$'
all_tools=$(FOUNDRY_ADMIN_PASSWORD=x "$BIN" --dump-tools | jq -r '.[].name')
for f in README.md README.fr.md; do
  missing=$(while read -r t; do
    [[ "$t" =~ $singular ]] && continue
    grep -qF "\`$t\`" "$f" || echo "$t"
  done <<<"$all_tools")
  if [ -n "$missing" ]; then
    bad "$f: undocumented tools" "$(echo "$missing" | tr '\n' ' ')"
  else
    ok "$f: every tool documented" "$total/$total"
  fi
done

# --- …and the reverse: no ghost tool in the docs (the most misleading kind)
known=$(FOUNDRY_ADMIN_PASSWORD=x "$BIN" --dump-tools | jq -r '.[].name')
for f in README.md README.fr.md docs/integrators.md; do
  ghosts=$(grep -ohE '`(get_|client_|admin_|manage_|copy_|cc_|al_|mc_|mat_|dnd5e_|dh_|roll_|adjust_|apply_|grant_|request_|draw_|move_|place_|update_|toggle_|activate_|control_|show_|share_|pull_|list_|import_|create_|delete_|modify_|browse_|upload_|search_|export_|set_|choose_|wait_|ping)[a-z0-9_]+`' "$f" \
    | tr -d '`' | sort -u | while read -r t; do
      grep -qxF "$t" <<<"$known" || echo "$t"
    done)
  if [ -n "$ghosts" ]; then
    bad "$f: non-existent tools cited" "$(echo "$ghosts" | tr '\n' ' ')"
  else
    ok "$f: no ghost tools" "—"
  fi
done

# --- the companion-command map must be in sync with the code.
# This is the contract the companion repo's check-parity.sh fetches to verify
# neither half has drifted ahead of the other. Regenerate it from the same
# table the gateway uses at runtime, so it can't silently go stale.
if diff -q docs/companion-commands.json <("$BIN" --dump-companion-commands) >/dev/null; then
  ok "docs/companion-commands.json" "up to date"
else
  bad "docs/companion-commands.json" "stale — run: $BIN --dump-companion-commands > docs/companion-commands.json"
fi

echo
if [ "$fail" -eq 0 ]; then
  echo "Docs match the code ($total tools; $base without an admin password, $readonly_count read-only)."
else
  echo "Docs have drifted — fix the lines above (or the code)." >&2
fi
exit "$fail"
