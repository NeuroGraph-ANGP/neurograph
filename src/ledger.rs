use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use serde::{Serialize, Deserialize};
use rayon::prelude::*;

use crate::transaction::{Transaction, Hash};

/// Tip de înregistrare în ledger (Etapa 1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EntryType {
    UserTx,
    /// Recompensă pentru nodul care a propus/finalizat tranzacția (Etapa 1).
    RewardTx { rewarded_node: String, source_tx: Hash },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub tx: Transaction,
    pub entry_type: EntryType,
    pub batch_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerData {
    pub entries: Vec<LedgerEntry>,
    pub dag_edges: Vec<(Hash, Vec<Hash>)>,
    pub next_batch_id: u64,
}

/// v2.5: Ledger cu index (sender, nonce) pentru O(1) nonce lookup
/// și flush periodic în loc de save_to_disk la fiecare add.
pub struct Ledger {
    pub entries: HashMap<Hash, LedgerEntry>,
    pub dag_edges: HashMap<Hash, Vec<Hash>>,
    pub finalized: HashMap<Hash, Transaction>,
    pub next_batch_id: u64,
    has_children_cache: HashSet<Hash>,
    file_path: String,

    /// v2.5: Index (sender, nonce) → max nonce finalizat per sender.
    /// Pentru O(1) `has_finalized_nonce` (înainte era O(n) paralelizat).
    finalized_nonces: HashMap<(String, u64), bool>,
    /// v2.5: Max nonce finalizat per sender (pentru check secvențialitate O(1)).
    last_finalized_nonce_per_sender: HashMap<String, u64>,

    dirty: AtomicBool,
}

impl Ledger {
    pub fn new(file_path: String) -> Self {
        let mut l = Ledger {
            entries: HashMap::new(),
            dag_edges: HashMap::new(),
            next_batch_id: 0,
            finalized: HashMap::new(),
            has_children_cache: HashSet::new(),
            file_path,
            finalized_nonces: HashMap::new(),
            last_finalized_nonce_per_sender: HashMap::new(),
            dirty: AtomicBool::new(false),
        };
        l.load_from_disk();
        l.rebuild_indexes();
        l.rebuild_children_cache_pub();
        l
    }

