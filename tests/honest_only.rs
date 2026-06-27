//! v3.5.12 Test: 10 honest nodes, ZERO attackers.
//! Verifică că bug-ul "honest0 moare singur" din v3.5.11 e rezolvat.
//!
//! Rulează cu: cargo test --release --test honest_only -- --nocapture --ignored honest_only_no_attackers

use neurograph::node::AngpNode;
use neurograph::attack::AttackType;
use neurograph::dag_logic::DagLogic;
use neurograph::attack_detection::AttackDetectionManager;
use neurograph::config::DIM;
use ndarray::Array1;
use std::collections::HashMap;

mod common;

/// Simulează 10 honest nodes (zero attackers) timp de 1000 pași.
/// v3.5.12 expectations: TOATE cele 10 noduri trebuie să rămână vii (rep > 0.5).
/// v3.5.11 bug: honest0 murea (rep=0.000) pentru că nu se observa pe sine.
#[test]
#[ignore]
fn honest_only_no_attackers() {
    let n_honest = 10;
    let n_steps = 1000;

    println!("\n=== v3.5.12 Test: {} honest nodes, ZERO attackers, {} steps ===",
        n_honest, n_steps);

    let mut nodes: Vec<AngpNode> = Vec::new();
    for i in 0..n_honest {
        nodes.push(AngpNode::new(format!("honest{}", i), AttackType::Honest));
    }

    let mut detector = AttackDetectionManager::new();
    let tmp = common::TempDir::new("honest_only_v3512");
    let logic = DagLogic::new(10000, tmp.path());

    for step in 0..n_steps {
        let t = step as f64 * 0.05;
        let mut proposals = Vec::new();
        for node in &mut nodes {
            let pred = node.generate_prediction(t, step as u64);
            proposals.push((node.id.clone(), pred));
        }

        use neurograph::dag_logic::DagProposal;
        let dag_proposals: Vec<DagProposal> = proposals.iter().map(|(sender, pred)| {
            DagProposal {
                sender: sender.clone(), step: step as u64, seq: 0, nonce: 0,
                prediction: pred.iter().copied().collect(),
                proposed_tips: vec![], state_root: [0u8; 32],
                seen_tx_hashes: vec![], seen_receipts: vec![],
            }
        }).collect();

        // Gossip: fiecare nod primește propunerile tuturor (inclusiv propria)
        for node in &mut nodes {
            for dp in &dag_proposals {
                // v3.5.12 FIX: fiecare nod primește TOATE propunerile, inclusiv propria.
                // Bug-ul v3.5.11: "if dp.sender != node.id" → nodul nu se observa pe sine.
                node.add_remote_proposal(dp.clone());
            }
        }

        let rep_map: HashMap<String, f64> = nodes[0].get_all_reputations();
        let (consensus, mut errors) = logic.compute_consensus(&dag_proposals, &rep_map);
        let consensus_mean: f64 = consensus.median_prediction.mean().unwrap_or(0.5);
        let penalties = detector.detect_all(&dag_proposals, &errors, consensus_mean);
        for (node, penalty) in &penalties {
            let base_err = errors.get(node).copied().unwrap_or(0.5);
            if *penalty > 1.0 {
                errors.insert(node.clone(), base_err * penalty);
            } else {
                errors.insert(node.clone(), base_err + penalty);
            }
        }
        for node in &mut nodes {
            node.update_reputations(step as u64, &errors);
        }

        if step > 0 && step % 200 == 0 {
            println!("\n--- Step {} ---", step);
            let observer = &nodes[0];
            for n in &nodes {
                let rep = observer.get_reputation(&n.id).unwrap_or(0.0);
                let status = if rep > 0.7 { "OK" } else if rep > 0.3 { "WARN" } else { "BAD" };
                println!("  {:<15} rep={:.3} {}", n.id, rep, status);
            }
        }
    }

    println!("\n=== Final (v3.5.12) ===");
    let observer = &nodes[0];
    let mut alive = 0;
    for n in &nodes {
        let rep = observer.get_reputation(&n.id).unwrap_or(0.0);
        let status = if rep > 0.7 { "OK" } else if rep > 0.3 { "WARN" } else { "BAD" };
        println!("  {:<15} rep={:.3} {}", n.id, rep, status);
        if rep > 0.5 { alive += 1; }
    }
    println!("\nHonest alive: {}/{} (v3.5.11 gave 9/10 due to bug; v3.5.12 should give 10/10)",
        alive, n_honest);

    // v3.5.12 strict assertion: ALL honest nodes must survive
    assert!(alive == n_honest,
        "v3.5.12 REGRESSION: {} honest nodes died without any attacker (expected all {} alive)",
        n_honest - alive, n_honest);
    println!("\n✓ v3.5.12 FIX CONFIRMED: All {} honest nodes survived without any attacker.", n_honest);
}
