use std::collections::{HashMap, HashSet};
use ndarray::Array1;
use rayon::prelude::*;

use crate::config::{
    DIM, EMBED_WINDOW_BYTES,
    FINALITY_BASE_THRESHOLD, FINALITY_MIN_THRESHOLD, FINALITY_MAX_THRESHOLD,
    EXPECTED_ACTIVE_NODES,
    PROPOSAL_TIPS_COUNT, STATE_ROOT_PENALTY, TIP_DIFF_PENALTY,
    FINALIZATION_INTERVAL, BATCH_APPROVAL_THRESHOLD,
    MCMC_WALK_LENGTH, MCMC_PREFER_LIGHT, MCMC_REP_INFLUENCE,
    PREDICTIVE_OWN_WEIGHT, PREDICTIVE_MOMENTUM_WEIGHT,
    SOFT_ERROR_SCALE, ERROR_NORMALIZATION,
    CLUSTER_WEIGHT_CAP_RATIO, CLUSTER_CAPPING_EPSILON,
};
use crate::transaction::{Transaction, Hash};
use crate::ledger::Ledger;
use crate::mempool::Mempool;
use crate::state::StateManager;
use crate::utils::{weighted_median_arrays, euclidean, vec_to_array};
use crate::cross_shard::CrossShardReceipt;

/// ════════════════════════════════════════════════════════════════════
/// DagProposal — conține VOTUL NEURAL (predicția Hebbian) + tips + state.
/// ════════════════════════════════════════════════════════════════════
///
/// `prediction` = vectorul 4D produs de AdaptiveDag (Hebbian) al sender-ului.
/// Acesta e "votul" neural al nodului. Consensul emergent se calculează
/// ca mediană ponderată de reputație a acestor voturi.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DagProposal {
    pub sender: String,
    pub step: u64,
    pub seq: u64,
    pub nonce: u64,
    /// VOTUL NEURAL — predicția 4D produsă de AdaptiveDag (Hebbian) al sender-ului.
    pub prediction: Vec<f64>,
    pub proposed_tips: Vec<Hash>,
    pub state_root: Hash,
    pub seen_tx_hashes: Vec<Hash>,
    /// v3.1: Receipts cross-shard văzute de acest nod.
    /// Propagate automat prin gossip — un nod din shard-ul destinație
    /// le poate vedea din propunerile altor noduri, chiar dacă nu a
    /// primit LockTx-ul direct.
    #[serde(default)]
    pub seen_receipts: Vec<CrossShardReceipt>,
}

/// ════════════════════════════════════════════════════════════════════
/// DagConsensus — rezultatul consensului emergent.
/// ════════════════════════════════════════════════════════════════════
#[derive(Debug, Clone, Default)]
pub struct DagConsensus {
    /// MEDIANA PONDERATĂ a predicțiilor (voturilor) — consensul neural emergent.
    /// Calculate din `prediction`-urile tuturor propunerilor active, ponderate
    /// după reputația sender-ului.
    pub median_prediction: Array1<f64>,
    pub common_tips: Vec<Hash>,           // tips cu cele mai multe voturi
    pub common_state_root: Hash,          // state_root-ul majoritar
    pub approved_tx_hashes: Vec<Hash>,    // tx-uri aprobate de ≥ BATCH_APPROVAL_THRESHOLD din noduri
    pub proposer_of: HashMap<Hash, String>, // tx_hash → primul node care l-a propus (pentru reward)
}

pub struct DagLogic {
    pub ledger: Ledger,
    pub mempool: Mempool,
    pub state: StateManager,
    embedding_cache: HashMap<Hash, Array1<f64>>,
    /// v3.5.4: Counter pentru retry-urile de finalizare per tx.
    /// Dacă un tx eșuează de prea multe ori (insufficient balance),
    /// e eliminat din mempool ca să nu spam log-ul.
    finalize_retry_count: HashMap<Hash, u32>,
}

impl DagLogic {
    pub fn new(mempool_max_size: usize, data_dir: &str) -> Self {
        let mempool_file = format!("{}/mempool.json", data_dir);
        let ledger_file = format!("{}/ledger.json", data_dir);
        let state_file = format!("{}/state.json", data_dir);
        std::fs::create_dir_all(data_dir).unwrap_or_default();

        let ledger = Ledger::new(ledger_file.clone());
        let mempool = Mempool::new(mempool_max_size, mempool_file);
        let state = StateManager::new(state_file);

        // Propagăm last_nonce per sender în mempool (după restart)
        let senders: HashSet<String> = ledger
            .finalized
            .values()
            .map(|tx| tx.sender.clone())
            .collect();
        let mut mempool = mempool;
        for sender in senders {
            if let Some(last_n) = ledger.last_finalized_nonce(&sender) {
                mempool.set_last_nonce(&sender, last_n);
            }
        }

        DagLogic {
            ledger,
            mempool,
            state,
            embedding_cache: HashMap::new(),
            finalize_retry_count: HashMap::new(),
        }
    }

