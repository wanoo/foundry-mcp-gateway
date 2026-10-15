//! Gestion : settings, ownership, compendiums, import/export, fichiers,
//! credentials/instances, attente de messages — port des comportements TS.

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use super::markdown::html_to_markdown;
use super::{ToolDef, str_arg, text_response};
use crate::mcp::McpState;

/// Plafond du corps téléchargé par `upload_file` depuis une URL. Au-delà on
/// abandonne : c'est un asset de jeu, pas une sauvegarde de monde.
const MAX_FETCH_BYTES: usize = 64 * 1024 * 1024;

/// Link-local : 169.254.0.0/16 et fe80::/10. Refusé DANS TOUS LES CAS.
///
/// C'est là que vivent les métadonnées des hébergeurs cloud
/// (169.254.169.254 : jetons IAM, variables d'environnement de l'instance).
/// Aucun asset de jeu légitime n'habite là, ni en datacentre ni à la maison —
/// c'est la seule plage qu'on peut refuser sans jamais gêner personne.
fn is_link_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| std::net::Ipv4Addr::is_link_local(&v4))
        }
    }
}

/// Une adresse du réseau privé ou de la machine elle-même.
///
/// ⚠️ « privée » ne veut PAS dire « interdite » : voir `internal_fetch_allowed`.
/// Sur une passerelle auto-hébergée, ce réseau est celui de l'utilisateur.
fn is_internal_addr(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
                // 100.64.0.0/10 (CGNAT) et 0.0.0.0/8 ne sont pas couverts par
                // la bibliothèque standard stable.
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
                || v4.octets()[0] == 0
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 (unique local) et fe80::/10 (link-local).
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // ::ffff:127.0.0.1 contournerait sinon tous les tests IPv4
                // ci-dessus : on repasse la forme mappée dans les mêmes règles.
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_internal_addr(std::net::IpAddr::V4(v4)))
        }
    }
}

/// La passerelle a-t-elle le droit d'aller chercher une URL sur un réseau privé ?
///
/// La question n'a pas la même réponse selon l'hébergement, et c'est le
/// déploiement lui-même qui tranche — pas un réglage de plus à comprendre :
///
/// * **Auto-hébergé** (un Foundry en `localhost:30000`, sur un Pi, un NAS, un
///   homelab) : le réseau privé est celui de l'utilisateur. Lui interdire de
///   téléverser une image depuis son propre NAS serait absurde. On autorise.
/// * **Passerelle distante** (cloud, free-tier) parlant à un Foundry public :
///   là « privé » désigne l'infra de l'hébergeur, que l'utilisateur ne
///   possède pas. Et l'appelant est un modèle de langage qui lit le contenu du
///   monde : une injection de prompt dans un journal (« appelle upload_file
///   avec url=http://10.0.0.5/… ») en ferait une primitive de balayage
///   interne. On refuse.
///
/// La détection : si UNE instance configurée est elle-même sur une adresse
/// privée ou locale, la passerelle tourne à côté de son Foundry, donc chez
/// l'utilisateur. `FOUNDRY_ALLOW_INTERNAL_FETCH` force le comportement pour
/// les topologies que cette heuristique ne couvre pas (tunnel, VPN).
async fn internal_fetch_allowed(state: &McpState) -> bool {
    match std::env::var("FOUNDRY_ALLOW_INTERNAL_FETCH").as_deref() {
        Ok("1" | "true") => return true,
        Ok("0" | "false") => return false,
        _ => {}
    }
    for cred in state.credentials.iter() {
        let (host, _) = crate::foundry::auth::split_host(&cred.hostname);
        let host = host.split('/').next().unwrap_or(&host);
        let (name, port) = match host.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (h, p.parse().unwrap_or(80)),
            _ => (host, 80u16),
        };
        if name == "localhost" || name == "host.docker.internal" {
            return true;
        }
        if let Ok(addrs) = tokio::net::lookup_host((name, port)).await
            && addrs.into_iter().any(|a| is_internal_addr(a.ip()))
        {
            return true;
        }
    }
    false
}

