//! v3.2 — Attack Detection: Clone, Coordination, Adaptive.
//!
//! Module care detectează și penalizează automat:
//!   1. **Clone attackers** — noduri care copiază predicțiile altor noduri
//!   2. **Coordinated clusters** — N noduri cu predicții identice (Sybil/coordination)
//!   3. **Adaptive attackers** — noduri care atacă doar când consens e aproape de 0.5
//!
//! IMPORTANT: Acest modul NU schimbă protocolul, DAG-ul Hebbian, sau consensul
//! emergent. Doar adaugă penalități la eroarea de reputație pentru noduri
//! detectate ca atacatoare.

use std::collections::HashMap;
use std::collections::VecDeque;
use crate::config::{
    CLONE_EPSILON, CLONE_STREAK_THRESHOLD, CLONE_PENALTY_MULTIPLIER,
    COORDINATION_EPSILON, COORDINATION_MIN_CLUSTER, COORDINATION_PENALTY,
    ADAPTIVE_ZONE, ADAPTIVE_DETECTION_WINDOW, ADAPTIVE_PENALTY,
    ADAPTIVE_ERR_HIGH, ADAPTIVE_ERR_LOW,
    DIM,
    TEMPORAL_WINDOW, TEMPORAL_INCONSISTENCY_THRESHOLD, TEMPORAL_PENALTY,
    PREDICTION_JUMP_THRESHOLD, PREDICTION_JUMP_STREAK, PREDICTION_JUMP_PENALTY,
};
use crate::dag_logic::DagProposal;
use crate::utils::euclidean;
use ndarray::Array1;

/// ════════════════════════════════════════════════════════════════════
/// Clone Detector
/// ════════════════════════════════════════════════════════════════════
///
/// Detectează noduri care copiază predicțiile altor noduri.
///
/// Mecanism:
///   - La fiecare pas, comparăm predicțiile tuturor perechilor de noduri.
///   - Dacă predicția lui B e în CLONE_EPSILON de predicția lui A (L2 distance),
///     B e marcat ca "clone suspect" la pasul curent.
///   - Dacă B e suspect pentru CLONE_STREAK_THRESHOLD pași consecutivi,
///     e confirmat ca clone și primește CLONE_PENALTY_MULTIplier pe eroare.
pub struct CloneDetector {
    /// Câte pași consecutivi un nod a fost clone suspect.
    clone_streak: HashMap<String, u32>,
    /// Set de noduri confirmate ca clone (pentru logging).
    confirmed_clones: HashMap<String, u32>,
}

impl CloneDetector {
    pub fn new() -> Self {
        CloneDetector {
            clone_streak: HashMap::new(),
            confirmed_clones: HashMap::new(),
        }
    }

    /// Analizează propunerile de la pasul curent.
    /// Returnează: set de noduri care sunt clone confirmate la acest pas.
    pub fn detect(&mut self, proposals: &[DagProposal]) -> HashMap<String, f64> {
        if proposals.len() < 2 {
            return HashMap::new();
        }

        // Convertim predicțiile în Array1 pentru comparare
        let predictions: Vec<(String, Array1<f64>)> = proposals.iter()
            .filter(|p| p.prediction.len() == DIM)
            .map(|p| (p.sender.clone(), Array1::from(p.prediction.clone())))
            .collect();

        // Pentru fiecare nod, verificăm dacă e clone al altui nod
        let mut suspects_this_step: std::collections::HashSet<String> = std::collections::HashSet::new();

        for i in 0..predictions.len() {
            for j in 0..predictions.len() {
                if i == j { continue; }
                // Verificăm dacă j e clone al i (j copiază pe i)
                let dist = euclidean(&predictions[j].1, &predictions[i].1);
                if dist < CLONE_EPSILON {
                    // j e suspect de clonare a lui i
                    // Dar nu marcam pe i (cel clonat) ca suspect
                    suspects_this_step.insert(predictions[j].0.clone());
                }
            }
        }

        // Actualizăm streak-ul pentru fiecare nod
        let mut penalties: HashMap<String, f64> = HashMap::new();
        for (node, _) in &predictions {
            let streak = self.clone_streak.entry(node.clone()).or_insert(0);
            if suspects_this_step.contains(node) {
                *streak += 1;
            } else {
                *streak = 0;
            }

            // Dacă streak-ul depășește pragul, e confirmat ca clone
            if *streak >= CLONE_STREAK_THRESHOLD {
                penalties.insert(node.clone(), CLONE_PENALTY_MULTIPLIER);
                *self.confirmed_clones.entry(node.clone()).or_insert(0) += 1;
            }
        }

        penalties
    }

