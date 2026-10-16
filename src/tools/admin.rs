//! Plan d'administration (/setup) — protocole vérifié dans le code client v13
//! servi par l'instance (foundry.mjs) : tout est POST JSON `{action, …}`.
//!   · editWorld  → POST /setup, accepté PENDANT que le monde tourne (session
//!     de jeu du bot — c'est ce que fait l'écran « Modifier le monde » du MJ).
//!   · shutdown   → POST /join {action:"shutdown", adminPassword}.
//!   · launchWorld / checkPackage / installPackage → POST /setup, monde ÉTEINT,
//!     après authentification admin sur /auth (formulaire HTML : on tente JSON
//!     puis formulaire, et on VÉRIFIE en re-GET /setup).
//! La progression (launch/install) est diffusée en events `progress` — visibles
//! via get_events une fois le monde revenu.
//!
//! Outils exposés seulement si FOUNDRY_ADMIN_PASSWORD est présent (sauf
//! admin_edit_world qui n'en a pas besoin : session de jeu suffit).

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};

use super::ToolDef;
use super::{str_arg, text_response};
use crate::foundry::auth::split_host;
use crate::mcp::McpState;

// Table alignée à la main : plus lisible ainsi, et les versions de rustfmt
// ne s'accordent pas sur ces tuples à longues descriptions.
#[rustfmt::skip]
/// Les 8 outils du plan /setup : exposés seulement si FOUNDRY_ADMIN_PASSWORD
/// est configuré. `handles` les reconnaît TOUJOURS, y compris sans mot de
/// passe — sinon ils tomberaient dans run_tool et l'agent lirait « Unknown
/// tool » au lieu du vrai message d'explication.
const SETUP_ONLY: [&str; 8] = [
    "admin_shutdown_world",
    "admin_launch_world",
    "admin_list_backups",
    "admin_backup_world",
    "admin_create_world",
    "admin_install_package",
    "admin_check_package",
    "admin_update_package",
];

static DEFS: std::sync::LazyLock<Vec<ToolDef>> = std::sync::LazyLock::new(build_definitions);

/// Le catalogue complet, mot de passe admin ou non — les tests de cohérence du
/// registre en ont besoin sans avoir à fabriquer un McpState.
#[cfg(test)]
pub static DEFS_FOR_TEST: &std::sync::LazyLock<Vec<ToolDef>> = &DEFS;

pub fn definitions(state: &McpState) -> Vec<&'static ToolDef> {
    let unlocked = state.admin_password.is_some();
    DEFS.iter()
        .filter(|(n, _, _)| unlocked || !SETUP_ONLY.contains(n))
        .collect()
}

