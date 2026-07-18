//! Authentification Foundry : détection de génération (/api/status), session
//! (GET /join), login (POST /join). Supporte les hostnames à préfixe de route
//! (« rpg.example.com/star-wars ») et le binding v13 (query) / v14 (cookie).

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct Credential {
    #[serde(rename = "_id")]
    pub id: String,
    pub hostname: String,
    pub userid: String,
    pub password: String,
}

pub fn parse_credentials(json: &str) -> Result<Vec<Credential>> {
    serde_json::from_str(json).context("FOUNDRY_CREDENTIALS_JSON invalide (tableau attendu)")
}

/// « host/prefixe » → (host, "/prefixe") — préfixe éventuellement vide.
pub fn split_host(hostname: &str) -> (String, String) {
    match hostname.find('/') {
        None => (hostname.to_string(), String::new()),
        Some(i) => (
            hostname[..i].to_string(),
            hostname[i..].trim_end_matches('/').to_string(),
        ),
    }
}

/// Génération majeure via GET /api/status (public). None si indéterminable.
pub async fn detect_generation(http: &reqwest::Client, hostname: &str) -> Option<u32> {
    let (host, base) = split_host(hostname);
    let url = format!("https://{host}{base}/api/status");
    let body: Value = http.get(&url).send().await.ok()?.json().await.ok()?;
    if let Some(g) = body.pointer("/release/generation").and_then(Value::as_u64) {
        return Some(g as u32);
    }
    let version = body.get("version")?.as_str()?;
    version.split('.').next()?.parse().ok()
}

/// GET /join : récupère (ou génère) l'id de session depuis le cookie.
pub async fn get_session(http: &reqwest::Client, hostname: &str) -> Result<String> {
    let (host, base) = split_host(hostname);
    let url = format!("https://{host}{base}/join");
    let resp = http.get(&url).send().await.context("GET /join")?;
    for cookie in resp.headers().get_all(reqwest::header::SET_COOKIE) {
        let raw = cookie.to_str().unwrap_or_default();
        if let Some(rest) = raw.strip_prefix("session=")
            && let Some(sid) = rest.split(';').next()
            && !sid.is_empty()
        {
            return Ok(sid.to_string());
        }
    }
    // pas de cookie : session aléatoire 24 hex, comme le client TS
    let sid: String = (0..24)
        .map(|_| char::from_digit(rand::random::<u32>() % 16, 16).unwrap())
        .collect();
    Ok(sid)
}

/// POST /join : authentifie la session sur le compte donné.
pub async fn authenticate(
    http: &reqwest::Client,
    hostname: &str,
    session_id: &str,
    cred: &Credential,
) -> Result<()> {
    let (host, base) = split_host(hostname);
    let url = format!("https://{host}{base}/join");
    let payload = serde_json::json!({
        "action": "join",
        "userid": cred.userid,
        "password": cred.password,
    });
    let resp = http
        .post(&url)
        .header(reqwest::header::COOKIE, format!("session={session_id}"))
        .json(&payload)
        .send()
        .await
        .context("POST /join")?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let ok = status.is_success() && body.get("status").and_then(Value::as_str) == Some("success");
    if !ok {
        bail!("authentification refusée ({status}) : {body}");
    }
    Ok(())
}

/// URL WebSocket selon la génération : v13 = session en query, v14 = cookie,
/// inconnue = les deux (stratégie compat, sans risque).
pub fn socket_url_and_cookie(
    hostname: &str,
    session_id: &str,
    generation: Option<u32>,
) -> (String, Option<String>) {
    let (host, base) = split_host(hostname);
    let transport = "EIO=4&transport=websocket";
    match generation {
        Some(g) if g >= 14 => (
            format!("wss://{host}{base}/socket.io/?{transport}"),
            Some(format!("session={session_id}")),
        ),
        Some(_) => (
            format!("wss://{host}{base}/socket.io/?session={session_id}&{transport}"),
            None,
        ),
        None => (
            format!("wss://{host}{base}/socket.io/?session={session_id}&{transport}"),
            Some(format!("session={session_id}")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_host_prefixe() {
        assert_eq!(
            split_host("rpg.example.com"),
            ("rpg.example.com".into(), "".into())
        );
        assert_eq!(
            split_host("rpg.example.com/star-wars/"),
            ("rpg.example.com".into(), "/star-wars".into())
        );
    }

    #[test]
    fn binding_par_generation() {
        let (u13, c13) = socket_url_and_cookie("h/sw", "S", Some(13));
        assert!(u13.contains("session=S") && c13.is_none());
        let (u14, c14) = socket_url_and_cookie("h/sw", "S", Some(14));
        assert!(!u14.contains("session=S") && c14.as_deref() == Some("session=S"));
        let (uu, cu) = socket_url_and_cookie("h", "S", None);
        assert!(uu.contains("session=S") && cu.is_some());
        assert!(u13.starts_with("wss://h/sw/socket.io/"));
    }
}