    pub fn get_mempool_len(&self) -> usize { self.mempool.len() }
    pub fn get_ledger_len(&self) -> usize { self.ledger.len() }
    pub fn get_ledger(&self) -> &Ledger { &self.ledger }
    pub fn get_mempool(&self) -> &Mempool { &self.mempool }
    pub fn get_state(&self) -> &StateManager { &self.state }
    pub fn get_state_mut(&mut self) -> &mut StateManager { &mut self.state }

    // ─── Pasul 1 + 3: Validare + adăugare ─────────────────────────────
    pub fn add_transaction(&mut self, tx: Transaction) -> bool {
        if !tx.verify_signature() {
            println!("[DagLogic] Rejected: invalid signature");
            return false;
        }
        if !self.check_double_spend(&tx) {
            println!("[DagLogic] Rejected: double-spend");
            return false;
        }
        // Etapa 3: validare sold
        if !self.state.has_balance(&tx.sender, tx.total_cost()) {
            println!("[DagLogic] Rejected: insufficient balance ({} has {}, needs {})",
                tx.sender, self.state.get_balance(&tx.sender), tx.total_cost());
            return false;
        }
        self.mempool.add(tx)
    }

    pub fn add_transaction_from_gossip(&mut self, tx: Transaction) -> bool {
        if !tx.verify_signature() { return false; }
        if !self.check_double_spend(&tx) { return false; }
        self.mempool.add(tx)
    }

    /// v3.1: Adaugă un batch de tranzacții cu BATCH VERIFICATION (ed25519-dalek).
    /// 5-10× mai rapid decât verify individual prin algebră Scheidt.
    ///
    /// Pipeline:
    ///   1. Batch verify all signatures simultaneously (5-10× faster)
    ///   2. Pentru txs valide: O(1) double-spend check + mempool add
    /// Returnează: (added_count, rejected_count).
    pub fn add_verified_batch(&mut self, txs: Vec<Transaction>) -> (usize, usize) {
        if txs.is_empty() { return (0, 0); }

        // v3.1: BATCH VERIFY — verificăm toate semnăturile simultan
        let tx_refs: Vec<&Transaction> = txs.iter().collect();
        let verify_results = Transaction::verify_batch(&tx_refs);

        let mut added = 0;
        let mut rejected = 0;
        for (tx, &valid) in txs.into_iter().zip(verify_results.iter()) {
            if !valid {
                rejected += 1;
                continue;
            }
            // O(1) double-spend check
            if !self.check_double_spend(&tx) {
                rejected += 1;
                continue;
            }
            if self.mempool.add(tx) {
                added += 1;
            } else {
                rejected += 1;
            }
        }
        (added, rejected)
    }

    // ─── Pasul 3: Double-Spend Detection ──────────────────────────────
    pub fn check_double_spend(&self, tx: &Transaction) -> bool {
        if self.mempool.is_known_nonce(&tx.sender, tx.nonce) { return false; }
        if self.ledger.has_finalized_nonce(&tx.sender, tx.nonce) { return false; }
        let mempool_last = self.mempool.get_last_nonce(&tx.sender);
        let ledger_last = self.ledger.last_finalized_nonce(&tx.sender);
        let last = mempool_last.max(ledger_last);
        if let Some(last_n) = last {
            if tx.nonce <= last_n { return false; }
        }
        if self.mempool.contains(&tx.hash) { return false; }
        true
    }