#[rustfmt::skip]
fn build_definitions() -> Vec<ToolDef> {
    let mut tools = vec![
        ("admin_status",
         "Instance status straight from /api/status: is a world active, which one, version, uptime — works even when the world is down (unlike every world tool). Also says whether the admin password is configured for the admin_* tools.",
         json!({"type":"object","properties":{}})),
        ("admin_edit_world",
         "Edit the world's metadata: title, description (HTML), background image (path or URL), next session date (ISO). Works while the world is running; players see it on the join page. Only the given fields change.",
         json!({"type":"object","properties":{
            "title":{"type":"string"},
            "description":{"type":"string"},
            "background":{"type":"string","description":"image path/URL shown on the join page"},
            "next_session":{"type":"string","description":"ISO date, or empty string to clear"}}})),
    ];
    {
        tools.extend([
            ("admin_shutdown_world",
             "Shut the running world down (back to setup mode). DISCONNECTS every player AND this MCP server's bot (world tools go down until relaunch; the bot reconnects automatically). Requires confirm:true.",
             json!({"type":"object","properties":{
                "confirm":{"type":"boolean","description":"must be true"}},
                "required":["confirm"]})),
            ("admin_launch_world",
             "Launch a world from setup mode (after admin_shutdown_world). Default: the last world seen active. The bot reconnects automatically once it is up (~10-30 s).",
             json!({"type":"object","properties":{
                "world":{"type":"string","description":"world id, e.g. star-wars"}}})),
            ("admin_list_backups",
             "List the backups Foundry holds (worlds, systems, modules) with their date, size and note. Setup mode only.",
             json!({"type":"object","properties":{}})),
            ("admin_backup_world",
             "Create a backup before doing something risky. Setup mode only (shut the world down first). Defaults to the last active world; pass type/id for a module or system instead.",
             json!({"type":"object","properties":{
                "type":{"type":"string","description":"world (default) | module | system"},
                "id":{"type":"string","description":"package id; defaults to the last active world"},
                "note":{"type":"string","description":"why you took it — shown in Foundry's backup list"}}})),
            ("admin_create_world",
             "Create a new (empty) world on this instance — the target of a migration. Needs the system id, which must already be installed (admin_install_package). Setup mode only.",
             json!({"type":"object","properties":{
                "id":{"type":"string","description":"world id, url-safe, e.g. star-wars-2"},
                "title":{"type":"string"},
                "system":{"type":"string","description":"game system id, e.g. starwarsffg"},
                "description":{"type":"string"},
                "background":{"type":"string","description":"image path for the join page"}},
                "required":["id","title","system"]})),
            ("admin_install_package",
             "Install a module or system that is NOT yet on this instance, from its manifest URL — how you bring a target server up to parity before copying a world onto it. Setup mode only. Already-installed packages are reported as such (use admin_update_package to upgrade them).",
             json!({"type":"object","properties":{
                "type":{"type":"string","description":"module (default) | system"},
                "id":{"type":"string","description":"package id"},
                "manifest":{"type":"string","description":"manifest URL (module.json / system.json)"}},
                "required":["id","manifest"]})),
            ("admin_check_package",
             "Check if an update is available for a module, system or world (compares installed vs remote manifest). Setup mode only (shut the world down first).",
             json!({"type":"object","properties":{
                "type":{"type":"string","description":"module | system | world (default module)"},
                "id":{"type":"string"}},
                "required":["id"]})),
            ("admin_update_package",
             "Update a module, system or world to its latest version. Setup mode only — refuses if a world is running. Takes a BACKUP first (unless backup:false), checks, installs only if an update exists, retries up to 5 times, then verifies.",
             json!({"type":"object","properties":{
                "type":{"type":"string","description":"module | system | world (default module)"},
                "id":{"type":"string"},
                "backup":{"type":"boolean","description":"back the package up before updating (default true)"}},
                "required":["id"]})),
        ]);
    }
    tools
}

/// Liste ses propres noms plutôt qu'un préfixe : `starts_with("admin_")`
/// revendiquait des outils que `run` ne traite pas, et ça ne fonctionnait que
/// grâce à la position du module dans la chaîne de `else if` du dispatch.
pub fn handles(name: &str) -> bool {
    DEFS.iter().any(|(n, _, _)| *n == name)
}

/* ------------------------------------------------------------- plumbing */

fn base_url(state: &McpState) -> String {
    let hostname = state.foundry.hostname();
    let (host, base) = split_host(&hostname);
    let (http_s, _) = crate::foundry::auth::schemes(&hostname);
    format!("{http_s}://{host}{base}")
}

/// Client HTTP des routes admin. Foundry v14 (build 368+) répond un 400
/// générique à tout POST /join ou /setup sans en-tête `Origin` : on l'envoie
/// par défaut, dérivé de l'URL de base.
fn http(url: &str) -> Result<reqwest::Client> {
    let origin = reqwest::Url::parse(url)
        .context("URL Foundry invalide")?
        .origin()
        .ascii_serialization();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ORIGIN,
        reqwest::header::HeaderValue::from_str(&origin).context("en-tête Origin")?,
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .context("client HTTP")
}

async fn api_status(url: &str) -> Result<Value> {
    Ok(reqwest::get(format!("{url}/api/status"))
        .await?
        .json()
        .await?)
}

