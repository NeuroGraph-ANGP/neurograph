//! v3.0 — Sharding: account-based partitioning pentru 1M+ TPS pe sistem.
//!
//! Principii (fără a schimba protocolul ANGP):
//!   - shard_id(addr) = SHA-512/256(addr) % N_SHARDS — determinist
//!   - Fiecare nod procesează un subset de shards (--shards 0,1,2)
//!   - Tx intra-shard (sender și receiver în același shard): procesat normal
//!   - Tx cross-shard: split în LockTx + CommitTx
//!   - Reputația rămâne GLOBALĂ — un nod e evaluat pe toate propunerile sale
//!   - AdaptiveDag Hebbian rămâne pe shard-ul procesat
//!
//! IMPORTANT: Acest modul NU schimbă protocolul. Doar decide CE txs
//! procesează fiecare nod. DagProposal, mediană ponderată, semnături — toate
//! rămân la fel.

use sha2::{Sha512_256, Digest};
use crate::transaction::Transaction;
use crate::config::N_SHARDS;

/// Calculează shard-ul unei adrese.
/// shard_id = SHA-512/256(addr)[0..4] as u32 % N_SHARDS
///
/// Determinist: aceeași adresă → același shard, oriunde, oricând.
/// Distribuție uniformă: SHA-512/256 e cryptographic hash, deci
/// adrese aleatoare se distribuie uniform pe shards.
pub fn shard_of_address(addr: &str) -> u32 {
    let mut hasher = Sha512_256::new();
    hasher.update(addr.as_bytes());
    let result = hasher.finalize();
    let first_u32 = u32::from_le_bytes([
        result[0], result[1], result[2], result[3],
    ]);
    first_u32 % N_SHARDS
}

/// Verifică dacă o tranzacție e intra-shard (sender și receiver în același shard).
pub fn is_intra_shard(tx: &Transaction) -> bool {
    shard_of_address(&tx.sender) == shard_of_address(&tx.receiver)
}

/// Returnează (from_shard, to_shard) pentru o tranzacție.
pub fn shards_of(tx: &Transaction) -> (u32, u32) {
    (shard_of_address(&tx.sender), shard_of_address(&tx.receiver))
}

/// Configurația de shards procesate de un nod.
#[derive(Debug, Clone)]
pub struct ShardSet {
    /// None = procesează TOATE shards (legacy mode, ca v2.5)
    /// Some(set) = procesează doar shards din set
    shards: Option<std::collections::HashSet<u32>>,
}

impl ShardSet {
    /// Procesează toate shards (legacy mode, fără sharding activ).
    pub fn all() -> Self {
        ShardSet { shards: None }
    }

    /// Procesează doar shards specificate.
    pub fn only(shard_ids: &[u32]) -> Self {
        let set: std::collections::HashSet<u32> = shard_ids.iter().copied().collect();
        ShardSet { shards: Some(set) }
    }

    /// True dacă acest nod procesează shard-ul dat.
    pub fn contains(&self, shard_id: u32) -> bool {
        match &self.shards {
            None => true,  // legacy mode
            Some(set) => set.contains(&shard_id),
        }
    }

    /// True dacă acest nod ar procesa tranzacția (sender e într-un shard al nostru).
    /// Pentru cross-shard, nodul procesează doar LockTx (din shard-ul sender).
    pub fn processes_tx(&self, tx: &Transaction) -> bool {
        let from_shard = shard_of_address(&tx.sender);
        self.contains(from_shard)
    }

    /// True dacă sharding e activ (nodul procesează doar un subset).
    pub fn is_sharded(&self) -> bool {
        self.shards.is_some()
    }

    /// Lista de shards (sau None pentru toate).
    pub fn list(&self) -> Option<Vec<u32>> {
        self.shards.as_ref().map(|s| {
            let mut v: Vec<u32> = s.iter().copied().collect();
            v.sort();
            v
        })
    }
}

impl Default for ShardSet {
    fn default() -> Self {
        Self::all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shard_assignment_deterministic() {
        let s1 = shard_of_address("alice");
        let s2 = shard_of_address("alice");
        assert_eq!(s1, s2, "Same address must always map to same shard");
        assert!(s1 < N_SHARDS);
    }

    #[test]
    fn test_different_addresses_different_shards_likely() {
        // Pentru 16 shards, adrese distincte ar trebui să se distribuie
        // pe cel puțin 8 shards diferite (cu 20 adrese).
        let mut shards = std::collections::HashSet::new();
        for i in 0..20 {
            shards.insert(shard_of_address(&format!("addr_{}", i)));
        }
        assert!(shards.len() >= 8,
            "20 addresses should map to at least 8 different shards, got {}", shards.len());
    }

    #[test]
    fn test_intra_shard_detection() {
        // v3.5.7: Crescut limita la 10000 pentru N_SHARDS=961
        let target_shard = shard_of_address("alice");
        let mut bob_shard_addr = String::new();
        for i in 0..10000 {
            let addr = format!("bob_{}", i);
            if shard_of_address(&addr) == target_shard {
                bob_shard_addr = addr;
                break;
            }
        }
        assert!(!bob_shard_addr.is_empty(), "Should find an address in the same shard as alice");

        // Intra-shard tx
        let tx = Transaction::new_with_nonce(
            "alice".to_string(), bob_shard_addr, 100, 1, vec![],
        );
        assert!(is_intra_shard(&tx), "Same-shard tx must be detected as intra-shard");
    }

    #[test]
    fn test_shard_set_legacy_mode() {
        let s = ShardSet::all();
        assert!(!s.is_sharded());
        assert!(s.contains(0));
        assert!(s.contains(15));
        assert!(s.contains(100));  // > N_SHARDS, dar legacy acceptă
    }

    #[test]
    fn test_shard_set_subset_mode() {
        let s = ShardSet::only(&[0, 5, 10]);
        assert!(s.is_sharded());
        assert!(s.contains(0));
        assert!(s.contains(5));
        assert!(s.contains(10));
        assert!(!s.contains(1));
        assert!(!s.contains(15));
    }

    #[test]
    fn test_processes_tx() {
        // v3.5.7: Crescut limita de la 1000 la 10000 pentru N_SHARDS=961
        // (probabilitatea de a gasi o adresa in shard 0 = 1/961 per incercare,
        //  deci cu 10000 incercari avem ~99.99% sanse sa gasim una).
        let mut alice_shard0 = String::new();
        for i in 0..10000 {
            let addr = format!("alice_{}", i);
            if shard_of_address(&addr) == 0 {
                alice_shard0 = addr;
                break;
            }
        }
        assert!(!alice_shard0.is_empty(), "Should find an address in shard 0 within 10000 tries");
        let s = ShardSet::only(&[0]);
        let tx = Transaction::new_with_nonce(
            alice_shard0, "bob".to_string(), 100, 1, vec![],
        );
        assert!(s.processes_tx(&tx), "Shard 0 node should process tx from shard 0 sender");

        let s_other = ShardSet::only(&[1]);
        assert!(!s_other.processes_tx(&tx), "Shard 1 node should NOT process tx from shard 0 sender");
    }
}