    // ─── Etapa 2: Weighted Random Walk (MCMC) ─────────────────────────
    /// Selectează tips-uri folosind un random walk invers ponderat.
    /// Pornind de la tips curente, merge înapoi în DAG cu probabilitate invers proporțională
    /// cu greutatea cumulată (numărul de copii) a fiecărui candidat.
    /// Repurația creatorului tranzacției influențează și ea selecția.
    pub fn select_tips_mcmc(&self, count: usize, rep_map: &HashMap<String, f64>) -> Vec<Hash> {
        let tips = self.ledger.get_tips();
        if tips.is_empty() { return Vec::new(); }
        let actual_count = count.min(tips.len());
        if actual_count == 0 { return Vec::new(); }

        let mut selected: Vec<Hash> = Vec::with_capacity(actual_count);
        let mut visited: HashSet<Hash> = HashSet::new();
        let mut rng = rand::thread_rng();

        while selected.len() < actual_count {
            // Pornește de la un tip aleator
            let start_idx = rand::Rng::gen_range(&mut rng, 0..tips.len());
            let mut current = *tips[start_idx];

            for _ in 0..MCMC_WALK_LENGTH {
                if visited.contains(&current) { break; }
                // Găsim părinții lui `current`
                let parents: Vec<Hash> = self.ledger.get_entry(&current)
                    .map(|e| e.tx.parents.clone())
                    .unwrap_or_default();
                if parents.is_empty() { break; }

                // Calculăm greutățile: invers proporțional cu numărul de copii
                let weights: Vec<f64> = parents.iter().map(|p| {
                    let children_count = self.ledger.get_children(p).len() as f64;
                    let mut w = 1.0 / (1.0 + children_count); // +1 pentru div/0
                    if !MCMC_PREFER_LIGHT { w = 1.0 / w; }
                    // Influență reputație
                    if let Some(parent_tx) = self.ledger.get(p) {
                        if let Some(rep) = rep_map.get(&parent_tx.sender) {
                            w = w * (1.0 - MCMC_REP_INFLUENCE) + MCMC_REP_INFLUENCE * *rep;
                        }
                    }
                    w.max(0.001)
                }).collect();

                let total: f64 = weights.iter().sum();
                if total <= 0.0 { break; }

                let r = rand::Rng::gen_range(&mut rng, 0.0..total);
                let mut cum = 0.0;
                let mut next = parents[0];
                for (i, w) in weights.iter().enumerate() {
                    cum += *w;
                    if cum >= r {
                        next = parents[i];
                        break;
                    }
                }
                current = next;
            }

            if !visited.contains(&current) {
                visited.insert(current);
                selected.push(current);
            }
        }
        selected
    }

    /// Selecție fallback simplă (aleatorie) pentru compatibilitate.
    pub fn select_parents(&self, count: usize) -> Vec<Hash> {
        let tips = self.ledger.get_tips();
        if tips.is_empty() { return Vec::new(); }
        let actual = count.min(tips.len());
        let mut selected = Vec::with_capacity(actual);
        let mut used = HashSet::new();
        while selected.len() < actual {
            let idx = rand::random::<usize>() % tips.len();
            let tip = tips[idx];
            if used.insert(*tip) { selected.push(*tip); }
        }
        selected
    }

    // ════════════════════════════════════════════════════════════════
    // SELECȚIE NEURALĂ DE TIPS — Hebbian-guided
    // ════════════════════════════════════════════════════════════════
    //
    // În loc de random/MCMC, folosim embedding-ul determinist al fiecărui tip
    // (Pasul 4) și alegem tips-urile ale căror embedding-uri sunt cele mai
    // apropiate (L2) de predicția Hebbian curentă.
    //
    // Asta înseamnă: AdaptiveDag decide CE predică nodul, iar selecția de tips
    // e ghidată de aceeași predicție. Există o legătură neurală directă între
    // "creier" (Hebbian) și "acțiune" (ce tips atinge nodul).
    //
    // Paralelizabil cu Rayon pe lista de tips candidate.
    pub fn select_tips_neural(&self, prediction: &Array1<f64>, count: usize) -> Vec<Hash> {
        let tips = self.ledger.get_tips();
        if tips.is_empty() { return Vec::new(); }
        let actual = count.min(tips.len());
        if actual == 0 { return Vec::new(); }

        // Calculăm distanța L2 dintre embedding-ul fiecărui tip și predicție.
        // Paralelizăm cu Rayon pentru volume mari de tips.
        let mut scored: Vec<(Hash, f64)> = tips
            .par_iter()
            .map(|tip| {
                let embedding = Self::embed_deterministic(tip);
                let dist = euclidean(&embedding, prediction);
                (**tip, dist)
            })
            .collect();

        // Sortăm crescător după distanță (cele mai apropiate primele)
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(actual).map(|(h, _)| h).collect()
    }

