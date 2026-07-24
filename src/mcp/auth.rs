//! Authentification des clients MCP par jeton biscuit.
//!
//! # Ce qui a changé, et pourquoi
//!
//! La passerelle s'authentifiait par un secret opaque placé dans le chemin
//! d'URL. Ça marchait, mais un secret partagé n'a ni portée, ni expiration, ni
//! révocation : le donner à un intégrateur, c'était lui donner les 135 outils
//! avec les droits de Gamemaster, pour toujours, sans moyen de revenir en
//! arrière autrement qu'en le changeant pour tout le monde.
//!
//! Un biscuit occupe exactement la même place — dans le chemin, parce que
//! plusieurs clients MCP ne savent pas envoyer d'en-tête arbitraire — mais il
//! porte ce que le secret ne portait pas :
//!
//! * une **expiration**, vérifiée à chaque requête ;
//! * une **portée** : un jeton atténué en lecture seule ne peut plus écrire,
//!   même si l'appelant le voulait ;
//! * une **révocation** par jeton, sans toucher aux autres ;
//! * une **atténuation hors ligne** : on restreint un jeton existant sans clé
//!   privée et sans redéployer.
//!
//! # Le piège à ne pas réintroduire
//!
//! Tout le Datalog dynamique passe par `code_with_params()`. Construire les
//! faits avec `format!()` rouvrirait une injection : une valeur contenant
//! `"); right("admin` ajouterait des faits au jeton. Le test
//! `injection_datalog` échoue si quelqu'un « simplifie » dans ce sens.

use anyhow::{Context, Result, anyhow, bail};
use biscuit_auth::builder::{Algorithm, AuthorizerBuilder, Term};
use biscuit_auth::{Biscuit, KeyPair, PrivateKey, PublicKey};
use std::collections::HashMap;

/// Durée de vie par défaut d'un jeton frappé par `--mint-token`.
pub const DEFAULT_TTL_DAYS: i64 = 365;

/// Portée d'un jeton, telle que le jeton lui-même la déclare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Full,
    ReadOnly,
}

/// Comment la passerelle authentifie ses appelants.
#[derive(Clone)]
pub enum McpAuth {
    /// Jetons biscuit vérifiés contre la clé publique racine.
    Biscuit(Box<PublicKey>),
}

impl McpAuth {
    /// Lit `MCP_ROOT_PUBLIC_KEY`, ou la dérive de `MCP_ROOT_PRIVATE_KEY`.
    ///
    /// La publique suffit à vérifier : une passerelle qui n'a pas besoin de
    /// frapper de nouveaux jetons n'a aucune raison de détenir la privée.
    pub fn from_env() -> Result<Self> {
        if let Ok(pk) = std::env::var("MCP_ROOT_PUBLIC_KEY")
            && !pk.trim().is_empty()
        {
            return Ok(Self::Biscuit(Box::new(parse_public(pk.trim())?)));
        }
        if let Ok(sk) = std::env::var("MCP_ROOT_PRIVATE_KEY")
            && !sk.trim().is_empty()
        {
            return Ok(Self::Biscuit(Box::new(
                KeyPair::from(&parse_private(sk.trim())?).public(),
            )));
        }
        bail!(
            "no MCP authentication configured. Generate a key pair with --generate-mcp-key, \
             set MCP_ROOT_PRIVATE_KEY (or MCP_ROOT_PUBLIC_KEY), then mint a client token \
             with --mint-token."
        )
    }

    /// Vérifie le jeton présenté dans le chemin d'URL.
    ///
    /// Rend la portée qu'il déclare — c'est elle qui décide ensuite du
    /// catalogue exposé et de ce que le dispatch accepte d'exécuter.
    pub fn verify(&self, token: &str) -> Result<Scope> {
        let Self::Biscuit(root) = self;
        let biscuit = Biscuit::from_base64(token, *root.as_ref())
            .map_err(|_| anyhow!("invalid or unreadable MCP token"))?;

        if let Some(revoked) = revoked_ids() {
            for id in biscuit.revocation_identifiers() {
                if revoked.contains(&hex::encode(&id)) {
                    bail!("this MCP token has been revoked");
                }
            }
        }

        // On demande l'écriture d'abord, puis la lecture. La portée n'est donc
        // pas un fait qu'on interroge : elle se DÉDUIT de ce que le jeton
        // autorise réellement.
        //
        // C'est ce qui rend l'atténuation utilisable. Les faits d'un bloc
        // d'atténuation ne sont PAS visibles des règles de l'autorisateur
        // (c'est la règle de portée de biscuit, et c'est elle qui garantit
        // qu'atténuer ne peut jamais élargir) : un `scope("readonly")` ajouté
        // après coup serait donc invisible. Une CONTRAINTE, elle, s'applique —
        // d'où `check if operation("read")`, qui fonctionne aussi bien dans le
        // bloc d'autorité que dans une atténuation faite hors ligne.
        if authorize_for(&biscuit, "write").is_ok() {
            return Ok(Scope::Full);
        }
        authorize_for(&biscuit, "read")
            .map(|()| Scope::ReadOnly)
            .map_err(|e| anyhow!("MCP token rejected: {e}"))
    }
}

