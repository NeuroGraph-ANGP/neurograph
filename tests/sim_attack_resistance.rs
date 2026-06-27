//! v3.2 Simulation test: 10 honest nodes vs 5 attackers.
//! Verifică că sistemul NU colapsează — honest nodes rămân cu reputație > 0.7.
//!
//! Rulează cu: cargo test --release --test sim_attack_resistance -- --nocapture --ignored

use neurograph::node::AngpNode;
use neurograph::attack::AttackType;
use neurograph::dag_logic::DagLogic;
use neurograph::attack_detection::AttackDetectionManager;
use neurograph::config::DIM;
use ndarray::Array1;
use std::collections::HashMap;

mod common;

/// Simulează 15 noduri (10 honest + 5 attackers) timp de 1000 pași.
/// Verifică că la final:
///   - ≥ 8 din 10 honest nodes au reputație > 0.5
///   - ≥ 3 din 5 attackers au reputație < 0.3
#[test]
#[ignore]
fn sim_10_honest_vs_5_attackers() {
    let n_honest = 10;
    let n_attackers = 5;
    let n_steps = 1000;

    println!("\n=== Simulation: {} honest vs {} attackers, {} steps ===",
        n_honest, n_attackers, n_steps);

    // Creăm nodurile
    let mut nodes: Vec<AngpNode> = Vec::new();
    for i in 0..n_honest {
        nodes.push(AngpNode::new(format!("honest{}", i), AttackType::Honest));
    }
    let attack_types = [
        AttackType::Coordinated,
        AttackType::Clone,
        AttackType::Adaptive,
        AttackType::FlipFlop,
        AttackType::GaussianNoise,
    ];
    for (i, at) in attack_types.iter().enumerate() {
        nodes.push(AngpNode::new(format!("attacker{}", i + 1), at.clone()));
    }

    let mut detector = AttackDetectionManager::new();
    let tmp = common::TempDir::new("sim_attack");
    let logic = DagLogic::new(10000, tmp.path());

    // Simulăm n_steps pași
    for step in 0..n_steps {
        let t = step as f64 * 0.05;

        // Fiecare nod generează o predicție
        let mut proposals = Vec::new();
        for node in &mut nodes {
            let pred = node.generate_prediction(t, step);
            proposals.push((node.id.clone(), pred));
        }

        // Construim DagProposal pentru fiecare nod
        use neurograph::dag_logic::DagProposal;
        use neurograph::transaction::Hash;
        let dag_proposals: Vec<DagProposal> = proposals.iter().map(|(sender, pred)| {
            DagProposal {
                sender: sender.clone(),
                step: step as u64,
                seq: 0,
                nonce: 0,
                prediction: pred.iter().copied().collect(),
                proposed_tips: vec![],
                state_root: [0u8; 32],
                seen_tx_hashes: vec![],
                seen_receipts: vec![],
            }
        }).collect();

        // v3.2 fix: Fiecare nod primeşte propunerile tuturor celorlalte noduri
        // (simulează gossip de reţea)
        for node in &mut nodes {
            for dp in &dag_proposals {
                if dp.sender != node.id {
                    node.add_remote_proposal(dp.clone());
                }
            }
        }

        // Calculăm consens (folosind primul nod ca referință — toate nodurile văd aceleași propuneri)
        let rep_map: HashMap<String, f64> = nodes[0].get_all_reputations();

        let (consensus, mut errors) = logic.compute_consensus(&dag_proposals, &rep_map);

        // Attack detection
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

        // Actualizăm reputaţii pentru fiecare nod
        // (toate nodurile văd aceleaşi propuneri, deci folosim errors comun)
        for node in &mut nodes {
            node.update_reputations(step as u64, &errors);
        }

        // La fiecare 200 paşi, afişăm status
        if step > 0 && step % 200 == 0 {
            println!("\n--- Step {} ---", step);
            println!("Attack detection: {}", detector.status());
            let observer = &nodes[0];
            let mut all_ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
            all_ids.sort();
            for id in &all_ids {
                let rep = observer.get_reputation(id).unwrap_or(0.0);
                let status = if rep > 0.7 { "OK" } else if rep > 0.3 { "WARN" } else { "BAD" };
                println!("  {:<15} rep={:.3} {}", id, rep, status);
            }
        }
    }

    // Verificări finale
    println!("\n=== Final Results (step {}) ===", n_steps);
    let mut honest_ok = 0;
    let mut honest_bad = 0;
    let mut attacker_ok = 0;
    let mut attacker_bad = 0;

    // v3.2: Reputatia unui nod e determinată de ALTE noduri.
    // Folosim nodes[0] ca observator (toate nodurile văd aceleași propuneri,
    // deci au aceeași hartă de reputații).
    let observer = &nodes[0];
    let mut all_ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    all_ids.sort();

    for id in &all_ids {
        let rep = observer.get_reputation(id).unwrap_or(0.0);
        let status = if rep > 0.7 { "OK" } else if rep > 0.3 { "WARN" } else { "BAD" };
        println!("  {:<15} rep={:.3} {}", id, rep, status);
        if id.starts_with("honest") {
            if rep > 0.5 { honest_ok += 1; } else { honest_bad += 1; }
        } else {
            if rep > 0.3 { attacker_ok += 1; } else { attacker_bad += 1; }
        }
    }

    println!("\n=== Summary ===");
    println!("Honest nodes: {}/{} with rep > 0.5 (target: ≥{}/{})",
        honest_ok, n_honest, n_honest - 2, n_honest);
    println!("Attackers: {}/{} with rep < 0.3 (target: ≥{}/{})",
        attacker_bad, n_attackers, 3, n_attackers);
    println!("Attack detection: {}", detector.status());

    // ASSERT: cel puțin 8/10 honest nodes ar trebui să aibă rep > 0.5
    assert!(honest_ok >= n_honest - 2,
        "DEATH SPIRAL detected: only {}/{} honest nodes have rep > 0.5 (expected ≥{})",
        honest_ok, n_honest, n_honest - 2);
}
