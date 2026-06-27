//! v2.5 Benchmark: măsoară throughput-ul real de procesare a tranzacțiilor.
//!
//! Rulează cu: cargo test --release --test bench_throughput -- --nocapture --ignored

use neurograph::dag_logic::DagLogic;
use neurograph::transaction::Transaction;
use neurograph::wallet::Wallet;
use rayon::prelude::*;
use std::time::Instant;

mod common;

/// Benchmark: câte tranzacții pot fi adăugate în mempool și validate pe secundă.
///
/// Măsoară:
///   1. Verificare semnătură Ed25519 (parallel cu Rayon)
///   2. Double-spend check O(1) (index v2.5)
///   3. Adăugare în mempool
#[test]
#[ignore]
fn bench_tx_processing_throughput() {
    let tmp = common::TempDir::new("bench_throughput");
    let mut logic = DagLogic::new(100_000, tmp.path());

    // Generăm 10 wallets pentru a semna tranzacții
    let mut wallets = Vec::new();
    for i in 0..10 {
        let sub_dir = format!("{}/w{}", tmp.path(), i);
        let w = Wallet::load_or_create(&sub_dir, &format!("w{}", i)).unwrap();
        wallets.push(w);
    }
    // Genesis allocate
    for w in &wallets {
        logic.get_state_mut().genesis_allocate(&w.public_key_hex);
    }

    let n_txs: usize = 50_000;
    println!("\n=== Benchmark: {} transactions ===", n_txs);

    // Generăm tranzacțiile (fără a le adăuga încă)
    let gen_start = Instant::now();
    let mut txs: Vec<Transaction> = Vec::with_capacity(n_txs);
    for i in 0..n_txs {
        let sender_idx = i % wallets.len();
        let receiver_idx = (i + 1) % wallets.len();
        let sender = wallets[sender_idx].public_key_hex.clone();
        let receiver = wallets[receiver_idx].public_key_hex.clone();
        let nonce = (i / wallets.len() + 1) as u64;
        let mut tx = Transaction::new_with_nonce(sender, receiver, 1, nonce, vec![]);
        tx.sign(&wallets[sender_idx].signing_key);
        txs.push(tx);
    }
    let gen_elapsed = gen_start.elapsed();
    println!("Generation: {} txs in {:.2?} ({:.0} txs/sec)",
        n_txs, gen_elapsed, n_txs as f64 / gen_elapsed.as_secs_f64());

    // Benchmark: batch signature verification (parallel)
    let sig_start = Instant::now();
    let verified: Vec<&Transaction> = txs.par_iter().filter(|tx| tx.verify_signature()).collect();
    let sig_elapsed = sig_start.elapsed();
    let sig_tps = n_txs as f64 / sig_elapsed.as_secs_f64();
    println!("Signature verify (parallel): {}/{} valid in {:.2?} ({:.0} sigs/sec)",
        verified.len(), n_txs, sig_elapsed, sig_tps);

    // Benchmark: adăugare în mempool cu O(1) double-spend check
    let add_start = Instant::now();
    let mut added = 0usize;
    let mut rejected = 0usize;
    for tx in &txs {
        if logic.add_transaction(tx.clone()) {
            added += 1;
        } else {
            rejected += 1;
        }
    }
    let add_elapsed = add_start.elapsed();
    let add_tps = added as f64 / add_elapsed.as_secs_f64();
    println!("Mempool add (O(1) double-spend): {} added, {} rejected in {:.2?} ({:.0} adds/sec)",
        added, rejected, add_elapsed, add_tps);

    // Benchmark: SHA-512/256 hash throughput
    let hash_start = Instant::now();
    let mut hash_count = 0;
    for tx in &txs {
        let _ = tx.compute_hash();
        hash_count += 1;
    }
    let hash_elapsed = hash_start.elapsed();
    let hash_tps = hash_count as f64 / hash_elapsed.as_secs_f64();
    println!("SHA-512/256 hash: {} in {:.2?} ({:.0} hashes/sec)",
        hash_count, hash_elapsed, hash_tps);

    // Benchmark: bincode vs JSON serialization
    let bincode_start = Instant::now();
    for tx in &txs {
        let _ = bincode::serialize(tx).unwrap();
    }
    let bincode_elapsed = bincode_start.elapsed();
    let bincode_tps = n_txs as f64 / bincode_elapsed.as_secs_f64();

    let json_start = Instant::now();
    for tx in &txs {
        let _ = serde_json::to_vec(tx).unwrap();
    }
    let json_elapsed = json_start.elapsed();
    let json_tps = n_txs as f64 / json_elapsed.as_secs_f64();

    println!("Serialization bincode: {:.2?} ({:.0} txs/sec) | JSON: {:.2?} ({:.0} txs/sec) | bincode is {:.1}× faster",
        bincode_elapsed, bincode_tps,
        json_elapsed, json_tps,
        json_elapsed.as_secs_f64() / bincode_elapsed.as_secs_f64());

    println!("\n=== Summary (extrapolat la 500K TPS) ===");
    let target_tps = 500_000.0;
    println!("Target TPS: {:.0}", target_tps);
    println!("  Signature verify capacity: {:.0} sigs/sec ({:.1}× target)",
        sig_tps, sig_tps / target_tps);
    println!("  Mempool add capacity:      {:.0} adds/sec ({:.1}× target)",
        add_tps, add_tps / target_tps);
    println!("  Hash capacity:             {:.0} hashes/sec ({:.1}× target)",
        hash_tps, hash_tps / target_tps);
    println!("  Serialization capacity:    {:.0} txs/sec ({:.1}× target)",
        bincode_tps, bincode_tps / target_tps);

    // Verificare finală
    assert_eq!(verified.len(), n_txs, "All txs should pass signature verification");
    assert!(added > 0, "Some txs should be added to mempool");
}

