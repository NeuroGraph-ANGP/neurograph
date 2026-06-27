use std::collections::{HashMap, VecDeque};
use ndarray::Array1;

use crate::config::{
    DIM, BOOTSTRAP_STEPS, GRACE_PERIOD, WINDOW_SIZE,
    SIGNAL_NOISE_STD, SIGNAL_AMPLITUDE, SIGNAL_BASELINE,
    SIGNAL_PHASE_OFFSET, SIGNAL_EMA_ALPHA, POW_DIFFICULTY,
    HEBBIAN_BASE_ALPHA, HEBBIAN_MIN_ALPHA, HEBBIAN_ALPHA_AGREEMENT_POWER, HEBBIAN_AGREEMENT_NORM,
    PREDICTIVE_MOMENTUM_HISTORY,
};
use crate::attack::AttackType;
use crate::reputation::ReputationEngine;
use crate::security::{mine_pow, verify_pow};
use crate::transaction::{PublicKey, Hash};
use crate::dag::AdaptiveDag;
use crate::dag_logic::{DagProposal, DagConsensus};
use crate::utils::euclidean;

/// ════════════════════════════════════════════════════════════════════
/// NeuroGraph Node — AdaptiveDag Hebbian este "creierul" central.
/// ════════════════════════════════════════════════════════════════════
///
/// Fluxul neural activ (la fiecare pas):
///   1. `generate_prediction(t, step)`:
///      a. Generează `real_signal(t, step)` → semnal onest Array1<f64> 4D
///         (sinusoidă lentă + zgomot mic + EMA smoothing pentru honest)
///      b. Adaugă semnalul în `AdaptiveDag.create_node` ca nod nou
///         cu părinți = ultimele noduri din DAG-ul Hebbian intern
///      c. Aplică `AdaptiveDag.update` — învățare Hebbiană:
///         weight[i] += α × (similarity - weight[i])
///         unde similarity = 1 / (1 + dist(pred, parent[i]))
///      d. Returnează `AdaptiveDag.predict` — predicția agregată ponderat Hebbian
///
///   2. Predicția (vector 4D) devine "votul" nodului și e inclusă în DagProposal
///
///   3. La recepție: propunerile peer-ilor se colectează, se calculează
///      mediană ponderată de reputație a voturilor (consens emergent)
///
///   4. Eroarea pentru reputație = L2 distance (predicție vs median)
///      + penalty pentru tips diferite + penalty pentru state_root diferit
///
/// Astfel AdaptiveDag (Hebbian) decide CE predice nodul, iar consensul
/// emergent (mediană ponderată) decide CE e "adevărat" pentru rețea.
pub struct AngpNode {
    pub id: String,
    pub attack_type: AttackType,
    pub is_offline: bool,
    pub pow_nonce: u64,

    /// ─── CREEIERUL NEURAL: AdaptiveDag Hebbian ────────────────────
    pub dag: AdaptiveDag,

    /// Propuneri primite de la peer-i: sender → queue de (step, DagProposal)
    pub received_proposals: HashMap<String, VecDeque<(u64, DagProposal)>>,
    /// Ultimul pas în care am primit de la fiecare sender
    pub last_seen_step: HashMap<String, u64>,
    /// Engine de reputație dual-EMA
    pub reputation_engine: ReputationEngine,
    /// Consensul curent (calculat la ultimul pas)
    pub last_consensus: DagConsensus,
    /// Contor secvență mesaje proprii
    pub next_seq: u64,
    /// Cheia publică
    pub public_key: Option<PublicKey>,
    /// Ultimul semnal EMA-smoothed (pentru real_signal)
    last_signal: Option<Array1<f64>>,
    /// ID-ul ultimului nod creat în AdaptiveDag
    last_dag_node_id: Option<String>,