    // ════════════════════════════════════════════════════════════════
    // SELECȚIE PREDICTIVĂ DE TIPS (v2.4 #3)
    // ════════════════════════════════════════════════════════════════
    //
    // În loc de a folosi doar propria predicție, blend-uim:
    //   expected_consensus = OWN_WEIGHT × own_prediction
    //                      + (1 - OWN_WEIGHT) × last_consensus_median
    //
    // Aplicăm momentum: tips-urile care au fost comune în istoricul recent
    // primesc un boost de PREDICTIVE_MOMENTUM_WEIGHT (pattern reinforcement).
    //
    // Scoring final pentru fiecare tip candidat:
    //   score = (1 - MOMENTUM_WEIGHT) × (1 - embedding_distance(expected_consensus))
    //         + MOMENTUM_WEIGHT × momentum_score
    //   unde momentum_score = de_câte_ori_tip_a_fost_comun / history_size
    //
    // Efect:
    //   - Nodurile oneste predic corect → expected_consensus apropiat de realitate
    //     → aleg tips care vor fi comune → reputație ↑ → α ↑ → feedback pozitiv
    //   - Atacatorii nu pot prezice → expected_consensus greșit → tips neobișnuite
    //     → reputație ↓ → α ↓ → feedback negativ
    pub fn select_tips_predictive(
        &self,
        own_prediction: &Array1<f64>,
        consensus_median: &Array1<f64>,
        recent_common_tips: &std::collections::VecDeque<Vec<Hash>>,
        count: usize,
    ) -> Vec<Hash> {
        let tips = self.ledger.get_tips();
        if tips.is_empty() { return Vec::new(); }
        let actual = count.min(tips.len());
        if actual == 0 { return Vec::new(); }

        // 1. Blend predicție proprie + consens (ca să se alinieze la rețea)
        // v3.5.3 FIX: Verificam shape-ul consensus_median ca sa evitam panic.
        let consensus_active = consensus_median.len() == DIM
            && consensus_median.iter().sum::<f64>() > 0.0;
        let expected_consensus: Array1<f64> = if consensus_active {
            own_prediction * PREDICTIVE_OWN_WEIGHT + consensus_median * (1.0 - PREDICTIVE_OWN_WEIGHT)
        } else {
            // Bootstrap — folosim doar propria predicție
            own_prediction.clone()
        };

        // 2. Calcul momentum scores (câte apariții per tip în istoric)
        let mut momentum_count: HashMap<Hash, usize> = HashMap::new();
        let history_size = recent_common_tips.len().max(1);
        for tips_set in recent_common_tips {
            for tip in tips_set {
                *momentum_count.entry(*tip).or_insert(0) += 1;
            }
        }

        // 3. Calcul scor final pentru fiecare tip candidat (paralelizabil)
        let mut scored: Vec<(Hash, f64)> = tips
            .par_iter()
            .map(|tip| {
                let embedding = Self::embed_deterministic(tip);
                let dist = euclidean(&embedding, &expected_consensus);
                let neural_score = (1.0 - dist).max(0.0);  // ∈ [0, 1]

                let mom = momentum_count.get(*tip).copied().unwrap_or(0) as f64 / history_size as f64;
                let mom_score = mom.min(1.0);

                let final_score = (1.0 - PREDICTIVE_MOMENTUM_WEIGHT) * neural_score
                    + PREDICTIVE_MOMENTUM_WEIGHT * mom_score;

                (**tip, final_score)
            })
            .collect();

        // 4. Sortăm descrescător după scor (cele mai probabile comune primele)
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(actual).map(|(h, _)| h).collect()
    }

    // ─── Pasul 4: Embedding Deterministic ─────────────────────────
    pub fn embed_deterministic(hash: &Hash) -> Array1<f64> {
        let mut vec = Array1::zeros(DIM);
        let w = EMBED_WINDOW_BYTES;
        let two_pi = std::f64::consts::TAU;
        for i in 0..DIM {
            let freq = (i + 1) as f64;
            let mut acc = 0.0;
            for k in 0..w {
                let byte_idx = (i * w + k) % 32;
                let byte_val = hash[byte_idx] as f64 / 255.0;
                let phase = two_pi * freq * (k as f64) / (w as f64);
                acc += byte_val * phase.cos();
            }
            acc /= w as f64;
            vec[i] = ((acc + 1.0) / 2.0).clamp(0.0, 1.0);
        }
        vec
    }

    pub fn embed(&mut self, hash: &Hash) -> Array1<f64> {
        if let Some(cached) = self.embedding_cache.get(hash) { return cached.clone(); }
        let v = Self::embed_deterministic(hash);
        self.embedding_cache.insert(*hash, v.clone());
        v
    }