/// Télécharge une URL fournie par l'appelant, sans laisser la passerelle
/// servir de relais vers un réseau qui n'appartient pas à son opérateur.
///
/// Voir `internal_fetch_allowed` : la politique dépend de l'hébergement. Le
/// link-local est refusé partout, et les redirections ne sont jamais suivies —
/// sans quoi un hôte public renverrait simplement un 302 vers l'IMDS.
async fn fetch_external_url(state: &McpState, url: &str) -> Result<(Vec<u8>, String)> {
    let parsed = reqwest::Url::parse(url).map_err(|e| anyhow!("invalid 'url': {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("'url' must be http or https (got '{}')", parsed.scheme());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("'url' has no host"))?
        .to_string();
    let port = parsed.port_or_known_default().unwrap_or(80);

    // Résolution explicite : un nom peut pointer vers une adresse interne.
    let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| anyhow!("cannot resolve '{host}': {e}"))?
        .collect();
    if addrs.is_empty() {
        bail!("cannot resolve '{host}'");
    }
    // Le link-local n'est légitime nulle part, quel que soit l'hébergement.
    if let Some(bad) = addrs.iter().find(|a| is_link_local(a.ip())) {
        bail!(
            "refusing to fetch '{host}': it resolves to the link-local address {} \
             (this is where cloud instance metadata lives)",
            bad.ip()
        );
    }
    // Le reste du réseau privé dépend de l'hébergement.
    if !internal_fetch_allowed(state).await
        && let Some(bad) = addrs.iter().find(|a| is_internal_addr(a.ip()))
    {
        bail!(
            "refusing to fetch '{host}': it resolves to the private address {}, and this gateway \
             is not hosted alongside a private network. If it should be able to reach your LAN, \
             set FOUNDRY_ALLOW_INTERNAL_FETCH=1",
            bad.ip()
        );
    }

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let resp = client.get(parsed).send().await?;
    if resp.status().is_redirection() {
        bail!(
            "refusing to follow a redirect from '{host}' (status {}) — point 'url' at the final location",
            resp.status()
        );
    }
    let resp = resp.error_for_status()?;
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = resp.bytes().await?;
    if bytes.len() > MAX_FETCH_BYTES {
        bail!(
            "downloaded file is too large ({} MB, maximum {} MB)",
            bytes.len() / 1_048_576,
            MAX_FETCH_BYTES / 1_048_576
        );
    }
    Ok((bytes.to_vec(), content_type))
}

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
/// Construites une fois : `handles` est appelé pour chaque module à chaque
/// dispatch, et rebâtir les schémas json! à chaque fois coûtait une
/// allocation par outil pour une simple question d'appartenance.
static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

pub fn definitions() -> &'static [ToolDef] {
    &DEFS
}

#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    vec![
        ("companion_trust_key",
         "Install this gateway's public key into the world, so the companion module accepts its commands. Run this once after installing the companion — it saves copying a long key by hand. Refuses to overwrite a different key already in place unless force is true (another gateway may be driving this world). Gamemaster browsers must reload afterwards.",
         json!({"type":"object","properties":{
            "force":{"type":"boolean","description":"replace a different key that is already trusted"}}})),
        ("manage_modules",
         "List the world's modules (installed vs enabled, with versions), or enable/disable some. Changes need every client to reload (F5) to take effect — the tool reminds you.",
         json!({"type":"object","properties":{
            "enable":{"type":"array","items":{"type":"string"},"description":"module ids to enable"},
            "disable":{"type":"array","items":{"type":"string"},"description":"module ids to disable"}}})),
        ("manage_users",
         "List the world's user accounts, or create/update/delete them: name, role (player/trusted/assistant/gamemaster), assigned character, colour. PASSWORDS ARE NOT HANDLED HERE — the GM sets them in Foundry (Configure Players); accounts created here start password-less, which Foundry allows.",
         json!({"type":"object","properties":{
            "create":{"type":"array","description":"[{name, role?, character?, color?}] — role: player|trusted|assistant|gamemaster (default player)",
                      "items":{"type":"object"}},
            "update":{"type":"array","description":"[{_id or name, role?, character?, color?}]",
                      "items":{"type":"object"}},
            "delete":{"type":"array","items":{"type":"string"},"description":"user _ids or names to delete (refuses the bot's own account)"}}})),
        ("set_setting",
         "Set a world-scoped setting (upsert). Keys are namespaced like \"module.setting\".",
         json!({"type":"object","properties":{
            "key":{"type":"string"},"value":{}},"required":["key","value"]})),
        ("list_actor_ownership",
         "Actor permissions: which users own/observe which actors (user names resolved). Levels: none/limited/observer/owner.",
         json!({"type":"object","properties":{"_id":{"type":"string"},"name":{"type":"string"}}})),
        ("set_actor_ownership",
         "Grant or revoke a user's permission on an actor (level \"none\" revokes). default_level sets the actor-wide default.",
         json!({"type":"object","properties":{
            "_id":{"type":"string"},"name":{"type":"string"},"user":{"type":"string"},
            "level":{"type":"string","enum":["none","limited","observer","owner"]},
            "default_level":{"type":"string","enum":["none","limited","observer","owner"]}}})),
        ("import_from_compendium",
         "Import a document from a pack into the world (by _id or name; keep_id, folder).",
         json!({"type":"object","properties":{
            "pack":{"type":"string"},"type":{"type":"string"},
            "_id":{"type":"string"},"name":{"type":"string"},
            "keep_id":{"type":"boolean"},"folder":{"type":"string"}},
            "required":["pack","type"]})),
        ("export_journals",
         "Export journals as Markdown (where filter, offset/limit — default limit 20).",
         json!({"type":"object","properties":{
            "where":{"type":"object","additionalProperties":true},
            "offset":{"type":"number"},"limit":{"type":"number"}}})),
        ("create_compendium",
         "Create a Compendium pack (label + document type).",
         json!({"type":"object","properties":{
            "label":{"type":"string"},"type":{"type":"string"}},"required":["label","type"]})),
        ("delete_compendium",
         "Delete a Compendium pack and ALL its documents. Permanent.",
         json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]})),
        ("create_directory",
         "Create a directory in Foundry's storage (parents must exist).",
         json!({"type":"object","properties":{
            "target":{"type":"string"},"source":{"type":"string"}},"required":["target"]})),
        ("browse_files",
         "Browse Foundry's file storage (dirs + files at a path).",
         json!({"type":"object","properties":{
            "target":{"type":"string"},"type":{"type":"string"},
            "extensions":{"type":"array","items":{"type":"string"}}},"required":["target"]})),
        ("upload_file",
         "Upload a file to Foundry storage. EXACTLY ONE of url or image_data (base64). Target dir must exist (create_directory).",
         json!({"type":"object","properties":{
            "target":{"type":"string"},"filename":{"type":"string"},
            "url":{"type":"string"},"image_data":{"type":"string"}},
            "required":["target","filename"]})),
        ("show_credentials",
         "Configured Foundry credentials (no passwords) + which is active.",
         json!({"type":"object","properties":{}})),
        ("choose_foundry_instance",
         "Set the DEFAULT instance for calls that don't pass `instance` (by item_order or _id). Other instances stay connected — every tool accepts `instance`, and copy_documents moves content between them.",
         json!({"type":"object","properties":{
            "item_order":{"type":"number"},"_id":{"type":"string"}}})),
        ("wait_for_message",
         "Block until a ChatMessage created by ANOTHER client arrives (where filter, timeout ≤120 s). Flow: get_events lastSeq → trigger → wait.",
         json!({"type":"object","properties":{
            "where":{"type":"object","additionalProperties":true},
            "timeout_seconds":{"type":"number"},"since_seq":{"type":"number"}}})),
    ]
}

