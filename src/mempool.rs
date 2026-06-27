use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use serde::{Serialize, Deserialize};
use crate::transaction::{Transaction, Hash};
use crate::config::NONCE_HISTORY_MAX_PER_SENDER;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolData {
    pub pending: Vec<Transaction>,
    pub arrival_time: Vec<(Hash, u64)>,
    pub nonce_history: Vec<(String, VecDeque<u64>)>,
    pub last_nonce: Vec<(String, u64)>,
}

/// v2.5: Mempool cu index (sender, nonce) pentru O(1) double-spend check
/// și in-memory cache cu flush periodic (în loc de save_to_disk la fiecare add).
pub struct Mempool {
    pub pending: HashMap<Hash, Transaction>,
    pub arrival_time: HashMap<Hash, u64>,
    nonce_history: HashMap<String, VecDeque<u64>>,
    last_nonce: HashMap<String, u64>,

    /// v2.5: Index (sender, nonce) → hash pentru O(1) double-spend check.
    /// Înainte: iteram toate pending txs (O(n))
    /// Acum: HashMap lookup (O(1))
    by_sender_nonce: HashMap<(String, u64), Hash>,

    max_size: usize,
    next_timestamp: u64,
    file_path: String,

    /// v2.5: dirty flag pentru flush periodic
    dirty: AtomicBool,
}

impl Mempool {
    pub fn new(max_size: usize, file_path: String) -> Self {
        let mut m = Mempool {
            pending: HashMap::new(),
            arrival_time: HashMap::new(),
            nonce_history: HashMap::new(),
            last_nonce: HashMap::new(),
            by_sender_nonce: HashMap::new(),
            max_size,
            next_timestamp: 0,
            file_path,
            dirty: AtomicBool::new(false),
        };
        m.load_from_disk();
        m
    }

    /// v2.5: Adaugă tx în mempool. Verificări O(1) datorită indexului.
    pub fn add(&mut self, tx: Transaction) -> bool {
        if self.pending.len() >= self.max_size { return false; }
        if self.pending.contains_key(&tx.hash) { return false; }

        let key = (tx.sender.clone(), tx.nonce);

        // O(1) double-spend check (v2.5)
        if self.by_sender_nonce.contains_key(&key) { return false; }
        if self.is_known_nonce(&tx.sender, tx.nonce) { return false; }
        if let Some(&last) = self.last_nonce.get(&tx.sender) {
            if tx.nonce <= last { return false; }
        }

        let ts = self.next_timestamp;
        self.next_timestamp += 1;
        let hash = tx.hash;
        self.pending.insert(hash, tx.clone());
        self.arrival_time.insert(hash, ts);
        self.by_sender_nonce.insert(key, hash);
        self.record_nonce(&tx.sender, tx.nonce);
        self.last_nonce.insert(tx.sender.clone(), tx.nonce);
        self.dirty.store(true, Ordering::Relaxed);
        true
    }

    pub fn is_known_nonce(&self, sender: &str, nonce: u64) -> bool {
        self.nonce_history.get(sender)
            .map(|h| h.iter().any(|&n| n == nonce))
            .unwrap_or(false)
    }

    pub fn record_nonce(&mut self, sender: &str, nonce: u64) {
        let h = self.nonce_history.entry(sender.to_string()).or_insert_with(VecDeque::new);
        h.push_back(nonce);
        while h.len() > NONCE_HISTORY_MAX_PER_SENDER { h.pop_front(); }
    }

    pub fn get_last_nonce(&self, sender: &str) -> Option<u64> {
        self.last_nonce.get(sender).copied()
    }

