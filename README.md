# foundry-mcp-gateway

Serveur **MCP** (Model Context Protocol) **indépendant** pour **Foundry VTT**,
écrit en Rust. Un seul binaire : client Foundry **natif** (il se connecte en
socket.io comme un vrai joueur — aucun module à installer dans Foundry, aucun
navigateur) + transport MCP **HTTP streamable** (Claude Code, Claude Desktop,
tout client MCP).

## Pourquoi

- **Autonome 24/7** : tourne tant que le monde est lancé, reconnexion à backoff.
- **Léger** : un binaire, quelques Mo de RAM — parfait pour les plus petites
  instances cloud.
- **Compatible v13 et v14** : détection de génération via `/api/status`,
  binding de session par query (v13) ou cookie (v14), préfixes de route
  (`mon-hote.fr/mon-monde`) supportés.
- **Rapide** : lectures par collection (jamais de dump complet du monde hors
  `get_world`), pushdown des filtres dans la query serveur, index de base de
  données pour les listings légers — 6 900 journaux listés en ~0,3 s.
- **Bon citoyen MCP** : tools annotés (readOnly/destructive), resources
  paginées par curseurs, prompts, notifications SSE, souscriptions.

## Démarrage

```sh
export MCP_SECRET="un-long-secret"          # chemin /mcp-<secret>
export FOUNDRY_CREDENTIALS_JSON='[{"_id":"mon-monde","hostname":"mon-hote.fr/mon-monde","userid":"<_id User 16c>","password":"…"}]'
export PORT=8080                             # optionnel
cargo run --release
```

Côté client MCP : `https://<hote>/mcp-<secret>` en transport « streamable HTTP ».
Utiliser un compte Foundry **dédié** (rôle Gamemaster conseillé) ; l'`userid`
est l'`_id` du document User (16 caractères).

## Outils

**78 outils.** Lectures génériques (13 collections × liste/unitaire, `where` à
chemins pointés et opérateurs, projection, pagination, index BDD), écritures
(documents imbriqués, compendiums, `keep_id`), packs, recherche plein-texte,
fichiers (upload/browse/mkdir), outils de séance (diffusion aux joueurs,
scènes, tokens, conditions, combat, playlists, tables), Campaign Codex,
événements (`get_events`, `wait_for_message`), et **modules de systèmes de
jeu** : starwarsffg (dés narratifs + dérivation des fiches), dnd5e (moteur d20
SRD), daggerheart (dés de Dualité) — extensibles via `src/systems/`.

## Licence

MIT.