    // ════════════════════════════════════════════════════════════════
    // Construire propunere — cu VOT NEURAL + TIPS PREDICTIVE
    // ════════════════════════════════════════════════════════════════
    //
    // `prediction` = vectorul 4D produs de AdaptiveDag (Hebbian) cu α adaptiv.
    // `consensus_median` = mediana ponderată a predicțiilor de la pasul anterior.
    // `recent_common_tips` = istoricul tips-urilor comune (pentru momentum).
    //
    // Se folosește pentru:
    //   1. Selecție predictivă de tips (blend own + consensus + momentum)
    //   2. Includerea votului în DagProposal pentru consens emergent
    pub fn build_proposal(
        &mut self,
        sender: &str,
        step: u64,
        seq: u64,
        pow_nonce: u64,
        _rep_map: &HashMap<String, f64>,
        prediction: &Array1<f64>,
        consensus_median: &Array1<f64>,
        recent_common_tips: &std::collections::VecDeque<Vec<Hash>>,
        // v3.1: receipts cross-shard văzute de acest nod (pentru propagare)
        seen_receipts: Vec<CrossShardReceipt>,
    ) -> DagProposal {
        // Selectează tips PREDICTIV: blend own + consensus + momentum
        let proposed_tips = if self.ledger.len() > 0 {
            self.select_tips_predictive(prediction, consensus_median, recent_common_tips, PROPOSAL_TIPS_COUNT)
        } else {
            Vec::new()
        };
        let state_root = self.state.state_root();
        let seen_tx_hashes = self.mempool.get_all_hashes();
        DagProposal {
            sender: sender.to_string(),
            step, seq, nonce: pow_nonce,
            prediction: prediction.iter().copied().collect(),
            proposed_tips,
            state_root,
            seen_tx_hashes,
            seen_receipts,
        }
    }

    // ════════════════════════════════════════════════════════════════
    // CONSENS EMERGENT — mediană ponderată de reputație a voturilor
    // ════════════════════════════════════════════════════════════════
    //
    // Pipeline:
    //   1. Colectăm predicțiile (voturile) din toate propunerile active
    //   2. Le filtrăm după reputație (> 0.9 = honest) pentru mediană
    //   3. Calculăm MEDIANA PONDERATĂ (weight = reputația sender-ului)
    //      Aceasta e consensul neural emergent al rețelei
    //   4. Eroarea per nod (pentru reputație) =
    //        L2_distance(prediction, median_prediction)     ← componenta neurală
    //      + TIP_DIFF_PENALTY × (număr tips diferite)       ← componenta structurală
    //      + STATE_ROOT_PENALTY × (state_root diferit)      ← componenta de stare
    //
    // Astfel, consensul emergent combină:
    //   - aspectul NEURAL (ce predică rețeaua prin Hebbian+median)
    //   - aspectul STRUCTURAL (ce tips agree rețeaua)
    //   - aspectul de STARE (ce balanțe agree rețeaua)
    pub fn compute_consensus(
        &self,
        proposals: &[DagProposal],
        rep_map: &HashMap<String, f64>,
    ) -> (DagConsensus, HashMap<String, f64>) {
        let mut errors: HashMap<String, f64> = HashMap::new();

        // v3.5.3 FIX: Returnam un DagConsensus cu median_prediction = zeros(DIM),
        // NU DagConsensus::default() (care are Array1::default() = shape [0] = empty).
        // Array1 gol duce la PANIC la urmatorul generate_prediction cand
        // euclidean(signal_DIM4, empty_median) da ShapeError.
        let empty_consensus = DagConsensus {
            median_prediction: Array1::zeros(DIM),
            common_tips: Vec::new(),
            common_state_root: [0u8; 32],
            approved_tx_hashes: Vec::new(),
            proposer_of: HashMap::new(),
        };

        if proposals.is_empty() {
            return (empty_consensus, errors);
        }

        // v3.2: ELIMINĂM filtrul rep > 0.9 — foloșim ALL proposals cu weighted median.
        // Motiv: filtrul crea death spiral — honest nodes care scădeau sub 0.9 erau
        // exclude din mediană → mediana se muta spre atacatori → honest nodes aveau
        // eroare și mai mare → reputație și mai mică → colaps.
        //
        // Weighted median cu weight = reputație (min 0.001 ca să nu excludem complet
        // nodurile cu reputație mică) e suficient. Honest nodes au rep mare → weight mare
        // → influențează mediana. Attackers au rep mic → weight mic → influență mică.
        let valid_proposals: Vec<&DagProposal> = proposals.iter().collect();

        // ── 1. MEDIANA PONDERATĂ a predicțiilor (consens neural) ──
        // v3.4: Cluster-aware Weight Capping
        //
        // Detectăm clustere de predicții identice (coordinated/clone attacks)
        // și cap-am weight-ul total al fiecărui cluster la:
        //   min(cluster_weight, total_weight × CLUSTER_WEIGHT_CAP_RATIO)
        //
        // Asta previne ca 7 atacatori coordonați (41%) cu predicții identice
        // să domine mediana. Honest nodes (cu zgomot natural) nu se grupează
        // în clustere identice, deci nu sunt afectate.
        let predictions: Vec<Array1<f64>> = valid_proposals.iter()
            .map(|p| vec_to_array(&p.prediction))
            .collect();
        let raw_weights: Vec<f64> = valid_proposals.iter()
            .map(|p| rep_map.get(&p.sender).copied().unwrap_or(0.5).max(0.01))
            .collect();

        // v3.4: Cluster detection + weight capping
        let n_props = predictions.len();
        let mut parent_uf: Vec<usize> = (0..n_props).collect();
        fn uf_find(parent: &mut Vec<usize>, x: usize) -> usize {
            let mut root = x;
            while parent[root] != root { root = parent[root]; }
            let mut curr = x;
            while parent[curr] != root {
                let next = parent[curr];
                parent[curr] = root;
                curr = next;
            }
            root
        }
        for i in 0..n_props {
            for j in (i + 1)..n_props {
                let dist = euclidean(&predictions[i], &predictions[j]);
                if dist < CLUSTER_CAPPING_EPSILON {
                    let ri = uf_find(&mut parent_uf, i);
                    let rj = uf_find(&mut parent_uf, j);
                    if ri != rj { parent_uf[ri] = rj; }
                }
            }
        }
        // Calculăm weight-ul per cluster și aplicăm cap
        let total_weight: f64 = raw_weights.iter().sum();
        let cluster_cap = total_weight * CLUSTER_WEIGHT_CAP_RATIO;
        let mut cluster_weight: HashMap<usize, f64> = HashMap::new();
        for i in 0..n_props {
            let root = uf_find(&mut parent_uf, i);
            *cluster_weight.entry(root).or_insert(0.0) += raw_weights[i];
        }
        let weights: Vec<f64> = (0..n_props).map(|i| {
            let root = uf_find(&mut parent_uf, i);
            let cw = cluster_weight[&root];
            if cw > cluster_cap {
                // Scalează weight-ul individual proporțional ca cluster-ul total
                // să nu depășească cluster_cap
                raw_weights[i] * (cluster_cap / cw)
            } else {
                raw_weights[i]
            }
        }).collect();

        let median_prediction = if predictions.is_empty() {
            Array1::zeros(DIM)
        } else {
            weighted_median_arrays(&predictions, &weights)
        };

        // ── 2. Tips: vote count (ponderat de reputație, cu cluster capping) ──
        // v3.4: Folosim weights-ul cu cluster capping (nu raw_weights)
        let mut tip_votes: HashMap<Hash, f64> = HashMap::new();
        for (idx, p) in valid_proposals.iter().enumerate() {
            let w = weights[idx];  // v3.4: capped weight
            for tip in &p.proposed_tips {
                *tip_votes.entry(*tip).or_insert(0.0) += w;
            }
        }
        let mut tip_list: Vec<(Hash, f64)> = tip_votes.into_iter().collect();
        tip_list.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let common_tips: Vec<Hash> = tip_list.into_iter()
            .take(PROPOSAL_TIPS_COUNT)
            .map(|(h, _)| h)
            .collect();
        let common_tips_set: HashSet<Hash> = common_tips.iter().copied().collect();

        // ── 3. State root: cel mai frecvent (ponderat, cu cluster capping) ──
        let mut sr_votes: HashMap<Hash, f64> = HashMap::new();
        for (idx, p) in valid_proposals.iter().enumerate() {
            let w = weights[idx];  // v3.4: capped weight
            *sr_votes.entry(p.state_root).or_insert(0.0) += w;
        }
        let common_state_root = sr_votes.iter()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(k, _)| *k)
            .unwrap_or([0u8; 32]);

