use serde::{Serialize, Deserialize};
use sha2::{Sha512_256, Digest};
use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Verifier, Signature};
use rayon::prelude::*;
use std::fmt;

use crate::config::DEFAULT_TX_FEE_MILLI;
use crate::cross_shard::TxKind;

pub type Hash = [u8; 32];
/// v3.1: Cheia publică e 32 bytes (ed25519-dalek format, nu ring format).
/// Compatibil backward cu v3.0 (ring folosea aceeași lungime 32 bytes).
pub const ED25519_PUBLIC_KEY_LEN: usize = 32;
pub const ED25519_SIGNATURE_LEN: usize = 64;
pub type PublicKey = [u8; ED25519_PUBLIC_KEY_LEN];

/// Tranzacție NeuroGraph cu fee (Etapa 1) + sharding (v3.0) + ed25519-dalek (v3.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub sender: String,
    pub receiver: String,
    pub amount: u64,
    pub fee: u64,
    pub nonce: u64,
    pub timestamp: u64,
    pub parents: Vec<Hash>,
    pub signature: Option<Vec<u8>>,
    #[serde(default)]
    pub public_key: Option<PublicKey>,
    pub hash: Hash,
    #[serde(default)]
    pub kind: TxKind,
}

impl Transaction {
    pub fn new_with_nonce(
        sender: String, receiver: String, amount: u64, nonce: u64, parents: Vec<Hash>,
    ) -> Self {
        Self::new_with_fee(sender, receiver, amount, nonce, DEFAULT_TX_FEE_MILLI, parents)
    }

    pub fn new_with_fee(
        sender: String, receiver: String, amount: u64, nonce: u64, fee: u64, parents: Vec<Hash>,
    ) -> Self {
        Self::new_with_kind(sender, receiver, amount, nonce, fee, parents, TxKind::Normal)
    }

