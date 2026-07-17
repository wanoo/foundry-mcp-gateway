# foundry-mcp-gateway (Rust) — contexte projet (Claude Code)

**Nom officiel : `foundry-mcp-gateway`** (libre sur GitHub et crates.io, aligné
sur l'alias de l'app Clever). Dossier local encore `foundry-mcp-rs` tant que
l'ancien dossier Node homonyme n'a pas été jeté.

**Réécriture Rust INDÉPENDANTE** du serveur MCP Foundry (plus un fork — dépôt
souverain). Un binaire unique = client Foundry natif + transport MCP HTTP
streamable (la passerelle Node disparaît). La référence fonctionnelle est
`../foundry-vtt-mcp` (TypeScript, 78 outils, 319 tests) qui **reste en prod**
sur l'app Clever `foundry-mcp-gateway` jusqu'à parité.

## Architecture

- `src/foundry/protocol.rs` — trames Engine.IO/Socket.IO (pur, testé) :
  `0` handshake · `2`/`3` ping-pong · `40` connect · `42[..]` broadcast ·
  `42N[..]` emit+ack · `43N[..]` réponse d'ack.
- `src/foundry/auth.rs` — /api/status (génération), GET/POST /join, binding
  v13 (query) / v14 (cookie) / inconnu (les deux), préfixe de route.
- `src/foundry/client.rs` — acteur tokio : possède le WS, corrèle les acks
  (oneshot), bufferise les broadcasts (ring 300, broadcast channel),
  reconnexion infinie à backoff 5→60 s. ⚠️ limites WS montées à 128 Mo (le
  dump du monde pèse ~18 Mo, défaut tungstenite 16 Mio = mortel).
- `src/foundry/documents.rs` — `where` (chemins pointés + __in/__contains/
  __ne/__exists), pushdown, canUseIndex (_id/name only), projection (pur, testé).
- `src/mcp/` — JSON-RPC 2.0 maison (AUCUN SDK, spec 2025-03-26) : sessions,
  POST=JSON, GET=SSE notifications, DELETE. Secret dans le chemin.
- `src/tools/` — registre + dispatch ; annotations calculées par nom.

## État du portage : ✅ PARITÉ 100 % (2026-07-17)

**78 outils** (identique au TS) : 66 génériques (13 paires get_* avec where/
pushdown/index/max_length/offset/limit, écritures, packs+index, fichiers via
manageFiles + upload HTTP multipart, compendiums via manageCompendium, séance
complète — journaux aux joueurs, share_image, pause, scènes/pull, tokens,
conditions, combat avec noms résolus, playlists, draw_from_table — settings,
ownership, import/export Markdown, credentials/instances multi, get_events +
wait_for_message) + Campaign Codex (4) + modules système : starwarsffg (7,
dont dés narratifs et MOTEUR DE DÉRIVATION des fiches), dnd5e (2, moteur d20
SRD), daggerheart (3, dés de Dualité). Prompts ×3, resources+subscribe,
notifications SSE. 22 tests unitaires (moteurs de dés déterministes,
dérivation, protocole, filtres, markdown, base64).

**Batterie de parité : 45/45 en réel** (monde v13.351) — le tout validé
contre le Foundry vivant, y compris upload PNG, blessure critique attachée
(« Hamstrung »), dérivation Pahas'Tis (Will 3 + 3 boosts), cycle Destinée.

⬜ Reste : déploiement (runtime Rust Clever — nouvelle app ou bascule de
foundry-mcp-gateway ; env identiques MCP_SECRET/FOUNDRY_CREDENTIALS_JSON/PORT)
puis DÉCOMMISSION du TS. Créer le dépôt GitHub wanoo/foundry-mcp-gateway.

## Test local

```sh
cargo test                # unitaires (protocole, filtres, auth)
FOUNDRY_CREDENTIALS_JSON="$(cat ../foundry-vtt-mcp/config/foundry_credentials.json)" \
  MCP_SECRET=test PORT=8940 cargo run
# puis POST initialize → mcp-session-id → tools/call sur http://localhost:8940/mcp-test
```

⚠️ Le fichier de creds local de ../foundry-vtt-mcp peut être écrasé par un
placeholder si son gateway est lancé sans env — le restaurer depuis
`clever env -a foundry-mcp-gateway` (sans afficher le secret).

## Règles

- Le TS reste la référence : porter les COMPORTEMENTS validés (pièges compris),
  pas réinventer. Chaque lot porté = tests unitaires + test réel contre le monde.
- Ne jamais committer de credentials. Pas de SDK MCP : la spec est implémentée
  ici, la faire évoluer consciemment.
