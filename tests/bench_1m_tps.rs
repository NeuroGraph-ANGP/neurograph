//! v3.5.7 — Stress Test Single-Node (1M TPS target)
//!
//! Simuleaza load de 100,000 txs pe un singur nod pentru a valida:
//!   1. Capacitatea sig verify (cu batch + rayon par_chunks)
//!   2. Capacitatea mempool add (O(1) double-spend check)
//!   3. Capacitatea hash SHA-512/256
//!   4. Capacitatea serializare bincode
//!
//! Rulare: cargo test --release --test bench_1m_tps -- --nocapture --include-ignored

use neurograph::dag_logic::DagLogic;
use neurograph::transaction::Transaction;
use neurograph::sharding::shard_of_address;
use neurograph::config::N_SHARDS;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rayon::prelude::*;
use std::time::Instant;

fn make_signing_keys(n: usize) -> Vec<SigningKey> {
    let mut rng = OsRng;
    (0..n).map(|_| SigningKey::generate(&mut rng)).collect()
}

fn make_signed_txs(keys: &[SigningKey], count: usize) -> Vec<Transaction> {
    let n_keys = keys.len();
    (0..count)
        .map(|i| {
            let key = &keys[i % n_keys];
            let sender = format!("sender_{}", i % n_keys);
            let receiver = format!("receiver_{}", i);
            let mut tx = Transaction::new_with_fee(
                sender,
                receiver,
                100,
                (i as u64) + 1,
                1,
                vec![],
            );
            tx.sign(key);
            tx
        })
        .collect()
}