    /// Returnează nodurile confirmate ca clone (pentru logging).
    pub fn confirmed_clones(&self) -> &HashMap<String, u32> {
        &self.confirmed_clones
    }
}

/// ════════════════════════════════════════════════════════════════════
/// Coordination Detector
/// ════════════════════════════════════════════════════════════════════
///
/// Detectează clustere de noduri cu predicții identice (coordinated attack).
///
/// Mecanism:
///   - Grupăm nodurile după predicții (cu toleranța COORDINATION_EPSILON).
///   - Dacă un cluster are ≥ COORDINATION_MIN_CLUSTER noduri, toate
///     nodurile din cluster primesc COORDINATION_PENALTY.
///
/// Notă: honest nodes au zgomot gaussian (σ=0.005), deci predicțiile lor
/// NU sunt identice. Doar atacatorii coordonați au predicții exact egale.
pub struct CoordinationDetector {
    detection_count: HashMap<String, u32>,
}

impl CoordinationDetector {
    pub fn new() -> Self {
        CoordinationDetector {
            detection_count: HashMap::new(),
        }
    }

    /// Detectează clustere coordonate.
    /// Returnează: map node → penalty (COORDINATION_PENALTY pentru nodurile în cluster).
    pub fn detect(&mut self, proposals: &[DagProposal]) -> HashMap<String, f64> {
        if proposals.len() < COORDINATION_MIN_CLUSTER {
            return HashMap::new();
        }

        let predictions: Vec<(String, Array1<f64>)> = proposals.iter()
            .filter(|p| p.prediction.len() == DIM)
            .map(|p| (p.sender.clone(), Array1::from(p.prediction.clone())))
            .collect();

        // Union-Find simplu pentru clustering (iterativ, nu recursiv — evită stack overflow)
        let n = predictions.len();
        let mut parent: Vec<usize> = (0..n).collect();
        // find iterativ
        fn find(parent: &mut Vec<usize>, x: usize) -> usize {
            let mut root = x;
            while parent[root] != root {
                root = parent[root];
            }
            // Path compression
            let mut curr = x;
            while parent[curr] != root {
                let next = parent[curr];
                parent[curr] = root;
                curr = next;
            }
            root
        }

        for i in 0..n {
            for j in (i + 1)..n {
                let dist = euclidean(&predictions[i].1, &predictions[j].1);
                if dist < COORDINATION_EPSILON {
                    let ri = find(&mut parent, i);
                    let rj = find(&mut parent, j);
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }

        // Numărăm noduri per cluster
        let mut cluster_sizes: HashMap<usize, usize> = HashMap::new();
        let mut node_cluster: HashMap<usize, usize> = HashMap::new();
        for i in 0..n {
            let root = find(&mut parent, i);
            *cluster_sizes.entry(root).or_insert(0) += 1;
            node_cluster.insert(i, root);
        }

        // Penalizăm nodurile din clustere mari
        let mut penalties: HashMap<String, f64> = HashMap::new();
        for (i, (node, _)) in predictions.iter().enumerate() {
            let cluster = node_cluster[&i];
            let size = cluster_sizes[&cluster];
            if size >= COORDINATION_MIN_CLUSTER {
                penalties.insert(node.clone(), COORDINATION_PENALTY);
                *self.detection_count.entry(node.clone()).or_insert(0) += 1;
            }
        }

        penalties
    }

    pub fn detection_count(&self) -> &HashMap<String, u32> {
        &self.detection_count
    }
}

/// ════════════════════════════════════════════════════════════════════
/// Adaptive Attacker Detector
/// ════════════════════════════════════════════════════════════════════
///
/// Detectează atacatori care atacă DOAR când consensul e aproape de 0.5.
///
/// Mecanism:
///   - Pentru fiecare nod, păstrăm istoricul (error, |consensus_mean - 0.5|)
///     pe ultimii ADAPTIVE_DETECTION_WINDOW pași.
///   - Dacă un nod are erori mari (error > 0.5) PREDOMINANT când
///     |consensus - 0.5| < ADAPTIVE_ZONE, e marcat ca adaptive.
///   - Concret: calculăm corelația dintre error și 1/(|consensus-0.5|+ε).
///     Dacă corelația e > 0.5, nodul e adaptive.
pub struct AdaptiveDetector {
    /// Pentru fiecare nod: VecDeque de (error, consensus_deviation)
    history: HashMap<String, VecDeque<(f64, f64)>>,
    detection_count: HashMap<String, u32>,
}

impl AdaptiveDetector {
    pub fn new() -> Self {
        AdaptiveDetector {
            history: HashMap::new(),
            detection_count: HashMap::new(),
        }
    }

    /// Înregistrează o observație pentru un nod.
    /// `error` = eroarea nodului la acest pas.
    /// `consensus_mean` = media predicției consensului (pentru a calcula deviația de la 0.5).
    pub fn observe(&mut self, node: &str, error: f64, consensus_mean: f64) {
        let deviation = (consensus_mean - 0.5).abs();
        let hist = self.history.entry(node.to_string()).or_insert_with(VecDeque::new);
        hist.push_back((error, deviation));
        while hist.len() > ADAPTIVE_DETECTION_WINDOW {
            hist.pop_front();
        }
    }

    /// Detectează atacatori adaptive pe baza istoricului acumulat.
    /// Returnează: map node → penalty (ADAPTIVE_PENALTY pentru detected nodes).
    pub fn detect(&mut self) -> HashMap<String, f64> {
        let mut penalties: HashMap<String, f64> = HashMap::new();

        for (node, hist) in &self.history {
            if hist.len() < ADAPTIVE_DETECTION_WINDOW / 2 {
                continue;
            }

            // Calculăm:
            //   - mean error când deviation < ADAPTIVE_ZONE (în zona de atac)
            //   - mean error când deviation >= ADAPTIVE_ZONE (în afara zonei)
            let mut err_in_zone: Vec<f64> = Vec::new();
            let mut err_out_zone: Vec<f64> = Vec::new();

            for &(err, dev) in hist {
                if dev < ADAPTIVE_ZONE {
                    err_in_zone.push(err);
                } else {
                    err_out_zone.push(err);
                }
            }

            // Avem nevoie de enough samples în ambele zone
            if err_in_zone.len() < 10 || err_out_zone.len() < 10 {
                continue;
            }

            let mean_in: f64 = err_in_zone.iter().sum::<f64>() / err_in_zone.len() as f64;
            let mean_out: f64 = err_out_zone.iter().sum::<f64>() / err_out_zone.len() as f64;

            // Adaptive attacker: erori MARI în zonă (când atacă), erori MICI în afara zonei
            // Honest: erori similare în ambele zone (zgomot constant)
            // v3.3: Praguri ajustate pentru soft error (tanh cap-ează la ~0.3)
            if mean_in > ADAPTIVE_ERR_HIGH && mean_out < ADAPTIVE_ERR_LOW && mean_in > mean_out * 2.0 {
                penalties.insert(node.clone(), ADAPTIVE_PENALTY);
                *self.detection_count.entry(node.clone()).or_insert(0) += 1;
            }
        }

        penalties
    }

    pub fn detection_count(&self) -> &HashMap<String, u32> {
        &self.detection_count
    }
}

/// ════════════════════════════════════════════════════════════════════
/// Temporal Consistency Detector (v3.3)
/// ════════════════════════════════════════════════════════════════════
///
/// Detectează atacatori care generează noise (GaussianNoise, RandomNoise).
///
/// Mecanism:
///   - Pentru fiecare nod, păstrăm istoricul predicțiilor pe ultimii
///     TEMPORAL_WINDOW pași.
///   - Calculăm variația medie (L2 între pași consecutivi).
///   - Honest nodes (EMA + sinusoidă lentă) au variație mică (~0.01-0.05).
///   - GaussianNoise (σ=0.2) are variație mare (~0.2-0.4).
///   - RandomNoise (uniform [0,1]) are și mai mare (~0.4-0.6).
///
/// Dacă variația medie depășește TEMPORAL_INCONSISTENCY_THRESHOLD,
/// nodul primește TEMPORAL_PENALTY.
pub struct TemporalConsistencyDetector {
    prediction_history: HashMap<String, VecDeque<Array1<f64>>>,
    detection_count: HashMap<String, u32>,
}

impl TemporalConsistencyDetector {
    pub fn new() -> Self {
        TemporalConsistencyDetector {
            prediction_history: HashMap::new(),
            detection_count: HashMap::new(),
        }
    }

    /// Analizează propunerile de la pasul curent.
    /// Returnează: map node → penalty pentru noduri cu temporal inconsistency.
    pub fn detect(&mut self, proposals: &[DagProposal]) -> HashMap<String, f64> {
        let mut penalties: HashMap<String, f64> = HashMap::new();

        for p in proposals {
            if p.prediction.len() != DIM { continue; }
            let pred = Array1::from(p.prediction.clone());
            let hist = self.prediction_history.entry(p.sender.clone()).or_insert_with(VecDeque::new);
            hist.push_back(pred);
            while hist.len() > TEMPORAL_WINDOW { hist.pop_front(); }

            // Avem nevoie de cel puțin jumătate din window pentru a decide
            if hist.len() < TEMPORAL_WINDOW / 2 { continue; }

            // Calculăm variația medie (L2 între pași consecutivi)
            let preds: Vec<&Array1<f64>> = hist.iter().collect();
            let mut total_dist = 0.0;
            let mut count = 0;
            for i in 1..preds.len() {
                total_dist += euclidean(preds[i], preds[i - 1]);
                count += 1;
            }
            if count == 0 { continue; }
            let avg_variation = total_dist / count as f64;

            if avg_variation > TEMPORAL_INCONSISTENCY_THRESHOLD {
                penalties.insert(p.sender.clone(), TEMPORAL_PENALTY);
                *self.detection_count.entry(p.sender.clone()).or_insert(0) += 1;
            }
        }

        penalties
    }

    pub fn detection_count(&self) -> &HashMap<String, u32> {
        &self.detection_count
    }
}

/// ════════════════════════════════════════════════════════════════════
/// Prediction Jump Detector (v3.4)
/// ════════════════════════════════════════════════════════════════════
///
/// Detectează atacatori adaptive care comută brusc între mod honest și atac.
///
/// Adaptive attacker behavior:
///   - When |consensus - 0.5| > zone: produces honest-like predictions (~0.5)
///   - When |consensus - 0.5| < zone: jumps to attack prediction (~0.95)
///   - This creates large jumps in the prediction time series
///
/// Honest nodes (EMA + slow sinusoid) have smooth predictions — no jumps.
pub struct PredictionJumpDetector {
    last_prediction: HashMap<String, Array1<f64>>,
    jump_streak: HashMap<String, u32>,
    detection_count: HashMap<String, u32>,
}

impl PredictionJumpDetector {
    pub fn new() -> Self {
        PredictionJumpDetector {
            last_prediction: HashMap::new(),
            jump_streak: HashMap::new(),
            detection_count: HashMap::new(),
        }
    }

    pub fn detect(&mut self, proposals: &[DagProposal]) -> HashMap<String, f64> {
        let mut penalties: HashMap<String, f64> = HashMap::new();

        for p in proposals {
            if p.prediction.len() != DIM { continue; }
            let pred = Array1::from(p.prediction.clone());

            if let Some(last) = self.last_prediction.get(&p.sender) {
                let jump = euclidean(&pred, last);
                let streak = self.jump_streak.entry(p.sender.clone()).or_insert(0);
                if jump > PREDICTION_JUMP_THRESHOLD {
                    *streak += 1;
                } else {
                    *streak = 0;
                }
                if *streak >= PREDICTION_JUMP_STREAK {
                    penalties.insert(p.sender.clone(), PREDICTION_JUMP_PENALTY);
                    *self.detection_count.entry(p.sender.clone()).or_insert(0) += 1;
                }
            }
            self.last_prediction.insert(p.sender.clone(), pred);
        }

        penalties
    }

    pub fn detection_count(&self) -> &HashMap<String, u32> {
        &self.detection_count
    }
}

/// ════════════════════════════════════════════════════════════════════
/// Attack Detection Manager — combină toate detectoarele
/// ════════════════════════════════════════════════════════════════════
pub struct AttackDetectionManager {
    pub clone_detector: CloneDetector,
    pub coordination_detector: CoordinationDetector,
    pub adaptive_detector: AdaptiveDetector,
    pub temporal_detector: TemporalConsistencyDetector,
    pub jump_detector: PredictionJumpDetector,
}

impl AttackDetectionManager {
    pub fn new() -> Self {
        AttackDetectionManager {
            clone_detector: CloneDetector::new(),
            coordination_detector: CoordinationDetector::new(),
            adaptive_detector: AdaptiveDetector::new(),
            temporal_detector: TemporalConsistencyDetector::new(),
            jump_detector: PredictionJumpDetector::new(),
        }
    }

    pub fn detect_all(
        &mut self,
        proposals: &[DagProposal],
        errors: &HashMap<String, f64>,
        consensus_mean: f64,
    ) -> HashMap<String, f64> {
        let mut total_penalties: HashMap<String, f64> = HashMap::new();

        let clone_penalties = self.clone_detector.detect(proposals);
        for (node, pen) in &clone_penalties {
            *total_penalties.entry(node.clone()).or_insert(0.0) += pen;
        }

        let coord_penalties = self.coordination_detector.detect(proposals);
        for (node, pen) in &coord_penalties {
            *total_penalties.entry(node.clone()).or_insert(0.0) += pen;
        }

        for (node, &err) in errors {
            self.adaptive_detector.observe(node, err, consensus_mean);
        }
        let adaptive_penalties = self.adaptive_detector.detect();
        for (node, pen) in &adaptive_penalties {
            *total_penalties.entry(node.clone()).or_insert(0.0) += pen;
        }

        let temporal_penalties = self.temporal_detector.detect(proposals);
        for (node, pen) in &temporal_penalties {
            *total_penalties.entry(node.clone()).or_insert(0.0) += pen;
        }

        // v3.4: Prediction Jump detection
        let jump_penalties = self.jump_detector.detect(proposals);
        for (node, pen) in &jump_penalties {
            *total_penalties.entry(node.clone()).or_insert(0.0) += pen;
        }

        total_penalties
    }

    pub fn status(&self) -> String {
        let clones = self.clone_detector.confirmed_clones().len();
        let coord = self.coordination_detector.detection_count().len();
        let adaptive = self.adaptive_detector.detection_count().len();
        let temporal = self.temporal_detector.detection_count().len();
        let jumps = self.jump_detector.detection_count().len();
        format!("clones={}, coordinated={}, adaptive={}, temporal={}, jumps={}",
            clones, coord, adaptive, temporal, jumps)
    }
}

impl Default for AttackDetectionManager {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag_logic::DagProposal;
    use crate::transaction::Hash;

    fn make_proposal(sender: &str, prediction: Vec<f64>) -> DagProposal {
        DagProposal {
            sender: sender.to_string(),
            step: 0, seq: 0, nonce: 0,
            prediction,
            proposed_tips: vec![], state_root: [0u8; 32],
            seen_tx_hashes: vec![], seen_receipts: vec![],
        }
    }

    #[test]
    fn test_clone_detector_detects_copycat() {
        let mut det = CloneDetector::new();
        let original = vec![0.5, 0.5, 0.5, 0.5];
        let clone_pred = vec![0.5001, 0.4999, 0.5001, 0.4999]; // within CLONE_EPSILON

        // Simulăm CLONE_STREAK_THRESHOLD pași
        for _ in 0..CLONE_STREAK_THRESHOLD {
            let proposals = vec![
                make_proposal("honest", original.clone()),
                make_proposal("clone", clone_pred.clone()),
            ];
            let penalties = det.detect(&proposals);
            // Ar trebui să detecteze "clone" după CLONE_STREAK_THRESHOLD pași
        }

        // La ultimul pas, "clone" ar trebui să fie penalizat
        let proposals = vec![
            make_proposal("honest", original.clone()),
            make_proposal("clone", clone_pred.clone()),
        ];
        let penalties = det.detect(&proposals);
        assert!(penalties.contains_key("clone"),
            "Clone should be detected after {} streak steps", CLONE_STREAK_THRESHOLD);
    }

    #[test]
    fn test_clone_detector_ignores_honest_noise() {
        let mut det = CloneDetector::new();
        // Honest nodes au zgomot — predicțiile NU sunt identice
        let p1 = vec![0.50, 0.51, 0.49, 0.50];
        let p2 = vec![0.52, 0.48, 0.51, 0.49]; // L2 ≈ 0.04 > CLONE_EPSILON

        for _ in 0..10 {
            let proposals = vec![
                make_proposal("h1", p1.clone()),
                make_proposal("h2", p2.clone()),
            ];
            let penalties = det.detect(&proposals);
            // Nu ar trebui să detecteze niciun clone
            assert!(!penalties.contains_key("h1") && !penalties.contains_key("h2"),
                "Honest nodes with natural noise should NOT be flagged as clones");
        }
    }

    #[test]
    fn test_coordination_detector_finds_cluster() {
        let mut det = CoordinationDetector::new();
        // 3 noduri cu predicții identice (coordinated)
        let coord_pred = vec![0.9, 0.9, 0.9, 0.9];
        let proposals = vec![
            make_proposal("att1", coord_pred.clone()),
            make_proposal("att2", coord_pred.clone()),
            make_proposal("att3", coord_pred.clone()),
            make_proposal("honest", vec![0.5, 0.5, 0.5, 0.5]),
        ];
        let penalties = det.detect(&proposals);
        assert!(penalties.contains_key("att1"), "Coordinated attacker 1 should be detected");
        assert!(penalties.contains_key("att2"), "Coordinated attacker 2 should be detected");
        assert!(penalties.contains_key("att3"), "Coordinated attacker 3 should be detected");
        assert!(!penalties.contains_key("honest"), "Honest node should NOT be flagged");
    }

    #[test]
    fn test_coordination_detector_ignores_small_groups() {
        let mut det = CoordinationDetector::new();
        // Doar 2 noduri cu predicții similare — sub COORDINATION_MIN_CLUSTER (3)
        let proposals = vec![
            make_proposal("a", vec![0.9, 0.9, 0.9, 0.9]),
            make_proposal("b", vec![0.9, 0.9, 0.9, 0.9]),
            make_proposal("c", vec![0.5, 0.5, 0.5, 0.5]),
            make_proposal("d", vec![0.3, 0.3, 0.3, 0.3]),
        ];
        let penalties = det.detect(&proposals);
        assert!(penalties.is_empty(), "Pairs should not trigger coordination detection");
    }

    #[test]
    fn test_adaptive_detector_pattern() {
        let mut det = AdaptiveDetector::new();

        // Simulăm 100 pași: nodul "adaptive" are erori mari când consens e aproape de 0.5
        // și erori mici când consens e departe de 0.5
        for i in 0..100 {
            let consensus_mean: f64 = if i % 2 == 0 { 0.50 } else { 0.80 };
            let adaptive_err: f64 = if (consensus_mean - 0.5).abs() < ADAPTIVE_ZONE { 0.8 } else { 0.05 };
            let honest_err: f64 = 0.1; // honest are eroare constantă mică

            det.observe("adaptive", adaptive_err, consensus_mean);
            det.observe("honest", honest_err, consensus_mean);
        }

        let penalties = det.detect();
        assert!(penalties.contains_key("adaptive"),
            "Adaptive attacker should be detected");
        assert!(!penalties.contains_key("honest"),
            "Honest node should NOT be flagged as adaptive");
    }
}
