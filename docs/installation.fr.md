# Installer le Gateway et le Companion

[English](installation.md) · [Accueil](../README.fr.md) · [Capacités](capabilities.fr.md) · [Guide avancé](advanced.fr.md)

Voici le parcours recommandé : l'image Docker publiée du Gateway et le module
Companion dans Foundry. Les images Docker prennent en charge **amd64 et arm64**.
Les binaires natifs et la compilation sont décrits dans le [guide avancé](advanced.fr.md).

Ces commandes utilisent un shell POSIX (macOS, Linux ou WSL). Gardez le même
terminal ouvert pendant l'installation et remplacez les exemples par vos valeurs.

## 1. Vérifier les prérequis

Il vous faut Docker, une installation Foundry sous licence avec un **monde lancé**,
un accès MJ et un client IA prenant en charge MCP via **Streamable HTTP**.
Le Gateway ne fournit ni modèle IA ni licence Foundry.

Le Gateway doit pouvoir joindre Foundry et votre client IA doit pouvoir joindre
le Gateway. Ce sont deux connexions distinctes :

| Emplacement de Foundry | Exemple de `hostname` vu depuis le conteneur Gateway |
|---|---|
| Sur l'hôte Docker | `http://host.docker.internal:30000` |
| Une autre machine du réseau local | `http://192.168.1.50:30000` |
| Un hébergement Internet, avec préfixe éventuel | `https://foundry.example.com/my-world` |

Dans Docker, `localhost` désigne le conteneur lui-même. Sous Linux, la commande
ci-dessous ajoute explicitement `host.docker.internal` ; Docker Desktop le
fournit aussi. Pour Foundry dans un autre conteneur, utilisez son nom de service
sur un réseau Docker commun et indiquez explicitement le schéma `http://`.

