//! v3.0 — Cross-shard communication: LockTx + CommitTx + receipts.
//!
//! Când o tranzacție e cross-shard (sender și receiver în shards diferite):
//!
//!   Phase 1 (LockTx, în shard-ul sender):
//!     - Debitează sender cu amount + fee
//!     - Generează un CrossShardReceipt (cantitate + receiver + expiry)
//!     - Receipt-ul e inclus în DagProposal (ca "seen_tx_hashes" extended)
//!     - Nodurile din shard-ul receiver văd receipt-ul prin gossip
//!
//!   Phase 2 (CommitTx, în shard-ul receiver):
//!     - Consumă receipt-ul (verifică semnătura + nonce)
//!     - Creditează receiver cu amount
//!     - Fee-ul rămâne la proposer-ul din shard-ul sender
//!
//! Dacă receipt-ul nu e consumat în CROSS_SHARD_RECEIPT_EXPIRY pași:
//!   - Refund automat: sender primește amount înapoi (fără fee)
//!
//! IMPORTANT: Acest modul NU schimbă protocolul ANGP. LockTx și CommitTx
//! sunt tranzacții normale cu flag-uri specifice. DagProposal rămâne
//! aceeași structură.

use serde::{Serialize, Deserialize};
use sha2::{Sha512_256, Digest};
use crate::transaction::{Hash, Transaction};
use crate::config::{N_SHARDS, CROSS_SHARD_RECEIPT_EXPIRY};
use crate::sharding::shard_of_address;

/// Un receipt cross-shard = dovada că sender-ul a fost debitat în shard-ul său.
/// Receiver-ul îl consumă pentru a se credita în shard-ul său.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CrossShardReceipt {
    /// Hash-ul LockTx-ului care a generat receipt-ul.
    pub source_tx_hash: Hash,
    /// Adresa sender-ului (care a fost debitat).
    pub sender: String,
    /// Adresa receiver-ului (care va fi creditat).
    pub receiver: String,
    /// Suma de creditat receiver-ului (fără fee).
    pub amount: u64,
    /// Shard-ul sursă (pentru verificare).
    pub from_shard: u32,
    /// Shard-ul destinație (pentru verificare).
    pub to_shard: u32,
    /// Pasul la care a fost generat receipt-ul (pentru expiry).
    pub created_at_step: u64,
    /// Nonce unic per receipt (anti-replay).
    pub receipt_nonce: u64,
}

impl CrossShardReceipt {
    /// Construiește un receipt dintr-o tranzacție cross-shard.
    /// Apelat după ce LockTx a fost finalizat în shard-ul sender.
    pub fn from_tx(tx: &Transaction, step: u64) -> Self {
        CrossShardReceipt {
            source_tx_hash: tx.hash,
            sender: tx.sender.clone(),
            receiver: tx.receiver.clone(),
            amount: tx.amount,
            from_shard: shard_of_address(&tx.sender),
            to_shard: shard_of_address(&tx.receiver),
            created_at_step: step,
            receipt_nonce: tx.nonce,
        }
    }

    /// Hash-ul canonic al receipt-ului (pentru dedup + lookup).
    pub fn receipt_hash(&self) -> Hash {
        let data = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            hex::encode(&self.source_tx_hash),
            self.sender, self.receiver, self.amount,
            self.from_shard, self.to_shard,
            self.created_at_step, self.receipt_nonce,
        );
        let mut hasher = Sha512_256::new();
        hasher.update(data.as_bytes());
        let result = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&result);
        h
    }

    /// Verifică dacă receipt-ul a expirat.
    pub fn is_expired(&self, current_step: u64) -> bool {
        current_step > self.created_at_step + CROSS_SHARD_RECEIPT_EXPIRY
    }

    /// Verifică validitatea receipt-ului (shard-uri consistente cu adresele).
    pub fn is_valid(&self) -> bool {
        let expected_from = shard_of_address(&self.sender);
        let expected_to = shard_of_address(&self.receiver);
        self.from_shard == expected_from
            && self.to_shard == expected_to
            && self.from_shard != self.to_shard  // trebuie să fie cross-shard
            && self.from_shard < N_SHARDS
            && self.to_shard < N_SHARDS
    }
}

