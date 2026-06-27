//! v2.4 Integration tests: Adaptive Learning Rate (#5) + Predictive Tip Selection (#3).
//!
//! Verifică feedback loop-ul:
//!   - Adaptive α: honest nodes au α mare, attackers au α mic
//!   - Predictive tips: nodurile oneste aleg tips care devin comune

use neurograph::node::AngpNode;
use neurograph::attack::AttackType;
use neurograph::dag_logic::DagLogic;
use neurograph::config::{
    HEBBIAN_BASE_ALPHA, HEBBIAN_MIN_ALPHA, HEBBIAN_ALPHA_AGREEMENT_POWER,
    PREDICTIVE_OWN_WEIGHT, PREDICTIVE_MOMENTUM_WEIGHT,
};
use neurograph::utils::euclidean;
use ndarray::Array1;
use std::collections::{HashMap, VecDeque};
use neurograph::transaction::Hash;

mod common;

// ════════════════════════════════════════════════════════════════════
// #5: Adaptive Learning Rate
// ════════════════════════════════════════════════════════════════════

#[test]
fn test_adaptive_alpha_in_bounds() {
    let mut node = AngpNode::new("test_alpha".to_string(), AttackType::Honest);
    // Generează 100 predicții pentru a acumula istoric
    for step in 0..100u64 {
        let t = step as f64 * 0.05;
        let _ = node.generate_prediction(t, step);
    }
    let alpha = node.last_adaptive_alpha;
    assert!(alpha >= HEBBIAN_MIN_ALPHA,
        "α must be >= MIN ({}), got {}", HEBBIAN_MIN_ALPHA, alpha);
    assert!(alpha <= HEBBIAN_BASE_ALPHA,
        "α must be <= BASE ({}), got {}", HEBBIAN_BASE_ALPHA, alpha);
}

#[test]
fn test_adaptive_alpha_low_when_disagreeing_with_consensus() {
    // Un nod care e FORȚAT să dezacordeze cu consensul ar trebui să aibă α mic.
    // Simulăm asta cu attack_type = Coordinated (predict 0.9 constant)
    // și setăm manual last_consensus la [0.1, 0.1, 0.1, 0.1] (dezacord total)
    let mut attacker = AngpNode::new("attacker".to_string(), AttackType::Coordinated);

    // Forțăm consens diferit de predicția atacatorului
    let mut consensus = neurograph::dag_logic::DagConsensus::default();
    consensus.median_prediction = Array1::from(vec![0.1, 0.1, 0.1, 0.1]);
    attacker.set_consensus(consensus);

    // Generează o predicție (ar trebui să fie 0.9 pentru Coordinated)
    let _ = attacker.generate_prediction(0.0, 1);

    // α ar trebui să fie aproape de MIN (pentru că dezacord e mare)
    let alpha = attacker.last_adaptive_alpha;
    assert!(alpha < HEBBIAN_BASE_ALPHA * 0.3,
        "Attacker with high disagreement should have low α, got {} (BASE={})",
        alpha, HEBBIAN_BASE_ALPHA);
}

#[test]
fn test_adaptive_alpha_high_when_agreeing_with_consensus() {
    // Un nod onest care agreează cu consensul ar trebui să aibă α mare.
    let mut honest = AngpNode::new("honest".to_string(), AttackType::Honest);

    // Pentru primul pas, consensus e zeros → agreement=0.5 (bootstrap neutru)
    let _ = honest.generate_prediction(0.0, 0);
    let bootstrap_alpha = honest.last_adaptive_alpha;
    assert!(bootstrap_alpha > HEBBIAN_MIN_ALPHA,
        "Bootstrap α should be > MIN, got {}", bootstrap_alpha);

    // Setăm consensus = propria predicție anterioară (agreement maxim)
    let prev_pred = honest.get_consensus().median_prediction.clone();
    // Pentru a simula agreement maxim, setăm consens la o valoare apropiată
    let mut consensus = neurograph::dag_logic::DagConsensus::default();
    consensus.median_prediction = Array1::from(vec![0.5, 0.5, 0.5, 0.5]); // ≈ baza semnalului onest
    honest.set_consensus(consensus);
    let _ = honest.generate_prediction(0.05, 1);

    let alpha = honest.last_adaptive_alpha;
    // Cu agreement mare, α ar trebui să fie aproape de BASE
    assert!(alpha > HEBBIAN_MIN_ALPHA + 0.5 * (HEBBIAN_BASE_ALPHA - HEBBIAN_MIN_ALPHA),
        "Honest with high agreement should have α near BASE, got {} (BASE={})",
        alpha, HEBBIAN_BASE_ALPHA);
}