    /// v2.4 #3: Istoric recent al tips-urilor comune din consens.
    /// Folosit pentru momentum în predictive tip selection.
    recent_common_tips: VecDeque<Vec<Hash>>,
    /// v2.4 #5: Ultimul α adaptiv folosit (pentru logging/debugging)
    pub last_adaptive_alpha: f64,

    /// v3.5.11: Cache pentru predicție — dacă generate_prediction e chemat
    /// de mai multe ori pentru același step (ex: 3 shards), returnează cache.
    last_prediction_step: u64,
    cached_prediction: Option<Array1<f64>>,
}

impl AngpNode {
    pub fn new(name: String, attack_type: AttackType) -> Self {
        let pow_nonce = mine_pow(&name, POW_DIFFICULTY);
        assert!(verify_pow(pow_nonce, &name, POW_DIFFICULTY), "PoW verification failed");
        AngpNode {
            id: name,
            attack_type,
            is_offline: false,
            pow_nonce,
            dag: AdaptiveDag::new(),
            received_proposals: HashMap::new(),
            last_seen_step: HashMap::new(),
            reputation_engine: ReputationEngine::new(),
            last_consensus: DagConsensus::default(),
            next_seq: 0,
            public_key: None,
            last_signal: None,
            last_dag_node_id: None,
            recent_common_tips: VecDeque::new(),
            last_adaptive_alpha: HEBBIAN_BASE_ALPHA,
            last_prediction_step: u64::MAX,  // v3.5.11: invalid step = cache miss
            cached_prediction: None,
        }
    }

    /// ════════════════════════════════════════════════════════════════
    /// CREEIERUL NEURAL — generează predicția pentru pasul curent.
    /// ════════════════════════════════════════════════════════════════
    ///
    /// v2.4 — adaptive learning rate (#5):
    ///   1. Generează semnalul onest (real_signal)
    ///   2. Calculează AGREEMENT cu ultimul consens emergent:
    ///        agreement = max(0, 1 - L2(signal, consensus_median) / NORM)
    ///   3. Derivează α adaptiv:
    ///        α = MIN + (BASE - MIN) × agreement^POWER
    ///      Honest (signal ≈ consensus): α mare → consolidare rapidă
    ///      Attacker (signal ≠ consensus): α mic → consolidare lentă
    ///   4. create_node + update_with_alpha în AdaptiveDag
    ///   5. predict → predicția agregată (votul neural)
    pub fn generate_prediction(&mut self, t: f64, step: u64) -> Array1<f64> {
        // v3.5.11: Cache — dacă am mai calculat predicția pentru acest step, returnează cache.
        // Acest cache e SIGUR pentru că: la același step, (t, step) sunt identice,
        // deci predicția ar fi identică oricum. Salvăm doar CPU.
        if step == self.last_prediction_step {
            if let Some(ref cached) = self.cached_prediction {
                return cached.clone();
            }
        }

        if self.is_offline {
            return Array1::zeros(DIM);
        }

        // 1. Semnalul onest (sau perturbat, în funcție de attack_type)
        let signal = self.real_signal(t, step);

        // 2. Calcul agreement cu ultimul consens (v2.4 #5)
        // v3.5.3 FIX: Verificam si shape-ul, nu doar sum > 0.
        // Daca consensus_median are shape [0] (empty, de la Default), evitam panic.
        let consensus_median = &self.last_consensus.median_prediction;
        let agreement = if consensus_median.len() == DIM && consensus_median.iter().sum::<f64>() > 0.0 {
            let dist = euclidean(&signal, consensus_median);
            (1.0 - dist / HEBBIAN_AGREEMENT_NORM).max(0.0).min(1.0)
        } else {
            // Bootstrap — nu avem consens încă, agreement neutru = 0.5
            0.5
        };

        // 3. α adaptiv (v2.4 #5)
        let alpha = HEBBIAN_MIN_ALPHA
            + (HEBBIAN_BASE_ALPHA - HEBBIAN_MIN_ALPHA) * agreement.powf(HEBBIAN_ALPHA_AGREEMENT_POWER);
        self.last_adaptive_alpha = alpha;

        // 4. Adăugăm ca nod nou în AdaptiveDag (chain simplu)
        let parents: Vec<String> = match &self.last_dag_node_id {
            Some(id) => vec![id.clone()],
            None => Vec::new(),
        };
        let node_id = self.dag.create_node(&parents, signal.clone());

        // 5. Învățare Hebbiană cu α adaptiv
        self.dag.update_with_alpha(&node_id, signal.clone(), alpha);

        // 6. Predicție agregată ponderat Hebbian
        let prediction = self.dag.predict(&node_id).unwrap_or(signal.clone());

        self.last_dag_node_id = Some(node_id);

        // v3.5.11: Pruning AdaptiveDag — păstrăm doar ultimele 50 de noduri.
        // Asta previne creșterea liniară a memoriei + menține predict() O(1).
        self.dag.prune(50);

        // v3.5.11: Salvăm cache pentru acest step
        self.last_prediction_step = step;
        self.cached_prediction = Some(prediction.clone());

        prediction
    }

