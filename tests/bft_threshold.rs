//! v3.4 BFT Threshold Tests: 50% attackers (pragul teoretic).
//!
//! Rulează cu: cargo test --release --test bft_threshold -- --nocapture --ignored

use neurograph::node::AngpNode;
use neurograph::attack::AttackType;
use neurograph::dag_logic::DagLogic;
use neurograph::attack_detection::AttackDetectionManager;
use std::collections::HashMap;
use std::time::Instant;

mod common;

fn run_sim(n_honest: usize, attackers: Vec<AttackType>, n_steps: usize) -> (Vec<f64>, Vec<f64>, f64) {
    let tmp = common::TempDir::new(&format!("bft_{}_{}", n_honest, attackers.len()));
    let logic = DagLogic::new(10000, tmp.path());
    let mut detector = AttackDetectionManager::new();

    let mut nodes: Vec<AngpNode> = Vec::new();
    for i in 0..n_honest {
        nodes.push(AngpNode::new(format!("honest{}", i), AttackType::Honest));
    }
    for (i, at) in attackers.iter().enumerate() {
        nodes.push(AngpNode::new(format!("attacker{}", i + 1), at.clone()));
    }

    let mut total_consensus_time = 0.0;

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

        // Măsurăm timpul compute_consensus
        let consensus_start = Instant::now();
        let (consensus, mut errors) = logic.compute_consensus(&dag_proposals, &rep_map);
        total_consensus_time += consensus_start.elapsed().as_secs_f64();

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
    let avg_consensus_us = total_consensus_time / n_steps as f64 * 1_000_000.0;
    (honest_reps, attacker_reps, avg_consensus_us)
}

fn print_results(name: &str, honest_reps: &[f64], attacker_reps: &[f64], consensus_us: f64) {
    let honest_alive = honest_reps.iter().filter(|&&r| r > 0.5).count();
    let attacker_dead = attacker_reps.iter().filter(|&&r| r < 0.3).count();
    let honest_avg = if honest_reps.is_empty() { 0.0 } else { honest_reps.iter().sum::<f64>() / honest_reps.len() as f64 };
    let attacker_avg = if attacker_reps.is_empty() { 0.0 } else { attacker_reps.iter().sum::<f64>() / attacker_reps.len() as f64 };

    println!("\n══════ {} ══════", name);
    println!("  Honest:   {}/{} alive (avg={:.3})", honest_alive, honest_reps.len(), honest_avg);
    println!("  Attackers: {}/{} dead  (avg={:.3})", attacker_dead, attacker_reps.len(), attacker_avg);
    println!("  Consensus time: {:.1} μs/step (O(n²) cluster detection)", consensus_us);

    let verdict = if honest_alive == honest_reps.len() && attacker_dead == attacker_reps.len() {
        "✓ PERFECT — all honest alive, all attackers eliminated"
    } else if honest_alive >= honest_reps.len() * 8 / 10 {
        "✓ HONEST PROTECTED — ≥80% honest alive (attackers may survive at BFT threshold)"
    } else if honest_alive >= honest_reps.len() / 2 {
        "⚠ DEGRADED — honest majority but under attack"
    } else {
        "✗ COLLAPSE — honest nodes destroyed"
    };
    println!("  Verdict: {}", verdict);

    // Detalii per nod
    println!("\n  Per-node details:");
    for (i, &r) in honest_reps.iter().enumerate() {
        let s = if r > 0.7 { "OK" } else if r > 0.3 { "WARN" } else { "BAD" };
        println!("    honest{:>2}: {:.3} {}", i, r, s);
    }
    for (i, &r) in attacker_reps.iter().enumerate() {
        let s = if r > 0.7 { "OK" } else if r > 0.3 { "WARN" } else { "BAD" };
        println!("    attacker{:>2}: {:.3} {}", i + 1, r, s);
    }
}

#[test]
#[ignore]
fn bft_test_1_clone_50pct() {
    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("TEST 1: 10 honest + 10 Clone (50%) — BFT threshold");
    println!("Verifică dacă CLONE_EPSILON=0.01 + cluster capping prinde clone-urile");
    println!("═══════════════════════════════════════════════════════════════════");

    let (h, a, t) = run_sim(10, vec![AttackType::Clone; 10], 1000);
    print_results("10 honest vs 10 Clone (50%)", &h, &a, t);
}

#[test]
#[ignore]
fn bft_test_2_adaptive_50pct() {
    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("TEST 2: 10 honest + 10 Adaptive (50%) — BFT threshold");
    println!("Verifică dacă PredictionJumpDetector îi elimină înainte să captureze mediana");
    println!("═══════════════════════════════════════════════════════════════════");

    let (h, a, t) = run_sim(10, vec![AttackType::Adaptive; 10], 1000);
    print_results("10 honest vs 10 Adaptive (50%)", &h, &a, t);
}

#[test]
#[ignore]
fn bft_test_3_mixed_coord_clone_50pct() {
    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("TEST 3: 10 honest + 5 Coordinated + 5 Clone (50% mixt)");
    println!("Verifică interacțiunea dintre cluster capping și clone detection");
    println!("═══════════════════════════════════════════════════════════════════");

    let mut attackers = vec![AttackType::Coordinated; 5];
    attackers.extend(vec![AttackType::Clone; 5]);
    let (h, a, t) = run_sim(10, attackers, 1000);
    print_results("10 honest vs 5 Coordinated + 5 Clone (50%)", &h, &a, t);
}

#[test]
#[ignore]
fn bft_test_4_performance_50_nodes() {
    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("TEST 4: Performance — compute_consensus cu O(n²) cluster detection");
    println!("Măsoară timpul la 20, 30, 50, 100 noduri");
    println!("═══════════════════════════════════════════════════════════════════");

    for n_total in [20, 30, 50, 100] {
        let n_honest = n_total / 2;
        let n_attackers = n_total - n_honest;
        let attackers: Vec<AttackType> = (0..n_attackers)
            .map(|i| match i % 5 {
                0 => AttackType::Coordinated,
                1 => AttackType::Clone,
                2 => AttackType::Adaptive,
                3 => AttackType::GaussianNoise,
                _ => AttackType::FlipFlop,
            })
            .collect();
        let (h, a, t) = run_sim(n_honest, attackers, 200);
        let honest_alive = h.iter().filter(|&&r| r > 0.5).count();
        let attacker_dead = a.iter().filter(|&&r| r < 0.3).count();
        println!("\n  {:>3} nodes ({}h+{}a): {:.1} μs/step | honest {}/{} alive | attackers {}/{} dead",
            n_total, n_honest, n_attackers, t,
            honest_alive, n_honest, attacker_dead, n_attackers);
    }

    println!("\n  Analiză complexitate:");
    println!("    Cluster detection: O(n²) — pairwise comparison între predicții");
    println!("    Weight capping: O(n) — single pass după clustering");
    println!("    Weighted median: O(n × DIM × log(n)) — sortare per dimensiune");
    println!("    Total: O(n²) dominant");
    println!("    La 50 noduri: ~2500 comparisons × ~100ns = ~250μs (neglijabil)");
    println!("    La 100 noduri: ~10000 comparisons × ~100ns = ~1ms (încă rapid)");
}
