//! v2.3 Integration tests: Hebbian DAG reactivat ca "creier" al predicției.
//!
//! Aceste teste verifică că AdaptiveDag (Hebbian) e conectat la fluxul activ:
//!   - generate_prediction() adaugă noduri în AdaptiveDag
//!   - build_proposal() include predicția în DagProposal
//!   - compute_consensus() calculează mediană ponderată a predicțiilor
//!   - Eroarea per nod include componenta neurală (L2 la median)

use neurograph::node::AngpNode;
use neurograph::attack::AttackType;
use neurograph::dag_logic::{DagLogic, DagProposal};
use neurograph::config::DIM;
use neurograph::utils::euclidean;
use ndarray::Array1;
use std::collections::HashMap;

mod common;

#[test]
fn test_adaptive_dag_receives_predictions() {
    // Verifică că generate_prediction adaugă noduri în AdaptiveDag
    let mut node = AngpNode::new("test_node".to_string(), AttackType::Honest);
    assert_eq!(node.dag_node_count(), 0, "AdaptiveDag should start empty");

    // Generează 10 predicții
    for step in 0..10u64 {
        let t = step as f64 * 0.05;
        let _prediction = node.generate_prediction(t, step);
    }

    assert_eq!(node.dag_node_count(), 10,
        "AdaptiveDag should have 10 nodes after 10 predictions");
}

#[test]
fn test_prediction_is_stable_across_steps() {
    // Predicțiile Hebbian ar trebui să fie mai stabile decât semnalul brut
    // datorită EMA smoothing + Hebbian aggregation
    let mut node = AngpNode::new("stable_node".to_string(), AttackType::Honest);
    let mut predictions: Vec<Array1<f64>> = Vec::new();
    for step in 0..50u64 {
        let t = step as f64 * 0.05;
        let pred = node.generate_prediction(t, step);
        predictions.push(pred);
    }
    // Verifică că predicțiile consecutive nu sar brusc
    for i in 1..predictions.len() {
        let delta = euclidean(&predictions[i-1], &predictions[i]);
        assert!(delta < 0.2,
            "Prediction delta too large at step {}: {} (expected < 0.2)", i, delta);
    }
}

#[test]
fn test_dag_proposal_contains_prediction() {
    // Verifică că DagProposal include câmpul prediction (votul neural)
    let tmp = common::TempDir::new("dag_proposal");
    let mut logic = DagLogic::new(100, tmp.path());

    let prediction = Array1::from(vec![0.5, 0.6, 0.7, 0.8]);
    let consensus_median = Array1::from(vec![0.5, 0.5, 0.5, 0.5]);
    let history = std::collections::VecDeque::new();
    let proposal = logic.build_proposal(
        "alice", 42, 1, 12345, &HashMap::new(),
        &prediction, &consensus_median, &history,
        vec![],  // v3.1: seen_receipts
    );

    assert_eq!(proposal.prediction.len(), DIM,
        "DagProposal must contain prediction vector");
    assert_eq!(proposal.prediction, vec![0.5, 0.6, 0.7, 0.8],
        "Prediction must be the one passed to build_proposal");
    assert_eq!(proposal.sender, "alice");
    assert_eq!(proposal.step, 42);
}

#[test]
fn test_compute_consensus_includes_median_prediction() {
    // Verifică că compute_consensus calculează median_prediction
    let tmp = common::TempDir::new("median_pred");
    let logic = DagLogic::new(100, tmp.path());

    // 3 propuneri cu predicții diferite
    let proposals = vec![
        DagProposal {
            sender: "alice".to_string(), step: 1, seq: 0, nonce: 0,
            prediction: vec![0.1, 0.2, 0.3, 0.4],
            proposed_tips: vec![], state_root: [0u8; 32], seen_tx_hashes: vec![], seen_receipts: vec![],
        },
        DagProposal {
            sender: "bob".to_string(), step: 1, seq: 0, nonce: 0,
            prediction: vec![0.5, 0.5, 0.5, 0.5],
            proposed_tips: vec![], state_root: [0u8; 32], seen_tx_hashes: vec![], seen_receipts: vec![],
        },
        DagProposal {
            sender: "carol".to_string(), step: 1, seq: 0, nonce: 0,
            prediction: vec![0.9, 0.8, 0.7, 0.6],
            proposed_tips: vec![], state_root: [0u8; 32], seen_tx_hashes: vec![], seen_receipts: vec![],
        },
    ];

    let mut rep_map = HashMap::new();
    rep_map.insert("alice".to_string(), 0.95);
    rep_map.insert("bob".to_string(), 0.95);
    rep_map.insert("carol".to_string(), 0.95);

    let (consensus, errors) = logic.compute_consensus(&proposals, &rep_map);

    // Median prediction ar trebui să fie ~ [0.5, 0.5, 0.5, 0.5] (median din cele 3)
    assert_eq!(consensus.median_prediction.len(), DIM,
        "Median prediction must have DIM components");
    for i in 0..DIM {
        let m = consensus.median_prediction[i];
        assert!(m >= 0.4 && m <= 0.6,
            "Median component {} = {} (expected ~0.5)", i, m);
    }

    // Toate nodurile trebuie să aibă erori calculate (L2 distance to median)
    assert_eq!(errors.len(), 3, "All 3 nodes must have error computed");
    // Bob are predicția exact la median → eroare 0 (doar componenta neurală)
    let bob_err = errors.get("bob").copied().unwrap_or(999.0);
    assert!(bob_err < 0.01,
        "Bob's error should be ~0 (his prediction == median), got {}", bob_err);
}

#[test]
fn test_hebbian_learning_affects_predictions() {
    // Verifică că AdaptiveDag învață (Hebbian) — predicția curentă
    // e influențată de istoric, nu doar de semnalul curent.
    let mut node = AngpNode::new("learning_node".to_string(), AttackType::Honest);

    // Generează 5 predicții și salvează ultima
    let mut last_prediction = Array1::zeros(DIM);
    for step in 0..5u64 {
        let t = step as f64 * 0.05;
        last_prediction = node.generate_prediction(t, step);
    }

    // La pasul 6, generăm o predicție — ar trebui să fie influențată de istoric
    let pred_6 = node.generate_prediction(0.30, 6);
    // Nu trebuie să fie zeros (Hebbian memory)
    assert!(pred_6.iter().sum::<f64>() > 0.0,
        "Prediction after Hebbian learning must not be zero");

    // AdaptiveDag trebuie să aibă 6 noduri
    assert_eq!(node.dag_node_count(), 6,
        "AdaptiveDag should have 6 nodes after 6 generate_prediction calls");
}

#[test]
fn test_attack_types_produce_different_predictions() {
    // Noduri honest vs atacatori ar trebui să producă predicții diferite
    let mut honest = AngpNode::new("h1".to_string(), AttackType::Honest);
    let mut attacker = AngpNode::new("a1".to_string(), AttackType::Coordinated);

    let honest_pred = honest.generate_prediction(0.0, 0);
    let attacker_pred = attacker.generate_prediction(0.0, 0);

    let dist = euclidean(&honest_pred, &attacker_pred);
    assert!(dist > 0.1,
        "Honest and Coordinated attacker should produce very different predictions, dist={}",
        dist);
}
