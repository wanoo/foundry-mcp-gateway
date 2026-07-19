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

echo "== chiffres de la doc vs code =="

BIN=target/release/foundry-mcp
[ -x "$BIN" ] || cargo build --release --quiet

# Le catalogue complet suppose les outils admin déverrouillés (8 de plus).
total=$(FOUNDRY_ADMIN_PASSWORD=x "$BIN" --dump-tools | jq 'length')
base=$("$BIN" --dump-tools | jq 'length')
readonly_count=$(FOUNDRY_ADMIN_PASSWORD=x FOUNDRY_READONLY=1 "$BIN" --dump-tools | jq 'length')

# --- nombre total d'outils, tel qu'annoncé dans les deux READMEs
for f in README.md README.fr.md; do
  # Uniquement la forme en gras (**134 tools**) : c'est l'annonce officielle,
  # les autres nombres de la page parlent d'autre chose (lecture seule, etc.).
  claimed=$(grep -ohE '\*\*[0-9]{2,4} (tools|outils)\*\*' "$f" | grep -ohE '[0-9]{2,4}' | sort -u)
  count=$(echo "$claimed" | grep -c . || true)
  if [ "$count" -ne 1 ]; then
    bad "$f : nombre d'outils" "valeurs contradictoires ou absentes → $(echo "$claimed" | tr '\n' ' ')"
  elif [ "$claimed" != "$total" ]; then
    bad "$f : nombre d'outils" "annonce $claimed, le binaire en expose $total"
  else
    ok "$f : nombre d'outils" "$total"
  fi
done

# --- outils en lecture seule
for f in README.md README.fr.md; do
  if grep -qE "\b$readonly_count (read-only tools|outils de lecture)" "$f"; then
    ok "$f : outils lecture seule" "$readonly_count"
  else
    bad "$f : outils lecture seule" "attendu $readonly_count, introuvable tel quel"
  fi
done

# --- nombre de tests unitaires, tel qu'annoncé dans CONTRIBUTING
tests=$(cargo test --release 2>/dev/null | grep -oE '^test result: ok\. [0-9]+' | grep -oE '[0-9]+$' | paste -sd+ - | bc)
claimed_tests=$(grep -oE '[0-9]+ unit tests' CONTRIBUTING.md | grep -oE '^[0-9]+' | head -1)
if [ "$claimed_tests" = "$tests" ]; then
  ok "CONTRIBUTING.md : tests unitaires" "$tests"
else
  bad "CONTRIBUTING.md : tests unitaires" "annonce ${claimed_tests:-rien}, il y en a $tests"
fi

# --- tout outil doit être documenté…
# Exception : les lectures au singulier (get_actor, get_item…) sont couvertes
# collectivement par « (+ singular forms) » dans le tableau, pas une par une.
singular='^get_(actor|item|folder|user|scene|journal|macro|card|playlist|table|message|combat|setting)$'
missing=$(FOUNDRY_ADMIN_PASSWORD=x "$BIN" --dump-tools | jq -r '.[].name' | while read -r t; do
  [[ "$t" =~ $singular ]] && continue
  grep -qF "\`$t\`" README.md || echo "$t"
done)
if [ -n "$missing" ]; then
  bad "README.md : outils non documentés" "$(echo "$missing" | tr '\n' ' ')"
else
  ok "README.md : tous les outils documentés" "$total/$total"
fi

# --- …et réciproquement : pas d'outil fantôme dans la doc (le plus trompeur)
known=$(FOUNDRY_ADMIN_PASSWORD=x "$BIN" --dump-tools | jq -r '.[].name')
for f in README.md README.fr.md docs/integrators.md; do
  ghosts=$(grep -ohE '`(get_|client_|admin_|manage_|copy_|cc_|al_|mc_|mat_|dnd5e_|dh_|roll_|adjust_|apply_|grant_|request_|draw_|move_|place_|update_|toggle_|activate_|control_|show_|share_|pull_|list_|import_|create_|delete_|modify_|browse_|upload_|search_|export_|set_|choose_|wait_|ping)[a-z0-9_]+`' "$f" \
    | tr -d '`' | sort -u | while read -r t; do
      grep -qxF "$t" <<<"$known" || echo "$t"
    done)
  if [ -n "$ghosts" ]; then
    bad "$f : outils inexistants cités" "$(echo "$ghosts" | tr '\n' ' ')"
  else
    ok "$f : aucun outil fantôme" "—"
  fi
done

echo
if [ "$fail" -eq 0 ]; then
  echo "Doc conforme au code ($total outils, dont $base sans mot de passe admin, $readonly_count en lecture seule)."
else
  echo "Doc en dérive — corrige les lignes ci-dessus (ou le code)." >&2
fi
exit "$fail"