/// Tipuri de tranzacții extinse pentru sharding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TxKind {
    /// Tranzacție normală (intra-shard).
    #[default]
    Normal,
    /// LockTx — phase 1 a unei tx cross-shard (în shard-ul sender).
    /// Debitează sender, generează receipt.
    Lock,
    /// CommitTx — phase 2 a unei tx cross-shard (în shard-ul receiver).
    /// Consumă receipt, creditează receiver.
    Commit {
        receipt_hash: Hash,
    },
    /// RefundTx — returnează fonduri unui sender a cărui tx cross-shard
    /// nu a fost consumată în timp (expiry).
    Refund {
        receipt_hash: Hash,
    },
}

/// Manager de receipts cross-shard.
/// Mentine 2 stări:
///   - `outgoing`: receipts generate de LockTx-uri din shards noastre
///     (așteptăm consumare în shard-ul destinație)
///   - `incoming`: receipts recepționate de la alte shards
///     (așteptăm CommitTx-uri în shards noastre)
pub struct CrossShardManager {
    /// Receipts generate de acest nod (LockTx-uri ale căror receipts așteaptă consumare).
    /// Map: receipt_hash → (receipt, status)
    pub outgoing: std::collections::HashMap<Hash, (CrossShardReceipt, ReceiptStatus)>,
    /// Receipts primite de la alte shards (așteptăm CommitTx).
    pub incoming: std::collections::HashMap<Hash, (CrossShardReceipt, ReceiptStatus)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReceiptStatus {
    /// Receipt creat, așteaptă consumare.
    Pending,
    /// Consumat printr-un CommitTx.
    Consumed,
    /// Expirat — refund emis.
    Expired,
}

impl CrossShardManager {
    pub fn new() -> Self {
        CrossShardManager {
            outgoing: std::collections::HashMap::new(),
            incoming: std::collections::HashMap::new(),
        }
    }

    /// Înregistrează un receipt generat local (după LockTx finalizat).
    pub fn register_outgoing(&mut self, receipt: CrossShardReceipt) {
        let h = receipt.receipt_hash();
        self.outgoing.insert(h, (receipt, ReceiptStatus::Pending));
    }

    /// Înregistrează un receipt primit de la alt shard (prin gossip).
    pub fn register_incoming(&mut self, receipt: CrossShardReceipt) {
        let h = receipt.receipt_hash();
        // Nu suprascriem dacă există deja (idempotent)
        self.incoming.entry(h).or_insert((receipt, ReceiptStatus::Pending));
    }

    /// Marchează un receipt ca fiind consumat (CommitTx finalizat).
    pub fn consume(&mut self, receipt_hash: &Hash) -> Option<CrossShardReceipt> {
        // Încercăm incoming (nodul nostru a consumat un receipt primit)
        if let Some((receipt, status)) = self.incoming.get_mut(receipt_hash) {
            if *status == ReceiptStatus::Pending {
                *status = ReceiptStatus::Consumed;
                return Some(receipt.clone());
            }
        }
        // Sau outgoing (alt nod a consumat receipt-ul nostru)
        if let Some((receipt, status)) = self.outgoing.get_mut(receipt_hash) {
            if *status == ReceiptStatus::Pending {
                *status = ReceiptStatus::Consumed;
                return Some(receipt.clone());
            }
        }
        None
    }

    /// Returnează receipts expirate care trebuie refund-uite.
    pub fn expired_receipts(&mut self, current_step: u64) -> Vec<CrossShardReceipt> {
        let mut expired = Vec::new();
        for (receipt, status) in self.outgoing.values_mut() {
            if *status == ReceiptStatus::Pending && receipt.is_expired(current_step) {
                *status = ReceiptStatus::Expired;
                expired.push(receipt.clone());
            }
        }
        expired
    }

    /// Verifică dacă un receipt e deja consumat (pentru anti-replay).
    pub fn is_consumed(&self, receipt_hash: &Hash) -> bool {
        let in_consumed = self.incoming.get(receipt_hash)
            .map(|(_, s)| *s == ReceiptStatus::Consumed)
            .unwrap_or(false);
        let out_consumed = self.outgoing.get(receipt_hash)
            .map(|(_, s)| *s == ReceiptStatus::Consumed)
            .unwrap_or(false);
        in_consumed || out_consumed
    }

    /// Verifică dacă un receipt e pending (pentru CommitTx-uri).
    pub fn is_pending_incoming(&self, receipt_hash: &Hash) -> bool {
        self.incoming.get(receipt_hash)
            .map(|(_, s)| *s == ReceiptStatus::Pending)
            .unwrap_or(false)
    }