pub fn handles(name: &str) -> bool {
    definitions().iter().any(|(n, _, _)| *n == name)
}

const LEVELS: [(&str, u64); 4] = [("none", 0), ("limited", 1), ("observer", 2), ("owner", 3)];
fn level_num(name: &str) -> Option<u64> {
    LEVELS.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}
fn level_name(v: u64) -> String {
    LEVELS
        .iter()
        .find(|(_, n)| *n == v)
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| v.to_string())
}

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let foundry = &state.foundry;
    match name {
        "companion_trust_key" => {
            const KEY: &str = "foundry-mcp-gateway-companion.rootPublicKey";
            let ours = state.signer.public_hex();
            let w = json!({ "key": KEY }).as_object().cloned().unwrap();
            let existing = foundry
                .get_documents("settings", Some(&w), None, 0, Some(1))
                .await?;
            // La valeur stockée est du JSON (un Setting sérialise sa valeur).
            let current = existing.first().and_then(|d| {
                let raw = d.get("value")?.as_str()?;
                serde_json::from_str::<String>(raw).ok()
            });

            // TOFU : on n'écrase jamais en silence une clé DIFFÉRENTE. Sans ce
            // garde-fou, une seconde passerelle branchée sur le même monde
            // détournerait l'ancrage de confiance sans que personne ne le voie.
            if let Some(ref old) = current
                && !old.is_empty()
                && old != ours
                && args.get("force").and_then(Value::as_bool) != Some(true)
            {
                bail!(
                    "this world already trusts a different gateway key ({}…). Another gateway may \
                     already be driving it. Re-run with force: true only if you mean to replace it.",
                    &old[..old.len().min(16)]
                );
            }
            if current.as_deref() == Some(ours) {
                return Ok(text_response(
                    &json!({ "action": "unchanged", "publicKey": ours }),
                ));
            }

            let serialized = Value::String(ours.to_string()).to_string();
            let (action, result) = match existing.first() {
                Some(doc) => {
                    let r = foundry.modify_document("Setting", "update", json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{"_id": doc["_id"], "value": serialized}],
                    })).await?;
                    ("updated", r)
                }
                None => {
                    let r = foundry
                        .modify_document(
                            "Setting",
                            "create",
                            json!({
                                "action": "create", "renderSheet": false, "keepId": false,
                                "data": [{"key": KEY, "value": serialized}],
                            }),
                        )
                        .await?;
                    ("created", r)
                }
            };
            Ok(text_response(&json!({
                "action": action,
                "publicKey": ours,
                "note": "Gamemaster browsers must reload (F5) before the new key takes effect — \
                         the companion freezes it at startup on purpose.",
                "result": result,
            })))
        }
        "manage_modules" => {
            // état : liste installée (dump monde) × configuration activée (setting)
            let world = state.foundry.request_world().await?;
            let empty = vec![];
            let installed = world
                .get("modules")
                .and_then(Value::as_array)
                .unwrap_or(&empty);
            let w = json!({"key": "core.moduleConfiguration"})
                .as_object()
                .cloned()
                .unwrap();
            let setting = state
                .foundry
                .get_documents("settings", Some(&w), None, 0, Some(1))
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("core.moduleConfiguration introuvable"))?;
            let mut config: Map<String, Value> = setting
                .get("value")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();

            let ids = |key: &str| -> Vec<String> {
                args.get(key)
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let (to_enable, to_disable) = (ids("enable"), ids("disable"));
            if to_enable.is_empty() && to_disable.is_empty() {
                // lecture seule : le tableau de bord
                let list: Vec<Value> = installed
                    .iter()
                    .map(|m| {
                        let id = m.get("id").and_then(Value::as_str).unwrap_or("?");
                        json!({
                            "id": id,
                            "title": m.get("title"),
                            "version": m.get("version"),
                            "enabled": config.get(id).and_then(Value::as_bool).unwrap_or(false),
                        })
                    })
                    .collect();
                let enabled = list.iter().filter(|m| m["enabled"] == json!(true)).count();
                return Ok(text_response(&json!({
                    "installed": list.len(), "enabled": enabled, "modules": list,
                })));
            }
            // écriture : vérifier que les ids existent avant de toucher au réglage
            let known: Vec<&str> = installed
                .iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str))
                .collect();
            for id in to_enable.iter().chain(to_disable.iter()) {
                if !known.contains(&id.as_str()) {
                    bail!("module inconnu : {id} (voir manage_modules sans argument)");
                }
            }
            for id in &to_enable {
                config.insert(id.clone(), json!(true));
            }
            for id in &to_disable {
                config.insert(id.clone(), json!(false));
            }
            state
                .foundry
                .modify_document(
                    "Setting",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{
                            "_id": setting["_id"],
                            "value": Value::Object(config).to_string(),
                        }],
                    }),
                )
                .await?;
            Ok(text_response(&json!({
                "enabled": to_enable, "disabled": to_disable,
                "note": "chaque client doit recharger (F5) pour que le changement prenne effet",
            })))
        }

        "manage_users" => {
            let bot = state.foundry.user_id().await;
            let users = state
                .foundry
                .get_documents("users", None, None, 0, None)
                .await?;
            let find = |key: &str| -> Option<Value> {
                users
                    .iter()
                    .find(|u| u["_id"] == json!(key) || u["name"] == json!(key))
                    .cloned()
            };
            let role_of = |v: Option<&Value>| -> Result<Option<i64>> {
                let Some(v) = v else { return Ok(None) };
                if let Some(n) = v.as_i64() {
                    return Ok(Some(n));
                }
                let s = v.as_str().unwrap_or_default().to_lowercase();
                Ok(Some(match s.as_str() {
                    "none" => 0,
                    "player" => 1,
                    "trusted" => 2,
                    "assistant" | "assistant gm" | "assistantgm" => 3,
                    "gamemaster" | "gm" => 4,
                    other => bail!(
                        "rôle inconnu : '{other}' (player | trusted | assistant | gamemaster)"
                    ),
                }))
            };
            let label = |r: i64| match r {
                0 => "none",
                1 => "player",
                2 => "trusted",
                3 => "assistant",
                4 => "gamemaster",
                _ => "?",
            };
            let (create, update, delete) = (
                args.get("create").and_then(Value::as_array),
                args.get("update").and_then(Value::as_array),
                args.get("delete").and_then(Value::as_array),
            );
            // lecture seule : le tableau de bord des comptes
            if create.is_none() && update.is_none() && delete.is_none() {
                let list: Vec<Value> = users
                    .iter()
                    .map(|u| {
                        let r = u.get("role").and_then(Value::as_i64).unwrap_or(0);
                        json!({
                            "_id": u["_id"], "name": u["name"],
                            "role": r, "roleName": label(r),
                            "character": u.get("character"),
                        })
                    })
                    .collect();
                // Foundry blanchit password/passwordSalt pour TOUS les clients
                // (vérifié : chaîne vide même pour le compte du bot) — on ne
                // peut donc PAS dire ici si un compte a un mot de passe.
                return Ok(text_response(&json!({
                    "count": list.len(), "users": list,
                    "note": "passwords are never visible client-side (Foundry blanks them) — set them in Configure Players",
                })));
            }
            let mut report = json!({});
            if let Some(items) = create {
                let mut data = Vec::new();
                for item in items {
                    let name = str_arg(item, "name")
                        .ok_or_else(|| anyhow!("chaque compte à créer a besoin d'un 'name'"))?;
                    if find(&name).is_some() {
                        bail!("un utilisateur nommé '{name}' existe déjà");
                    }
                    let mut doc = json!({
                        "name": name,
                        "role": role_of(item.get("role"))?.unwrap_or(1),
                    });
                    for k in ["character", "color"] {
                        if let Some(v) = item.get(k) {
                            doc[k] = v.clone();
                        }
                    }
                    data.push(doc);
                }
                let r = state
                    .foundry
                    .modify_document(
                        "User",
                        "create",
                        json!({ "action": "create", "renderSheet": false,
                                "keepId": false, "data": data }),
                    )
                    .await?;
                report["created"] = r
                    .get("result")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|u| json!({"_id": u["_id"], "name": u["name"], "role": u["role"]}))
                            .collect::<Vec<_>>()
                            .into()
                    })
                    .unwrap_or(Value::Null);
                report["passwordNote"] = json!(
                    "comptes sans mot de passe — à définir dans Foundry (Configurer les joueurs)"
                );
            }
            if let Some(items) = update {
                let mut updates = Vec::new();
                for item in items {
                    let key = str_arg(item, "_id")
                        .or_else(|| str_arg(item, "name"))
                        .ok_or_else(|| anyhow!("chaque mise à jour a besoin de '_id' ou 'name'"))?;
                    let existing =
                        find(&key).ok_or_else(|| anyhow!("utilisateur introuvable : {key}"))?;
                    let mut u = json!({ "_id": existing["_id"] });
                    if let Some(r) = role_of(item.get("role"))? {
                        u["role"] = json!(r);
                    }
                    for k in ["character", "color", "name"] {
                        if let Some(v) = item.get(k) {
                            u[k] = v.clone();
                        }
                    }
                    updates.push(u);
                }
                state
                    .foundry
                    .modify_document(
                        "User",
                        "update",
                        json!({ "action": "update", "diff": false, "recursive": true,
                                "render": true, "updates": updates }),
                    )
                    .await?;
                report["updated"] = json!(updates.len());
            }
            if let Some(items) = delete {
                let mut ids = Vec::new();
                for item in items {
                    let key = item.as_str().unwrap_or_default();
                    let existing =
                        find(key).ok_or_else(|| anyhow!("utilisateur introuvable : {key}"))?;
                    let id = existing["_id"].as_str().unwrap_or_default().to_string();
                    if Some(&id) == bot.as_ref() {
                        bail!("refus de supprimer le compte du bot MCP lui-même ({key})");
                    }
                    ids.push(json!(id));
                }
                state
                    .foundry
                    .modify_document(
                        "User",
                        "delete",
                        json!({ "action": "delete", "deleteAll": false,
                                "ids": ids }),
                    )
                    .await?;
                report["deleted"] = json!(ids);
            }
            Ok(text_response(&report))
        }

        "set_setting" => {
            let key = str_arg(args, "key").ok_or_else(|| anyhow!("'key' is required"))?;
            let value = args
                .get("value")
                .cloned()
                .ok_or_else(|| anyhow!("'value' is required"))?;
            let serialized = value.to_string();
            let w = json!({"key": key}).as_object().cloned().unwrap();
            let existing = foundry
                .get_documents("settings", Some(&w), None, 0, Some(1))
                .await?;
            let (action, result) = match existing.first() {
                Some(doc) => {
                    let r = foundry.modify_document("Setting", "update", json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [{"_id": doc["_id"], "value": serialized}],
                    })).await?;
                    ("updated", r)
                }
                None => {
                    let r = foundry
                        .modify_document(
                            "Setting",
                            "create",
                            json!({
                                "action": "create", "renderSheet": false, "keepId": false,
                                "data": [{"key": key, "value": serialized}],
                            }),
                        )
                        .await?;
                    ("created", r)
                }
            };
            Ok(text_response(
                &json!({"action": action, "key": key, "value": value, "result": result}),
            ))
        }
        "list_actor_ownership" | "set_actor_ownership" => {
            let ufields = vec!["_id".into(), "name".into()];
            let users = foundry
                .get_documents("users", None, Some(&ufields), 0, None)
                .await?;
            let uname = |id: &str| {
                users
                    .iter()
                    .find(|u| u["_id"] == json!(id))
                    .and_then(|u| u["name"].as_str().map(String::from))
                    .unwrap_or_else(|| id.to_string())
            };
            let describe = |actor: &Value| {
                let default_map = serde_json::Map::new();
                let ownership = actor
                    .get("ownership")
                    .and_then(Value::as_object)
                    .unwrap_or(&default_map);
                let entries: Vec<Value> = ownership
                    .iter()
                    .filter(|(k, _)| *k != "default")
                    .map(|(uid, lvl)| {
                        json!({
                            "user": uname(uid), "userId": uid,
                            "level": level_name(lvl.as_u64().unwrap_or(0)),
                        })
                    })
                    .collect();
                json!({
                    "_id": actor["_id"], "name": actor["name"],
                    "default": level_name(ownership.get("default").and_then(Value::as_u64).unwrap_or(0)),
                    "users": entries,
                })
            };
            let afields = vec!["_id".into(), "name".into(), "ownership".into()];

            if name == "list_actor_ownership" {
                if let Some(ident) = str_arg(args, "_id").or_else(|| str_arg(args, "name")) {
                    let actor = foundry
                        .find_document("actors", &ident, Some(&afields))
                        .await?
                        .ok_or_else(|| anyhow!("Actor not found"))?;
                    return Ok(text_response(&describe(&actor)));
                }
                let actors = foundry
                    .get_documents("actors", None, Some(&afields), 0, None)
                    .await?;
                let interesting: Vec<Value> = actors
                    .iter()
                    .map(&describe)
                    .filter(|d| {
                        !d["users"].as_array().unwrap().is_empty() || d["default"] != "none"
                    })
                    .collect();
                return Ok(text_response(&Value::Array(interesting)));
            }

            let ident = str_arg(args, "_id")
                .or_else(|| str_arg(args, "name"))
                .ok_or_else(|| anyhow!("Must provide one of: _id or name (actor)"))?;
            let actor = foundry
                .find_document("actors", &ident, Some(&afields))
                .await?
                .ok_or_else(|| anyhow!("Actor not found"))?;
            let mut update = serde_json::Map::new();
            match (str_arg(args, "user"), str_arg(args, "level")) {
                (Some(uarg), Some(level)) => {
                    let user = users
                        .iter()
                        .find(|u| u["_id"] == json!(uarg) || u["name"] == json!(uarg))
                        .ok_or_else(|| anyhow!("User not found: {uarg}"))?;
                    let uid = user["_id"].as_str().unwrap_or("");
                    if level == "none" {
                        update.insert(format!("ownership.-={uid}"), Value::Null);
                    } else {
                        let lvl =
                            level_num(&level).ok_or_else(|| anyhow!("Unknown level: {level}"))?;
                        update.insert(format!("ownership.{uid}"), json!(lvl));
                    }
                }
                (None, None) => {}
                _ => bail!("'user' and 'level' go together"),
            }
            if let Some(dl) = str_arg(args, "default_level") {
                let lvl = level_num(&dl).ok_or_else(|| anyhow!("Unknown level: {dl}"))?;
                update.insert("ownership.default".into(), json!(lvl));
            }
            if update.is_empty() {
                bail!("Nothing to change (provide user+level and/or default_level)");
            }
            let mut update_doc = Value::Object(update.clone());
            update_doc["_id"] = actor["_id"].clone();
            let result = foundry
                .modify_document(
                    "Actor",
                    "update",
                    json!({
                        "action": "update", "diff": false, "recursive": true, "render": true,
                        "updates": [update_doc],
                    }),
                )
                .await?;
            Ok(text_response(&json!({
                "actor": {"_id": actor["_id"], "name": actor["name"]},
                "applied": update, "result": result,
            })))
        }
        "import_from_compendium" => {
            let pack = str_arg(args, "pack").ok_or_else(|| anyhow!("'pack' is required"))?;
            let doc_type = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            let id = str_arg(args, "_id");
            let dname = str_arg(args, "name");
            if id.is_none() && dname.is_none() {
                bail!("Must provide one of: _id or name");
            }
            let mut query = serde_json::Map::new();
            match (&id, &dname) {
                (Some(i), _) => {
                    query.insert("_id".into(), json!(i));
                }
                (None, Some(n)) => {
                    query.insert("name".into(), json!(n));
                }
                _ => unreachable!(),
            }
            let docs = foundry
                .get_collection(&doc_type, query, false, Some(&pack))
                .await?;
            let mut doc = docs
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("Document not found in pack {pack}"))?;
            doc["folder"] = str_arg(args, "folder")
                .map(|f| json!(f))
                .unwrap_or(Value::Null);
            let keep_id = args
                .get("keep_id")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let imported = json!({"_id": doc["_id"], "name": doc["name"]});
            let result = foundry
                .modify_document(
                    &doc_type,
                    "create",
                    json!({
                        "action": "create", "renderSheet": false, "keepId": keep_id,
                        "data": [doc],
                    }),
                )
                .await?;
            Ok(text_response(
                &json!({"imported": imported, "from": pack, "result": result}),
            ))
        }
        "export_journals" => {
            let where_ = args.get("where").and_then(Value::as_object).cloned();
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            let journals = foundry
                .get_documents("journal", where_.as_ref(), None, 0, None)
                .await?;
            let total = journals.len();
            let page: Vec<Value> = journals.iter().skip(offset).take(limit).map(|j| {
                let empty = vec![];
                let pages: Vec<Value> = j.get("pages").and_then(Value::as_array).unwrap_or(&empty)
                    .iter().map(|p| json!({
                        "name": p["name"],
                        "markdown": html_to_markdown(
                            p.pointer("/text/content").and_then(Value::as_str).unwrap_or("")),
                    })).collect();
                json!({"_id": j["_id"], "name": j["name"], "pages": pages})
            }).collect();
            Ok(text_response(&json!({
                "total": total, "offset": offset, "count": page.len(), "journals": page,
            })))
        }
        "create_compendium" => {
            let label = str_arg(args, "label").ok_or_else(|| anyhow!("'label' is required"))?;
            let doc_type = str_arg(args, "type").ok_or_else(|| anyhow!("'type' is required"))?;
            let result = foundry
                .manage_compendium("create", json!({"label": label, "type": doc_type}))
                .await?;
            Ok(text_response(&result))
        }
        "delete_compendium" => {
            let pname = str_arg(args, "name").ok_or_else(|| anyhow!("'name' is required"))?;
            let result = foundry.manage_compendium("delete", json!(pname)).await?;
            Ok(text_response(&result))
        }
        "create_directory" => {
            let target = str_arg(args, "target").ok_or_else(|| anyhow!("'target' is required"))?;
            let source = str_arg(args, "source").unwrap_or_else(|| "data".into());
            let result = foundry
                .manage_files(
                    json!({"action": "createDirectory", "storage": source, "target": target}),
                    json!({}),
                )
                .await?;
            Ok(text_response(&result))
        }
        "browse_files" => {
            let target = str_arg(args, "target").ok_or_else(|| anyhow!("'target' is required"))?;
            let ftype = str_arg(args, "type").unwrap_or_else(|| "image".into());
            let extensions = args.get("extensions").cloned().unwrap_or(json!([
                ".apng", ".avif", ".bmp", ".gif", ".jpeg", ".jpg", ".png", ".svg", ".tiff", ".webp"
            ]));
            let result = foundry.manage_files(
                json!({"action": "browseFiles", "storage": "data", "target": target}),
                json!({"type": ftype, "extensions": extensions, "wildcard": false, "render": true}),
            ).await?;
            Ok(text_response(&result))
        }
        "upload_file" => {
            let target = str_arg(args, "target").ok_or_else(|| anyhow!("'target' is required"))?;
            let filename =
                str_arg(args, "filename").ok_or_else(|| anyhow!("'filename' is required"))?;
            let url = str_arg(args, "url").filter(|s| !s.is_empty());
            let image_data = str_arg(args, "image_data").filter(|s| !s.is_empty());
            let (bytes, content_type) = match (url, image_data) {
                (Some(_), Some(_)) => bail!("Cannot provide both 'url' and 'image_data'"),
                (None, None) => bail!("Must provide either 'url' or 'image_data'"),
                (Some(u), None) => fetch_external_url(state, &u).await?,
                (None, Some(b64)) => {
                    let bytes = base64_decode(&b64)?;
                    (bytes, content_type_for(&filename))
                }
            };
            let result = foundry
                .upload_file(&target, &filename, bytes, &content_type)
                .await?;
            Ok(text_response(&result))
        }
        "show_credentials" => {
            let (active, list) = foundry.credentials_info();
            Ok(text_response(
                &json!({"active_index": active, "credentials": list}),
            ))
        }
        "choose_foundry_instance" => {
            let (_, list) = foundry.credentials_info();
            let index = match (
                args.get("item_order").and_then(Value::as_u64),
                str_arg(args, "_id"),
            ) {
                (Some(i), _) => i as usize,
                (None, Some(id)) => list
                    .iter()
                    .position(|c| c["_id"] == json!(id))
                    .ok_or_else(|| anyhow!("Unknown instance _id: {id}"))?,
                _ => bail!("Provide item_order or _id"),
            };
            // Multi-mondes : on ne coupe plus rien, on déplace juste le défaut.
            // Les autres instances restent connectées et adressables via `instance`.
            let id = list[index]["_id"].as_str().unwrap_or_default().to_string();
            *state.active.lock().await = id.clone();
            Ok(text_response(&json!({
                "default_instance": id, "item_order": index,
                "note": "les autres instances restent connectées — un appel peut viser n'importe laquelle avec `instance`",
            })))
        }
        "wait_for_message" => {
            let where_ = args.get("where").and_then(Value::as_object).cloned();
            let timeout_s = args
                .get("timeout_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(60)
                .min(120);
            let since = args
                .get("since_seq")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| foundry.event_seq());
            let matched = foundry
                .wait_for_event(since, std::time::Duration::from_secs(timeout_s), |e| {
                    if e.event != "modifyDocument" {
                        return false;
                    }
                    let Some(payload) = e
                        .args
                        .iter()
                        .find(|a| a.get("type") == Some(&json!("ChatMessage")))
                    else {
                        return false;
                    };
                    if payload.get("action") != Some(&json!("create")) {
                        return false;
                    }
                    let empty = vec![];
                    let docs = payload
                        .get("result")
                        .and_then(Value::as_array)
                        .unwrap_or(&empty);
                    docs.iter().any(|d| {
                        where_
                            .as_ref()
                            .is_none_or(|w| crate::foundry::documents::matches_where(d, w))
                    })
                })
                .await;
            match matched {
                None => Ok(text_response(
                    &json!({"timeout": true, "waited_seconds": timeout_s, "messages": []}),
                )),
                Some(e) => {
                    let payload = e
                        .args
                        .iter()
                        .find(|a| a.get("type") == Some(&json!("ChatMessage")))
                        .cloned()
                        .unwrap_or(Value::Null);
                    let empty = vec![];
                    let docs = payload
                        .get("result")
                        .and_then(Value::as_array)
                        .unwrap_or(&empty);
                    let hits: Vec<Value> = docs
                        .iter()
                        .filter(|d| {
                            where_
                                .as_ref()
                                .is_none_or(|w| crate::foundry::documents::matches_where(d, w))
                        })
                        .cloned()
                        .collect();
                    Ok(text_response(&json!({"timeout": false, "messages": hits})))
                }
            }
        }
        other => bail!("Unknown tool: {other}"),
    }
}