#[test]
#[ignore]
fn bench_1m_tps_stress_test() {
    println!("\n═══════════════════════════════════════════════════════════════");
    println!("  NEUROGRAPH v3.5.7 — STRESS TEST (single-node, target 1M TPS)");
    println!("═══════════════════════════════════════════════════════════════\n");
    println!("Config: N_SHARDS = {}", N_SHARDS);
    println!("Target: 1,000,000 TPS pe sistem (cu 961 shards × ~1300 TPS/shard)");
    println!("Acest test masoara capacitatea PER NOD (bottleneck-ul).\n");

    // ─── 1. Generare 100K tranzactii semnate ─────────────────────────
    let tx_count = 100_000;
    println!("1. Generare {} tranzactii semnate...", tx_count);
    let keys = make_signing_keys(100);
    let start = Instant::now();
    let txs = make_signed_txs(&keys, tx_count);
    let gen_time = start.elapsed();
    println!("   Gata in {:.2}s ({:.0} txs/sec)\n",
             gen_time.as_secs_f64(),
             tx_count as f64 / gen_time.as_secs_f64());

    // ─── 2. Sig verify cu batch + rayon par_chunks ───────────────────
    println!("2. Signature verify (batch + rayon par_chunks(64))...");
    let tx_refs: Vec<&Transaction> = txs.iter().collect();
    let start = Instant::now();
    let results = Transaction::verify_batch(&tx_refs);
    let verify_time = start.elapsed();
    let valid_count = results.iter().filter(|&&v| v).count();
    println!("   Gata in {:.2}s", verify_time.as_secs_f64());
    println!("   Valid: {}/{}", valid_count, tx_count);
    println!("   >>> Sig verify capacity: {:.0} sigs/sec <<<\n",
             tx_count as f64 / verify_time.as_secs_f64());

    // ─── 3. Mempool add cu O(1) double-spend ────────────────────────
    println!("3. Mempool add (O(1) double-spend detection)...");
    let data_dir = format!("/tmp/neurograph_stress_{}", std::process::id());
    let mut logic = DagLogic::new(200_000, &data_dir);

    // Alocam genesis pentru toti senderii
    for i in 0..100 {
        logic.get_state_mut().genesis_allocate(&format!("sender_{}", i));
    }

    let start = Instant::now();
    let mut added = 0u64;
    let mut rejected = 0u64;
    for tx in &txs {
        if logic.add_transaction(tx.clone()) {
            added += 1;
        } else {
            rejected += 1;
        }
    }
    let mempool_time = start.elapsed();
    println!("   Gata in {:.2}s", mempool_time.as_secs_f64());
    println!("   Added: {}, Rejected: {}", added, rejected);
    println!("   >>> Mempool add capacity: {:.0} adds/sec <<<\n",
             added as f64 / mempool_time.as_secs_f64());

    // ─── 4. Hash SHA-512/256 ────────────────────────────────────────
    println!("4. SHA-512/256 hash...");
    let start = Instant::now();
    let _hashes: Vec<_> = txs.par_iter().map(|tx| tx.compute_hash()).collect();
    let hash_time = start.elapsed();
    println!("   Gata in {:.2}s", hash_time.as_secs_f64());
    println!("   >>> Hash capacity: {:.0} hashes/sec <<<\n",
             tx_count as f64 / hash_time.as_secs_f64());

    // ─── 5. Serializare bincode ─────────────────────────────────────
    println!("5. Serializare bincode...");
    let start = Instant::now();
    let _serialized: Vec<Vec<u8>> = txs.par_iter()
        .map(|tx| bincode::serialize(tx).unwrap_or_default())
        .collect();
    let serialize_time = start.elapsed();
    println!("   Gata in {:.2}s", serialize_time.as_secs_f64());
    println!("   >>> Serialize capacity: {:.0} txs/sec <<<\n",
             tx_count as f64 / serialize_time.as_secs_f64());

    // ─── 6. Pipeline end-to-end (simuleaza procesare reala) ─────────
    println!("6. Pipeline end-to-end (sig verify + mempool add)...");
    // Generam alte 100K txs ca sa nu fie dubluri in mempool
    let txs2 = make_signed_txs(&keys[50..], tx_count);
    let tx_refs2: Vec<&Transaction> = txs2.iter().collect();

    let start = Instant::now();
    // Sig verify in paralel
    let results2 = Transaction::verify_batch(&tx_refs2);
    // Add la mempool doar cele valide
    let mut added2 = 0u64;
    for (i, tx) in txs2.iter().enumerate() {
        if results2[i] && logic.add_transaction(tx.clone()) {
            added2 += 1;
        }
    }
    let pipeline_time = start.elapsed();
    println!("   Gata in {:.2}s", pipeline_time.as_secs_f64());
    println!("   Added: {}/{}", added2, tx_count);
    let pipeline_tps = added2 as f64 / pipeline_time.as_secs_f64();
    println!("   >>> Pipeline capacity: {:.0} TPS (per nod) <<<\n", pipeline_tps);

    // ─── 7. Calcul proiectie 1M TPS ─────────────────────────────────
    println!("═══════════════════════════════════════════════════════════════");
    println!("  PROIECTIE 1M TPS");
    println!("═══════════════════════════════════════════════════════════════\n");

    let tps_node = pipeline_tps;
    let cross_shard_overhead = 0.20;

    println!("Capacitate per nod (masurata): {:.0} TPS", tps_node);
    println!("N_SHARDS: {}", N_SHARDS);
    println!("Cross-shard overhead: {:.0}%", cross_shard_overhead * 100.0);
    println!();

    let tps_system_current = N_SHARDS as f64 * tps_node * (1.0 - cross_shard_overhead);
    println!("TPS sistem (cu configuratia actuala):");
    println!("  {} shards x {:.0} TPS/nod x {:.1} = {:.0} TPS",
             N_SHARDS, tps_node, 1.0 - cross_shard_overhead, tps_system_current);
    println!();

    if tps_system_current >= 1_000_000.0 {
        println!(">>> TARGET 1M TPS ATINS cu configuratia actuala! <<<");
    } else {
        let gap = 1_000_000.0 / tps_system_current;
        println!("Gap fata de 1M TPS: {:.1}x", gap);
        println!();
        println!("Pentru a atinge 1M TPS:");
        println!("  1. Batch verify nativ (ed25519-dalek fork): 10x speedup");
        println!("     -> TPS_nod: {:.0} -> {:.0}", tps_node, tps_node * 10.0);
        println!("     -> TPS sistem: {:.0} TPS",
                 N_SHARDS as f64 * tps_node * 10.0 * (1.0 - cross_shard_overhead));
    }
    println!();

    // ─── 8. Distributie shards ──────────────────────────────────────
    println!("Distributie tranzactii pe {} shards:", N_SHARDS);
    let mut shard_counts = std::collections::HashMap::new();
    for tx in txs.iter().take(10000) {
        let shard = shard_of_address(&tx.sender);
        *shard_counts.entry(shard).or_insert(0u32) += 1;
    }
    let used_shards = shard_counts.len();
    let avg_per_shard = 10000.0 / used_shards as f64;
    println!("  Din 10,000 tranzactii: distribuite pe {} shards", used_shards);
    println!("  Medie per shard: {:.1} txs (expected: {:.2})", avg_per_shard, 10000.0 / N_SHARDS as f64);

    // Cleanup
    let _ = std::fs::remove_dir_all(&data_dir);

    println!("\n═══════════════════════════════════════════════════════════════");
    println!("  STRESS TEST COMPLET");
    println!("═══════════════════════════════════════════════════════════════");
}
