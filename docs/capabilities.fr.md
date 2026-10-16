# Capacités et intégrations

[English](capabilities.md) · [Accueil](../README.fr.md) · [Installation](installation.fr.md) · [Guide avancé](advanced.fr.md)

Le catalogue 1.7 comprend **166 outils** : 158 sans mot de passe administrateur,
plus 8 outils d'administration activés par `FOUNDRY_ADMIN_PASSWORD`. Il compte
69 outils de lecture seule et 46 outils compagnon (inclus dans le total).
Un jeton de lecture seule ou `FOUNDRY_READONLY=1` réduit le catalogue exposé ;
`FOUNDRY_SYSTEMS` permet aussi de le restreindre.

Les outils de documents fonctionnent avec tous les systèmes de jeu. Les noms
commençant par `client_` nécessitent le Companion et un navigateur MJ connecté
avec son onglet actif. Les autres intégrations nécessitent le système ou le
module Foundry correspondant, sauf indication contraire.
Les dépendances facultatives sont précisées ci-dessous ; un module absent
produit un message d'erreur. `foundry-mcp --dump-tools` donne les schémas de
votre déploiement.

## 📖 Préparer — construire et interroger votre monde

| Outils | Pour quoi |
|---|---|
| `get_actors` `get_items` `get_journals` `get_scenes` `get_tables` `get_macros` `get_playlists` `get_cards` `get_combats` `get_messages` `get_folders` `get_users` `get_settings` (+ formes singulières) | Lire **toutes** les collections — filtres à chemins pointés et opérateurs (`__in`, `__contains`, `__ne`, `__exists`), projection, pagination. |
| `search_journals` · `export_journals` | Recherche plein texte · export Markdown de tout votre lore |
| `create_document` `modify_document` `delete_document` | Tout écrire : journaux, acteurs, scènes… docs imbriqués via `parent_uuid`, compendiums via `pack` |
| `list_compendium_packs` `get_pack_documents` `import_from_compendium` `create_compendium` `delete_compendium` | Le cycle compendium complet |
| `browse_files` `create_directory` `upload_file` | Fichiers — uploadez cartes et illustrations par URL ou base64 |
| `get_world` `ping` `get_setting` `set_setting` `show_credentials` `choose_foundry_instance` | Métadonnées, santé, réglages, multi-mondes |
| `companion_trust_key` | Installe la clé de signature de la passerelle dans le monde, pour que le compagnon accepte ses commandes |

## 🎬 Animer — la télécommande du MJ

| Outils | Pour quoi |
|---|---|
| `show_journal_to_players` · `share_image` | « Regardez tous ça » — handouts, révélations |
| `activate_scene` `get_current_scene` `pull_users_to_scene` | Changer de scène, y entraîner le groupe |
| `list_tokens` `place_token` `move_token` `update_token` | Les tokens de la scène vivante |
| `toggle_actor_condition` (27 statuts) · `apply_critical_injury` | Conditions et critiques |
| `manage_combat` (création / initiative / tours / fin) · `get_combat` | Tout le cycle du combat |
| `control_playlist` · `draw_from_table` | Musique · tirages sur vos tables d100 |
| `toggle_pause` · `wait_for_message` · `get_events` | Pause · attendre la réponse chat d'un joueur · flux d'événements en direct |
| `list_actor_ownership` `set_actor_ownership` · `grant_xp` | Permissions et récompenses |

## 👁️ Percevoir — l'IA *voit* votre table *(compagnon)*