        // ── 4. Tx aprobate ──
        let mut tx_votes: HashMap<Hash, Vec<String>> = HashMap::new();
        let mut proposer_of: HashMap<Hash, String> = HashMap::new();
        for p in &valid_proposals {
            for tx_hash in &p.seen_tx_hashes {
                tx_votes.entry(*tx_hash).or_default().push(p.sender.clone());
                proposer_of.entry(*tx_hash).or_insert_with(|| p.sender.clone());
            }
        }
        let threshold_count = (valid_proposals.len() as f64 * BATCH_APPROVAL_THRESHOLD).ceil() as usize;
        let approved_tx_hashes: Vec<Hash> = tx_votes.iter()
            .filter(|(_, voters)| voters.len() >= threshold_count)
            .map(|(h, _)| *h)
            .collect();

        // ── 5. Calcul eroare per nod ──
        // v3.3: Soft error function (tanh) + adaptive normalization
        //
        // Soft error: raw L2 → tanh(L2 / SOFT_ERROR_SCALE)
        //   - Cap-ează eroarea maximă la 1.0 (preventing single-step spikes)
        //   - Honest nodes (L2 ~0.03) → tanh(0.1) ≈ 0.1 (eroare mică)
        //   - Attackers (L2 ~0.5) → tanh(1.67) ≈ 0.93 (eroare mare dar nu catastrofală)
        //
        // Adaptive normalization: împart eroarea la mediana distanțelor pairwise
        //   - Honest nodes (natural apropiate) → normalizator mic → eroare ~ normal
        //   - Dacă toată lumea e departe (ex: 2 clustere), normalizator e mare
        //     → eroările se normalizează → honest nodes nu sunt supra-penalizați