    /// v2.4 #3: Înregistrează tips-urile comune din consensul curent.
    /// Apelat din main loop după ce `set_consensus` e apelat.
    /// Folosit pentru momentum în `select_tips_predictive`.
    pub fn record_common_tips(&mut self, tips: Vec<Hash>) {
        self.recent_common_tips.push_back(tips);
        while self.recent_common_tips.len() > PREDICTIVE_MOMENTUM_HISTORY {
            self.recent_common_tips.pop_front();
        }
    }

    /// v2.4 #3: Returnează istoricul recent al tips-urilor comune.
    /// Folosit de `DagLogic::select_tips_predictive` pentru momentum.
    pub fn get_recent_common_tips(&self) -> &VecDeque<Vec<Hash>> {
        &self.recent_common_tips
    }

    /// Adaugă o propunere de la un peer.
    pub fn add_remote_proposal(&mut self, proposal: DagProposal) {
        let sender = proposal.sender.clone();
        let step = proposal.step;
        let queue = self.received_proposals
            .entry(sender.clone())
            .or_insert_with(VecDeque::new);
        queue.push_back((step, proposal));
        while queue.len() > WINDOW_SIZE { queue.pop_front(); }
        self.last_seen_step.insert(sender, step);
    }

    /// v3.5.12 FIX: Adaugă propria propunere la received_proposals.
    /// Într-o rețea P2P reală, fiecare nod își cunoaște propria propunere.
    /// Bug-ul v3.5.11: nodul nu se observa pe sine → honest0 apărea cu rep=0
    /// în propriul reputation_engine, deși toate celelalte noduri îl vedeau corect.
    pub fn add_own_proposal(&mut self, proposal: DagProposal) {
        let sender = proposal.sender.clone();
        let step = proposal.step;
        let queue = self.received_proposals
            .entry(sender.clone())
            .or_insert_with(VecDeque::new);
        queue.push_back((step, proposal));
        while queue.len() > WINDOW_SIZE { queue.pop_front(); }
        self.last_seen_step.insert(sender, step);
    }

