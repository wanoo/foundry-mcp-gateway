//! Signature des commandes envoyées au module compagnon.
//!
//! # Pourquoi
//!
//! Foundry relaie les canaux `module.*` à TOUS les clients du monde et ne
//! donne au récepteur aucune information sur l'émetteur. N'importe quel joueur
//! pouvait donc écrire dans sa console :
//!
//! ```js
//! game.socket.emit("module.foundry-mcp-gateway-companion",
//!   { mcp: true, cmd: "run_macro", id: "x", args: { macro: "…" } });
//! ```
//!
//! …et le MJ « responder » exécutait la commande avec ses droits. Le champ
//! `user` du message est déclaratif, donc inutilisable comme preuve.
//!
//! # Le choix : ECDSA P-256, pas Ed25519
//!
//! La vérification a lieu dans un navigateur, avec `crypto.subtle` et sans
//! aucune dépendance. Ed25519 y est plus récent (Chrome 137+), alors que
//! Foundry tourne aussi dans l'Electron de son application de bureau, dont le
//! Chromium est plus ancien. P-256 est supporté partout depuis dix ans, pour
//! une sécurité équivalente à cet usage.
//!
//! # Ce que ça résout, et ce que ça ne résout pas
//!
//! Résolu : seul le détenteur de la clé privée peut faire exécuter une
//! commande. Un joueur peut toujours ÉMETTRE, mais le compagnon refuse.
//!
//! PAS résolu : les réponses restent diffusées à tout le monde — Foundry
//! n'offre aucun ciblage sur un canal `module.*`. Voir SECURITY.md.

use anyhow::{Context, Result, bail};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use sha2::{Digest, Sha256};

/// Validité d'une commande signée.
///
/// Assez court pour qu'un rejeu capturé soit sans intérêt, assez long pour
/// absorber une horloge de navigateur mal réglée — un TTL de quelques
/// secondes produirait des refus incompréhensibles chez l'utilisateur.
pub const COMMAND_TTL_MS: u64 = 120_000;

/// Version du format signé. Préfixe la charge : si le format évolue, une
/// signature v1 ne pourra pas être acceptée pour une charge v2.
const FORMAT: &str = "fmcg-v1";

#[derive(Clone)]
pub struct CommandSigner {
    key: SigningKey,
    /// Clé publique au format SEC1 non compressé (65 octets, préfixe 0x04),
    /// en hexadécimal — c'est exactement ce que `crypto.subtle.importKey`
    /// attend en format "raw" pour ECDSA P-256.
    public_hex: String,
}

impl CommandSigner {
    /// Depuis `COMPANION_SIGNING_KEY` (32 octets en hex), ou une clé éphémère.
    ///
    /// Éphémère = régénérée à chaque démarrage, donc la clé publique inscrite
    /// dans le monde ne correspond plus après un redémarrage. C'est utilisable
    /// pour essayer, pas pour durer : l'appelant doit le dire à l'utilisateur.
    pub fn from_env() -> Result<(Self, bool)> {
        match std::env::var("COMPANION_SIGNING_KEY") {
            Ok(hex_key) if !hex_key.trim().is_empty() => {
                Ok((Self::from_hex(hex_key.trim())?, true))
            }
            _ => Ok((Self::generate(), false)),
        }
    }

    pub fn from_hex(hex_key: &str) -> Result<Self> {
        let bytes = hex::decode(hex_key)
            .context("COMPANION_SIGNING_KEY must be hex (generate one with --generate-key)")?;
        if bytes.len() != 32 {
            bail!(
                "COMPANION_SIGNING_KEY must be 32 bytes (64 hex chars), got {}",
                bytes.len()
            );
        }
        let key = SigningKey::from_slice(&bytes)
            .context("COMPANION_SIGNING_KEY is not a valid P-256 private key")?;
        Ok(Self::from_key(key))
    }

    pub fn generate() -> Self {
        // On tire les octets avec le `rand` du projet plutôt qu'avec
        // `SigningKey::random` : p256 dépend d'une autre version de rand_core,
        // et aligner les deux n'apporterait rien ici. Une clé P-256 invalide
        // est astronomiquement improbable, mais on retire jusqu'à en obtenir
        // une valide au lieu de paniquer.
        loop {
            let mut bytes = [0u8; 32];
            rand::RngExt::fill(&mut rand::rng(), &mut bytes);
            if let Ok(key) = SigningKey::from_slice(&bytes) {
                return Self::from_key(key);
            }
        }
    }

    fn from_key(key: SigningKey) -> Self {
        let point = key.verifying_key().to_sec1_point(false);
        let public_hex = hex::encode(point.as_bytes());
        Self { key, public_hex }
    }

    /// La clé privée en hex — pour `--generate-key` uniquement, JAMAIS journalisée.
    pub fn private_hex(&self) -> String {
        hex::encode(self.key.to_bytes())
    }

    /// La clé publique, à coller dans les réglages du module compagnon.
    pub fn public_hex(&self) -> &str {
        &self.public_hex
    }

