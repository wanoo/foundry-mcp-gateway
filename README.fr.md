# foundry-mcp-gateway

**🇬🇧 [English version](README.md)**

Un serveur **MCP** (Model Context Protocol) **indépendant** pour **Foundry VTT**,
écrit en Rust. Un seul binaire qui se connecte à votre monde Foundry **comme un
joueur ordinaire** (client socket natif — aucun module à installer dans Foundry,
aucun navigateur à laisser ouvert) et l'expose à n'importe quel client MCP :
Claude Code, Claude Desktop, ou tout ce qui parle MCP en HTTP streamable.

Laissez votre assistant IA lire vos journaux, lancer les dés, mener les combats,
gérer vos compendiums, préparer vos séances — 24 h/24, tant que le monde tourne.

## Points forts

- **Zéro empreinte sur Foundry** — le serveur parle le même protocole socket que
  le client officiel. Compatible Foundry **v13 et v14** (binding de session
  auto-détecté), y compris les instances servies sous un préfixe de route
  (`mon-hote.fr/mon-monde`).
- **Un petit binaire unique** — Rust, quelques Mo de RAM. À l'aise sur les plus
  petites instances cloud.
- **Rapide** — lectures par collection (jamais de dump complet du monde hors
  `get_world`), filtres poussés côté serveur, listings par index de base de
  données : ~7 000 journaux listés en ~0,3 s.
- **100 outils** — CRUD générique des documents, outils MJ de séance (montrer un
  journal aux joueurs, combats, playlists, tokens…), plus des **modules de
  systèmes de jeu** (Star Wars FFG, D&D 5e, Daggerheart) que chacun peut étendre.
- **Bon citoyen MCP** — annotations d'outils (lecture seule/destructif),
  resources paginées, prompts, souscriptions, notifications SSE.
- **Auto-réparant** — reconnexion infinie avec backoff ; survit aux redémarrages
  du monde et même à une migration serveur v13→v14 en vol.

## Installation (pas à pas, sans magie noire)

### 1. Créer un utilisateur Foundry dédié au bot

Dans Foundry, en MJ : **Configuration des joueurs → Créer un utilisateur**.
Nommez-le p. ex. `MCP-Bot`, donnez-lui le rôle **Gamemaster** (ou moins pour un
bot restreint) et un **mot de passe**.

Il vous faut ensuite l'**`_id` de 16 caractères** de cet utilisateur — le plus
simple une fois le serveur lancé est l'outil `get_users`, mais pour la première
installation :

```sh
curl -s https://VOTRE-HOTE/join | grep -o '{"name":"MCP-Bot"[^}]*'
# → ..."_id":"AbCdEfGh12345678"...
```

### 2. Configurer les trois variables d'environnement

```sh
# Le secret qui protège votre endpoint (chemin d'URL : /mcp-<secret>)
export MCP_SECRET="une-longue-chaine-aleatoire"

# Qui se connecte, où. hostname PEUT inclure un préfixe de route.
export FOUNDRY_CREDENTIALS_JSON='[{
  "_id": "mon-monde",
  "hostname": "mon-hote.fr/mon-monde",
  "userid": "AbCdEfGh12345678",
  "password": "le-mot-de-passe-du-bot"
}]'

# Optionnel (défaut 8080)
export PORT=8080
```

Plusieurs mondes/instances ? Mettez plusieurs objets dans le tableau et basculez
à chaud avec l'outil `choose_foundry_instance`.

### 3. Lancer le serveur

**En local / sur tout serveur avec Rust :**

```sh
cargo run --release        # binaire : target/release/foundry-mcp
```

**Clever Cloud (5 commandes) :**

```sh
clever create --type rust foundry-mcp-gateway
clever env set MCP_SECRET "une-longue-chaine-aleatoire"
clever env set FOUNDRY_CREDENTIALS_JSON '[{"_id":"…","hostname":"…","userid":"…","password":"…"}]'
clever env set CC_RUST_BIN foundry-mcp
clever deploy
```

**Ailleurs :** compilez une fois (`cargo build --release`), copiez l'unique
binaire `foundry-mcp`, posez les variables d'env, servez derrière HTTPS.

Vérifiez : `curl https://VOTRE-DEPLOIEMENT/health` → `ok`.
Le monde doit être **lancé** (page de connexion visible) pour que le bot se
connecte ; s'il est éteint, le serveur attend et se reconnecte tout seul.

### 4. Brancher votre client MCP

```sh
# Claude Code
claude mcp add foundry --transport http https://VOTRE-DEPLOIEMENT/mcp-<secret>
```