    pub fn set_last_nonce(&mut self, sender: &str, nonce: u64) {
        let cur = self.last_nonce.get(sender).copied().unwrap_or(0);
        if nonce > cur {
            self.last_nonce.insert(sender.to_string(), nonce);
            self.record_nonce(sender, nonce);
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    pub fn remove(&mut self, hash: &Hash) -> Option<Transaction> {
        let tx = self.pending.remove(hash);
        if let Some(ref t) = tx {
            self.arrival_time.remove(hash);
            self.by_sender_nonce.remove(&(t.sender.clone(), t.nonce));
            self.dirty.store(true, Ordering::Relaxed);
        }
        tx
    }

    /// Elimină un batch de txs. Mai eficient decât N remove-uri individuale.
    pub fn remove_batch(&mut self, hashes: &[Hash]) -> Vec<Transaction> {
        let mut removed = Vec::with_capacity(hashes.len());
        for h in hashes {
            if let Some(tx) = self.pending.remove(h) {
                self.arrival_time.remove(h);
                self.by_sender_nonce.remove(&(tx.sender.clone(), tx.nonce));
                removed.push(tx);
            }
        }
        if !removed.is_empty() {
            self.dirty.store(true, Ordering::Relaxed);
        }
        removed
    }

    pub fn get(&self, hash: &Hash) -> Option<&Transaction> {
        self.pending.get(hash)
    }

    pub fn get_all(&self) -> Vec<&Transaction> {
        self.pending.values().collect()
    }

    pub fn get_all_hashes(&self) -> Vec<Hash> {
        self.pending.keys().copied().collect()
    }

    pub fn get_oldest_tx(&self) -> Option<&Transaction> {
        if self.pending.is_empty() { return None; }
        let mut oldest_hash = None;
        let mut oldest_time = u64::MAX;
        for (h, t) in self.arrival_time.iter() {
            if *t < oldest_time { oldest_time = *t; oldest_hash = Some(h); }
        }
        oldest_hash.and_then(|h| self.pending.get(h))
    }

    pub fn get_all_by_arrival(&self) -> Vec<&Transaction> {
        let mut entries: Vec<(&Hash, u64)> = self.arrival_time.iter().map(|(h, t)| (h, *t)).collect();
        entries.sort_by_key(|(_, t)| *t);
        entries.into_iter().filter_map(|(h, _)| self.pending.get(h)).collect()
    }

    pub fn contains(&self, hash: &Hash) -> bool { self.pending.contains_key(hash) }
    pub fn len(&self) -> usize { self.pending.len() }
    pub fn is_empty(&self) -> bool { self.pending.is_empty() }

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

    fn save_to_disk(&mut self) {
        // v2.5: bincode în loc de JSON
        let data = MempoolData {
            pending: self.pending.values().cloned().collect(),
            arrival_time: self.arrival_time.iter().map(|(h, t)| (*h, *t)).collect(),
            nonce_history: self.nonce_history.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            last_nonce: self.last_nonce.iter().map(|(k, v)| (k.clone(), *v)).collect(),
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
        // v2.5: încercăm bincode prima dată, fallback JSON (pentru migrare)
        let data: Option<MempoolData> = bincode::deserialize(&buf).ok()
            .or_else(|| serde_json::from_slice(&buf).ok());
        let Some(data) = data else { return };
        self.pending.clear();
        self.arrival_time.clear();
        self.nonce_history.clear();
        self.last_nonce.clear();
        self.by_sender_nonce.clear();
        for tx in data.pending {
            self.by_sender_nonce.insert((tx.sender.clone(), tx.nonce), tx.hash);
            self.pending.insert(tx.hash, tx);
        }
        for (h, t) in data.arrival_time { self.arrival_time.insert(h, t); }
        for (s, h) in data.nonce_history { self.nonce_history.insert(s, h); }
        for (s, n) in data.last_nonce { self.last_nonce.insert(s, n); }
        if let Some(&max_ts) = self.arrival_time.values().max() {
            self.next_timestamp = max_ts + 1;
        }
    }
}

impl Drop for Mempool {
    fn drop(&mut self) {
        if self.is_dirty() {
            self.save_to_disk();
        }
    }
}