    pub fn add(&mut self, tx: Transaction) {
        let hash = tx.hash;
        let batch_id = self.next_batch_id;
        // Indexăm (sender, nonce) — v2.5
        self.finalized_nonces.insert((tx.sender.clone(), tx.nonce), true);
        self.last_finalized_nonce_per_sender.insert(tx.sender.clone(), tx.nonce);
        for parent in &tx.parents {
            self.dag_edges.entry(*parent).or_insert_with(Vec::new).push(hash);
            self.has_children_cache.insert(*parent);
        }
        let entry = LedgerEntry { tx: tx.clone(), entry_type: EntryType::UserTx, batch_id };
        self.entries.insert(hash, entry);
        self.finalized.insert(hash, tx);
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Etapa 1 — Batch finalization: adaugă un set de tranzacții + reward-urile aferente.
    pub fn add_batch(
        &mut self,
        txs: Vec<Transaction>,
        reward_map: &HashMap<Hash, String>,
    ) -> Vec<Hash> {
        let batch_id = self.next_batch_id;
        self.next_batch_id += 1;
        let mut added = Vec::with_capacity(txs.len() * 2);

        for tx in txs {
            let hash = tx.hash;
            // v2.5: Index update
            self.finalized_nonces.insert((tx.sender.clone(), tx.nonce), true);
            self.last_finalized_nonce_per_sender.insert(tx.sender.clone(), tx.nonce);
            for parent in &tx.parents {
                self.dag_edges.entry(*parent).or_insert_with(Vec::new).push(hash);
                self.has_children_cache.insert(*parent);
            }
            let entry = LedgerEntry { tx: tx.clone(), entry_type: EntryType::UserTx, batch_id };
            self.entries.insert(hash, entry);
            self.finalized.insert(hash, tx.clone());
            added.push(hash);

            if let Some(node) = reward_map.get(&hash) {
                let mut reward_tx = Transaction::new_with_fee(
                    "__system__".to_string(),
                    node.clone(),
                    tx.fee, 0, 0,
                    vec![hash],
                );
                reward_tx.hash = reward_tx.compute_hash();
                let reward_hash = reward_tx.hash;
                let reward_entry = LedgerEntry {
                    tx: reward_tx.clone(),
                    entry_type: EntryType::RewardTx {
                        rewarded_node: node.clone(),
                        source_tx: hash,
                    },
                    batch_id,
                };
                self.entries.insert(reward_hash, reward_entry);
                self.finalized.insert(reward_hash, reward_tx);
                self.dag_edges.entry(hash).or_insert_with(Vec::new).push(reward_hash);
                self.has_children_cache.insert(hash);
                added.push(reward_hash);
            }
        }

        self.dirty.store(true, Ordering::Relaxed);
        added
    }

    pub fn get(&self, hash: &Hash) -> Option<&Transaction> {
        self.finalized.get(hash)
    }

    pub fn get_entry(&self, hash: &Hash) -> Option<&LedgerEntry> {
        self.entries.get(hash)
    }

    pub fn get_children(&self, hash: &Hash) -> Vec<&Hash> {
        self.dag_edges.get(hash).map_or(Vec::new(), |v| v.iter().collect())
    }

    pub fn get_tips(&self) -> Vec<&Hash> {
        self.finalized.keys()
            .filter(|h| !self.has_children_cache.contains(*h))
            .collect()
    }

    fn rebuild_indexes(&mut self) {
        self.finalized_nonces.clear();
        self.last_finalized_nonce_per_sender.clear();
        for tx in self.finalized.values() {
            if tx.sender != "__system__" {
                self.finalized_nonces.insert((tx.sender.clone(), tx.nonce), true);
                let cur = self.last_finalized_nonce_per_sender.get(&tx.sender).copied().unwrap_or(0);
                if tx.nonce > cur {
                    self.last_finalized_nonce_per_sender.insert(tx.sender.clone(), tx.nonce);
                }
            }
        }
    }

    pub fn rebuild_children_cache_pub(&mut self) {
        self.rebuild_children_cache();
    }

    fn rebuild_children_cache(&mut self) {
        let all_parents: Vec<Hash> = self.dag_edges.keys().copied().collect();
        let collected: Vec<Hash> = all_parents.into_par_iter()
            .filter(|p| self.dag_edges.get(p).map_or(false, |v| !v.is_empty()))
            .collect();
        self.has_children_cache = collected.into_iter().collect();
    }

    pub fn len(&self) -> usize { self.finalized.len() }

    /// v2.5: O(1) datorită indexului (înainte era O(n) paralelizat).
    pub fn has_finalized_nonce(&self, sender: &str, nonce: u64) -> bool {
        self.finalized_nonces.contains_key(&(sender.to_string(), nonce))
    }

    /// v2.5: O(1) datorită indexului.
    pub fn last_finalized_nonce(&self, sender: &str) -> Option<u64> {
        self.last_finalized_nonce_per_sender.get(sender).copied()
    }

    pub fn finalized_count_per_node(&self) -> HashMap<String, usize> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for entry in self.entries.values() {
            if let EntryType::RewardTx { rewarded_node, .. } = &entry.entry_type {
                *counts.entry(rewarded_node.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    pub fn total_fees_collected(&self) -> u64 {
        self.finalized.values()
            .filter(|tx| tx.sender != "__system__")
            .map(|tx| tx.fee)
            .sum()
    }

    // ─── v2.5: dirty tracking + periodic flush ──────────────────────

    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    pub fn flush_if_dirty(&mut self) -> bool {
        if !self.is_dirty() { return false; }
        self.save_to_disk();
        true
    }

    pub fn flush_to_disk(&mut self) {
        self.save_to_disk();
    }

    pub(crate) fn save_to_disk_pub(&mut self) {
        self.save_to_disk();
    }

    fn save_to_disk(&mut self) {
        // v2.5: bincode în loc de JSON
        let data = LedgerData {
            entries: self.entries.values().cloned().collect(),
            dag_edges: self.dag_edges.iter().map(|(k, v)| (*k, v.clone())).collect(),
            next_batch_id: self.next_batch_id,
        };
        if let Ok(s) = bincode::serialize(&data) {
            let _ = File::create(&self.file_path).and_then(|mut f| f.write_all(&s));
        }
        self.dirty.store(false, Ordering::Relaxed);
    }

    fn load_from_disk(&mut self) {
        let Ok(mut f) = File::open(&self.file_path) else { return };
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_err() { return; }
        let data: Option<LedgerData> = bincode::deserialize(&buf).ok()
            .or_else(|| serde_json::from_slice(&buf).ok());
        let Some(data) = data else { return };
        self.entries.clear();
        self.dag_edges.clear();
        self.finalized.clear();
        for entry in data.entries {
            let h = entry.tx.hash;
            self.finalized.insert(h, entry.tx.clone());
            self.entries.insert(h, entry);
        }
        for (h, children) in data.dag_edges {
            self.dag_edges.insert(h, children);
        }
        self.next_batch_id = data.next_batch_id;
    }
}

impl Drop for Ledger {
    fn drop(&mut self) {
        if self.is_dirty() {
            self.save_to_disk();
        }
    }
}
