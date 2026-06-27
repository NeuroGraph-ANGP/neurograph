//! Snapshot Sync — bootstrap pentru noduri noi.
//!
//! Problema: un nod care se alătură unei rețele existente nu are istoric.
//! Fără snapshot, ar trebui să primească toate tranzacțiile de la genesis și
//! să le re-proceseze — impracticabil pentru rețele cu istoric lung.
//!
//! Soluție (fără a schimba protocolul):
//!   1. Nodul nou cere un snapshot de la orice peer conectat: `SnapshotRequest { from_step }`.
//!   2. Peer-ul răspunde cu `SnapshotResponse` care conține:
//!      - ledger entries (toate tranzacțiile finalizate)
//!      - state (toate balanțele)
//!      - last_step (pasul la care a fost făcut snapshot-ul)
//!   3. Nodul nou aplică snapshot-ul local (prin `DagLogic::apply_snapshot`),
//!      apoi începe să proceseze mesaje noi.
//!
//! IMPORTANT: Acest modul NU schimbă consensul. Doar sincronizează starea locală
//! cu cea a rețelei. Consensul emergent continuă să funcționeze la fel.

use serde::{Serialize, Deserialize};
use crate::transaction::Hash;
use crate::ledger::LedgerData;
use crate::state::StateData;

/// Cerere de snapshot de la un nod nou.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRequest {
    pub requester: String,
    /// Pasul de la care nodul nou vrea să primească date.
    /// 0 = de la genesis (full snapshot).
    pub from_step: u64,
}

/// Răspuns cu snapshot complet de la un peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub responder: String,
    pub last_step: u64,
    pub ledger: LedgerData,
    pub state: StateData,
    /// Hash-ul stării pentru verificare.
    pub state_root: Hash,
}

impl SnapshotResponse {
    /// Construiește un snapshot din ledger-ul și starea curentă.
    /// Apelat de peer-ul care răspunde la o cerere.
    pub fn build(
        responder: &str,
        last_step: u64,
        ledger: &crate::ledger::Ledger,
        state: &crate::state::StateManager,
    ) -> Self {
        let ledger_data = LedgerData {
            entries: ledger.entries.values().cloned().collect(),
            dag_edges: ledger.dag_edges.iter().map(|(k, v)| (*k, v.clone())).collect(),
            next_batch_id: ledger.next_batch_id,
        };
        let state_data = StateData {
            balances: state.balances.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        };
        let state_root = state.state_root();
        SnapshotResponse {
            responder: responder.to_string(),
            last_step,
            ledger: ledger_data,
            state: state_data,
            state_root,
        }
    }

    /// Aplică snapshot-ul pe un ledger și state externe.
    /// Pentru uz intern (DagLogic are metoda proprie `apply_snapshot`).
    #[doc(hidden)]
    pub fn apply(
        &self,
        ledger: &mut crate::ledger::Ledger,
        state: &mut crate::state::StateManager,
    ) -> (usize, usize) {
        ledger.entries.clear();
        ledger.dag_edges.clear();
        ledger.finalized.clear();
        for entry in &self.ledger.entries {
            let h = entry.tx.hash;
            ledger.finalized.insert(h, entry.tx.clone());
            ledger.entries.insert(h, entry.clone());
        }
        for (h, children) in &self.ledger.dag_edges {
            ledger.dag_edges.insert(*h, children.clone());
        }
        ledger.next_batch_id = self.ledger.next_batch_id;
        ledger.rebuild_children_cache_pub();
        ledger.save_to_disk_pub();

        state.balances.clear();
        for (addr, bal) in &self.state.balances {
            state.balances.insert(addr.clone(), *bal);
        }
        state.save_to_disk_pub();

        (ledger.entries.len(), state.balances.len())
    }
}
