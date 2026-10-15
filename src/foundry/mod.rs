pub mod auth;
pub mod client;
pub mod documents;
pub mod protocol;
pub mod signing;

use serde_json::Value;

/// GET /api/status (public) — utilisé par ping et world-overview.
pub async fn auth_status(http: &reqwest::Client, hostname: &str) -> Option<Value> {
    let (host, base) = auth::split_host(hostname);
    // Le schéma vient de `schemes()` et non d'un https en dur : une instance
    // locale (« localhost:30000 ») parle HTTP simple, et le forcer en HTTPS
    // faisait échouer /api/status — donc `ping` — sur tous les Foundry
    // auto-hébergés, qui sont précisément ceux qu'on veut voir marcher.
    let (http_s, _) = auth::schemes(hostname);
    let url = format!("{http_s}://{host}{base}/api/status");
    http.get(&url).send().await.ok()?.json().await.ok()
}