fn base64_decode(input: &str) -> Result<Vec<u8>> {
    // Décodage base64 standard sans dépendance.
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut rev = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        rev[c as usize] = i as u8;
    }
    let clean: Vec<u8> = input.bytes().filter(|b| !b" \n\r\t".contains(b)).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    for &b in &clean {
        if b == b'=' {
            break;
        }
        let v = rev[b as usize];
        if v == 255 {
            bail!("base64 invalide");
        }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

fn content_type_for(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "json" => "application/json",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_local_refuse_partout() {
        let ip = |s: &str| s.parse::<std::net::IpAddr>().unwrap();
        // Métadonnées d'instance cloud : jetons IAM, env de la machine.
        // Jamais un asset de jeu, ni en datacentre ni à la maison.
        assert!(is_link_local(ip("169.254.169.254")));
        assert!(is_link_local(ip("169.254.0.1")));
        assert!(is_link_local(ip("fe80::1")));
        assert!(is_link_local(ip("::ffff:169.254.169.254")));

        // Le reste du réseau privé n'est PAS link-local : sur une passerelle
        // auto-hébergée (Pi, NAS, homelab) c'est le réseau de l'utilisateur,
        // et lui interdire son propre NAS n'aurait aucun sens.
        assert!(!is_link_local(ip("192.168.1.50")));
        assert!(!is_link_local(ip("127.0.0.1")));
        assert!(!is_link_local(ip("10.0.0.5")));
        assert!(!is_link_local(ip("1.1.1.1")));
    }

    #[test]
    fn adresses_internes_classees() {
        let ip = |s: &str| s.parse::<std::net::IpAddr>().unwrap();
        assert!(is_internal_addr(ip("169.254.169.254")));
        assert!(is_internal_addr(ip("127.0.0.1")));
        assert!(is_internal_addr(ip("10.0.0.5")));
        assert!(is_internal_addr(ip("192.168.1.1")));
        assert!(is_internal_addr(ip("172.16.0.1")));
        assert!(is_internal_addr(ip("100.64.0.1"))); // CGNAT
        assert!(is_internal_addr(ip("0.0.0.0")));
        assert!(is_internal_addr(ip("::1")));
        assert!(is_internal_addr(ip("fd00::1"))); // unique local
        assert!(is_internal_addr(ip("fe80::1"))); // link-local
        // La forme mappée doit tomber sous les mêmes règles que sa forme v4,
        // sinon ::ffff:127.0.0.1 passait tout droit.
        assert!(is_internal_addr(ip("::ffff:127.0.0.1")));
        assert!(is_internal_addr(ip("::ffff:169.254.169.254")));

        // Et le trafic légitime doit continuer de passer.
        assert!(!is_internal_addr(ip("1.1.1.1")));
        assert!(!is_internal_addr(ip("93.184.216.34")));
        assert!(!is_internal_addr(ip("2606:4700:4700::1111")));
    }

    #[test]
    fn base64_roundtrip() {
        assert_eq!(base64_decode("SGVsbG8=").unwrap(), b"Hello");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a");
        assert!(base64_decode("$$$").is_err());
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type_for("a.PNG"), "image/png");
        assert_eq!(content_type_for("x.bin"), "application/octet-stream");
    }
}