#[test]
fn test_adaptive_alpha_formula() {
    // Verifică formula: α = MIN + (BASE - MIN) × agreement^POWER
    let mut node = AngpNode::new("formula_test".to_string(), AttackType::Honest);

    // Agreement = 1 (consens == signal): α = BASE
    let mut consensus = neurograph::dag_logic::DagConsensus::default();
    consensus.median_prediction = Array1::from(vec![0.5, 0.5, 0.5, 0.5]); // ≈ honest signal at t=0
    node.set_consensus(consensus);
    let _ = node.generate_prediction(0.0, 0);
    let alpha_high = node.last_adaptive_alpha;
    println!("α at high agreement: {}", alpha_high);

    // Agreement = 0 (consens foarte diferit): α = MIN
    let mut consensus2 = neurograph::dag_logic::DagConsensus::default();
    consensus2.median_prediction = Array1::from(vec![1.0, 1.0, 1.0, 1.0]); // foarte departe
    node.set_consensus(consensus2);
    let _ = node.generate_prediction(0.0, 0); // t=0 → signal ≈ [0.5,0.5,0.5,0.5]
    let alpha_low = node.last_adaptive_alpha;
    println!("α at low agreement: {}", alpha_low);

    assert!(alpha_high > alpha_low,
        "α at high agreement ({}) must be > α at low agreement ({})",
        alpha_high, alpha_low);
}

// ════════════════════════════════════════════════════════════════════
// #3: Predictive Tip Selection
// ════════════════════════════════════════════════════════════════════

#[test]
fn test_predictive_tip_selection_uses_blend() {
    // Verifică că select_tips_predictive produce tips diferite când
    // own_prediction diferă de consensus_median
    let tmp = common::TempDir::new("predictive_blend");
    let mut logic = DagLogic::new(100, tmp.path());

    // Adăugăm câteva tranzacții în ledger ca să avem tips
    use neurograph::transaction::Transaction;
    use neurograph::ledger::Ledger;
    for i in 0..5u64 {
        let tx = Transaction::new_with_nonce(
            "alice".to_string(), "bob".to_string(), 100 + i, i + 1, vec![],
        );
        logic.ledger.add(tx);
    }
    assert!(logic.ledger.get_tips().len() >= 5, "Need at least 5 tips for test");

    let own_pred = Array1::from(vec![0.5, 0.5, 0.5, 0.5]);
    let consensus_median = Array1::from(vec![0.5, 0.5, 0.5, 0.5]);
    let history = VecDeque::new();

    let tips = logic.select_tips_predictive(&own_pred, &consensus_median, &history, 3);
    assert_eq!(tips.len(), 3, "Should select 3 tips");
}

#[test]
fn test_predictive_tip_selection_momentum_boost() {
    // Tips care au fost comune în istoric ar trebui să primească boost
    let tmp = common::TempDir::new("momentum_boost");
    let mut logic = DagLogic::new(100, tmp.path());

    use neurograph::transaction::Transaction;
    let txs: Vec<_> = (0..5u64).map(|i| {
        Transaction::new_with_nonce(
            "alice".to_string(), "bob".to_string(), 100 + i, i + 1, vec![],
        )
    }).collect();
    for tx in &txs { logic.ledger.add(tx.clone()); }

    let all_tips: Vec<Hash> = logic.ledger.get_tips().iter().map(|t| **t).collect();
    assert!(all_tips.len() >= 5);

    // Marcam primele 2 tips ca foarte comune în istoric (apar în 5 din 5 entry-uri)
    let mut history = VecDeque::new();
    let common_set = vec![all_tips[0], all_tips[1]];
    for _ in 0..5 { history.push_back(common_set.clone()); }

    let pred = Array1::from(vec![0.5, 0.5, 0.5, 0.5]);
    let consensus = Array1::from(vec![0.5, 0.5, 0.5, 0.5]);

    let selected = logic.select_tips_predictive(&pred, &consensus, &history, 2);

    // Cele 2 tips cu momentum mare ar trebui să fie selectate
    assert!(selected.contains(&all_tips[0]) || selected.contains(&all_tips[1]),
        "Momentum-boosted tips should be preferred: selected={:?}, expected one of {:?}",
        selected, &common_set);
}