Claude Desktop : *Paramètres → Connecteurs → Ajouter un connecteur personnalisé*
avec la même URL (le secret vit dans l'URL car Desktop ne sait pas poser
d'en-têtes).

## Les 100 outils

### Génériques (66) — pour tous les systèmes de jeu

| Catégorie | Outils | Notes |
|---|---|---|
| **Lecture** | `get_actors`/`get_actor`, `get_items`/`get_item`, `get_journals`/`get_journal`, `get_scenes`/`get_scene`, `get_folders`, `get_users`, `get_macros`, `get_cards`, `get_playlists`, `get_tables`, `get_combats`, `get_messages`, `get_settings` (+ formes singulières) | filtres `where` à chemins pointés et opérateurs (`__in`, `__contains`, `__ne`, `__exists`), projection de champs, `offset`/`limit`/`max_length`, index BDD automatique pour les listings légers |
| **Écriture** | `create_document`, `modify_document`, `delete_document` | documents imbriqués via `parent_uuid`, compendiums via `pack`, `keep_id` |
| **Compendiums** | `list_compendium_packs`, `get_pack_documents`, `import_from_compendium`, `create_compendium`, `delete_compendium` | l'index BDD sert aussi aux packs |
| **Fichiers** | `browse_files`, `create_directory`, `upload_file` (URL ou base64) | |
| **Séance (MJ)** | `show_journal_to_players`, `share_image`, `toggle_pause`, `activate_scene`, `get_current_scene`, `pull_users_to_scene`, `list_tokens`, `place_token`, `move_token`, `update_token`, `toggle_actor_condition` (27 statuts core), `manage_combat` (création/initiative/tours/statut/fin), `control_playlist`, `draw_from_table` (tables d100 de critiques & co) | |
| **Addons famille CC** | Campaign Codex, Asset Librarian, Mini Calendar — voir la section dédiée ci-dessous | suite [wgtnGM](https://campaigncodex.wgtngm.com/) |
| **Événements** | `get_events` (polling incrémental), `wait_for_message` (attente bloquante d'un message d'un autre client) | |
| **Divers** | `ping` (santé, léger), `get_world`, `search_journals` (plein-texte), `export_journals` (Markdown), `set_setting`, `list_actor_ownership`, `set_actor_ownership`, `show_credentials`, `choose_foundry_instance` | |

### Modules de systèmes de jeu (12)

| Système | Outils |
|---|---|
| **Star Wars FFG** (`starwarsffg`) | `roll_actor_skill` (le vrai pool de dés **dérivé de la fiche** — valeurs stockées + mods d'espèce/équipement/talents appris), `roll_ffg_pool` (dés narratifs côté serveur, faces officielles), `request_player_roll` (bouton de chat qui ouvre le dialogue de jet pré-rempli), `adjust_actor_stats` (blessures/stress/crédits/XP/obligation/devoir/moralité + coque/surcharge des véhicules), `adjust_destiny` (points de Destinée), `grant_xp`, `apply_critical_injury` (+10 par blessure existante, attache l'item du compendium) |
| **D&D 5e** (`dnd5e`) | `dnd5e_roll_check` (carac/compétence/sauvegarde, modificateurs dérivés de la fiche, avantage/désavantage, DD, 20/1 naturels), `dnd5e_adjust_stats` (pv plafonnés au max, pv temporaires, xp, épuisement, monnaies) |
| **Daggerheart** (`daggerheart`) | `dh_roll_duality` (2d12 Espoir/Peur, doubles = critique, ±d6 d'avantage), `dh_roll_actor_trait`, `dh_adjust_stats` (points de vie/stress/espoir, bornés) |

Tous les modules sont chargés par défaut ; restreignez avec
`FOUNDRY_SYSTEMS=starwarsffg,dnd5e`.

### Addons famille CC — la suite [wgtnGM](https://campaigncodex.wgtngm.com/)

Outils regroupés pour la famille de modules Campaign Codex. Les outils serveur
agissent sur les documents ; les `client_*` requièrent le module compagnon.

| Addon | Outils |
|---|---|
| **[Campaign Codex](https://foundryvtt.com/packages/campaign-codex)** | `cc_list_sheets`, `cc_get_sheet`, `cc_create_sheet`, `cc_link` (bidirectionnel) · compagnon : `client_cc_convert` (journal → fiche CC, migration en masse), `client_cc_export_obsidian`, `client_cc_open_toc` |
| **Asset Librarian** | `al_tag` / `al_find` (lire & écrire les tags `flags.asset-librarian` sur les documents) · compagnon : `client_al_open` (navigateur d'assets filtré) |
| **Mini Calendar** | `mc_get_time` / `mc_set_time` (temps du monde via `core.time`), `mc_list_notes` (journaux de notes du calendrier) · compagnon : `client_mc_set_time` (dont dawn/dusk), `client_mc_open` |

### Capacités MCP au-delà des outils

- **Resources** : parcourez les acteurs (JSON) et journaux (HTML, données
  Campaign Codex jointes) avec pagination par curseurs — épinglez-les dans le
  contexte de votre client.
- **Prompts** : `session-recap`, `world-overview`, `prep-checklist` — des
  gabarits MJ remplis avec l'état live du monde.
- **Souscriptions & notifications** : souscrivez à l'URI d'un document et
  recevez `resources/updated` ; chaque broadcast Foundry est relayé en
  notification de logging sur le flux SSE.
- **Annotations** : les outils en lecture seule sont marqués (auto-approbables
  par les clients) ; seuls les deux `delete_*` sont marqués destructifs.

## Actions côté client (module compagnon optionnel) — 11 outils

Le protocole socket ne touche que des *documents*. Pour atteindre l'API
navigateur, installez le module optionnel
**[foundry-mcp-gateway-companion](https://github.com/wanoo/foundry-mcp-gateway-companion)**.
Il ajoute ces 11 outils `client_*`. Sans lui, ils expirent avec un message clair ;
tout le reste fonctionne à l'identique.

| Outil | Ce qu'il fait (côté navigateur) |
|---|---|
| `client_status` | Le compagnon est-il installé/actif ? Renvoie sa version, le MJ qui répond, et quelles dépendances optionnelles (Dice So Nice, Campaign Codex, Sequencer) sont disponibles |
| `client_run_macro` | Exécute n'importe quelle macro Foundry (par _id ou nom) sur le client MJ (renvoie sa valeur) — le passe-partout vers tout ce qui est scriptable |
| `client_run_script` | Exécute du JS async arbitraire sur le client MJ (⚠️ désactivé par défaut ; le MJ doit l'activer dans les réglages du module) |
| `client_roll_pool_native` | *starwarsffg* : lance un pool avec le vrai moteur FFG — carte de chat native + dés 3D **Dice So Nice** sur la table |
| `client_pan_camera` | Déplace/zoome la caméra des clients ciblés — « regardez tous ici » (x/y ou un _id de token) |
| `client_ping` | Ping un point de la carte pour les clients ciblés |
| `client_play_sound` | Son ponctuel (un *stinger* dramatique) sur les clients ciblés, sans playlist |
| `client_notify` | Notification UI (info/warn/error) sur les clients ciblés |
| `client_show_document` | Ouvre une fiche (par uuid) sur les clients ciblés |
| `client_play_effect` | Un effet visuel [Sequencer](https://foundryvtt.com/packages/sequencer) sur un token ou un point (si installé) |
| `client_get_state` | Télémétrie : utilisateurs actifs, scène affichée et personnage de chacun. Les sélections/cibles en direct arrivent via `get_events` |

Les actions de scène (`client_pan_camera`, `client_ping`, `client_play_sound`,
`client_notify`, `client_show_document`) acceptent un argument `targets`
(`all` / `gm` / `players` / liste d'_id d'utilisateurs). Les intégrations
optionnelles dégradent proprement : `client_roll_pool_native` requiert
starwarsffg, `client_play_effect` requiert Sequencer, `client_cc_*` requièrent
Campaign Codex.

## Contribuer un système de jeu

Le cœur est 100 % agnostique ; tout le spécifique vit dans `src/systems/`, un
fichier par système. Pour ajouter le vôtre :

1. Créez `src/systems/<id_systeme>.rs` avec trois fonctions : `definitions()`
   (triplets nom/description/schéma JSON — préfixez les noms d'outils par l'id
   du système), `handles(name)` et `async run(state, name, args)`.
2. Enregistrez-le dans `src/systems/mod.rs` (`all_modules()`).
3. **Vérifiez vos chemins de données sur un monde réel** avant de les figer
   (les structures varient entre versions de système — notez la version
   validée). Les moteurs de dés prennent une closure `rng` injectable pour des
   tests déterministes.
4. `cargo test` + ouvrez une PR. Guide complet dans `src/systems/README.md` ;
   `swffg.rs` est l'implémentation de référence (y compris un moteur de
   dérivation de fiche pour les systèmes dont les documents source ne stockent
   pas les valeurs affichées).

## Licence

MIT.