    pub fn new_with_kind(
        sender: String, receiver: String, amount: u64, nonce: u64, fee: u64,
        parents: Vec<Hash>, kind: TxKind,
    ) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut tx = Transaction {
            sender, receiver, amount, fee, nonce, timestamp, parents,
            signature: None, public_key: None,
            hash: [0u8; 32], kind,
        };
        tx.hash = tx.compute_hash();
        tx
    }

    /// Backwards-compat: nonce random + fee default.
    pub fn new(sender: String, receiver: String, amount: u64, parents: Vec<Hash>) -> Self {
        let nonce = rand::random::<u64>();
        Self::new_with_fee(sender, receiver, amount, nonce, DEFAULT_TX_FEE_MILLI, parents)
    }

    /// Hash canonic: SHA-512/256 (v2.5).
    pub fn compute_hash(&self) -> Hash {
        let data = format!(
            "{}|{}|{}|{}|{}|{}|{:?}|{:?}",
            self.sender, self.receiver, self.amount, self.fee,
            self.nonce, self.timestamp, self.parents, self.kind
        );
        let mut hasher = Sha512_256::new();
        hasher.update(data.as_bytes());
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        hash
    }

    /// v3.1: Semnează cu ed25519-dalek.
    /// Semnătura e interschimbabilă cu ring (același algoritm Ed25519 standard).
    pub fn sign(&mut self, signing_key: &SigningKey) {
        let msg = self.compute_hash();
        let sig = signing_key.sign(&msg);
        self.signature = Some(sig.to_bytes().to_vec());
        self.public_key = Some(signing_key.verifying_key().to_bytes());
    }

    /// v3.1: Verificare individuală cu ed25519-dalek.
    pub fn verify_signature(&self) -> bool {
        let (Some(sig_bytes), Some(pub_key_bytes)) = (&self.signature, self.public_key) else {
            return false;
        };
        if sig_bytes.len() != ED25519_SIGNATURE_LEN {
            return false;
        }
        let Ok(verifying_key) = VerifyingKey::from_bytes(&pub_key_bytes) else {
            return false;
        };
        let mut sig_array = [0u8; ED25519_SIGNATURE_LEN];
        sig_array.copy_from_slice(sig_bytes);
        let signature = Signature::from_bytes(&sig_array);
        let msg = self.compute_hash();
        verifying_key.verify(&msg, &signature).is_ok()
    }

    /// v3.1: BATCH VERIFICATION — 5-10× mai rapid decât verify individual.
    ///
    /// Verifică N semnături simultan folosind algebră Scheidt (suma ponderată
    /// a tuturor verificărilor individuale). Implementare: ed25519-dalek::verify_batch.
    ///
    /// Returnează: Vec<bool> — care txs sunt valide (true = semnătură corectă).
    ///
    /// IMPORTANT: dacă o tx e invalidă, batch verify poate da rezultate imprecise
    /// pentru txs specifice (matematica batch nu izolează eșecul). De aceea,
    /// dacă batch-ul eșuează, trebuie re-verificat individual.
    pub fn verify_batch(txs: &[&Transaction]) -> Vec<bool> {
        if txs.is_empty() { return Vec::new(); }

        // Colectăm (msg, sig, pubkey) pentru fiecare tx care are semnătură
        let mut messages: Vec<Vec<u8>> = Vec::with_capacity(txs.len());
        let mut signatures: Vec<Signature> = Vec::with_capacity(txs.len());
        let mut pubkeys: Vec<VerifyingKey> = Vec::with_capacity(txs.len());
        let mut indices_with_sig: Vec<usize> = Vec::with_capacity(txs.len());

        for (i, tx) in txs.iter().enumerate() {
            let (Some(sig_bytes), Some(pub_key_bytes)) = (&tx.signature, tx.public_key) else {
                continue;
            };
            if sig_bytes.len() != ED25519_SIGNATURE_LEN { continue; }
            let Ok(vk) = VerifyingKey::from_bytes(&pub_key_bytes) else { continue; };
            let mut sig_arr = [0u8; ED25519_SIGNATURE_LEN];
            sig_arr.copy_from_slice(sig_bytes);
            messages.push(tx.compute_hash().to_vec());
            signatures.push(Signature::from_bytes(&sig_arr));
            pubkeys.push(vk);
            indices_with_sig.push(i);
        }

        if indices_with_sig.is_empty() {
            return vec![false; txs.len()];
        }

        // Pregătim slice-uri pentru verify_batch
        let msg_refs: Vec<&[u8]> = messages.iter().map(|m| m.as_slice()).collect();

        // v3.5.10: BATCH VERIFY NATIV + RAYON PARALLEL CHUNKS
        //
        // STRATEGIE:
        //   1. Împărțim batch-ul în chunks de BATCH_CHUNK_SIZE (256)
        //   2. Paralelizăm chunk-urile cu rayon (par_chunks)
        //   3. În interiorul fiecărui chunk: batch verify nativ (multiscalar mul)
        //      - Dacă trece (Ok): toate semnăturile din chunk sunt valide
        //      - Dacă eșuează (Err): verificăm individual pentru izolare
        //
        // Aceasta combină paralelizarea rayon (4× pe 4 cores) cu algebra
        // batch nativ (2-3× per chunk) = 8-12× speedup total.
        //
        // Benchmark estimat (4 cores, 50K sigs):
        //   - v3.5.2 rayon par_chunks (individual):  96K sigs/sec
        //   - v3.5.10 rayon + batch nativ:          ~300-500K sigs/sec

        const BATCH_CHUNK_SIZE: usize = 1000;

        // Pregătim perechi (orig_index_in_txs, batch_index_in_collected)
        let pairs: Vec<(usize, usize)> = indices_with_sig
            .iter()
            .enumerate()
            .map(|(batch_i, &orig_i)| (orig_i, batch_i))
            .collect();

        let per_tx_results: Vec<(usize, bool)> = pairs
            .par_chunks(BATCH_CHUNK_SIZE)
            .flat_map(|chunk| {
                // Construim slice-uri pentru batch verify nativ pe acest chunk
                let chunk_msgs: Vec<&[u8]> = chunk.iter()
                    .map(|&(_, batch_i)| msg_refs[batch_i])
                    .collect();
                let chunk_sigs: Vec<ed25519_dalek::Signature> = chunk.iter()
                    .map(|&(_, batch_i)| signatures[batch_i].clone())
                    .collect();
                let chunk_keys: Vec<ed25519_dalek::VerifyingKey> = chunk.iter()
                    .map(|&(_, batch_i)| pubkeys[batch_i].clone())
                    .collect();

                // Încercăm batch verify nativ pe acest chunk
                let batch_ok = ed25519_dalek::batch::verify_batch(
                    &chunk_msgs,
                    &chunk_sigs,
                    &chunk_keys,
                );

                if batch_ok.is_ok() {
                    // Toate semnăturile din chunk sunt valide
                    chunk.iter().map(|&(orig_i, _)| (orig_i, true)).collect::<Vec<_>>()
                } else {
                    // Batch eșuat — verificăm individual pentru izolare
                    chunk.iter().map(|&(orig_i, batch_i)| {
                        let ok = pubkeys[batch_i]
                            .verify(msg_refs[batch_i], &signatures[batch_i])
                            .is_ok();
                        (orig_i, ok)
                    }).collect::<Vec<_>>()
                }
            })
            .collect();

        let mut results = vec![false; txs.len()];
        for (i, ok) in per_tx_results {
            results[i] = ok;
        }
        results
    }

    pub fn public_key_bytes(&self) -> Option<PublicKey> {
        self.public_key
    }

    pub fn total_cost(&self) -> u64 {
        self.amount.saturating_add(self.fee)
    }
}

