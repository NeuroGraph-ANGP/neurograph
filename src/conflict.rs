//! Conflict Vote — Etapa 1 completion.
//!
//! Problema: când 2 tranzacții cu ACELAȘI (sender, nonce) ajung la noduri diferite
//! aproximativ simultan (network race), fiecare nod acceptă prima pe care o vede.
//! Rezultat: mempool-urile nodurilor deviază, iar la finalization unele tx-uri sunt
//! refuzate de unii noduri și acceptate de alții → fork virtual.
//!
//! Soluție (fără a schimba protocolul de consens):
//!   1. Când un nod detectează un conflict (primește tx cu sender+nonce deja văzut
//!      dar hash diferit), generează un `ConflictVote` pentru tx-ul pe care l-a văzut
//!      PRIMUL (first-seen rule).
//!   2. Toate `ConflictVote`s pentru același (sender, nonce) se colectează timp de
//!      CONFLICT_VOTE_WINDOW pași.
//!   3. La expirarea ferestrei, tx-ul cu cele mai multe voturi ponderate de reputație
//!      câștigă. Celelalte sunt abandonate (eliminate din mempool).
//!
//! IMPORTANT: Acest modul NU schimbă consensul emergent (mediană ponderată de reputație).
//! Doar rezolvă conflicte locale de mempool ca un tiebreaker, înainte ca tx-urile
//! să ajungă la `compute_consensus`.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::transaction::{Transaction, Hash};
use crate::config::CONFLICT_VOTE_WINDOW;

/// Un vot pentru o tranzacție într-un conflict (sender, nonce).
/// Nodul votează tx-ul pe care l-a văzut PRIMUL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictVote {
    pub voter: String,
    pub sender: String,        // sender-ul tranzacției conflictuale
    pub nonce: u64,            // nonce-ul comun
    pub favored_tx: Hash,      // hash-ul tx-ului favorit (first-seen)
    pub step: u64,             // pasul la care a fost emis votul
}

/// Starea conflictului pentru un (sender, nonce).
#[derive(Debug, Clone)]
struct ConflictState {
    /// Toate tx-urile văzute pentru acest (sender, nonce).
    candidates: HashMap<Hash, Transaction>,
    /// Voturi primite: voter → (favored_tx, step)
    votes: HashMap<String, (Hash, u64)>,
    /// Pasul la care a început conflictul.
    started_at_step: u64,
}

/// Rezultatul observării unei tranzacții potențial conflictuale.
#[derive(Debug)]
pub enum ConflictStatus {
    /// Prima tx văzută pentru acest (sender, nonce).
    FirstSeen,
    /// Conflict detectat — emite acest vot către peers.
    Conflict(ConflictVote),
    /// Am mai văzut exact acest hash (duplicat).
    Duplicate,
}

/// Managerul de conflicte per nod.
pub struct ConflictResolver {
    /// (sender, nonce) → ConflictState
    conflicts: HashMap<(String, u64), ConflictState>,
}

impl ConflictResolver {
    pub fn new() -> Self {
        ConflictResolver { conflicts: HashMap::new() }
    }