    /// Signe une commande et rend `(signature_hex, expiration_ms)`.
    ///
    /// `args_raw` est la sérialisation EXACTE transmise dans l'enveloppe, pas
    /// le `Value` : le message traverse socket.io, qui le décode et le
    /// ré-encode côté serveur Foundry. `serde_json` trie les clés,
    /// `JSON.stringify` conserve l'ordre d'insertion, et les nombres ne se
    /// formatent pas pareil — hacher du JSON re-sérialisé donnerait deux
    /// empreintes différentes de part et d'autre. On signe la chaîne, et le
    /// compagnon exécute ce qu'il a vérifié plutôt qu'une reconstruction.
    pub fn sign(
        &self,
        cmd: &str,
        id: &str,
        world: &str,
        args_raw: &str,
        now_ms: u64,
    ) -> (String, u64) {
        let expiry = now_ms + COMMAND_TTL_MS;
        let payload = payload(cmd, id, world, args_raw, expiry);
        let signature: Signature = self.key.sign(payload.as_bytes());
        (hex::encode(signature.to_bytes()), expiry)
    }
}

/// La charge signée, sous une forme que le navigateur reconstruit à
/// l'identique. Les champs sont séparés par `\n`, qui ne peut apparaître ni
/// dans un nom de commande, ni dans un UUID, ni dans un id de monde — donc
/// aucun risque de confusion entre deux découpages différents.
fn payload(cmd: &str, id: &str, world: &str, args_raw: &str, expiry_ms: u64) -> String {
    let args_hash = hex::encode(Sha256::digest(args_raw.as_bytes()));
    format!("{FORMAT}\n{cmd}\n{id}\n{world}\n{expiry_ms}\n{args_hash}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{VerifyingKey, signature::Verifier};

    fn verify(public_hex: &str, sig_hex: &str, payload: &str) -> bool {
        let key = VerifyingKey::from_sec1_bytes(&hex::decode(public_hex).unwrap()).unwrap();
        let sig = Signature::from_slice(&hex::decode(sig_hex).unwrap()).unwrap();
        key.verify(payload.as_bytes(), &sig).is_ok()
    }

    #[test]
    fn signature_verifiable() {
        let signer = CommandSigner::generate();
        let args = r#"{"macro":"Open Calendar"}"#;
        let (sig, exp) = signer.sign("run_macro", "id-1", "star-wars", args, 1_000);
        assert_eq!(exp, 1_000 + COMMAND_TTL_MS);
        assert!(verify(
            signer.public_hex(),
            &sig,
            &payload("run_macro", "id-1", "star-wars", args, exp)
        ));
    }

    #[test]
    fn chaque_champ_est_lie() {
        // Si un seul champ n'était pas couvert, un joueur pourrait recopier
        // une enveloppe légitime en changeant ce champ-là.
        let signer = CommandSigner::generate();
        let args = r#"{"macro":"Safe"}"#;
        let (sig, exp) = signer.sign("run_macro", "id-1", "star-wars", args, 1_000);

        let altere = [
            payload("run_script", "id-1", "star-wars", args, exp), // commande
            payload("run_macro", "id-2", "star-wars", args, exp),  // nonce
            payload("run_macro", "id-1", "autre-monde", args, exp), // monde
            payload("run_macro", "id-1", "star-wars", r#"{"macro":"Evil"}"#, exp), // args
            payload("run_macro", "id-1", "star-wars", args, exp + 1), // expiration
        ];
        for p in altere {
            assert!(
                !verify(signer.public_hex(), &sig, &p),
                "champ non lié : {p}"
            );
        }
    }

    #[test]
    fn cle_publique_au_format_attendu_par_webcrypto() {
        // `crypto.subtle.importKey("raw", …, {name:"ECDSA", namedCurve:"P-256"})`
        // exige un point SEC1 NON COMPRESSÉ : 65 octets, préfixe 0x04.
        let signer = CommandSigner::generate();
        let raw = hex::decode(signer.public_hex()).unwrap();
        assert_eq!(raw.len(), 65);
        assert_eq!(raw[0], 0x04);
        // Et la signature doit être en P1363 (r||s), pas en DER : c'est le
        // seul format que crypto.subtle.verify accepte.
        let (sig, _) = signer.sign("ping_module", "i", "w", "{}", 0);
        assert_eq!(hex::decode(sig).unwrap().len(), 64);
    }

    #[test]
    fn cle_rechargeable_depuis_son_hex() {
        let signer = CommandSigner::generate();
        let rechargee = CommandSigner::from_hex(&signer.private_hex()).unwrap();
        assert_eq!(signer.public_hex(), rechargee.public_hex());

        assert!(CommandSigner::from_hex("pas-de-l-hex").is_err());
        assert!(CommandSigner::from_hex("aabb").is_err()); // trop court
    }
}

#[cfg(test)]
mod vecteur {
    use super::*;
    /// Imprime un vecteur d'interopérabilité pour le vérificateur navigateur.
    /// `cargo test imprime_vecteur -- --nocapture --ignored`
    #[test]
    #[ignore]
    fn imprime_vecteur() {
        let signer = CommandSigner::from_hex(
            "4c0883a69102937d6231471b5dbb6204fe512961708279e0f2c5b1b1c1a1d1e1",
        )
        .unwrap();
        let args = r#"{"macro":"Open Calendar"}"#;
        let (sig, exp) = signer.sign("run_macro", "id-1", "star-wars", args, 1_000_000);
        println!("PUB={}", signer.public_hex());
        println!("SIG={sig}");
        println!(
            "PAYLOAD={}",
            payload("run_macro", "id-1", "star-wars", args, exp)
        );
    }
}