    /// Actualizează reputațiile pe baza consensului calculat de `DagLogic::compute_consensus`.
    /// `errors` = mapping sender → eroare (calculată în DagLogic pe baza L2 + tips + state_root).
    ///
    /// v3.5.12 FIX: nodul își actualizează și PROPRIA reputație pe baza erorii sale.
    /// Bug-ul v3.5.11: nodul nu se observa pe sine în received_proposals →
    /// propria reputație nu se actualiza niciodată cu erorile reale.
    pub fn update_reputations(&mut self, current_step: u64, errors: &HashMap<String, f64>) {
        if self.is_offline { return; }

        let mut active_senders = Vec::new();
        // v3.5.12 FIX: includem self.id în senderii activi
        active_senders.push(self.id.clone());
        self.last_seen_step.insert(self.id.clone(), current_step);
        for (sender, _) in self.received_proposals.iter() {
            let last_seen = self.last_seen_step.get(sender).copied().unwrap_or(0);
            if current_step >= last_seen && current_step - last_seen <= GRACE_PERIOD {
                active_senders.push(sender.clone());
            }
        }

        // Bootstrap: dacă suntem în BOOTSTRAP_STEPS și nu avem erori, dăm reputație de pornire
        if errors.is_empty() && current_step < BOOTSTRAP_STEPS {
            for sender in &active_senders {
                self.reputation_engine.update_reputation(sender, 0.0, current_step);
            }
            return;
        }

        for sender in &active_senders {
            let err = errors.get(sender).copied().unwrap_or(0.5);
            let new_rep = self.reputation_engine.update_reputation(sender, err, current_step);
            if new_rep < 0.2 && new_rep > 0.0 {
                println!("[{}] WARNING: {} reputation dropped to {:.3}", self.id, sender, new_rep);
            }
        }
    }

    /// Returnează propunerile primite la pasul curent (doar de la senderi activi).
    pub fn collect_active_proposals(&self, current_step: u64) -> Vec<DagProposal> {
        let mut out = Vec::new();
        for (sender, queue) in self.received_proposals.iter() {
            let last_seen = self.last_seen_step.get(sender).copied().unwrap_or(0);
            if current_step >= last_seen && current_step - last_seen <= GRACE_PERIOD {
                if let Some((_, p)) = queue.back() {
                    out.push(p.clone());
                }
            }
        }
        out
    }

    /// ════════════════════════════════════════════════════════════════
    /// SEMNALUL ONEST — sursa de "adevăr" pe care AdaptiveDag învață.
    /// ════════════════════════════════════════════════════════════════
    ///
    /// Honest: sinusoidă lentă 4D + zgomot mic gaussian + EMA smoothing
    /// Atacatori: perturbări specifice (coordinated, random, gaussian, flipflop, etc.)
    ///
    /// Acest semnal alimentează AdaptiveDag.create_node la fiecare pas.
    /// AdaptiveDag învață din istoric (Hebbian) și produce o predicție agregată
    /// care e mai stabilă decât semnalul brut.
    pub fn real_signal(&mut self, t: f64, _step: u64) -> Array1<f64> {
        if self.is_offline { return Array1::zeros(DIM); }

        // Baza onestă: sinusoidă lentă 4D
        let mut signal = Array1::zeros(DIM);
        for i in 0..DIM {
            let phase = (i as f64) * SIGNAL_PHASE_OFFSET;
            signal[i] = SIGNAL_BASELINE + SIGNAL_AMPLITUDE * (t + phase).sin();
        }

        // Perturbare în funcție de attack_type
        let mut rng = rand::thread_rng();
        match self.attack_type {
            AttackType::Honest => {
                // Zgomot mic gaussian (σ=SIGNAL_NOISE_STD)
                let normal = rand_distr::Normal::new(0.0, SIGNAL_NOISE_STD).unwrap();
                use rand_distr::Distribution;
                for i in 0..DIM { signal[i] += normal.sample(&mut rng); }
            }
            AttackType::Coordinated => {
                for i in 0..DIM { signal[i] = 0.9; }
            }
            AttackType::RandomNoise => {
                use rand::Rng;
                for i in 0..DIM { signal[i] = rng.gen_range(0.0..1.0); }
            }
            AttackType::GaussianNoise => {
                let normal = rand_distr::Normal::new(0.5, 0.2).unwrap();
                use rand_distr::Distribution;
                for i in 0..DIM { signal[i] = normal.sample(&mut rng); }
            }
            AttackType::FlipFlop => {
                let high = (_step / 10) % 2 == 0;
                let val = if high { 0.9 } else { 0.1 };
                for i in 0..DIM { signal[i] = val; }
            }
            AttackType::Sleeper => {
                if _step > 1000 { for i in 0..DIM { signal[i] = 0.95; } }
            }
            AttackType::Drift => {
                let drift = (_step as f64) * 0.0001;
                for i in 0..DIM { signal[i] = (signal[i] + drift).min(1.0); }
            }
            AttackType::OutlierBurst => {
                use rand::Rng;
                if rng.gen_range(0.0..1.0) < 0.05 { for i in 0..DIM { signal[i] = 1.0; } }
            }
            AttackType::Adaptive => {
                let consensus_mean: f64 = self.last_consensus.median_prediction.mean().unwrap_or(0.5);
                if (consensus_mean - 0.5).abs() < 0.1 { for i in 0..DIM { signal[i] = 0.95; } }
            }
            AttackType::Clone => {
                // Copiază predicția ultimului peer primit
                if let Some(last_prop) = self.received_proposals.values()
                    .flat_map(|q| q.back())
                    .last() {
                    if !last_prop.1.prediction.is_empty() {
                        for i in 0..DIM.min(last_prop.1.prediction.len()) {
                            signal[i] = last_prop.1.prediction[i];
                        }
                    }
                }
            }
            AttackType::Sybil => {
                let normal = rand_distr::Normal::new(0.0, 0.02).unwrap();
                use rand_distr::Distribution;
                for i in 0..DIM { signal[i] += normal.sample(&mut rng); }
            }
        }

        // Clamp la [0, 1]
        for i in 0..DIM { signal[i] = signal[i].clamp(0.0, 1.0); }

        // EMA smoothing — DOAR pentru noduri oneste
        // (ca să nu mascheze atacurile)
        if self.attack_type == AttackType::Honest {
            let smoothed = match &self.last_signal {
                None => signal.clone(),
                Some(prev) => &signal * SIGNAL_EMA_ALPHA + prev * (1.0 - SIGNAL_EMA_ALPHA),
            };
            self.last_signal = Some(smoothed.clone());
            smoothed
        } else {
            signal
        }
    }

