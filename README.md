# foundry-mcp-rs

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

Lectures génériques (13 collections × liste/unitaire, `where` à chemins pointés
et opérateurs `__in`/`__contains`/`__ne`/`__exists`, projection, pagination),
écritures (`create/modify/delete_document`, documents imbriqués via
`parent_uuid`, compendiums via `pack`), packs, recherche plein-texte,
événements. *(Portage en cours depuis l'implémentation TypeScript de référence :
outils de séance, Campaign Codex, combat, playlists et modules de systèmes de
jeu — starwarsffg, dnd5e, daggerheart — arrivent par lots.)*

## Licence

MIT.
