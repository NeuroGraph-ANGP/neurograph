//! Wallet — identitate persistentă Ed25519 (v3.1: ed25519-dalek).
//!
//! v3.1 schimbă implementarea internă de la `ring` la `ed25519-dalek` pentru
//! a beneficia de `verify_batch()` (5-10× speedup pe sig verify).
//!
//! **Compatibilitate backward**:
//!   - Fișierele `identity.pem` (PKCS8 v2, 48 bytes) din v3.0 sunt încărcate
//!     prin extragerea seed-ului Ed25519 (ultimii 32 bytes).
//!   - Semnăturile Ed25519 produse de `ring` și `ed25519-dalek` sunt
//!     interschimbabile (același algoritm standard RFC 8032).
//!
//! **Securitate**:
//!   - Algoritmul Ed25519 rămâne același — doar implementarea se schimbă
//!   - Cheile rămân pe disk în format PKCS8 (compatibilitate)
//!   - La runtime folosim `ed25519_dalek::SigningKey` pentru semnare

use std::fs;
use std::path::Path;
use ring::signature::Ed25519KeyPair as RingKeyPair;
use ring::rand::SystemRandom;
use ring::error::Unspecified;
use ed25519_dalek::{SigningKey, VerifyingKey};

pub struct Wallet {
    /// v3.1: Cheia de semnare ed25519-dalek (înlocuiește ring::Ed25519KeyPair).
    pub signing_key: SigningKey,
    /// Cheia publică derivată (pentru verificare).
    pub verifying_key: VerifyingKey,
    /// PKCS8 bytes (păstrat pe disk, pentru backward compat cu v3.0).
    pub pkcs8_bytes: Vec<u8>,
    /// Seed-ul brut Ed25519 (32 bytes) — pentru libp2p::Keypair.
    pub seed_bytes: Vec<u8>,
    /// Cheia publică ca hex string.
    pub public_key_hex: String,
}

/// Extrage seed-ul Ed25519 (32 bytes) dintr-un document PKCS8 v2 (48 bytes).
/// ring::Ed25519KeyPair::generate_pkcs8 produce 48 bytes (PKCS8 v1).
/// Seed-ul e la final (ultimii 32 bytes).
fn extract_seed_from_pkcs8(pkcs8: &[u8]) -> Option<Vec<u8>> {
    if pkcs8.len() < 48 { return None; }
    let seed = &pkcs8[pkcs8.len() - 32..];
    if seed.len() == 32 {
        Some(seed.to_vec())
    } else {
        None
    }
}

impl Wallet {
    /// Încarcă sau generează identitatea pentru un nod.
    pub fn load_or_create(data_dir: &str, node_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let identity_path = format!("{}/identity.pem", data_dir);
        fs::create_dir_all(data_dir)?;

        let (pkcs8_bytes, seed_bytes) = if Path::new(&identity_path).exists() {
            // Încărcăm PKCS8 existent și extragem seed-ul
            let pkcs8 = fs::read(&identity_path)?;
            let seed = extract_seed_from_pkcs8(&pkcs8)
                .ok_or("Failed to extract Ed25519 seed from PKCS8")?;
            println!("[wallet] Loaded existing identity for {} from {}", node_name, identity_path);
            (pkcs8, seed)
        } else {
            // Generăm cheie nouă cu ring (pentru PKCS8) și extragem seed-ul
            // Apoi construim ed25519-dalek SigningKey din seed
            let rng = SystemRandom::new();
            let pkcs8_ring = RingKeyPair::generate_pkcs8(&rng)
                .map_err(|_: Unspecified| "Failed to generate key pair".to_string())?;
            let pkcs8_vec = pkcs8_ring.as_ref().to_vec();
            fs::write(&identity_path, &pkcs8_vec)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&identity_path)?.permissions();
                perms.set_mode(0o600);
                let _ = fs::set_permissions(&identity_path, perms);
            }
            let seed = extract_seed_from_pkcs8(&pkcs8_vec)
                .ok_or("Failed to extract seed from just-generated PKCS8")?;
            println!("[wallet] Generated new identity for {} → saved to {}", node_name, identity_path);
            (pkcs8_vec, seed)
        };

        // v3.1: Construim SigningKey din seed (32 bytes)
        let mut seed_array = [0u8; 32];
        seed_array.copy_from_slice(&seed_bytes);
        let signing_key = SigningKey::from_bytes(&seed_array);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = crate::transaction::hex_encode(verifying_key.as_bytes());

        Ok(Wallet {
            signing_key,
            verifying_key,
            pkcs8_bytes,
            seed_bytes,
            public_key_hex,
        })
    }

    /// v3.1: Returnează o copie a cheii de semnare ed25519-dalek.
    /// (Ed25519 seed e determinist din pkcs8, deci e infailibil.)
    pub fn signing_key_clone(&self) -> SigningKey {
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&self.seed_bytes);
        SigningKey::from_bytes(&seed)
    }

    /// Returnează primele 8 hex chars ale cheii publice — util pentru afișare scurtă.
    pub fn short_id(&self) -> String {
        self.public_key_hex[..8].to_string()
    }
}