/// Le jeton autorise-t-il cette classe d'opération ?
fn authorize_for(biscuit: &Biscuit, operation: &str) -> Result<()> {
    let mut authorizer = AuthorizerBuilder::new()
        .time()
        .code_with_params(
            r#"operation({op});"#,
            HashMap::from([("op".to_string(), Term::Str(operation.to_string()))]),
            HashMap::new(),
        )
        .context("adding the ambient operation")?
        // Datalog statique : aucune valeur interpolée, donc `code()` est sûr.
        .code(
            r#"
            allow if gateway("foundry-mcp");
            deny if true;
        "#,
        )
        .context("building the MCP authorizer")?
        // Borne le temps d'évaluation : un jeton peut embarquer du Datalog
        // arbitraire dans ses blocs d'atténuation.
        .set_limits(biscuit_auth::datalog::RunLimits {
            max_time: std::time::Duration::from_millis(50),
            ..Default::default()
        })
        .build(biscuit)
        .map_err(|e| anyhow!("{e}"))?;
    authorizer.authorize().map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

/// Identifiants de révocation refusés, depuis `MCP_REVOKED_TOKENS`
/// (hexadécimal, séparés par des virgules).
fn revoked_ids() -> Option<Vec<String>> {
    let raw = std::env::var("MCP_REVOKED_TOKENS").ok()?;
    let ids: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    (!ids.is_empty()).then_some(ids)
}

fn parse_public(hex_key: &str) -> Result<PublicKey> {
    PublicKey::from_bytes_hex(hex_key, Algorithm::Ed25519)
        .map_err(|e| anyhow!("MCP_ROOT_PUBLIC_KEY is not a valid key: {e}"))
}

fn parse_private(hex_key: &str) -> Result<PrivateKey> {
    PrivateKey::from_bytes_hex(hex_key, Algorithm::Ed25519)
        .map_err(|e| anyhow!("MCP_ROOT_PRIVATE_KEY is not a valid key: {e}"))
}

/// Une paire de clés racine, pour `--generate-mcp-key`.
pub fn generate_root() -> (String, String) {
    let kp = KeyPair::new();
    (
        kp.private().to_bytes_hex().to_string(),
        kp.public().to_bytes_hex(),
    )
}

/// Frappe un jeton client. `label` sert uniquement à s'y retrouver plus tard.
///
/// Toutes les valeurs dynamiques passent par `code_with_params` : voir la note
/// d'injection en tête de module.
pub fn mint_token(
    private_hex: &str,
    label: &str,
    scope: Scope,
    ttl_days: i64,
    now_unix: i64,
) -> Result<(String, String)> {
    let kp = KeyPair::from(&parse_private(private_hex)?);
    let expiry = now_unix + ttl_days * 86_400;

    let mut builder = Biscuit::builder()
        .code_with_params(
            r#"gateway("foundry-mcp"); label({label});"#,
            HashMap::from([("label".to_string(), Term::Str(label.to_string()))]),
            HashMap::new(),
        )
        .context("building the token authority block")?
        .code_with_params(
            r#"check if time($t), $t < {expiry};"#,
            HashMap::from([("expiry".to_string(), Term::Date(expiry as u64))]),
            HashMap::new(),
        )
        .context("adding the expiry check")?;

    if scope == Scope::ReadOnly {
        // Une CONTRAINTE et non un fait : c'est la seule forme qui survit à
        // une atténuation (voir la note dans `verify`).
        builder = builder
            .code(r#"check if operation("read");"#)
            .context("adding the read-only restriction")?;
    }

    let biscuit = builder.build(&kp).context("signing the token")?;
    let revocation = biscuit
        .revocation_identifiers()
        .last()
        .map(hex::encode)
        .unwrap_or_default();
    Ok((
        biscuit.to_base64().context("encoding the token")?,
        revocation,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// L'autorisateur lit l'horloge réelle : les jetons de test doivent donc
    /// être datés par rapport à elle, pas à une date arbitraire.
    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn root() -> (String, String) {
        generate_root()
    }

    fn auth(public_hex: &str) -> McpAuth {
        McpAuth::Biscuit(Box::new(parse_public(public_hex).unwrap()))
    }

    #[test]
    fn jeton_valide_accepte() {
        let (sk, pk) = root();
        let (token, _) = mint_token(&sk, "claude-code", Scope::Full, 30, now()).unwrap();
        assert_eq!(auth(&pk).verify(&token).unwrap(), Scope::Full);
    }

    #[test]
    fn portee_lecture_seule_portee_par_le_jeton() {
        // La portée vient du JETON, pas de l'appel : un client ne peut pas se
        // réattribuer l'écriture en changeant sa requête.
        let (sk, pk) = root();
        let (token, _) = mint_token(&sk, "readonly-client", Scope::ReadOnly, 30, now()).unwrap();
        assert_eq!(auth(&pk).verify(&token).unwrap(), Scope::ReadOnly);
    }

    #[test]
    fn jeton_expire_refuse() {
        let (sk, pk) = root();
        // TTL négatif : déjà expiré au moment de la frappe.
        // Frappé avec une expiration déjà passée (epoch + 1 jour).
        let (token, _) = mint_token(&sk, "vieux", Scope::Full, 1, 0).unwrap();
        assert!(auth(&pk).verify(&token).is_err());
    }

    #[test]
    fn jeton_d_une_autre_racine_refuse() {
        let (sk, _) = root();
        let (_, autre_pk) = root();
        let (token, _) = mint_token(&sk, "etranger", Scope::Full, 30, now()).unwrap();
        assert!(auth(&autre_pk).verify(&token).is_err());
    }

    #[test]
    fn jeton_illisible_refuse() {
        let (_, pk) = root();
        for mauvais in ["", "pas-un-jeton", "AAAA", "../../etc/passwd"] {
            assert!(auth(&pk).verify(mauvais).is_err(), "accepté : {mauvais}");
        }
    }

    #[test]
    fn attenuation_restreint_en_lecture_seule() {
        // Un détenteur peut restreindre son jeton hors ligne, jamais l'élargir :
        // c'est la propriété qui justifie biscuit ici.
        let (sk, pk) = root();
        let (token, _) = mint_token(&sk, "client", Scope::Full, 30, now()).unwrap();
        let biscuit = Biscuit::from_base64(&token, parse_public(&pk).unwrap()).unwrap();
        // Hors ligne, sans la clé privée : le détenteur se restreint lui-même.
        let restreint = biscuit
            .append(
                biscuit_auth::builder::BlockBuilder::new()
                    .code(r#"check if operation("read");"#)
                    .unwrap(),
            )
            .unwrap();
        let attenue = restreint.to_base64().unwrap();
        assert_eq!(auth(&pk).verify(&attenue).unwrap(), Scope::ReadOnly);
    }

    #[test]
    fn injection_datalog() {
        // Une étiquette hostile doit rester UNE chaîne, jamais du Datalog.
        // Ce test échoue si quelqu'un remplace code_with_params par format!().
        let (sk, pk) = root();
        let hostile = r#"x"); scope("readonly"); label(""#;
        let (token, _) = mint_token(&sk, hostile, Scope::Full, 30, now()).unwrap();
        // La portée doit rester Full : le faux `scope("readonly")` injecté dans
        // l'étiquette ne doit pas être devenu un fait du jeton.
        assert_eq!(auth(&pk).verify(&token).unwrap(), Scope::Full);
    }

    #[test]
    fn revocation_par_jeton() {
        let (sk, pk) = root();
        let (token, revocation) = mint_token(&sk, "compromis", Scope::Full, 30, now()).unwrap();
        assert!(auth(&pk).verify(&token).is_ok());

        // SAFETY: test mono-thread, on restaure juste après.
        unsafe { std::env::set_var("MCP_REVOKED_TOKENS", &revocation) };
        let refuse = auth(&pk).verify(&token);
        unsafe { std::env::remove_var("MCP_REVOKED_TOKENS") };

        assert!(refuse.is_err(), "un jeton révoqué a été accepté");
        assert!(refuse.unwrap_err().to_string().contains("revoked"));
    }
}