        // Calculăm mediana distanțelor pairwise pentru normalizare
        let normalization_factor = if ERROR_NORMALIZATION && predictions.len() > 1 {
            let mut pairwise_dists: Vec<f64> = Vec::new();
            for i in 0..predictions.len() {
                for j in (i + 1)..predictions.len() {
                    pairwise_dists.push(euclidean(&predictions[i], &predictions[j]));
                }
            }
            if pairwise_dists.is_empty() {
                1.0
            } else {
                pairwise_dists.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let mid = pairwise_dists.len() / 2;
                pairwise_dists[mid].max(0.01) // min 0.01 ca să evităm div/0
            }
        } else {
            1.0
        };

        for p in proposals {
            let mut err = 0.0;
            // Componenta NEURALĂ: soft error (tanh) + normalizare adaptivă
            if !p.prediction.is_empty() {
                let pred = vec_to_array(&p.prediction);
                let raw_l2 = euclidean(&pred, &median_prediction);
                let normalized_l2 = raw_l2 / normalization_factor;
                let soft_err = normalized_l2.tanh() * SOFT_ERROR_SCALE;
                err += soft_err;
            }
            // Componenta STRUCTURALĂ: penalty normalizat
            let n_tips = p.proposed_tips.len().max(1);
            let tip_penalty = TIP_DIFF_PENALTY / n_tips as f64;
            for tip in &p.proposed_tips {
                if !common_tips_set.contains(tip) {
                    err += tip_penalty;
                }
            }
            // Componenta de STARE (v3.2: redus la 0.15)
            if p.state_root != common_state_root {
                err += STATE_ROOT_PENALTY;
            }
            errors.insert(p.sender.clone(), err);
        }