| Outil | Pour quoi |
|---|---|
| 📸 `client_capture` | Capture de la vue MJ, renvoyée en vraie image — l'IA voit littéralement la carte |
| 🗺️ `client_scene_report` | La scène lisible par machine : tokens en cases, disposition, visibilité **réelle**, portes, lumières, gabarits |
| 📊 `client_get_derived` | Les valeurs **préparées** d'une fiche (post-`prepareData` + effets actifs) — la source stocke souvent 0 là où le joueur voit la vraie stat |
| 🔗 `client_enrich` | HTML enrichi : liens `@UUID` résolus, jets inline évalués, secrets MJ sur demande explicite uniquement ; les réponses passent par un canal socket partagé |
| 🔎 `client_search` | Recherche par nom sur toutes les collections via l'index client |
| 🌍 `client_babele` | Traductions [Babele](https://foundryvtt.com/packages/babele) : recherche inverse par nom **affiché** (retrouver « Force Lightning » depuis « Éclair de Force »), index et documents traduits |

## 🗣️ Interagir — un dialogue avec vos joueurs *(compagnon)*

| Outil | Pour quoi |
|---|---|
| ❓ `client_ask` | Poser une question dans un vrai dialogue **sur l'écran d'un joueur** et récupérer sa réponse |
| 🎲 `client_roll_formula` | **Toute formule, tout système** via le vrai moteur `Roll` de Foundry — carte de chat native, Dice So Nice, données de l'acteur, détail dé par dé |
| 📣 `client_notify` · 🔔 `client_ping` · 🎥 `client_pan_camera` | Notifications, pings de carte, mouvements de caméra « regardez ici » — tous ciblables (`gm` / `players` / ids) |
| 📜 `client_show_document` | Ouvrir une fiche chez les clients ciblés |
| 🎯 `client_select` / `client_target` · 🌫️ `client_fog` | Sélection et cibles réelles · réinitialiser le brouillard exploré |
| 📡 `client_get_state` · `client_status` | Qui est connecté, qui regarde quoi · santé du compagnon et dépendances détectées |
| 🚀 `client_run_macro` · `client_run_script` | Toute macro (le passe-partout) · JS arbitraire (**désactivé par défaut**, opt-in MJ) |

## 🌦️ Ambiance — la mise en scène *(compagnon)*

| Outil | Pour quoi |
|---|---|
| 🌧️ `client_weather` / `client_weather_types` | Particules [FXMaster](https://foundryvtt.com/packages/fxmaster) : pluie, brouillard, braises, neige, chauves-souris… |
| ✨ `client_play_effect` · `client_seq_between` · `client_seq_sound` | Effets [Sequencer](https://foundryvtt.com/packages/sequencer) sur un point, **entre deux tokens** (projectiles !), sons |
| 🎇 `client_token_fx` / `client_token_fx_presets` | [Token Magic FX](https://foundryvtt.com/packages/tokenmagic) : lueur, feu, ombre… 70 préréglages |
| 🗂️ `client_effect_catalog` | Chercher dans la base d'effets installée (JB2A & co) un chemin valide |
| 🔊 `client_play_sound` | Un sting sonore ponctuel, sans playlist |
| 🎞️ `mat_list` · `client_mat_trigger` | [Monk's Active Tiles](https://foundryvtt.com/packages/monks-active-tiles) : lister et déclencher les tuiles-actions |

## 🎲 Dés et systèmes de jeu

La logique spécifique vit dans des modules enfichables ([ajoutez le vôtre !](../CONTRIBUTING.md)) :

| Système | Outils |
|---|---|
| **Star Wars FFG** | `roll_actor_skill` (pool **dérivé de la fiche** : espèce, talents, équipement), `roll_ffg_pool`, `request_player_roll` (bouton de chat qui ouvre le dialogue du joueur), `adjust_actor_stats`, `adjust_destiny`, `grant_xp`, `apply_critical_injury` · compagnon : `client_roll_pool_native` (**vrai moteur FFG + dés 3D Dice So Nice**) |
| **D&D 5e** | `dnd5e_roll_check` (modificateurs dérivés de la fiche, avantage, DD, 20/1 naturels), `dnd5e_adjust_stats` |
| **Daggerheart** | `dh_roll_duality` (2d12 Espoir/Peur), `dh_roll_actor_trait`, `dh_adjust_stats` |

Tous chargés par défaut ; restreignez avec `FOUNDRY_SYSTEMS=starwarsffg,dnd5e`.

## 🗃️ Gestion de campagne — la suite [wgtnGM](https://campaigncodex.wgtngm.com/)

| Addon | Outils | Pour quoi faire |
|---|---|---|
| **[Campaign Codex](https://foundryvtt.com/packages/campaign-codex)** 6.9 | `cc_list_sheets` `cc_get_sheet` `cc_create_sheet` `cc_link` | Fiches au format 6.x (pages gérées Information / Notes MJ, cachées aux joueurs sauf `player_visible`), liens écrits **des deux côtés** pour chaque relation, avec suppression et garde-fous contre les cycles |
| | `cc_quest_update` / `cc_objective` | Quêtes : statut, épinglage, urgence, donneur, fiches liées, récompenses, suivi, annonce facultative dans le chat — et l'arbre des objectifs |
| | `cc_set_visibility` / `cc_set_content` | Ce que voient les joueurs (fiche, onglets réservés au MJ, liens masqués) et la prose (Information, Notes MJ, onglets personnalisés) |
| | `cc_shop_set_item` · compagnon : `client_cc_shop_add_item` | Inventaire de boutique : ajout via Campaign Codex, puis quantité, prix, stock infini, visibilité, majoration |
| | `cc_webhub_status` / `cc_webhub_exclude` | Web Hub Sync : quelles fiches sont synchronisées, exclure les fiches sensibles — un drapeau, **jamais** de synchronisation lancée |
| | compagnon : `client_cc_convert` `client_cc_open_toc` `client_cc_open_quest_board` `client_cc_open_sheet` `client_cc_migrate_content` `client_cc_export_obsidian` | Convertir des journaux en fiches, ouvrir le Hub / le tableau des quêtes / une fiche sans les refermer, réparer les anciennes fiches, export Obsidian |
| **Asset Librarian** 1.4 | `al_tag` `al_find` `al_list_tags` `al_image_tags` · compagnon : `client_al_search` `client_al_open` | Étiqueter (catégorie + tags de filtre, en tableaux), retrouver par tag, lister le registre et les groupes de tags, lire les tags d'images, chercher et ouvrir le navigateur déjà filtré |
| **Mini Calendar** 10 | `mc_get_time` `mc_set_time` `mc_list_notes` `mc_add_note` `mc_update_note` `mc_delete_note` | Heure du monde, et notes du calendrier lues et écrites comme le module les stocke |
| | `mc_get_weather` `mc_weather_types` `mc_calendar_info` · compagnon : `client_mc_set_weather` | Prévisions (°C et °F), types de météo, configuration du calendrier (mois, saisons, lunes, soleil) ; forcer la météo d'un jour |
| | compagnon : `client_mc_get_date` `client_mc_set_date` `client_mc_set_time` `client_mc_open` | Date en direct (saison, phases de lune), aller à une date ou à l'aube / midi / crépuscule / minuit, *suivant* ou dans N jours |
| **Mini Countdown Dice** | `cd_list` / `cd_set` | Horloges de menace et comptes à rebours visibles de tous : créer, fixer, ±delta, décompte auto au tour ou au round, montrer/cacher, verrouiller sur une scène |
| **Mini Light Switch** | `ls_lights` / `ls_toggle` | Lister les lumières d'une scène, les allumer/éteindre par id ou tag Tagger, poser le décor animé — fonctionne même sans le module |
| **Mini Player** | `mp_list_presets` `mp_scene_audio` · compagnon : `client_mp_play_preset` `client_mp_play_tags` | Lancer une ambiance enregistrée ou de la musique par tags ; régler l'audio joué à l'activation d'une scène |
| **Mini Rolltable** | `draw_from_table` | Quantités (`1d4+1` gobelins) et tables liées respectées, jusqu'à 5 niveaux |

## 🛠️ Administration — le monde lui-même

`manage_modules`, `manage_users` et `admin_edit_world` sont disponibles sans mot de passe administrateur. Les autres outils administrateur n'apparaissent que
si `FOUNDRY_ADMIN_PASSWORD` est défini.

| Outil | Pour quoi |
|---|---|
| 🩺 `admin_status` | `/api/status` — marche même monde éteint |
| ✏️ `admin_edit_world` | Titre, description, image de fond, prochaine session — monde allumé |
| 🧩 `manage_modules` | Installés vs activés (avec versions) · activer/désactiver |
| 👥 `manage_users` | Comptes joueur / MJ / assistant MJ : lister, créer, changer les rôles, assigner un personnage (les mots de passe restent dans Foundry) |
| 💾 `admin_list_backups` / `admin_backup_world` | Les sauvegardes natives de Foundry — et `admin_update_package` en prend une automatiquement avant |
| ⏻ `admin_shutdown_world` / `admin_launch_world` | Éteindre et lancer des mondes (le bot se reconnecte seul) |
| ⬆️ `admin_check_package` / `admin_update_package` | Mettre à jour modules, **systèmes**, mondes — check → install → vérification, refuse si un monde tourne |

## Documents individuels

Les lectures de collections possèdent aussi des formes singulières : `get_actor`,
`get_item`, `get_journal`, `get_scene`, `get_table`, `get_macro`, `get_playlist`,
`get_card`, `get_combat`, `get_message`, `get_folder`, `get_user` et `get_setting`.

## Transférer et préparer des mondes

| Outil | Utilité |
|---|---|
| `copy_documents` | Transférer des documents et leur arborescence de dossiers entre instances ; prévisualiser avec `dry_run` |
| `copy_assets` | Copier les fichiers absents entre les stockages, ou écraser explicitement les existants ; prévisualiser avec `dry_run` |
| `admin_install_package` | Installer un système, module ou monde sur le serveur cible ; nécessite le mot de passe administrateur |
| `admin_create_world` | Créer un monde de destination ; nécessite le mot de passe administrateur |

Le [guide avancé](advanced.fr.md#plusieurs-mondes-et-transferts) explique l'ordre
des opérations, la conservation des identifiants et les limites du transfert.

## Au-delà des appels d'outils

Les ressources MCP exposent acteurs et journaux avec pagination par curseur.
Les prompts `session-recap`, `world-overview` et `prep-checklist` apportent les
données du monde à la préparation. Les abonnements aux ressources et le flux
SSE relaient les changements aux clients connectés. Le transport négocie MCP
2025-06-18 ou 2025-03-26 via Streamable HTTP.

Vous développez un client ? Le [contrat d'intégration](integrators.md), en anglais,
décrit les réponses. Un autre système ou module vous intéresse ?
[Les contributions sont bienvenues](../CONTRIBUTING.md).