impl fmt::Display for Transaction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Tx({} -> {}: {} milliANGP (fee={}), nonce={}, hash={:?})",
            self.sender, self.receiver, self.amount, self.fee,
            self.nonce, &self.hash[..4]
        )
    }
}

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes { s.push_str(&format!("{:02x}", b)); }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[test]
    fn test_sign_and_verify_individual() {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let mut tx = Transaction::new_with_nonce(
            "alice".to_string(), "bob".to_string(), 100, 1, vec![],
        );
        tx.sign(&signing_key);
        assert!(tx.verify_signature(), "Just-signed tx must verify");
    }

    #[test]
    fn test_verify_rejects_tampered_tx() {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let mut tx = Transaction::new_with_nonce(
            "alice".to_string(), "bob".to_string(), 100, 1, vec![],
        );
        tx.sign(&signing_key);
        // Tampering: change amount after signing
        tx.amount = 999;
        assert!(!tx.verify_signature(), "Tampered tx must fail verification");
    }

    #[test]
    fn test_verify_batch_all_valid() {
        let mut rng = OsRng;
        let mut txs: Vec<Transaction> = Vec::new();
        for i in 0..10 {
            let sk = SigningKey::generate(&mut rng);
            let mut tx = Transaction::new_with_nonce(
                format!("alice_{}", i), "bob".to_string(), 100, i + 1, vec![],
            );
            tx.sign(&sk);
            txs.push(tx);
        }
        let refs: Vec<&Transaction> = txs.iter().collect();
        let results = Transaction::verify_batch(&refs);
        assert_eq!(results.len(), 10);
        assert!(results.iter().all(|&r| r), "All txs should be valid");
    }

    #[test]
    fn test_verify_batch_with_one_invalid() {
        let mut rng = OsRng;
        let mut txs: Vec<Transaction> = Vec::new();
        for i in 0..5 {
            let sk = SigningKey::generate(&mut rng);
            let mut tx = Transaction::new_with_nonce(
                format!("alice_{}", i), "bob".to_string(), 100, i + 1, vec![],
            );
            tx.sign(&sk);
            txs.push(tx);
        }
        // Tampering pe tx 2
        txs[2].amount = 999;
        let refs: Vec<&Transaction> = txs.iter().collect();
        let results = Transaction::verify_batch(&refs);
        assert_eq!(results.len(), 5);
        // Tx 2 ar trebui să fie invalid
        assert!(!results[2], "Tampered tx should fail");
        // Celelalte ar trebui să fie valide
        for i in [0, 1, 3, 4] {
            assert!(results[i], "Tx {} should be valid", i);
        }
    }

    #[test]
    fn test_compatibility_ring_and_dalek_interswappable() {
        // Verificăm că o cheie generată cu ring poate semna txs verificabile cu ed25519-dalek
        // (prin seed comun)
        use ring::signature::{Ed25519KeyPair as RingKeyPair, KeyPair as _};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = RingKeyPair::generate_pkcs8(&rng).unwrap();
        let ring_kp = RingKeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();

        // Extragem seed-ul (ultimii 32 bytes din PKCS8)
        let seed = &pkcs8.as_ref()[pkcs8.as_ref().len() - 32..];
        let mut seed_arr = [0u8; 32];
        seed_arr.copy_from_slice(seed);
        let dalek_sk = SigningKey::from_bytes(&seed_arr);

        // Semnăm cu ring
        let mut tx1 = Transaction::new_with_nonce(
            "alice".to_string(), "bob".to_string(), 100, 1, vec![],
        );
        let msg = tx1.compute_hash();
        let ring_sig = ring_kp.sign(&msg);
        tx1.signature = Some(ring_sig.as_ref().to_vec());
        tx1.public_key = Some(ring_kp.public_key().as_ref().try_into().unwrap());

        // Verificăm cu ed25519-dalek
        assert!(tx1.verify_signature(),
            "Tx signed with ring must verify with ed25519-dalek (same algorithm)");

        // Semnăm cu ed25519-dalek
        let mut tx2 = Transaction::new_with_nonce(
            "alice".to_string(), "bob".to_string(), 100, 2, vec![],
        );
        tx2.sign(&dalek_sk);

        // Verificăm că și tx2 trece
        assert!(tx2.verify_signature());
    }
}