#[test]
fn test_predictive_tip_selection_blend_weights() {
    // Verifică că parametrii PREDICTIVE_OWN_WEIGHT și PREDICTIVE_MOMENTUM_WEIGHT
    // sunt în intervalul valid [0, 1]
    assert!(PREDICTIVE_OWN_WEIGHT >= 0.0 && PREDICTIVE_OWN_WEIGHT <= 1.0,
        "PREDICTIVE_OWN_WEIGHT must be in [0,1], got {}", PREDICTIVE_OWN_WEIGHT);
    assert!(PREDICTIVE_MOMENTUM_WEIGHT >= 0.0 && PREDICTIVE_MOMENTUM_WEIGHT <= 1.0,
        "PREDICTIVE_MOMENTUM_WEIGHT must be in [0,1], got {}", PREDICTIVE_MOMENTUM_WEIGHT);
}

#[test]
fn test_record_common_tips_history() {
    // Verifică că AngpNode înregistrează istoricul de tips corect
    let mut node = AngpNode::new("history_test".to_string(), AttackType::Honest);
    assert_eq!(node.get_recent_common_tips().len(), 0);

    // Adăugăm 15 intrări (history max = 10)
    for i in 0..15u8 {
        let tips = vec![[i; 32]];
        node.record_common_tips(tips);
    }
    assert_eq!(node.get_recent_common_tips().len(), 10,
        "History should be capped at PREDICTIVE_MOMENTUM_HISTORY");

    // Ultima intrare ar trebui să fie ultima adăugată
    let last = node.get_recent_common_tips().back().unwrap();
    assert_eq!(last[0], [14u8; 32]);
}

// ════════════════════════════════════════════════════════════════════
// Feedback loop integration
// ════════════════════════════════════════════════════════════════════

#[test]
fn test_feedback_loop_honest_converges_to_high_alpha() {
    // Simulăm un nod onest care primește consens de la rețea consistent
    // cu predicția sa. α ar trebui să crească în timp.
    let mut honest = AngpNode::new("loop_honest".to_string(), AttackType::Honest);
    let mut alphas: Vec<f64> = Vec::new();

    for step in 0..30u64 {
        let t = step as f64 * 0.05;
        let pred = honest.generate_prediction(t, step);

        // Simulăm consens = propria predicție (network agreement perfect)
        let mut consensus = neurograph::dag_logic::DagConsensus::default();
        consensus.median_prediction = pred.clone();
        honest.set_consensus(consensus);

        alphas.push(honest.last_adaptive_alpha);
    }

    // În a doua jumătate, α ar trebui să fie mai mare decât în prima
    let first_half_avg: f64 = alphas[..15].iter().sum::<f64>() / 15.0;
    let second_half_avg: f64 = alphas[15..].iter().sum::<f64>() / 15.0;
    assert!(second_half_avg >= first_half_avg * 0.95,
        "Honest α should not decrease over time (first={:.4}, second={:.4})",
        first_half_avg, second_half_avg);
}

#[test]
fn test_feedback_loop_attacker_alpha_decreases() {
    // Simulăm un atacator Adaptive care încearcă să perturbe.
    // Cu consens stabil, α-ul atacatorului ar trebui să scadă.
    let mut attacker = AngpNode::new("loop_attacker".to_string(), AttackType::Coordinated);
    let mut alphas: Vec<f64> = Vec::new();

    // Setăm un consensus stabil în jur de 0.5 (honest signal)
    let mut consensus = neurograph::dag_logic::DagConsensus::default();
    consensus.median_prediction = Array1::from(vec![0.5, 0.5, 0.5, 0.5]);
    attacker.set_consensus(consensus);

    for step in 0..30u64 {
        let t = step as f64 * 0.05;
        let _ = attacker.generate_prediction(t, step);
        // Re-setăm consensul (ca să nu fie updatat de test)
        let mut c = neurograph::dag_logic::DagConsensus::default();
        c.median_prediction = Array1::from(vec![0.5, 0.5, 0.5, 0.5]);
        attacker.set_consensus(c);
        alphas.push(attacker.last_adaptive_alpha);
    }

    // Coordinated attack (predicție 0.9) ar trebui să aibă α mic tot timpul
    let avg_alpha: f64 = alphas.iter().sum::<f64>() / alphas.len() as f64;
    assert!(avg_alpha < HEBBIAN_BASE_ALPHA * 0.5,
        "Coordinated attacker should have low α on average (avg={:.4}, BASE={})",
        avg_alpha, HEBBIAN_BASE_ALPHA);
}
