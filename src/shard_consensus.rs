//! v3.5 — Hybrid Consensus: shard-level + global-level, AMBELE emergente.
//!
//! Principii ANGP respectate 100%:
//!   - compute_consensus() rămâne INTACT — nici o modificare
//!   - Shard consensus = compute_consensus() pe subset de propuneri din shard
//!   - Global consensus = compute_consensus() pe shard_digests (ca predicții)
//!   - Cross-shard receipts trec prin consens emergent la destinație
//!   - FĂRĂ BFT voting, FĂRĂ staking, FĂRĂ coordinator central
//!
//! Arhitectură:
//!   ┌─────────────────────────────────────────────────┐
//!   │ Layer 1: SHARD CONSENSUS (fast, local)          │
//!   │  Shard 0: compute_consensus(props_shard_0)     │
//!   │  Shard 1: compute_consensus(props_shard_1)     │
//!   │  ...                                            │
//!   │  Shard N: compute_consensus(props_shard_N)     │
//!   │  → produce ShardDigest per shard                │
//!   └────────────────────┬────────────────────────────┘
//!                        │
//!   ┌────────────────────▼────────────────────────────┐
//!   │ Layer 2: GLOBAL ANCHOR (security, cross-shard)  │
//!   │  compute_consensus(shard_digests as proposals)  │
//!   │  → detectează shard-uri compromise               │
//!   │  → validează cross-shard receipts                │
//!   └─────────────────────────────────────────────────┘

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use sha2::{Sha512_256, Digest};
use crate::dag_logic::{DagProposal, DagConsensus, DagLogic};
use crate::sharding::{ShardSet, shard_of_address};
use crate::transaction::Hash;
use crate::utils::{vec_to_array, euclidean};

/// ShardDigest = rezumatul stării unui shard după consens local.
/// Incluzând: median_prediction, common_tips, state_root, approved_txs count.
///
/// Acest digest e folosit ca "predicție" în consensul global —
/// shards cu digest-uri similare sunt considerate "de acord",
/// shards cu digest-uri diferite sunt potențial compromise.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardDigest {
    pub shard_id: u32,
    pub median_prediction: Vec<f64>,
    pub state_root: Hash,
    pub approved_tx_count: usize,
    pub proposer_count: usize,
    pub step: u64,
    /// Hash canonic al digest-ului (pentru comparație în consens global).
    pub digest_hash: Hash,
}

impl ShardDigest {
    pub fn from_consensus(shard_id: u32, consensus: &DagConsensus, step: u64) -> Self {
        let mut hasher = Sha512_256::new();
        for v in &consensus.median_prediction {
            hasher.update(v.to_le_bytes());
        }
        hasher.update(consensus.common_state_root);
        hasher.update((consensus.approved_tx_hashes.len() as u64).to_le_bytes());
        let result = hasher.finalize();
        let mut digest_hash = [0u8; 32];
        digest_hash.copy_from_slice(&result);

        ShardDigest {
            shard_id,
            median_prediction: consensus.median_prediction.iter().copied().collect(),
            state_root: consensus.common_state_root,
            approved_tx_count: consensus.approved_tx_hashes.len(),
            proposer_count: 0, // setat de caller
            step,
            digest_hash,
        }
    }

    /// Convertește digest-ul într-o DagProposal "fake" pentru consensul global.
    /// median_prediction devine predicția, digest_hash devine state_root.
    pub fn to_proposal(&self) -> DagProposal {
        DagProposal {
            sender: format!("shard_{}", self.shard_id),
            step: self.step,
            seq: 0,
            nonce: 0,
            prediction: self.median_prediction.clone(),
            proposed_tips: vec![],
            state_root: self.digest_hash,
            seen_tx_hashes: vec![],
            seen_receipts: vec![],
        }
    }
}

/// Rezultatul consensului hybrid — conține atât consensul per-shard cât și global.
#[derive(Debug, Clone)]
pub struct HybridConsensusResult {
    /// Consens per shard (shard_id → DagConsensus)
    pub shard_consensus: HashMap<u32, DagConsensus>,
    /// Shard digests (shard_id → ShardDigest)
    pub shard_digests: HashMap<u32, ShardDigest>,
    /// Consens global pe shard_digests
    pub global_consensus: DagConsensus,
    /// Erori per nod (combinate din toate shard-urile + global)
    pub errors: HashMap<String, f64>,
}

/// Manager pentru Hybrid Consensus.
///
/// Nu modifică compute_consensus() — doar îl cheamă pe subseturi.
pub struct ShardConsensusManager;