/// Authentifie la session du client `http` sur /auth (mode setup uniquement).
/// Tente JSON puis formulaire, et VÉRIFIE : /setup ne doit plus rediriger
/// vers /auth.
async fn admin_auth(http: &reqwest::Client, url: &str, password: &str) -> Result<()> {
    let is_authed = |resp: &reqwest::Response| {
        !resp.url().path().ends_with("/auth") && !resp.url().path().ends_with("/join")
    };
    // amorce la session
    let _ = http.get(format!("{url}/auth")).send().await;
    for attempt in 0..2 {
        let req = http.post(format!("{url}/auth"));
        let req = if attempt == 0 {
            req.json(&json!({ "action": "adminAuth", "adminPassword": password }))
        } else {
            req.form(&[("action", "adminAuth"), ("adminPassword", password)])
        };
        let _ = req.send().await;
        let check = http.get(format!("{url}/setup")).send().await?;
        if is_authed(&check) {
            return Ok(());
        }
    }
    bail!("authentification admin refusée (mot de passe FOUNDRY_ADMIN_PASSWORD incorrect ?)")
}

async fn setup_post(http: &reqwest::Client, url: &str, body: &Value) -> Result<Value> {
    view_post(http, url, "setup", body).await
}

/// POST JSON sur une vue Foundry (`/setup`, `/create`…) ; une réponse
/// `{error}` ou un statut non-2xx devient une erreur.
async fn view_post(http: &reqwest::Client, url: &str, view: &str, body: &Value) -> Result<Value> {
    let resp = http
        .post(format!("{url}/{view}"))
        .json(body)
        .send()
        .await
        .with_context(|| format!("POST /{view}"))?;
    let status = resp.status();
    let data: Value = resp.json().await.unwrap_or(Value::Null);
    if let Some(err) = data.get("error").and_then(Value::as_str) {
        bail!("{view} a répondu une erreur : {err}");
    }
    if !status.is_success() {
        bail!("POST /{view} {} → HTTP {status} : {data}", body["action"]);
    }
    Ok(data)
}

/// createBackup : l'id porte la convention de Foundry (`type.pkg.AAAA-MM-JJ.epoch`)
/// — un timestamp est nécessaire, on le prend sur l'horloge système.
async fn run_backup(
    http: &reqwest::Client,
    url: &str,
    pkg_type: &str,
    id: &str,
    note: &str,
) -> Result<Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let days = now / 86_400_000;
    // date civile depuis l'epoch (algorithme de Howard Hinnant, sans dépendance)
    let (y, m, d) = {
        let z = days as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (era * 400 + yoe + i64::from(m <= 2), m, d)
    };
    let backup_id = format!("{pkg_type}.{id}.{y:04}-{m:02}-{d:02}.{now}");
    let body = json!({
        "action": "createBackup",
        "backups": [{ "type": pkg_type, "packageId": id, "note": note, "id": backup_id }],
    });
    let data = setup_post(http, url, &body).await?;
    Ok(json!({
        "backedUp": id, "type": pkg_type, "backupId": backup_id,
        "note": note, "response": data,
    }))
}

fn admin_password(state: &McpState) -> Result<&str> {
    state
        .admin_password
        .as_deref()
        .map(String::as_str)
        .ok_or_else(|| anyhow!("FOUNDRY_ADMIN_PASSWORD n'est pas configuré sur le serveur"))
}

/* ---------------------------------------------------------------- tools */

