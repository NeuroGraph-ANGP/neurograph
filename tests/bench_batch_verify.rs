//! v3.1 Benchmark: batch verify (ed25519-dalek) vs individual verify.
//!
//! Rulează cu: cargo test --release --test bench_batch_verify -- --nocapture --ignored

use neurograph::transaction::Transaction;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rayon::prelude::*;
use std::time::Instant;

mod common;

#[test]
#[ignore]
fn bench_batch_verify_vs_individual() {
    let n_txs = 50_000;
    println!("\n=== v3.1 Batch Verify Benchmark: {} txs ===", n_txs);

    // Generăm txs semnate
    println!("Generating {} signed transactions...", n_txs);
    let mut rng = OsRng;
    let mut txs: Vec<Transaction> = Vec::with_capacity(n_txs);
    let mut signing_keys: Vec<SigningKey> = Vec::with_capacity(n_txs);
    for i in 0..n_txs {
        let sk = SigningKey::generate(&mut rng);
        let mut tx = Transaction::new_with_nonce(
            format!("sender_{}", i), "receiver".to_string(), 100, i as u64 + 1, vec![],
        );
        tx.sign(&sk);
        txs.push(tx);
        signing_keys.push(sk);
    }

    // 1. Individual verify (sequential)
    let ind_start = Instant::now();
    let mut ind_valid = 0;
    for tx in &txs {
        if tx.verify_signature() { ind_valid += 1; }
    }
    let ind_elapsed = ind_start.elapsed();
    let ind_tps = n_txs as f64 / ind_elapsed.as_secs_f64();
    println!("Individual verify (sequential): {} valid in {:.2?} ({:.0} sigs/sec)",
        ind_valid, ind_elapsed, ind_tps);

    // 2. Individual verify (parallel, Rayon) — ca v3.0
    let par_start = Instant::now();
    let par_valid = txs.par_iter().filter(|tx| tx.verify_signature()).count();
    let par_elapsed = par_start.elapsed();
    let par_tps = n_txs as f64 / par_elapsed.as_secs_f64();
    println!("Individual verify (parallel, {} cores): {} valid in {:.2?} ({:.0} sigs/sec)",
        rayon::current_num_threads(), par_valid, par_elapsed, par_tps);

    // 3. v3.1: BATCH verify (ed25519-dalek verify_batch)
    let batch_start = Instant::now();
    let refs: Vec<&Transaction> = txs.iter().collect();
    let batch_results = Transaction::verify_batch(&refs);
    let batch_valid = batch_results.iter().filter(|&&r| r).count();
    let batch_elapsed = batch_start.elapsed();
    let batch_tps = n_txs as f64 / batch_elapsed.as_secs_f64();
    println!("BATCH verify (ed25519-dalek): {} valid in {:.2?} ({:.0} sigs/sec)",
        batch_valid, batch_elapsed, batch_tps);

    // 4. v3.1: BATCH verify + Rayon (împărțim în chunks paralele)
    let chunk_size = 1024;
    let par_batch_start = Instant::now();
    let chunks: Vec<&[Transaction]> = txs.chunks(chunk_size).collect();
    let total_valid = chunks
        .par_iter()
        .map(|chunk| {
            let refs: Vec<&Transaction> = chunk.iter().collect();
            let results = Transaction::verify_batch(&refs);
            results.iter().filter(|&&r| r).count()
        })
        .sum::<usize>();
    let par_batch_elapsed = par_batch_start.elapsed();
    let par_batch_tps = n_txs as f64 / par_batch_elapsed.as_secs_f64();
    println!("BATCH verify + Rayon (chunks of {}): {} valid in {:.2?} ({:.0} sigs/sec)",
        chunk_size, total_valid, par_batch_elapsed, par_batch_tps);

    println!("\n=== Speedup Analysis ===");
    println!("Individual (parallel) → Batch (single): {:.2}× speedup", par_tps / batch_tps);
    println!("Individual (parallel) → Batch + Rayon:  {:.2}× speedup", par_tps / par_batch_tps);
    println!("Best: Batch + Rayon at {:.0} sigs/sec", par_batch_tps);

    println!("\n=== Sharded System Projection (16 shards) ===");
    // Per-shard: parallel_batch_tps (single shard gets 1/N_cores of CPU)
    // Total: 16 shards × per-shard-tps
    let ncores = rayon::current_num_threads() as f64;
    let per_shard_tps = par_batch_tps / (16.0 / ncores).max(1.0);
    let system_tps = per_shard_tps * 16.0;
    println!("Per-shard (with {} cores shared across 16 shards): {:.0} sigs/sec",
        ncores, per_shard_tps);
    println!("System total (16 shards): {:.0} sigs/sec", system_tps);
    println!("Target: 1,000,000 TPS");
    println!("Achieved (projected): {:.0} TPS ({:.1}% of target)",
        system_tps, 100.0 * system_tps / 1_000_000.0);
    if system_tps < 1_000_000.0 {
        println!("Cores needed for 1M TPS: {:.0}",
            16.0 * 1_000_000.0 / par_batch_tps);
    }

    // Verificări
    assert_eq!(ind_valid, n_txs);
    assert_eq!(par_valid, n_txs);
    assert_eq!(batch_valid, n_txs);
    assert_eq!(total_valid, n_txs);
}

#[test]
#[ignore]
fn bench_batch_verify_with_invalids() {
    let n_valid = 45_000;
    let n_invalid = 5_000;
    let n_total = n_valid + n_invalid;
    println!("\n=== Batch Verify with {} invalid txs (out of {}) ===", n_invalid, n_total);

    let mut rng = OsRng;
    let mut txs: Vec<Transaction> = Vec::with_capacity(n_total);
    for i in 0..n_valid {
        let sk = SigningKey::generate(&mut rng);
        let mut tx = Transaction::new_with_nonce(
            format!("sender_{}", i), "receiver".to_string(), 100, i as u64 + 1, vec![],
        );
        tx.sign(&sk);
        txs.push(tx);
    }
    // Adăugăm txs invalide (tampered)
    for i in 0..n_invalid {
        let sk = SigningKey::generate(&mut rng);
        let mut tx = Transaction::new_with_nonce(
            format!("sender_{}_tampered", i), "receiver".to_string(), 100, (n_valid + i) as u64 + 1, vec![],
        );
        tx.sign(&sk);
        tx.amount = 99999;  // tampering după semnare
        txs.push(tx);
    }

    // Shuffle ca invalid-urile să fie distribuite random
    use rand::seq::SliceRandom;
    txs.shuffle(&mut rng);

    // BATCH verify (cu fallback individual pentru izolare)
    let batch_start = Instant::now();
    let refs: Vec<&Transaction> = txs.iter().collect();
    let results = Transaction::verify_batch(&refs);
    let batch_elapsed = batch_start.elapsed();
    let valid_count = results.iter().filter(|&&r| r).count();
    let invalid_count = results.len() - valid_count;

    println!("Batch verify: {} valid, {} invalid in {:.2?}",
        valid_count, invalid_count, batch_elapsed);
    println!("Batch TPS: {:.0}", n_total as f64 / batch_elapsed.as_secs_f64());

    assert_eq!(valid_count, n_valid);
    assert_eq!(invalid_count, n_invalid);
}