    /// Înregistrează o tranzacție potențial conflictuală.
    pub fn observe(&mut self, tx: &Transaction, observer: &str, current_step: u64) -> ConflictStatus {
        let key = (tx.sender.clone(), tx.nonce);
        let hash = tx.hash;

        let state = self.conflicts.entry(key.clone()).or_insert_with(|| ConflictState {
            candidates: HashMap::new(),
            votes: HashMap::new(),
            started_at_step: current_step,
        });

        // Dacă am mai văzut exact acest hash → duplicat
        if state.candidates.contains_key(&hash) {
            return ConflictStatus::Duplicate;
        }

        // Dacă era deja o candidată diferită → conflict
        let was_conflict = !state.candidates.is_empty();

        state.candidates.insert(hash, tx.clone());

        if was_conflict {
            // Conflict: vote pentru PRIMA tx văzută (cea cu cel mai mic hash? nu —
            // prima inserată, dar HashMap nu păstrează ordinea). Pentru determinism,
            // votăm pentru candidata cu cel mai mic hash (regulă obiectivă, toți
            // nodurile oneste vor vota la fel dacă văd aceleași txs).
            // NOTE: first-seen ar fi subiectiv; min-hash e determinist și onest.
            let favored = *state.candidates.keys().min().unwrap_or(&hash);
            let vote = ConflictVote {
                voter: observer.to_string(),
                sender: tx.sender.clone(),
                nonce: tx.nonce,
                favored_tx: favored,
                step: current_step,
            };
            // Înregistrăm propriul vot
            state.votes.insert(observer.to_string(), (favored, current_step));
            ConflictStatus::Conflict(vote)
        } else {
            ConflictStatus::FirstSeen
        }
    }

    /// Înregistrează un vot primit de la un peer.
    pub fn record_vote(&mut self, vote: ConflictVote) {
        let key = (vote.sender.clone(), vote.nonce);
        let state = self.conflicts.entry(key).or_insert_with(|| ConflictState {
            candidates: HashMap::new(),
            votes: HashMap::new(),
            started_at_step: vote.step,
        });
        // Ultimul vot al unui observer suprascrie (nu putem avea 2 voturi de la același)
        state.votes.insert(vote.voter.clone(), (vote.favored_tx, vote.step));
    }

    /// Pentru un (sender, nonce), returnează tx-ul câștigător ponderat de reputație.
    /// Dacă fereastra de vot nu a expirat încă, returnează None.
    /// Dacă a expirat, returnează Some(winner_hash) și elimină conflictele pierdute.
    pub fn resolve(
        &mut self,
        sender: &str,
        nonce: u64,
        current_step: u64,
        reputations: &HashMap<String, f64>,
    ) -> Option<Hash> {
        let key = (sender.to_string(), nonce);
        let state = self.conflicts.get(&key)?;

        // Verificăm dacă fereastra a expirat
        if current_step < state.started_at_step + CONFLICT_VOTE_WINDOW {
            return None;
        }

        // Calculăm voturile ponderate
        let mut weighted: HashMap<Hash, f64> = HashMap::new();
        for (voter, (favored, _)) in &state.votes {
            let weight = reputations.get(voter).copied().unwrap_or(0.5);
            *weighted.entry(*favored).or_insert(0.0) += weight;
        }

        // Câștigătorul = hash-ul cu cel mai mare scor ponderat
        // Tiebreak: cel mai mic hash (determinist)
        let winner = weighted.iter()
            .max_by(|a, b| {
                a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.0.cmp(a.0))  // tiebreak: smaller hash wins
            })
            .map(|(h, _)| *h);

        // Curățăm conflictul rezolvat
        self.conflicts.remove(&key);

        winner
    }

    /// Returnează toți (sender, nonce) pentru care fereastra de vot a expirat
    /// și trebuie rezolvate în acest pas.
    pub fn expired_conflicts(&self, current_step: u64) -> Vec<(String, u64)> {
        self.conflicts.iter()
            .filter(|(_, state)| current_step >= state.started_at_step + CONFLICT_VOTE_WINDOW)
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Lista de hash-uri pierdătoare pentru un conflict rezolvat
    /// (pentru a le elimina din mempool).
    pub fn losers_for(&mut self, sender: &str, nonce: u64, winner: Hash) -> Vec<Hash> {
        let key = (sender.to_string(), nonce);
        let Some(state) = self.conflicts.get(&key) else { return Vec::new(); };
        state.candidates.keys()
            .filter(|h| **h != winner)
            .copied()
            .collect()
    }

    pub fn pending_count(&self) -> usize {
        self.conflicts.len()
    }
}

impl Default for ConflictResolver {
    fn default() -> Self { Self::new() }
}
