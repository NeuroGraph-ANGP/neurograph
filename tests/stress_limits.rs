//! v3.3 Stress Test: determină limitele de robusteție ale sistemului.
//!
//! Testează diverse proporții de atacatori (10% → 47%) și tipuri de atacuri.
//!
//! Rulează cu: cargo test --release --test stress_limits -- --nocapture --ignored

use neurograph::node::AngpNode;
use neurograph::attack::AttackType;
use neurograph::dag_logic::DagLogic;
use neurograph::attack_detection::AttackDetectionManager;
use std::collections::HashMap;

mod common;

fn run_simulation(
    n_honest: usize,
    attackers: Vec<AttackType>,
    n_steps: usize,
) -> (Vec<f64>, Vec<f64>) {
    let n_attackers = attackers.len();
    let tmp = common::TempDir::new(&format!("stress_{}_{}", n_honest, n_attackers));
    let logic = DagLogic::new(10000, tmp.path());
    let mut detector = AttackDetectionManager::new();

    let mut nodes: Vec<AngpNode> = Vec::new();
    for i in 0..n_honest {
        nodes.push(AngpNode::new(format!("honest{}", i), AttackType::Honest));
    }
    for (i, at) in attackers.iter().enumerate() {
        nodes.push(AngpNode::new(format!("attacker{}", i + 1), at.clone()));
    }

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

        for node in &mut nodes {
            for dp in &dag_proposals {
                if dp.sender != node.id {
                    node.add_remote_proposal(dp.clone());
                }
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
    }

    let observer = &nodes[0];
    let mut honest_reps = Vec::new();
    let mut attacker_reps = Vec::new();
    for node in &nodes {
        let rep = observer.get_reputation(&node.id).unwrap_or(0.0);
        if node.id.starts_with("honest") {
            honest_reps.push(rep);
        } else {
            attacker_reps.push(rep);
        }
    }
    (honest_reps, attacker_reps)
}

fn analyze(name: &str, honest_reps: &[f64], attacker_reps: &[f64]) {
    let honest_alive = honest_reps.iter().filter(|&&r| r > 0.5).count();
    let honest_dead = honest_reps.iter().filter(|&&r| r < 0.3).count();
    let attacker_alive = attacker_reps.iter().filter(|&&r| r > 0.5).count();
    let attacker_dead = attacker_reps.iter().filter(|&&r| r < 0.3).count();
    let honest_avg = if honest_reps.is_empty() { 0.0 } else { honest_reps.iter().sum::<f64>() / honest_reps.len() as f64 };
    let attacker_avg = if attacker_reps.is_empty() { 0.0 } else { attacker_reps.iter().sum::<f64>() / attacker_reps.len() as f64 };

    let n_honest = honest_reps.len();
    let n_attackers = attacker_reps.len();
    let total = n_honest + n_attackers;
    let pct = if total > 0 { 100.0 * n_attackers as f64 / total as f64 } else { 0.0 };

    let verdict = if honest_alive == n_honest && attacker_dead == n_attackers {
        "✓ PERFECT"
    } else if honest_alive >= n_honest * 8 / 10 && attacker_dead >= n_attackers * 7 / 10 {
        "✓ OK"
    } else if honest_alive >= n_honest / 2 {
        "⚠ DEGRADED"
    } else {
        "✗ COLLAPSE"
    };

    println!("  {:<35} attackers={:>2} ({:>4.0}%) | honest: {}/{} alive (avg={:.2}) | attackers: {}/{} dead (avg={:.2}) | {}",
        name, n_attackers, pct,
        honest_alive, n_honest, honest_avg,
        attacker_dead, n_attackers, attacker_avg,
        verdict);
}

#[test]
#[ignore]
fn stress_test_all_proportions() {
    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("NEUROGRAPH v3.3 — STRESS TEST: LIMITS OF ROBUSTNESS");
    println!("═══════════════════════════════════════════════════════════════════");
    println!("Setup: 10 honest nodes + N attackers, 1000 steps per simulation\n");

    let n_honest = 10;
    let n_steps = 1000;

    println!("─── PROPORTION TEST: Coordinated attackers ───────────────────────");
    for n_att in [1, 2, 3, 5, 7, 9] {
        let attackers: Vec<AttackType> = (0..n_att).map(|_| AttackType::Coordinated).collect();
        let (h, a) = run_simulation(n_honest, attackers, n_steps);
        analyze(&format!("{} Coordinated", n_att), &h, &a);
    }

    println!("\n─── PROPORTION TEST: Clone attackers ──────────────────────────────");
    for n_att in [1, 2, 3, 5, 7, 9] {
        let attackers: Vec<AttackType> = (0..n_att).map(|_| AttackType::Clone).collect();
        let (h, a) = run_simulation(n_honest, attackers, n_steps);
        analyze(&format!("{} Clone", n_att), &h, &a);
    }

    println!("\n─── PROPORTION TEST: Adaptive attackers ───────────────────────────");
    for n_att in [1, 2, 3, 5, 7, 9] {
        let attackers: Vec<AttackType> = (0..n_att).map(|_| AttackType::Adaptive).collect();
        let (h, a) = run_simulation(n_honest, attackers, n_steps);
        analyze(&format!("{} Adaptive", n_att), &h, &a);
    }

    println!("\n─── PROPORTION TEST: GaussianNoise attackers ──────────────────────");
    for n_att in [1, 2, 3, 5, 7, 9] {
        let attackers: Vec<AttackType> = (0..n_att).map(|_| AttackType::GaussianNoise).collect();
        let (h, a) = run_simulation(n_honest, attackers, n_steps);
        analyze(&format!("{} GaussianNoise", n_att), &h, &a);
    }

    println!("\n─── PROPORTION TEST: FlipFlop attackers ───────────────────────────");
    for n_att in [1, 2, 3, 5, 7, 9] {
        let attackers: Vec<AttackType> = (0..n_att).map(|_| AttackType::FlipFlop).collect();
        let (h, a) = run_simulation(n_honest, attackers, n_steps);
        analyze(&format!("{} FlipFlop", n_att), &h, &a);
    }

    println!("\n─── MIXED ATTACKS (5 attackers, different types) ──────────────────");
    let mixed_configs: Vec<(&str, Vec<AttackType>)> = vec![
        ("Clone+Coordinated", vec![AttackType::Clone, AttackType::Coordinated, AttackType::Clone, AttackType::Coordinated, AttackType::Clone]),
        ("Adaptive+Gaussian", vec![AttackType::Adaptive, AttackType::GaussianNoise, AttackType::Adaptive, AttackType::GaussianNoise, AttackType::Adaptive]),
        ("FlipFlop+Sleeper", vec![AttackType::FlipFlop, AttackType::Sleeper, AttackType::FlipFlop, AttackType::Sleeper, AttackType::FlipFlop]),
        ("All different", vec![AttackType::Coordinated, AttackType::Clone, AttackType::Adaptive, AttackType::GaussianNoise, AttackType::FlipFlop]),
        ("3 Clone + 2 Coord", vec![AttackType::Clone, AttackType::Clone, AttackType::Clone, AttackType::Coordinated, AttackType::Coordinated]),
    ];
    for (name, attackers) in &mixed_configs {
        let (h, a) = run_simulation(n_honest, attackers.clone(), n_steps);
        analyze(name, &h, &a);
    }

    println!("\n─── EXTREME: 9 attackers (47%) — near BFT threshold ───────────────");
    let extreme_configs: Vec<(&str, Vec<AttackType>)> = vec![
        ("9 Coordinated", vec![AttackType::Coordinated; 9]),
        ("9 Clone", vec![AttackType::Clone; 9]),
        ("9 Adaptive", vec![AttackType::Adaptive; 9]),
        ("9 GaussianNoise", vec![AttackType::GaussianNoise; 9]),
        ("9 FlipFlop", vec![AttackType::FlipFlop; 9]),
        ("9 Mixed", vec![
            AttackType::Coordinated, AttackType::Clone, AttackType::Adaptive,
            AttackType::GaussianNoise, AttackType::FlipFlop, AttackType::Sleeper,
            AttackType::Drift, AttackType::OutlierBurst, AttackType::RandomNoise,
        ]),
    ];
    for (name, attackers) in &extreme_configs {
        let (h, a) = run_simulation(n_honest, attackers.clone(), n_steps);
        analyze(name, &h, &a);
    }

    println!("\n─── SYBIL ATTACK: many identities from one attacker ───────────────");
    // Sybil = mai mulți atacatori cu același tip, simulând identități multiple
    for n_att in [5, 10, 15, 19] {
        let attackers: Vec<AttackType> = (0..n_att).map(|_| AttackType::Coordinated).collect();
        let (h, a) = run_simulation(n_honest, attackers, n_steps);
        let pct = 100.0 * n_att as f64 / (n_honest + n_att) as f64;
        analyze(&format!("Sybil {} Coordinated ({}%)", n_att, pct as u32), &h, &a);
    }

    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("LEGEND:");
    println!("  ✓ PERFECT   = all honest alive, all attackers dead");
    println!("  ✓ OK        = ≥80% honest alive, ≥70% attackers dead");
    println!("  ⚠ DEGRADED  = ≥50% honest alive");
    println!("  ✗ COLLAPSE  = <50% honest alive");
    println!("═══════════════════════════════════════════════════════════════════");
}