/// Benchmark: throughput realist cu sig verify PARALLEL + add_verified_batch.
/// Acesta e scenariul de producție: txs vin în batch, sig verify în paralel,
/// apoi batch add fără re-verificare.
#[test]
#[ignore]
fn bench_realistic_pipeline_throughput() {
    let tmp = common::TempDir::new("bench_pipeline");
    let mut logic = DagLogic::new(1_000_000, tmp.path());

    // 10 wallets
    let mut wallets = Vec::new();
    for i in 0..10 {
        let sub_dir = format!("{}/w{}", tmp.path(), i);
        let w = Wallet::load_or_create(&sub_dir, &format!("w{}", i)).unwrap();
        wallets.push(w);
    }
    for w in &wallets {
        logic.get_state_mut().genesis_allocate(&w.public_key_hex);
    }

    let n_txs: usize = 100_000;
    let batch_size = 256;  // MAX_TX_BATCH_SIZE
    println!("\n=== Realistic Pipeline Benchmark: {} txs (batch={}) ===", n_txs, batch_size);

    // Generăm txs
    let mut txs: Vec<Transaction> = Vec::with_capacity(n_txs);
    for i in 0..n_txs {
        let sender_idx = i % wallets.len();
        let receiver_idx = (i + 1) % wallets.len();
        let mut tx = Transaction::new_with_nonce(
            wallets[sender_idx].public_key_hex.clone(),
            wallets[receiver_idx].public_key_hex.clone(),
            1,
            (i / wallets.len() + 1) as u64,
            vec![],
        );
        tx.sign(&wallets[sender_idx].signing_key);
        txs.push(tx);
    }

    // Pipeline complet: sig verify parallel + add_verified_batch per batch
    let pipeline_start = Instant::now();
    let mut total_added = 0usize;
    for chunk in txs.chunks(batch_size) {
        // Sig verify parallel în batch
        let verified: Vec<Transaction> = chunk.par_iter()
            .filter(|tx| tx.verify_signature())
            .cloned()
            .collect();
        // Add fără re-verificare
        let (added, _) = logic.add_verified_batch(verified);
        total_added += added;
    }
    let pipeline_elapsed = pipeline_start.elapsed();
    let pipeline_tps = total_added as f64 / pipeline_elapsed.as_secs_f64();

    println!("Pipeline (sig verify parallel + batch add): {} added in {:.2?}",
        total_added, pipeline_elapsed);
    println!("Pipeline TPS: {:.0} txs/sec", pipeline_tps);
    println!("\n=== Target: 500,000 TPS ===");
    println!("  Single-node capacity: {:.0} ({:.1}% of target)",
        pipeline_tps, 100.0 * pipeline_tps / 500_000.0);
    let ncores = rayon::current_num_threads();
    println!("  Cores used: {}", ncores);
    println!("  To reach 500K TPS, need either:");
    println!("    - {:.1}× more cores (≈{:.0} cores)", 500_000.0 / pipeline_tps, ncores as f64 * 500_000.0 / pipeline_tps);
    println!("    - OR sharding across {:.1} nodes (each handles {:.0} TPS)",
        500_000.0 / pipeline_tps, pipeline_tps);
    println!("    - OR batch Ed25519 verify (5-10× speedup on sig verify)");

    assert!(total_added > 0, "Should add at least some txs");
}