pub async fn run(state: &McpState, name: &str, args: &Value) -> Result<Value> {
    let url = base_url(state);
    match name {
        "admin_status" => {
            let status = api_status(&url).await?;
            if let Some(w) = status.get("world").and_then(Value::as_str) {
                *state.last_world_id.lock().await = Some(w.to_string());
            }
            Ok(text_response(&json!({
                "status": status,
                "adminPasswordConfigured": state.admin_password.is_some(),
            })))
        }

        "admin_edit_world" => {
            let status = api_status(&url).await?;
            let world_id = status.get("world").and_then(Value::as_str).ok_or_else(|| {
                anyhow!("aucun monde actif — admin_edit_world passe par la session de jeu du bot")
            })?;
            // état courant (le formulaire envoie le document complet) —
            // les métadonnées vivent sous la clé `world` du dump.
            let dump = state.foundry.request_world().await?;
            let world = dump.get("world").cloned().unwrap_or(Value::Null);
            let current = |k: &str| world.get(k).cloned().unwrap_or(Value::Null);
            let mut payload = Map::new();
            payload.insert("action".into(), json!("editWorld"));
            payload.insert("id".into(), json!(world_id));
            for (arg, field) in [
                ("title", "title"),
                ("description", "description"),
                ("background", "background"),
                ("next_session", "nextSession"),
            ] {
                let v = match str_arg(args, arg) {
                    Some(s) if s.is_empty() => Value::Null,
                    Some(s) => json!(s),
                    None => current(field),
                };
                payload.insert(field.into(), v);
            }
            // v14 vérifie aussi le mot de passe admin sur /setup et répond 401
            // (modification pourtant appliquée) quand il manque : on le joint.
            if let Ok(pw) = admin_password(state) {
                payload.insert("adminPassword".into(), json!(pw));
            }
            // session de jeu du bot (même canal que l'écran MJ « Edit World »)
            let session = state
                .foundry
                .session_id()
                .await
                .ok_or_else(|| anyhow!("session Foundry indisponible (bot déconnecté ?)"))?;
            let resp = http(&url)?
                .post(format!("{url}/setup"))
                .header(reqwest::header::COOKIE, format!("session={session}"))
                .json(&Value::Object(payload.clone()))
                .send()
                .await
                .context("POST /setup editWorld")?;
            let status_code = resp.status();
            let data: Value = resp.json().await.unwrap_or(Value::Null);
            if let Some(err) = data.get("error").and_then(Value::as_str) {
                bail!("editWorld refusé : {err}");
            }
            if !status_code.is_success() {
                bail!("editWorld → HTTP {status_code} : {data}");
            }
            Ok(text_response(&json!({
                "edited": world_id,
                "title": payload["title"], "background": payload["background"],
                "nextSession": payload["nextSession"],
                "note": "visible sur la page de connexion (les joueurs en jeu ne voient rien changer)",
            })))
        }

        "admin_create_world" => {
            let password = admin_password(state)?;
            let id = str_arg(args, "id").ok_or_else(|| anyhow!("'id' est requis"))?;
            let title = str_arg(args, "title").ok_or_else(|| anyhow!("'title' est requis"))?;
            let system = str_arg(args, "system").ok_or_else(|| anyhow!("'system' est requis"))?;
            let status = api_status(&url).await?;
            if let Some(active) = status.get("world").and_then(Value::as_str) {
                bail!("le monde '{active}' tourne — la création exige le mode setup");
            }
            let client = http(&url)?;
            admin_auth(&client, &url, password).await?;
            let mut body = json!({
                "action": "createWorld",
                "id": id, "title": title, "system": system,
                // description est requis par le schéma World (HTMLField)
                "description": str_arg(args, "description").unwrap_or_default(),
            });
            if let Some(bg) = str_arg(args, "background") {
                body["background"] = json!(bg);
            }
            // v14 a déplacé createWorld vers la vue /create ; v13 le prend sur /setup
            let data = match view_post(&client, &url, "create", &body).await {
                Ok(d) => d,
                Err(_) => setup_post(&client, &url, &body).await?,
            };
            Ok(text_response(&json!({
                "created": id, "title": title, "system": system,
                "note": "monde vide créé — le lancer avec admin_launch_world, ou y verser du contenu avec copy_documents / copy_assets",
                "response": data,
            })))
        }

        "admin_install_package" => {
            let password = admin_password(state)?;
            let pkg_type = str_arg(args, "type").unwrap_or_else(|| "module".into());
            let id = str_arg(args, "id").ok_or_else(|| anyhow!("'id' est requis"))?;
            let manifest =
                str_arg(args, "manifest").ok_or_else(|| anyhow!("'manifest' est requis"))?;
            let status = api_status(&url).await?;
            if let Some(active) = status.get("world").and_then(Value::as_str) {
                bail!("le monde '{active}' tourne — l'installation exige le mode setup");
            }
            // déjà présent ? le manifest statique fait foi
            let static_manifest = match pkg_type.as_str() {
                "system" => format!("{url}/systems/{id}/system.json"),
                _ => format!("{url}/modules/{id}/module.json"),
            };
            if let Ok(resp) = reqwest::get(&static_manifest).await
                && resp.status().is_success()
                && let Ok(m) = resp.json::<Value>().await
            {
                return Ok(text_response(&json!({
                    "package": id, "installed": m.get("version"),
                    "created": false,
                    "reason": "déjà installé — utiliser admin_update_package pour le mettre à jour",
                })));
            }
            let client = http(&url)?;
            admin_auth(&client, &url, password).await?;
            setup_post(
                &client,
                &url,
                &json!({
                    "action": "installPackage", "type": pkg_type, "id": id, "manifest": manifest,
                }),
            )
            .await?;
            // confirmation : le manifest statique apparaît quand l'extraction est finie
            let mut installed = None;
            for _ in 0..24 {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if let Ok(resp) = reqwest::get(&static_manifest).await
                    && resp.status().is_success()
                    && let Ok(m) = resp.json::<Value>().await
                {
                    installed = m.get("version").and_then(Value::as_str).map(String::from);
                    break;
                }
            }
            Ok(text_response(&json!({
                "package": id, "type": pkg_type,
                "installed": installed, "created": installed.is_some(),
                "note": if installed.is_some() { "vérifié installé (manifest statique)" }
                        else { "installation lancée mais non confirmée en 2 min — re-vérifier" },
            })))
        }

        "admin_list_backups" => {
            let password = admin_password(state)?;
            let status = api_status(&url).await?;
            if let Some(active) = status.get("world").and_then(Value::as_str) {
                bail!("le monde '{active}' tourne — les sauvegardes exigent le mode setup");
            }
            let client = http(&url)?;
            admin_auth(&client, &url, password).await?;
            let data = setup_post(&client, &url, &json!({ "action": "listBackups" })).await?;
            Ok(text_response(&data))
        }

        "admin_backup_world" => {
            let password = admin_password(state)?;
            let pkg_type = str_arg(args, "type").unwrap_or_else(|| "world".into());
            let id = match str_arg(args, "id") {
                Some(i) => i,
                None => state
                    .last_world_id
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| anyhow!("précisez 'id' (aucun monde vu actif récemment)"))?,
            };
            let note = str_arg(args, "note").unwrap_or_else(|| "sauvegarde via MCP".into());
            let status = api_status(&url).await?;
            if let Some(active) = status.get("world").and_then(Value::as_str) {
                bail!(
                    "le monde '{active}' tourne — la sauvegarde exige le mode setup \
                     (admin_shutdown_world d'abord)"
                );
            }
            let client = http(&url)?;
            admin_auth(&client, &url, password).await?;
            let backup = run_backup(&client, &url, &pkg_type, &id, &note).await?;
            Ok(text_response(&backup))
        }

        "admin_shutdown_world" => {
            if args.get("confirm").and_then(Value::as_bool) != Some(true) {
                bail!("confirm:true requis — éteint le monde pour TOUT LE MONDE, bot compris");
            }
            let password = admin_password(state)?;
            let status = api_status(&url).await?;
            let world = status.get("world").and_then(Value::as_str);
            let Some(world) = world else {
                bail!("aucun monde actif — rien à éteindre");
            };
            *state.last_world_id.lock().await = Some(world.to_string());
            let client = http(&url)?;
            let _ = client.get(format!("{url}/join")).send().await; // session
            // v14 : POST /setup {action: worldShutdown} (répond par une redirection) ;
            // v13 : POST /join {action: shutdown}. On tente v14 puis v13, et on
            // vérifie sur /api/status — les deux répondent 2xx même sans effet.
            let attempts = [
                (
                    "setup",
                    json!({ "action": "worldShutdown", "adminPassword": password }),
                ),
                (
                    "join",
                    json!({ "action": "shutdown", "adminPassword": password }),
                ),
            ];
            let mut data = Value::Null;
            let mut last = String::new();
            for (view, body) in attempts {
                let resp = client
                    .post(format!("{url}/{view}"))
                    .json(&body)
                    .send()
                    .await
                    .with_context(|| format!("POST /{view} shutdown"))?;
                let code = resp.status();
                let text = resp.text().await.unwrap_or_default();
                data = serde_json::from_str(&text).unwrap_or(Value::Null);
                last = format!(
                    "/{view} → HTTP {code} : {}",
                    text.chars().take(200).collect::<String>()
                );
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                let after = api_status(&url).await?;
                if after.get("world").and_then(Value::as_str).is_none() {
                    break;
                }
            }
            let after = api_status(&url).await?;
            if let Some(w) = after.get("world").and_then(Value::as_str) {
                bail!(
                    "shutdown sans effet — le monde '{w}' tourne toujours ({last}) ; mot de passe admin correct ?"
                );
            }
            Ok(text_response(&json!({
                "shutdown": world,
                "note": "monde éteint — relancer avec admin_launch_world (le bot se reconnectera seul)",
                "response": data,
            })))
        }

        "admin_launch_world" => {
            let password = admin_password(state)?;
            let world =
                match str_arg(args, "world") {
                    Some(w) => w,
                    None => state.last_world_id.lock().await.clone().ok_or_else(|| {
                        anyhow!("précisez 'world' (aucun monde vu actif récemment)")
                    })?,
                };
            let status = api_status(&url).await?;
            if let Some(active) = status.get("world").and_then(Value::as_str) {
                bail!(
                    "le monde '{active}' tourne déjà — l'éteindre d'abord (admin_shutdown_world)"
                );
            }
            let client = http(&url)?;
            admin_auth(&client, &url, password).await?;
            let data = setup_post(
                &client,
                &url,
                &json!({ "action": "launchWorld", "world": world }),
            )
            .await?;
            Ok(text_response(&json!({
                "launching": world,
                "note": "démarrage en cours (~10-30 s) — admin_status ou ping pour suivre ; le bot se reconnecte seul",
                "response": data,
            })))
        }

        "admin_check_package" | "admin_update_package" => {
            let pkg_type = str_arg(args, "type").unwrap_or_else(|| "module".into());
            let id = str_arg(args, "id").ok_or_else(|| anyhow!("'id' est requis"))?;
            let status = api_status(&url).await?;
            let world_active = status
                .get("world")
                .and_then(Value::as_str)
                .map(String::from);

            // Version installée + URL du manifest distant : lues sur le manifest
            // STATIQUE servi par l'instance — la réponse checkPackage ne contient
            // que le côté distant (vérifié en réel sur v13).
            let static_manifest = match pkg_type.as_str() {
                "module" => Some(format!("{url}/modules/{id}/module.json")),
                "system" => Some(format!("{url}/systems/{id}/system.json")),
                _ => None, // worlds : pas de manifest statique servi
            };
            let read_local = || async {
                let p = static_manifest.as_ref()?;
                let m: Value = reqwest::get(p).await.ok()?.json().await.ok()?;
                Some((
                    m.get("version").and_then(Value::as_str).map(String::from),
                    m.get("manifest").and_then(Value::as_str).map(String::from),
                ))
            };
            let (installed, manifest_url) = read_local().await.unwrap_or((None, None));

            if name == "admin_check_package" {
                // Check possible même monde allumé : le manifest distant se lit
                // directement à son URL. Monde éteint : checkPackage fait foi.
                let remote =
                    if world_active.is_some() {
                        let mu = manifest_url.clone().ok_or_else(|| {
                            anyhow!(
                                "manifest distant introuvable pour '{id}' — paquet installé ? \
                         (sinon, éteindre le monde pour un checkPackage complet)"
                            )
                        })?;
                        let m: Value = reqwest::get(&mu).await?.json().await?;
                        m.get("version").and_then(Value::as_str).map(String::from)
                    } else {
                        let client = http(&url)?;
                        admin_auth(&client, &url, admin_password(state)?).await?;
                        let check = setup_post(&client, &url, &json!({
                        "action": "checkPackage", "strict": false, "type": pkg_type, "id": id,
                    }))
                    .await?;
                        check
                            .pointer("/remote/version")
                            .and_then(Value::as_str)
                            .map(String::from)
                    };
                let update_available =
                    matches!((&installed, &remote), (Some(a), Some(b)) if a != b);
                return Ok(text_response(&json!({
                    "package": id, "type": pkg_type,
                    "installed": installed, "remote": remote,
                    "updateAvailable": update_available,
                })));
            }

            // update : mode setup obligatoire (installPackage).
            if let Some(active) = world_active {
                bail!(
                    "le monde '{active}' tourne — la mise à jour exige le mode setup \
                     (admin_shutdown_world d'abord)"
                );
            }
            let client = http(&url)?;
            admin_auth(&client, &url, admin_password(state)?).await?;
            let check = setup_post(
                &client,
                &url,
                &json!({
                    "action": "checkPackage", "strict": false, "type": pkg_type, "id": id,
                }),
            )
            .await?;
            let remote = check
                .pointer("/remote/version")
                .and_then(Value::as_str)
                .map(String::from);
            let manifest = check
                .pointer("/remote/manifest")
                .and_then(Value::as_str)
                .map(String::from)
                .or(manifest_url)
                .ok_or_else(|| anyhow!("pas de manifest distant pour '{id}'"))?;
            if installed.is_some() && installed == remote {
                return Ok(text_response(&json!({
                    "package": id, "installed": installed,
                    "updated": false, "reason": "déjà à jour",
                })));
            }
            // Filet : on sauvegarde AVANT de toucher au paquet (désactivable).
            let backup = if args.get("backup").and_then(Value::as_bool) != Some(false) {
                match run_backup(&client, &url, &pkg_type, &id, "avant mise à jour (MCP)").await {
                    Ok(b) => b.get("backupId").cloned().unwrap_or(Value::Null),
                    Err(e) => bail!(
                        "sauvegarde préalable échouée ({e:#}) — mise à jour ANNULÉE. \
                         Relancer avec backup:false pour passer outre."
                    ),
                }
            } else {
                Value::Null
            };
            // installPackage répond vite ; la fin réelle se vérifie sur le
            // manifest statique (la version change quand l'extraction est finie).
            // Un téléchargement distant peut échouer (timeout, réseau, miroir) :
            // 5 tentatives complètes avant d'abandonner.
            let mut verified = None;
            let mut attempts = 0;
            let mut last_error = String::new();
            'attempts: for attempt in 1..=5 {
                attempts = attempt;
                if attempt > 1 {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
                if let Err(e) = setup_post(&client, &url, &json!({
                    "action": "installPackage", "type": pkg_type, "id": id, "manifest": manifest,
                }))
                .await
                {
                    last_error = format!("tentative {attempt} : {e:#}");
                    continue;
                }
                // vérification : jusqu'à 2 min pour voir la nouvelle version
                for _ in 0..24 {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    if let Some((now, _)) = read_local().await
                        && now.is_some()
                        && now != installed
                    {
                        verified = now;
                        break 'attempts;
                    }
                }
                last_error = format!(
                    "tentative {attempt} : installation lancée mais non confirmée en 2 min"
                );
            }
            Ok(text_response(&json!({
                "package": id, "type": pkg_type,
                "from": installed, "to": remote,
                "installedNow": verified,
                "updated": verified.is_some(),
                "attempts": attempts,
                "backupId": backup,
                "note": if verified.is_some() { "vérifié installé (manifest statique)".to_string() }
                        else { format!("échec après {attempts} tentatives — {last_error}") },
            })))
        }
        other => bail!("Unknown tool: {other}"),
    }
}