    pub fn get_consensus(&self) -> &DagConsensus { &self.last_consensus }
    pub fn set_consensus(&mut self, c: DagConsensus) {
        // v3.5.11: Invalidam cache-ul de predicție când consensul se schimbă.
        // Asta forțează recalcularea la următorul generate_prediction().
        self.cached_prediction = None;
        self.last_consensus = c;
    }

    /// Returnează reputația unui sender din perspectiva acestui nod.
    /// v3.5.12 FIX: dacă sender == self.id, returnează reputația proprie.
    pub fn get_reputation(&self, sender: &str) -> Option<f64> {
        self.reputation_engine.get_reputation(sender)
    }

    /// v3.5.12 FIX: include self.id în reputațiile returnate.
    /// Bug-ul v3.5.11: nodul nu se includea pe sine în rep_map → honest0
    /// avea reputație None în propria perspectivă, deci compute_consensus
    /// îl trata cu reputație implicită 0.
    pub fn get_all_reputations(&self) -> HashMap<String, f64> {
        let mut out = HashMap::new();
        // v3.5.12 FIX: includem propria reputație
        if let Some(rep) = self.reputation_engine.get_reputation(&self.id) {
            out.insert(self.id.clone(), rep);
        }
        for sender in self.received_proposals.keys() {
            if let Some(rep) = self.reputation_engine.get_reputation(sender) {
                out.insert(sender.clone(), rep);
            }
        }
        out
    }

    pub fn get_message_count(&self, sender: &str) -> usize {
        self.received_proposals.get(sender).map(|q| q.len()).unwrap_or(0)
    }

    pub fn get_last_proposal(&self, sender: &str) -> Option<&DagProposal> {
        self.received_proposals.get(sender)
            .and_then(|q| q.back())
            .map(|(_, p)| p)
    }

    /// Numărul de noduri din AdaptiveDag (pentru status/debug)
    pub fn dag_node_count(&self) -> usize {
        self.dag.order.len()
    }
}
