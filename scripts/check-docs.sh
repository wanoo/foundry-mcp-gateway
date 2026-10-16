#!/usr/bin/env bash
# Compare the bilingual documentation with the runtime tool registry.
# Coverage belongs to each language's README + guides, not to the README alone.
set -euo pipefail
cd "$(dirname "$0")/.."

# The check describes the full catalogue, independently of the caller's setup.
unset FOUNDRY_ADMIN_PASSWORD FOUNDRY_READONLY FOUNDRY_SYSTEMS

fail=0
note() { printf '  %-46s %s\n' "$1" "$2"; }
bad() { note "$1" "✗ $2"; fail=1; }
ok() { note "$1" "✓ $2"; }

# Return all distinct numbers claimed by a phrase, including bold claims.
claims() {
  local pattern=$1
  shift
  { grep -ohE "$pattern" "$@" || true; } | grep -oE '[0-9]+' | sort -u || true
}
check_claim() {
  local label=$1 expected=$2 pattern=$3
  shift 3
  local claimed
  claimed=$(claims "$pattern" "$@")
  if [ "$claimed" = "$expected" ]; then
    ok "$label" "$expected"
  else
    bad "$label" "expected $expected; found ${claimed:-nothing}"
  fi
}

echo "== docs vs code =="

BIN=target/release/foundry-mcp
[ -x "$BIN" ] || cargo build --release --quiet
catalogue=$(FOUNDRY_ADMIN_PASSWORD=docs-check "$BIN" --dump-tools)
total=$(jq 'length' <<<"$catalogue")
base=$("$BIN" --dump-tools | jq 'length')
readonly_count=$(FOUNDRY_ADMIN_PASSWORD=docs-check FOUNDRY_READONLY=1 "$BIN" --dump-tools | jq 'length')
companion_count=$("$BIN" --dump-companion-commands | jq 'length')
all_tools=$(jq -r '.[].name' <<<"$catalogue")

# Check each language separately so one translation cannot cover for the other.
for language in en fr; do
  suffix=""
  [ "$language" = en ] || suffix=".fr"
  readme="README$suffix.md"
  docs=("$readme" "docs/installation$suffix.md" "docs/capabilities$suffix.md" "docs/advanced$suffix.md")
  for f in "${docs[@]}"; do
    if [ ! -f "$f" ]; then
      bad "$language: documentation file" "missing $f"
      exit 1
    fi
  done

  check_claim "$readme: tool count" "$total" '\*\*[0-9]{2,4} (tools|outils)\*\*' "$readme"
  check_claim "$language: catalogue claims" "$total" '\*\*[0-9]{2,4} (tools|outils)\*\*' "${docs[@]}"
  check_claim "$language: base tools" "$base" '[0-9]+ (without an admin password|sans mot de passe administrateur)' "${docs[@]}"
  check_claim "$language: read-only tools" "$readonly_count" '[0-9]+ (read-only tools|outils de lecture seule)' "${docs[@]}"
  check_claim "$language: companion tools" "$companion_count" '[0-9]+ (companion tools|outils compagnon)' "${docs[@]}"

  # Every name, including singular reads, must be in this language's docs.
  missing=$(while read -r tool; do
    grep -qF "\`$tool\`" "${docs[@]}" || echo "$tool"
  done <<<"$all_tools")
  if [ -n "$missing" ]; then
    bad "$language: undocumented tools" "$(echo "$missing" | tr '\n' ' ')"
  else
    ok "$language: every tool documented" "$total/$total"
  fi
done

# No non-existent tool names in either translation or the integrator contract.
for f in README.md README.fr.md docs/installation*.md docs/capabilities*.md docs/advanced*.md docs/integrators.md; do
  ghosts=$({ grep -ohE '`((get_|client_|admin_|manage_|copy_|companion_|cc_|al_|mc_|cd_|ls_|mp_|mat_|dnd5e_|dh_|roll_|adjust_|apply_|grant_|request_|draw_|move_|place_|update_|toggle_|activate_|control_|show_|share_|pull_|list_|import_|create_|delete_|modify_|browse_|upload_|search_|export_|set_|choose_|wait_)[a-z0-9_]+|ping)`' "$f" || true; } \
    | tr -d '`' | sort -u | while read -r tool; do
      grep -qxF "$tool" <<<"$all_tools" || echo "$tool"
    done)
  if [ -n "$ghosts" ]; then
    bad "$f: non-existent tools cited" "$(echo "$ghosts" | tr '\n' ' ')"
  else
    ok "$f: no ghost tools" "—"
  fi
done

# Preserve the existing unit-test-count check from CONTRIBUTING.
tests=$(cargo test --release 2>/dev/null | grep -oE '^test result: ok\. [0-9]+' | grep -oE '[0-9]+$' | paste -sd+ - | bc)
claimed_tests=$(grep -oE '[0-9]+ unit tests' CONTRIBUTING.md | grep -oE '^[0-9]+' | head -1)
if [ "$claimed_tests" = "$tests" ]; then
  ok "CONTRIBUTING.md: unit tests" "$tests"
else
  bad "CONTRIBUTING.md: unit tests" "claims ${claimed_tests:-nothing}, there are $tests"
fi

# This exported contract is also consumed by the Companion's parity check.
if diff -q docs/companion-commands.json <("$BIN" --dump-companion-commands) >/dev/null; then
  ok "docs/companion-commands.json" "up to date"
else
  bad "docs/companion-commands.json" "stale — regenerate with --dump-companion-commands"
fi

echo
if [ "$fail" -eq 0 ]; then
  echo "Docs match the code ($total tools; $base without an admin password, $readonly_count read-only, $companion_count companion)."
else
  echo "Docs have drifted — fix the lines above (or the code)." >&2
fi
exit "$fail"
