pub mod auth;
pub mod client;
pub mod documents;
pub mod protocol;

use serde_json::Value;

/// GET /api/status (public) — utilisé par ping et world-overview.
pub async fn auth_status(http: &reqwest::Client, hostname: &str) -> Option<Value> {
    let (host, base) = auth::split_host(hostname);
    let url = format!("https://{host}{base}/api/status");
    http.get(&url).send().await.ok()?.json().await.ok()
}
