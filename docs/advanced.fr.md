# Configuration avancée et dépannage

[English](advanced.md) · [Accueil](../README.fr.md) · [Installation](installation.fr.md) · [Capacités](capabilities.fr.md)

Commencez par le [guide Docker](installation.fr.md) pour les clés, les identifiants
Foundry et la confiance du Companion. Ce guide couvre les autres déploiements,
les contrôles d'accès et l'exploitation courante.

## Installation native et compilation

Téléchargez le binaire adapté depuis les [versions publiées](https://github.com/wanoo/foundry-mcp-gateway/releases).
Les publications comprennent des fichiers SHA-256. Extrayez l'archive et placez
`foundry-mcp` dans votre PATH. Consultez les fichiers de la publication pour
connaître les plateformes disponibles.

Avec `cargo-binstall` installé, utilisez `cargo binstall foundry-mcp-gateway`.
Pour compiler le paquet publié, utilisez `cargo install foundry-mcp-gateway`.
La compilation des sources nécessite Rust **1.85 ou plus récent** :

```sh
git clone https://github.com/wanoo/foundry-mcp-gateway.git
cd foundry-mcp-gateway
cargo build --release
./target/release/foundry-mcp --version
```

Le binaire natif fournit les mêmes commandes de préparation :

```sh
foundry-mcp --generate-mcp-key
foundry-mcp --generate-key
```

Conservez les valeurs privées dans `MCP_ROOT_PRIVATE_KEY` et
`COMPANION_SIGNING_KEY`, puis exportez-les dans votre shell. Définissez
`FOUNDRY_CREDENTIALS_JSON` comme dans le
[guide d'installation](installation.fr.md#4-demarrer-le-gateway), avec
`http://localhost:30000` si le Gateway natif et Foundry partagent une machine.
Créez un jeton avec `foundry-mcp --mint-token assistant --days 365`, puis lancez
`foundry-mcp` (ou `./target/release/foundry-mcp` depuis le dépôt source).
Pour une compilation des sources, utilisez également ce chemin de binaire pour
toutes les commandes de préparation.

## Docker Compose

Reprenez les variables du guide d'installation. Enregistrez ce contenu dans
`compose.yaml` ; `${NAME:?message}` fait refuser une valeur absente ou vide.

```yaml
services:
  foundry-mcp:
    image: wanoo/foundry-mcp-gateway:latest
    ports:
      - "127.0.0.1:8080:8080"
    extra_hosts:
      - "host.docker.internal:host-gateway"
    environment:
      MCP_ROOT_PRIVATE_KEY: ${MCP_ROOT_PRIVATE_KEY:?Set the MCP root private key}
      COMPANION_SIGNING_KEY: ${COMPANION_SIGNING_KEY:?Set the Companion signing key}
      FOUNDRY_CREDENTIALS_JSON: ${FOUNDRY_CREDENTIALS_JSON:?Set the Foundry credentials}
    restart: unless-stopped
```

Validez avec `docker compose config --quiet`, puis lancez `docker compose up -d`.
L'image possède un contrôle `/health`. Gardez les clés stables dans votre
gestionnaire de secrets ; aucun volume n'est nécessaire pour les clés si elles
sont fournies par l'environnement. Pour un déploiement reproductible, utilisez
un tag publié précis plutôt que `latest` dès que la version correspondante existe.

## Référence des variables

| Variable | Défaut / rôle |
|---|---|
| `MCP_ROOT_PRIVATE_KEY` | Clé racine MCP privée ; nécessaire pour créer des jetons, permet aussi de les vérifier |
| `MCP_ROOT_PUBLIC_KEY` | Alternative pour vérifier uniquement ; prioritaire si les deux variables racines sont définies |
| `MCP_REVOKED_TOKENS` | Vide ; identifiants de révocation refusés, séparés par des virgules |
| `COMPANION_SIGNING_KEY` | Clé temporaire à chaque démarrage si absente ; définissez une clé privée stable pour le Companion |
| `FOUNDRY_CREDENTIALS_JSON` | Tableau JSON requis d'entrées `{ "_id", "hostname", "userid", "password" }` |
| `FOUNDRY_ADMIN_PASSWORD` | Absent ; ajoute 8 outils d'administration du serveur et des paquets Foundry |
| `FOUNDRY_READONLY` | Désactivé ; `1`, `true` ou `yes` n'expose et n'autorise que les outils de lecture seule |
| `FOUNDRY_SYSTEMS` | Tous les systèmes intégrés ; identifiants séparés par des virgules, par exemple `starwarsffg,dnd5e` |
| `FOUNDRY_ALLOW_INTERNAL_FETCH` | Déduit des instances configurées ; `1` autorise ou `0` interdit les téléchargements privés de `upload_file` ; les adresses link-local restent refusées |
| `PORT` | `8080` ; port d'écoute HTTP |
| `RUST_LOG` | `info` ; filtre de journalisation |

Un `hostname` sans schéma utilise généralement HTTPS. Les adresses de boucle
locale et `host.docker.internal` utilisent HTTP par défaut ; indiquez `http://`
explicitement pour les autres hôtes locaux. Les préfixes tels que
`https://foundry.example.com/my-world` sont pris en charge. L'`_id` de l'instance
est un nom Gateway, pas l'identifiant du monde ou de l'utilisateur Foundry.

<a id="hebergement"></a>

## Hébergement

Le Gateway peut fonctionner près de Foundry, sur votre réseau local ou sur un
hôte distant. Un Gateway distant doit pouvoir joindre Foundry, généralement
par une adresse HTTPS publique ou une connexion réseau privée. Le client MCP
doit indépendamment pouvoir joindre le Gateway.

Pour exposer le Gateway sur Internet, placez un proxy HTTPS devant lui et
transmettez les requêtes Streamable HTTP POST, GET/SSE et DELETE sur le même
chemin `/mcp-<token>`. Conservez l'en-tête `mcp-session-id` et les réponses SSE
longues. Évitez d'enregistrer les chemins contenant les jetons dans les journaux
d'accès. L'écoute locale du guide convient à un proxy sur l'hôte ; adaptez le
réseau si votre proxy fonctionne en conteneur.

### Clever Cloud

Depuis un dépôt positionné sur la version souhaitée, avec le CLI Clever configuré
et les variables de préparation exportées :

```sh
clever create --type rust foundry-mcp-gateway
clever env set MCP_ROOT_PRIVATE_KEY "$MCP_ROOT_PRIVATE_KEY"
clever env set COMPANION_SIGNING_KEY "$COMPANION_SIGNING_KEY"
clever env set FOUNDRY_CREDENTIALS_JSON "$FOUNDRY_CREDENTIALS_JSON"
clever env set CC_RUST_BIN foundry-mcp
clever deploy
```

Utilisez l'URL HTTPS accessible de l'application dans votre client MCP. Établissez
la même confiance Companion qu'avec Docker. Les offres et la facturation dépendent
de l'hébergeur ; le projet ne promet pas d'hébergement gratuit.

<a id="acces-et-confiance-du-companion"></a>

## Accès et confiance du Companion

### Accès client avec Biscuit

Biscuit authentifie et autorise la connexion **client IA → Gateway**. Créez un
jeton distinct par client ; la durée par défaut est de **365 jours** :

```sh
foundry-mcp --mint-token assistant --days 365
foundry-mcp --mint-token lore-reader --readonly --days 90
```

La commande affiche le jeton et son identifiant de révocation. Avec Docker,
remplacez `foundry-mcp` par `docker exec foundry-mcp foundry-mcp` devant les
mêmes arguments. Gardez l'URL contenant le jeton privée. Pour révoquer un jeton,
ajoutez son identifiant à `MCP_REVOKED_TOKENS`, puis redémarrez ou recréez le
Gateway avec cet environnement. Conservez les autres identifiants déjà présents.
Révoquer un ancêtre invalide aussi les jetons atténués qui en dérivent.

La clé racine privée est nécessaire à la création des jetons. Un Gateway qui se
limite à les vérifier peut utiliser `MCP_ROOT_PUBLIC_KEY`. Biscuit permet aussi
l'atténuation hors ligne sans clé privée ; le CLI intégré propose accès complet
ou lecture seule, pas de périmètre par outil ou par monde. Un jeton atteint sinon
les instances configurées avec les droits de leurs comptes bot.

Un jeton de lecture seule filtre le catalogue et refuse les écritures même si
le serveur les autorise normalement. `FOUNDRY_READONLY=1` applique cette restriction
à tous les clients. Le catalogue 1.7 complet possède 69 outils de lecture seule
avec l'administration activée, ou 67 sans elle, avant filtrage des systèmes.
Un jeton de lecture seule ne peut pas installer la clé du Companion : utilisez
d'abord un client d'accès complet ou collez la clé publique manuellement.
La lecture seule peut quand même révéler des informations MJ de votre campagne.

### Commandes signées vers le Companion

ECDSA P-256 authentifie **Gateway → Companion**. Il utilise
`COMPANION_SIGNING_KEY`, distincte de la clé racine Biscuit. Générez-la une fois
avec `--generate-key`, installez sa partie publique avec `companion_trust_key`,
puis **rechargez les navigateurs MJ**. Vous pouvez aussi coller la sortie de
`--print-public-key` dans le réglage de clé publique du Companion, puis recharger.

Si une autre clé existe, l'outil refuse de l'écraser. `force: true` remplace
volontairement cette confiance ; utilisez-le après avoir identifié quel Gateway
doit piloter le monde. Un navigateur conserve sa clé de démarrage jusqu'au
rechargement et signale toute modification du réglage pendant la session.

| Réglage Companion | Défaut | Comportement |
|---|---|---|
| Vérification de signature des commandes | Protégée / Guarded | Sans clé, refuse les commandes exécutant du code ; avec une clé, exige des signatures valides pour toutes les commandes |
| Clé publique de la passerelle | Vide | Partie publique de la clé de signature Companion stable |
| Exécution de script distante | Désactivée | `client_run_script` peut demander confirmation ou fonctionner automatiquement si le MJ l'autorise |
| Exécution distante de macros de script | Demander confirmation | Réglage distinct pour les macros de script invoquées à distance ; n'intercepte pas celles lancées manuellement dans Foundry |
| Relayer la télémétrie des clients | Activé, par client | Relaie sélections, cibles et scène ; chaque client peut le désactiver |

Le mode strict exige des commandes authentifiées même sans clé configurée.
Le mode désactivé supprime la vérification. Gardez le mode protégé pour le parcours
recommandé. L'accord d'exécution du code est distinct de l'authentification des
commandes ; les consignes du prompt ne l'imposent pas. La vérification nécessite
Web Crypto en contexte navigateur sécurisé (HTTPS ou localhost). Les signatures
ne chiffrent pas les réponses : le canal socket du module est partagé. En
particulier, `client_enrich` exclut les blocs secrets MJ par défaut ; les demander
explicitement les transmet sur ce canal partagé.

<a id="mises-a-jour"></a>

## Mises à jour

### De la 1.6 à la 1.7

Mettez les deux composants en 1.7.x lorsque les publications sont disponibles.
Conservez la clé racine Biscuit, les jetons clients et `COMPANION_SIGNING_KEY`
stable : la 1.7 n'exige pas de nouvelle migration de clés. Recréez le conteneur
Gateway après avoir récupéré l'image choisie, mettez le module Foundry à jour,
rechargez les navigateurs MJ et reconnectez le client MCP pour actualiser son
cache d'outils. Appelez ensuite `ping` et `client_status`.

Si vous n'aviez pas configuré de clé de signature stable en 1.6, établissez la
confiance avant de dépendre des commandes Companion. Les fiches Campaign Codex
créées par d'anciens Gateways peuvent être réparées avec `client_cc_migrate_content` ;
utilisez-le uniquement pour les fiches à migrer, après sauvegarde. Les nouvelles
fiches Campaign Codex sont cachées aux joueurs par défaut : partagez-les
explicitement lorsque c'est souhaité.

### Depuis la 1.5 ou une version antérieure

`MCP_SECRET` n'est plus pris en charge. Générez une clé racine MCP avec
`--generate-mcp-key`, créez un jeton avec `--mint-token`, remplacez l'ancien
réglage d'environnement et changez l'URL du client en `/mcp-<token>`. Séparément,
générez une clé Companion stable avec `--generate-key`, définissez
`COMPANION_SIGNING_KEY`, mettez le Companion à jour, installez la confiance et
rechargez les navigateurs MJ. Vérifiez les réglages distincts des scripts et des
macros de script distantes.

Les deux composants suivent le même **majeur.mineur** ; les correctifs peuvent
différer. `client_status` identifie un décalage de version mineure. Les changelogs
[Gateway](../CHANGELOG.md) et
[Companion](https://github.com/wanoo/foundry-mcp-gateway-companion/blob/main/CHANGELOG.md),
en anglais, détaillent les changements.

<a id="plusieurs-mondes-et-transferts"></a>

## Plusieurs mondes et transferts

Placez plusieurs objets d'identifiants dans `FOUNDRY_CREDENTIALS_JSON`. Chaque
entrée vise un monde lancé avec son nom d'instance ; le Gateway ouvre les
connexions à la demande. Un serveur Foundry n'exécute normalement qu'un monde
à la fois : deux mondes accessibles simultanément nécessitent généralement
des processus Foundry séparés.

Les outils qui agissent sur un monde acceptent un `instance` facultatif (`copy_documents` prend `from`/`to`). `show_credentials` liste les
instances sans les mots de passe ; `choose_foundry_instance` change celle par
défaut. Établissez la confiance du Companion dans chaque monde à piloter.

Pour prévisualiser `copy_documents`, transmettez ces arguments d'outil :

```json
{
  "from": "sandbox",
  "to": "live",
  "collection": "journals",
  "ids": ["AbCdEfGh12345678"],
  "dry_run": true
}
```

Examinez l'aperçu, puis recommencez avec `dry_run: false` pour effectuer le
transfert. Par défaut, les identifiants sont conservés, les dossiers recréés et
les documents existants ignorés. `overwrite: true` met à jour les documents
de destination existants. Copiez également les documents liés si vous voulez
que leurs références `@UUID` fonctionnent.

Entre serveurs différents, utilisez aussi `copy_assets` pour les images et sons
référencés. Il copie les chemins manquants ; les fichiers existants sont ignorés
sauf si `overwrite` est activé. Il ne compare pas leur contenu pour détecter
les changements. Les deux outils ont des limites de sécurité ; consultez leurs
schémas et résultats pour les transferts volumineux.

Pour migrer du contenu, préparez systèmes et modules avec `admin_install_package`
et créez la destination avec `admin_create_world`, puis lancez-la et configurez
ses identifiants. Transférez dans l'ordre dossiers, journaux/objets/tables,
acteurs et scènes, puis les fichiers et les utilisateurs via `manage_users`.
Seuls les outils de copie proposent l'aperçu `dry_run` décrit ici. Ce transfert
ne constitue pas une sauvegarde Foundry complète : réglages des paquets,
références propres au système et mots de passe demandent un traitement séparé.
Vérifiez le monde cible avant de l'utiliser en partie.

<a id="depannage"></a>

## Dépannage

| Symptôme | Vérification / action |
|---|---|
| `/health` ne répond pas | Journaux du conteneur/service, publication du port et routage du proxy |
| `/health` répond mais `ping` est déconnecté | Monde lancé, hôte accessible depuis le conteneur, identifiant et mot de passe du bot ; le Gateway réessaie de se connecter |
| Foundry local est inaccessible depuis Docker | Utilisez `host.docker.internal` et la correspondance Linux, ou un réseau Docker commun ; `localhost` vise le conteneur |
| L'URL MCP renvoie 404 | Chemin exact `/mcp-<token>`, expiration, clé racine et liste de révocation ; les échecs d'authentification renvoient volontairement 404 |
| Tous les outils Companion expirent | Module activé, même majeur.mineur, navigateur MJ avec onglet actif, clé de confiance correspondant au Gateway exécuté |
| Signature refusée après redémarrage | Conservez `COMPANION_SIGNING_KEY` au lieu de la régénérer ; installez la bonne clé publique et rechargez |
| Signature impossible sur une page HTTP du réseau local | Ouvrez Foundry en HTTPS ou sur le localhost du navigateur pour disposer de Web Crypto |
| Un script ou une macro est refusé | Vérifiez la confiance, puis son réglage script/macro ; confirmez la demande dans le navigateur MJ si ce mode est activé |
| Un outil propre à un module échoue | Vérifiez le module requis, son activation et sa compatibilité ; `client_status` indique les dépendances détectées |
| De nouveaux outils manquent après mise à jour | Reconnectez le client MCP (Claude Code : `/mcp`), vérifiez le jeton, `FOUNDRY_READONLY`, `FOUNDRY_SYSTEMS` et la version réellement installée |
| Les outils d'administration serveur manquent | Définissez `FOUNDRY_ADMIN_PASSWORD` si nécessaire ; 8 outils en dépendent, contrairement à `admin_edit_world` |
| Une action navigateur renvoie une erreur | Consultez la console MJ ; gardez l'onglet actif et cliquez une fois pour débloquer l'audio du navigateur |

Pour l'installation, utilisez les [Discussions](https://github.com/wanoo/foundry-mcp-gateway/discussions).
Pour un bug, indiquez versions, déploiement et étapes de reproduction en retirant
jetons, mots de passe et clés privées. Suivez la [politique de sécurité](../SECURITY.md)
pour les signalements privés de vulnérabilités.