impl ShardConsensusManager {
    /// Calculează hybrid consensus:
    ///   1. Partiționează propunerile pe shard (după shard_of_address(sender))
    ///   2. Pentru fiecare shard: compute_consensus() pe subset
    ///   3. Construiește ShardDigest per shard
    ///   4. Global: compute_consensus() pe shard_digests ca propuneri
    ///
    /// Dacă sharding nu e activ (legacy mode), cheamă compute_consensus() direct
    /// pe toate propunerile (compatibilitate cu v3.4).
    pub fn compute_hybrid(
        logic: &DagLogic,
        proposals: &[DagProposal],
        rep_map: &HashMap<String, f64>,
        shard_set: &ShardSet,
        step: u64,
    ) -> HybridConsensusResult {
        // Legacy mode: fără sharding → consens global simplu
        if !shard_set.is_sharded() {
            let (consensus, errors) = logic.compute_consensus(proposals, rep_map);
            let digest = ShardDigest::from_consensus(0, &consensus, step);
            let mut shard_consensus = HashMap::new();
            let mut shard_digests = HashMap::new();
            shard_consensus.insert(0, consensus.clone());
            shard_digests.insert(0, digest);
            return HybridConsensusResult {
                shard_consensus,
                shard_digests,
                global_consensus: consensus,
                errors,
            };
        }

        // v3.5: Hybrid mode
        // 1. Partiționează propunerile pe shard
        let mut shard_proposals: HashMap<u32, Vec<DagProposal>> = HashMap::new();
        for p in proposals {
            let shard = shard_of_address(&p.sender);
            shard_proposals.entry(shard).or_default().push(p.clone());
        }

        // 2. Consens per shard
        let mut shard_consensus: HashMap<u32, DagConsensus> = HashMap::new();
        let mut shard_digests: HashMap<u32, ShardDigest> = HashMap::new();
        let mut all_errors: HashMap<String, f64> = HashMap::new();

        for (shard_id, props) in &shard_proposals {
            let (consensus, errors) = logic.compute_consensus(props, rep_map);
            for (k, v) in errors {
                all_errors.insert(k, v);
            }
            let mut digest = ShardDigest::from_consensus(*shard_id, &consensus, step);
            digest.proposer_count = props.len();
            shard_consensus.insert(*shard_id, consensus);
            shard_digests.insert(*shard_id, digest);
        }

        // 3. Global anchor: compute_consensus pe shard_digests ca propuneri
        let global_proposals: Vec<DagProposal> = shard_digests.values()
            .map(|d| d.to_proposal())
            .collect();

        // Reputație per shard = media reputațiilor nodurilor din shard
        let shard_rep_map: HashMap<String, f64> = shard_proposals.iter()
            .map(|(shard_id, props)| {
                let reps: Vec<f64> = props.iter()
                    .map(|p| rep_map.get(&p.sender).copied().unwrap_or(0.5))
                    .collect();
                let avg = if reps.is_empty() { 0.5 } else { reps.iter().sum::<f64>() / reps.len() as f64 };
                (format!("shard_{}", shard_id), avg)
            })
            .collect();

        let (global_consensus, global_errors) = logic.compute_consensus(&global_proposals, &shard_rep_map);

        // 4. Combinație erori: eroarea globală a shard-ului e distribuită înapoi la noduri
        for (shard_name, global_err) in &global_errors {
            // Extragem shard_id din "shard_N"
            if let Some(shard_str) = shard_name.strip_prefix("shard_") {
                if let Ok(shard_id) = shard_str.parse::<u32>() {
                    // Pentru fiecare nod din acest shard, adăugăm o fracție din eroarea globală
                    if let Some(props) = shard_proposals.get(&shard_id) {
                        let n_nodes = props.len().max(1);
                        let per_node_penalty = global_err / n_nodes as f64 * 0.3; // 30% weight
                        for p in props {
                            *all_errors.entry(p.sender.clone()).or_insert(0.0) += per_node_penalty;
                        }
                    }
                }
            }
        }

        HybridConsensusResult {
            shard_consensus,
            shard_digests,
            global_consensus,
            errors: all_errors,
        }
    }

    /// Verifică dacă un shard e "compromis" — digest-ul diferă semnificativ
    /// de consensul global.
    pub fn is_shard_compromised(
        shard_digest: &ShardDigest,
        global_consensus: &DagConsensus,
    ) -> bool {
        let shard_pred = vec_to_array(&shard_digest.median_prediction);
        let global_pred = &global_consensus.median_prediction;
        let dist = euclidean(&shard_pred, global_pred);
        // Dacă shard-ul e la >0.3 L2 de consensul global, e suspect
        dist > 0.3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag_logic::DagLogic;

    #[test]
    fn test_hybrid_legacy_mode() {
        // Legacy mode (fără sharding) trebuie să dea același rezultat ca v3.4
        let tmp = common::TempDir::new("hybrid_legacy");
        let logic = DagLogic::new(100, tmp.path());
        let shard_set = ShardSet::all();

        let proposals = vec![
            DagProposal {
                sender: "alice".to_string(), step: 1, seq: 0, nonce: 0,
                prediction: vec![0.5, 0.5, 0.5, 0.5],
                proposed_tips: vec![], state_root: [0u8; 32],
                seen_tx_hashes: vec![], seen_receipts: vec![],
            },
            DagProposal {
                sender: "bob".to_string(), step: 1, seq: 0, nonce: 0,
                prediction: vec![0.5, 0.5, 0.5, 0.5],
                proposed_tips: vec![], state_root: [0u8; 32],
                seen_tx_hashes: vec![], seen_receipts: vec![],
            },
        ];

        let rep_map = HashMap::new();
        let result = ShardConsensusManager::compute_hybrid(
            &logic, &proposals, &rep_map, &shard_set, 1,
        );

        // Legacy mode: 1 shard (shard 0), conține toate propunerile
        assert_eq!(result.shard_consensus.len(), 1);
        assert!(result.shard_consensus.contains_key(&0));
    }

    mod common {
        use std::path::PathBuf;
        use std::fs;
        pub struct TempDir { path: PathBuf }
        impl TempDir {
            pub fn new(name: &str) -> Self {
                let path = std::env::temp_dir()
                    .join(format!("ng_test_{}_{}", name, std::process::id()));
                let _ = fs::remove_dir_all(&path);
                fs::create_dir_all(&path).unwrap();
                TempDir { path }
            }
            pub fn path(&self) -> &str { self.path.to_str().unwrap() }
        }
        impl Drop for TempDir {
            fn drop(&mut self) { let _ = fs::remove_dir_all(&self.path); }
        }
    }
}