        let consensus = DagConsensus {
            median_prediction,
            common_tips,
            common_state_root,
            approved_tx_hashes,
            proposer_of,
        };
        (consensus, errors)
    }

    // ─── Etapa 1: Finalizare în lot ───────────────────────────────────
    /// Finalizează tx-urile aprobate (din consens) într-un batch.
    /// Pentru fiecare tx:
    ///   1. Verifică din nou semnătura și soldul (defensiv).
    ///   2. Aplică tx pe stare.
    ///   3. Înregistrează reward pentru proposer (100% fee).
    /// Returnează (hash-uri finalizate, recompensă totală distribuită).
    ///
    /// v3.5.4 FIX: Tracking pentru tx-uri cu insufficient balance.
    /// Dacă o tx eșuează de MAX_FINALIZE_RETRIES ori la validarea balanței,
    /// e eliminată din mempool (altfel umple log-ul cu "Skipping ..." la infinit).
    pub fn finalize_batch(
        &mut self,
        consensus: &DagConsensus,
    ) -> (Vec<Hash>, u64) {
        const MAX_FINALIZE_RETRIES: u32 = 3;

        let mut to_finalize: Vec<Transaction> = Vec::new();
        let mut reward_map: HashMap<Hash, String> = HashMap::new();
        let mut total_rewards = 0u64;
        let mut to_evict: Vec<Hash> = Vec::new();  // tx-uri de eliminat (prea multe retry-uri)

        for hash in &consensus.approved_tx_hashes {
            // Verificăm dacă tx e încă în mempool (poate a fost eliminat între timp)
            let Some(tx) = self.mempool.get(hash) else { continue; };
            let tx = tx.clone();

            // Validare defensivă
            if !tx.verify_signature() {
                to_evict.push(*hash);
                continue;
            }
            if !self.state.has_balance(&tx.sender, tx.total_cost()) {
                // v3.5.4: Tracking retry count pentru tx-urile fără balanță.
                // Dacă depășesc MAX_FINALIZE_RETRIES, le eliminăm din mempool.
                let count = self.finalize_retry_count.entry(*hash).or_insert(0);
                *count += 1;
                if *count >= MAX_FINALIZE_RETRIES {
                    println!("[DagLogic] Evicting {}: insufficient balance after {} retries",
                             hash_to_short(hash), *count);
                    to_evict.push(*hash);
                } else {
                    // Log doar la prima încercare, ca să nu spam console
                    if *count == 1 {
                        println!("[DagLogic] Skipping {}: insufficient balance (retry {}/{})",
                                 hash_to_short(hash), *count, MAX_FINALIZE_RETRIES);
                    }
                }
                continue;
            }

            to_finalize.push(tx.clone());
            if let Some(proposer) = consensus.proposer_of.get(hash) {
                reward_map.insert(*hash, proposer.clone());
                total_rewards += tx.fee;
            }
        }

        // Evict tx-urile care au eșuat de prea multe ori
        if !to_evict.is_empty() {
            let _removed = self.mempool.remove_batch(&to_evict);
            for h in &to_evict {
                self.finalize_retry_count.remove(h);
            }
        }

        if to_finalize.is_empty() {
            return (Vec::new(), 0);
        }

        // Aplicăm starea (debit + credit) înainte de a adăuga în ledger
        for tx in &to_finalize {
            if !self.state.apply_tx(tx) {
                println!("[DagLogic] State apply failed for tx {:?}", &tx.hash[..4]);
            }
        }

        // Reward transactions creditează contul proposerilor
        for (tx_hash, proposer) in &reward_map {
            if let Some(tx) = to_finalize.iter().find(|t| &t.hash == tx_hash) {
                self.state.apply_reward(proposer, tx.fee);
            }
        }

        // Adăugăm în ledger ca batch
        let hashes = self.ledger.add_batch(to_finalize, &reward_map);

        // Eliminăm din mempool
        let _removed = self.mempool.remove_batch(&consensus.approved_tx_hashes);

        (hashes, total_rewards)
    }

    // ─── Pasul 5: Finality Threshold Dinamic ──────────────────────────
    pub fn compute_dynamic_threshold(active_count: usize, reputations: &HashMap<String, f64>) -> f64 {
        let participation = (active_count as f64 / EXPECTED_ACTIVE_NODES.max(1) as f64).clamp(0.0, 1.0);
        let participation_boost = 0.2 * participation;
        let variability_relief = if reputations.is_empty() { 0.0 } else {
            let mean: f64 = reputations.values().sum::<f64>() / reputations.len() as f64;
            if mean <= 0.0 { 0.0 } else {
                let variance: f64 = reputations.values()
                    .map(|r| (r - mean).powi(2))
                    .sum::<f64>() / reputations.len() as f64;
                let cv = (variance.sqrt() / mean).min(1.0);
                -0.1 * cv
            }
        };
        (FINALITY_BASE_THRESHOLD + participation_boost + variability_relief)
            .clamp(FINALITY_MIN_THRESHOLD, FINALITY_MAX_THRESHOLD)
    }

    /// Decide dacă se finalizează în acest pas (la fiecare FINALIZATION_INTERVAL).
    pub fn should_finalize(step: u64) -> bool {
        step > 0 && step % FINALIZATION_INTERVAL == 0
    }

    /// v2.2: Aplică un snapshot de la un peer (pentru bootstrap).
    /// Suprascrie complet ledger-ul și starea locală.
    pub fn apply_snapshot(&mut self, snap: &crate::snapshot::SnapshotResponse) -> (usize, usize) {
        // Ledger
        self.ledger.entries.clear();
        self.ledger.dag_edges.clear();
        self.ledger.finalized.clear();
        for entry in &snap.ledger.entries {
            let h = entry.tx.hash;
            self.ledger.finalized.insert(h, entry.tx.clone());
            self.ledger.entries.insert(h, entry.clone());
        }
        for (h, children) in &snap.ledger.dag_edges {
            self.ledger.dag_edges.insert(*h, children.clone());
        }
        self.ledger.next_batch_id = snap.ledger.next_batch_id;
        self.ledger.rebuild_children_cache_pub();
        self.ledger.save_to_disk_pub();

        // State
        self.state.balances.clear();
        for (addr, bal) in &snap.state.balances {
            self.state.balances.insert(addr.clone(), *bal);
        }
        self.state.save_to_disk_pub();

        (self.ledger.entries.len(), self.state.balances.len())
    }
}

fn hash_to_short(h: &Hash) -> String {
    format!("{:?}", &h[..4])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_deterministic() {
        let h = [42u8; 32];
        assert_eq!(DagLogic::embed_deterministic(&h), DagLogic::embed_deterministic(&h));
    }

    #[test]
    fn test_dynamic_threshold_bounds() {
        let mut reps = HashMap::new();
        reps.insert("a".to_string(), 0.8);
        reps.insert("b".to_string(), 0.7);
        let t = DagLogic::compute_dynamic_threshold(2, &reps);
        assert!(t >= FINALITY_MIN_THRESHOLD && t <= FINALITY_MAX_THRESHOLD);
    }

    #[test]
    fn test_consensus_empty_proposals() {
        let dl = DagLogic::new(100, "/tmp/ng_test_empty");
        let rep = HashMap::new();
        let (c, errs) = dl.compute_consensus(&[], &rep);
        assert!(c.common_tips.is_empty());
        assert!(errs.is_empty());
    }
}