Un client IA local peut joindre le Gateway sur `http://127.0.0.1:8080`. Un client
hébergé à distance a besoin d'une URL Gateway accessible en HTTPS ; l'exemple
limité à la boucle locale ci-dessous n'est pas accessible depuis un connecteur
hébergé. Consultez [l'hébergement](advanced.fr.md#hébergement).
Pour le Companion, ouvrez Foundry en HTTPS ou sur le localhost du navigateur :
la vérification des signatures utilise Web Crypto, qui exige un contexte sécurisé.

## 2. Créer un utilisateur Foundry dédié

Dans le monde, ouvrez **Configurer les joueurs → Créer un utilisateur**. Créez
`MCP-Bot`, attribuez-lui le rôle **Gamemaster / Maître du jeu** et un mot de passe.
Le Gateway utilise ce compte ; conservez votre compte MJ personnel pour le navigateur.

En tant que MJ, ouvrez la console du navigateur (F12 → Console) et exécutez :

```js
game.users.getName("MCP-Bot").id
```

Copiez l'identifiant obtenu. Dans la configuration ci-dessous, `_id` est votre
propre nom d'instance (`my-world`) ; `userid` est l'identifiant de cet utilisateur
Foundry. Un rôle inférieur est possible, mais Foundry refusera les opérations
qui dépassent ses permissions.

## 3. Générer les deux clés et un jeton client

Choisissez l'image publiée :

```sh
export GATEWAY_IMAGE=wanoo/foundry-mcp-gateway:latest
docker pull "$GATEWAY_IMAGE"
docker run --rm "$GATEWAY_IMAGE" foundry-mcp --generate-mcp-key
```

La commande affiche une paire de clés racines MCP. Copiez sa valeur **privée**
dans l'environnement du terminal, sans le préfixe `MCP_ROOT_PRIVATE_KEY=` :

```sh
export MCP_ROOT_PRIVATE_KEY='REPLACE_WITH_MCP_ROOT_PRIVATE_KEY'
```

Créez un jeton pour votre client IA :

```sh
docker run --rm -e MCP_ROOT_PRIVATE_KEY "$GATEWAY_IMAGE" \
  foundry-mcp --mint-token assistant --days 365
```

Conservez le jeton et son identifiant de révocation de manière confidentielle.
Le jeton entre dans l'URL de connexion à l'étape 5 ; l'identifiant permet de
révoquer uniquement ce jeton. Ajoutez `--readonly` pour un accès en lecture seule.

Générez la clé **distincte** de signature des commandes du Companion :

```sh
docker run --rm "$GATEWAY_IMAGE" foundry-mcp --generate-key
```

Copiez la valeur privée affichée dans l'environnement :

```sh
export COMPANION_SIGNING_KEY='REPLACE_WITH_COMPANION_SIGNING_KEY'
```

Conservez les deux clés privées entre les redémarrages. La clé de signature du
Companion doit rester stable ; sinon, la clé publique de confiance du monde ne
correspond plus. Enregistrez les identifiants dans votre gestionnaire de secrets,
hors du contrôle de version. Les deux paires de clés protègent des connexions
différentes et ne sont pas interchangeables.

<a id="4-demarrer-le-gateway"></a>

## 4. Démarrer le Gateway

Définissez les identifiants Foundry. C'est un tableau JSON, sans commentaires :

```sh
export FOUNDRY_CREDENTIALS_JSON='[
  {
    "_id": "my-world",
    "hostname": "http://host.docker.internal:30000",
    "userid": "AbCdEfGh12345678",
    "password": "REPLACE_WITH_BOT_PASSWORD"
  }
]'
```

Utilisez un sérialiseur JSON si votre mot de passe contient des caractères à
échapper. Démarrez le conteneur en transmettant les trois variables définies :

```sh
docker run -d --name foundry-mcp \
  --restart unless-stopped \
  --add-host=host.docker.internal:host-gateway \
  -p 127.0.0.1:8080:8080 \
  -e MCP_ROOT_PRIVATE_KEY \
  -e COMPANION_SIGNING_KEY \
  -e FOUNDRY_CREDENTIALS_JSON \
  "$GATEWAY_IMAGE"

curl --fail http://127.0.0.1:8080/health
```

Le point de contrôle confirme que le serveur HTTP répond. Il ne prouve **pas**
que l'authentification Foundry a réussi ; `ping` le vérifie à l'étape 7.
`docker logs foundry-mcp` permet de lire les erreurs de connexion. Les outils
d'administration du serveur et des paquets restent indisponibles tant que vous
n'ajoutez pas `FOUNDRY_ADMIN_PASSWORD`.

## 5. Connecter votre client IA

Ajoutez un serveur MCP Streamable HTTP avec cette URL, en remplaçant `<token>`
par le jeton de l'étape 3 :

```text
http://127.0.0.1:8080/mcp-<token>
```

Pour Claude Code exécuté sur la même machine :

```sh
claude mcp add foundry --transport http 'http://127.0.0.1:8080/mcp-<token>'
```

Pour un connecteur hébergé à distance ou un autre client, indiquez l'**URL HTTPS
accessible du Gateway**, par exemple `https://gateway.example.com/mcp-<token>`,
dans ses paramètres MCP. Les intitulés varient selon le client. L'URL entière
est un identifiant d'accès : ne la publiez pas dans un ticket, une capture ou
des journaux partagés.

## 6. Installer le Companion et établir la confiance

Dans **Modules complémentaires → Installer un module → URL du manifeste**, collez :

```text
https://github.com/wanoo/foundry-mcp-gateway-companion/releases/latest/download/module.json
```

Activez **Foundry MCP Companion** dans les modules du monde. Gardez un navigateur
MJ connecté avec son onglet actif. Les deux composants doivent partager le même
majeur.mineur (par exemple 1.7.x) ; les correctifs peuvent différer. Les liens
`latest` installent les versions publiées, qui peuvent être en retard sur la
prochaine version décrite par ces sources.

Demandez à votre assistant :

> « Appelle companion_trust_key pour mon instance Foundry afin d'y installer la clé publique du Gateway. »

**Rechargez ensuite tous les navigateurs MJ.** Chaque navigateur lit la clé de
confiance au démarrage du module. Avec plusieurs mondes, précisez le nom
`instance` voulu. L'outil refuse d'écraser une autre clé déjà présente : cherchez
l'origine de cet écart avant de la remplacer volontairement.

Vous pouvez aussi afficher la clé publique configurée et la coller dans
**Configurer les paramètres → Foundry MCP Companion → Clé publique du Gateway** :

```sh
docker exec foundry-mcp foundry-mcp --print-public-key
```

Rechargez également les navigateurs MJ après cette configuration manuelle.
Laissez la vérification des signatures sur **Guarded / Protégée**. Le JavaScript
distant est désactivé par défaut ; les macros de script distantes demandent
l'accord du MJ. Les fonctions de base n'exigent aucun module supplémentaire.
FXMaster, Sequencer, Dice So Nice et les autres intégrations sont facultatifs.

## 7. Vérifier et préparer un premier élément

Demandez à votre assistant d'appeler :

1. `ping` — confirme la connexion du Gateway au monde lancé.
2. `client_status` — confirme un répondant Companion, sa version et les intégrations disponibles.
3. `client_scene_report` — avec une scène ouverte, vérifie une commande navigateur signée.

Puis essayez :

> « Liste les journaux de ma campagne et propose trois tâches de préparation pour la prochaine séance. Ne modifie encore rien. »

Avec un jeton créé avec `--readonly`, installez la clé de confiance via un jeton
d'accès complet ou par la méthode manuelle avant d'utiliser le client en lecture
seule. Si une vérification échoue, consultez le [dépannage](advanced.fr.md#dépannage).