    /// Numărul de receipts pending (pentru status).
    pub fn pending_count(&self) -> (usize, usize) {
        let out = self.outgoing.values()
            .filter(|(_, s)| *s == ReceiptStatus::Pending)
            .count();
        let inc = self.incoming.values()
            .filter(|(_, s)| *s == ReceiptStatus::Pending)
            .count();
        (out, inc)
    }
}

impl Default for CrossShardManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Modulul hex inline (pentru a nu adăuga dependență suplimentară).
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_receipt_deterministic_hash() {
        let tx = Transaction::new_with_nonce(
            "alice".to_string(), "bob".to_string(), 100, 1, vec![],
        );
        let r1 = CrossShardReceipt::from_tx(&tx, 42);
        let r2 = CrossShardReceipt::from_tx(&tx, 42);
        assert_eq!(r1.receipt_hash(), r2.receipt_hash(),
            "Same receipt must always have same hash");
    }

    #[test]
    fn test_receipt_expiry() {
        let tx = Transaction::new_with_nonce(
            "alice".to_string(), "bob".to_string(), 100, 1, vec![],
        );
        let r = CrossShardReceipt::from_tx(&tx, 100);
        assert!(!r.is_expired(200));
        assert!(!r.is_expired(100 + CROSS_SHARD_RECEIPT_EXPIRY));
        assert!(r.is_expired(100 + CROSS_SHARD_RECEIPT_EXPIRY + 1));
    }

    #[test]
    fn test_receipt_validity_cross_shard() {
        // Găsim 2 adrese în shards diferite
        let mut alice_addr = String::new();
        let mut bob_addr = String::new();
        let alice_shard = shard_of_address("alice");
        for i in 0..1000 {
            let addr = format!("bob_{}", i);
            if shard_of_address(&addr) != alice_shard {
                bob_addr = addr;
                alice_addr = "alice".to_string();
                break;
            }
        }
        assert!(!alice_addr.is_empty(), "Should find cross-shard addresses");

        let tx = Transaction::new_with_nonce(
            alice_addr, bob_addr, 100, 1, vec![],
        );
        let r = CrossShardReceipt::from_tx(&tx, 42);
        assert!(r.is_valid(), "Cross-shard receipt should be valid");
        assert_ne!(r.from_shard, r.to_shard);
    }

    #[test]
    fn test_receipt_validity_intra_shard_rejected() {
        // Găsim 2 adrese în același shard
        let alice_shard = shard_of_address("alice");
        let mut bob_addr = String::new();
        for i in 0..1000 {
            let addr = format!("bob_{}", i);
            if shard_of_address(&addr) == alice_shard {
                bob_addr = addr;
                break;
            }
        }
        let tx = Transaction::new_with_nonce(
            "alice".to_string(), bob_addr, 100, 1, vec![],
        );
        let r = CrossShardReceipt::from_tx(&tx, 42);
        assert!(!r.is_valid(), "Intra-shard receipt should NOT be valid (must be cross-shard)");
    }

    #[test]
    fn test_cross_shard_manager_register_and_consume() {
        let mut mgr = CrossShardManager::new();
        let tx = Transaction::new_with_nonce(
            "alice".to_string(), "bob".to_string(), 100, 1, vec![],
        );
        let r = CrossShardReceipt::from_tx(&tx, 42);
        let h = r.receipt_hash();

        mgr.register_outgoing(r.clone());
        let (out, inc) = mgr.pending_count();
        assert_eq!(out, 1);
        assert_eq!(inc, 0);

        let consumed = mgr.consume(&h);
        assert!(consumed.is_some());
        let (out2, _) = mgr.pending_count();
        assert_eq!(out2, 0);
        assert!(mgr.is_consumed(&h));
    }

    #[test]
    fn test_expired_receipts_collected() {
        let mut mgr = CrossShardManager::new();
        let tx = Transaction::new_with_nonce(
            "alice".to_string(), "bob".to_string(), 100, 1, vec![],
        );
        let r = CrossShardReceipt::from_tx(&tx, 100);
        mgr.register_outgoing(r);

        // La step 100 + EXPIRY + 1, ar trebui să fie expirat
        let expired = mgr.expired_receipts(100 + CROSS_SHARD_RECEIPT_EXPIRY + 1);
        assert_eq!(expired.len(), 1);
        let (out, _) = mgr.pending_count();
        assert_eq!(out, 0);
    }
}
