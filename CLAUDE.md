# foundry-mcp-rs — contexte projet (Claude Code)

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

## État du portage (vs ../foundry-vtt-mcp)

- ✅ Noyau : 36 outils (13 paires get_*, ping, get_world, get_current_scene,
  search_journals, list_compendium_packs, get_pack_documents, create/modify/
  delete_document, get_events) + resources (list curseurs a:/j:, read
  actors|journal + data CC) + prompts (session-recap, world-overview) +
  souscriptions + notifications logging/resources-updated.
- ✅ Vérifié en réel (2026-07-17, monde v13.351) : auth MCP-Bot, monde 17,8 Mo,
  6 960 journaux en 0,37 s, fraîcheur create→read→delete, packs, prompts.
- ⬜ À porter : outils de séance (journaux aux joueurs, share_image, pause,
  scènes/pull, settings, ownership, tokens, combat, playlists, draw_from_table,
  import/export, cc_*, wait_for_message, adjust destiny…) ; modules systèmes
  (starwarsffg avec dice+derived, dnd5e, daggerheart) ; create/delete_compendium,
  upload/browse/create_directory (manageFiles), show_credentials/choose_instance.
- ⬜ Déploiement : runtime Rust Clever (nouvelle app ou bascule de
  foundry-mcp-gateway une fois la parité atteinte) — env identiques
  (MCP_SECRET, FOUNDRY_CREDENTIALS_JSON, PORT).

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
