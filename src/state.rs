use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use serde::{Serialize, Deserialize};
use sha2::{Sha512_256, Digest};
use rayon::prelude::*;

use crate::transaction::{Transaction, Hash};
use crate::config::GENESIS_BALANCE_MILLI;

/// Stare globală — Account Model simplu.
/// Soldurile sunt în milliANGP (1 ANGP = 1000 milliANGP).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateData {
    pub balances: Vec<(String, u64)>,
}

/// v2.5: StateManager cu in-memory cache + flush periodic.
///
/// Înainte (v2.4): fiecare `apply_tx` și `apply_reward` scria tot fișierul pe disk.
///   → la 500K TPS, 500K × ~100μs = 50s de disk I/O / sec → imposibil.
///
/// Acum (v2.5): modificările se fac în memorie, disk flush se face la:
///   1. Apel explicit `flush_to_disk()`
///   2. La fiecare FLUSH_INTERVAL_MS (verificat în main loop)
///   3. La shutdown (Drop)
///
/// WAL (write-ahead log) pentru crash recovery: TODO în v2.6.
/// Pentru v2.5, dacă nodul crash-uiește între flush-uri, pierdem ultimele modificări
/// (acceptable pentru un L2 cu finalitate periodică).
pub struct StateManager {
    pub balances: HashMap<String, u64>,
    file_path: String,
    /// v2.5: dirty flag — true dacă avem modificări ne-salvate
    dirty: AtomicBool,
    /// v2.5: counter pentru a decide când să flush-am
    ops_since_flush: u64,
}

impl StateManager {
    pub fn new(file_path: String) -> Self {
        let mut sm = StateManager {
            balances: HashMap::new(),
            file_path,
            dirty: AtomicBool::new(false),
            ops_since_flush: 0,
        };
        sm.load_from_disk();
        sm
    }

    pub fn get_balance(&self, addr: &str) -> u64 {
        self.balances.get(addr).copied().unwrap_or(0)
    }

    pub fn mint(&mut self, addr: &str, amount: u64) {
        let b = self.balances.entry(addr.to_string()).or_insert(0);
        *b = b.saturating_add(amount);
        self.mark_dirty();
    }

    pub fn genesis_allocate(&mut self, addr: &str) {
        self.balances.insert(addr.to_string(), GENESIS_BALANCE_MILLI);
        self.mark_dirty();
    }

    pub fn has_balance(&self, addr: &str, total_cost: u64) -> bool {
        self.get_balance(addr) >= total_cost
    }

    pub fn apply_tx(&mut self, tx: &Transaction) -> bool {
        let cost = tx.total_cost();
        let sender_bal = self.get_balance(&tx.sender);
        if sender_bal < cost {
            return false;
        }
        *self.balances.entry(tx.sender.clone()).or_insert(0) = sender_bal - cost;
        let r = self.balances.entry(tx.receiver.clone()).or_insert(0);
        *r = r.saturating_add(tx.amount);
        self.mark_dirty();
        true
    }

    pub fn apply_reward(&mut self, node: &str, amount: u64) {
        let b = self.balances.entry(node.to_string()).or_insert(0);
        *b = b.saturating_add(amount);
        self.mark_dirty();
    }

    pub fn slash(&mut self, node: &str, ratio: f64) {
        if let Some(b) = self.balances.get_mut(node) {
            *b = (*b as f64 * (1.0 - ratio.clamp(0.0, 1.0))) as u64;
            self.mark_dirty();
        }
    }

    /// v2.5: SHA-512/256 pentru state_root (criptografic, anti-coliziune).
    pub fn state_root(&self) -> Hash {
        let mut entries: Vec<(String, u64)> = self.balances.iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        // Paralelizăm sortarea pentru volume mari
        entries.par_sort_by(|a, b| a.0.cmp(&b.0));

        let mut hasher = Sha512_256::new();
        for (addr, bal) in &entries {
            hasher.update(addr.as_bytes());
            hasher.update(bal.to_le_bytes());
        }
        let result = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&result);
        h
    }

    pub fn len(&self) -> usize {
        self.balances.len()
    }

    pub fn top_balances(&self, n: usize) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = self.balances.iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.truncate(n);
        v
    }

    // ─── v2.5: dirty tracking + periodic flush ──────────────────────

    fn mark_dirty(&mut self) {
        self.dirty.store(true, Ordering::Relaxed);
        self.ops_since_flush += 1;
    }

    /// Returnează true dacă avem modificări nesalvate.
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    /// Forțează salvarea pe disk dacă e dirty.
    /// Apelat periodic din main loop.
    pub fn flush_if_dirty(&mut self) -> bool {
        if !self.is_dirty() { return false; }
        self.save_to_disk();
        true
    }

    /// Forțează salvarea pe disk.
    pub fn flush_to_disk(&mut self) {
        self.save_to_disk();
    }

    fn save_to_disk(&mut self) {
        // v2.5: bincode în loc de JSON (5-10× mai rapid + mai compact)
        let data = StateData {
            balances: self.balances.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        };
        if let Ok(serialized) = bincode::serialize(&data) {
            let _ = File::create(&self.file_path).and_then(|mut f| f.write_all(&serialized));
        }
        self.dirty.store(false, Ordering::Relaxed);
        self.ops_since_flush = 0;
    }

    fn load_from_disk(&mut self) {
        let Ok(mut file) = File::open(&self.file_path) else { return };
        let mut buffer = Vec::new();
        if file.read_to_end(&mut buffer).is_err() { return; }
        // v2.5: încercăm bincode prima dată, fallback la JSON (pentru migrare)
        let data: Option<StateData> = bincode::deserialize(&buffer).ok()
            .or_else(|| serde_json::from_slice(&buffer).ok());
        let Some(data) = data else { return };
        self.balances.clear();
        for (addr, bal) in data.balances {
            self.balances.insert(addr, bal);
        }
    }
}

impl Drop for StateManager {
    fn drop(&mut self) {
        // Flush la shutdown pentru a nu pierde modificări
        if self.is_dirty() {
            self.save_to_disk();
        }
    }
}

// Pentru snapshot sync — expune save_to_disk_pub
impl StateManager {
    pub(crate) fn save_to_disk_pub(&mut self) {
        self.save_to_disk();
    }
}
