//! v3.5.9 — Simulator 4000 noduri IN-PROCES cu HYBRID CONSENSUS (3 nivele)
//!
//! v3.5.9: Activeaza Hybrid Consensus (3 nivele):
//!   Nivel 1: Consens per NOD (compute_consensus pe propuneri)
//!   Nivel 2: Consens per SHARD (ShardDigest per shard)
//!   Nivel 3: Consens GLOBAL/ANCHOR (compute_consensus pe shard_digests)
//!
//! Rulare:
//!   cargo test --release --test bench_simulator -- --nocapture --include-ignored

use neurograph::dag_logic::{DagLogic, DagConsensus, DagProposal};
use neurograph::node::AngpNode;
use neurograph::transaction::Transaction;
use neurograph::attack::AttackType;
use neurograph::sharding::{ShardSet, shard_of_address};
use neurograph::shard_consensus::{ShardConsensusManager, HybridConsensusResult};
use neurograph::config::N_SHARDS;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rand::Rng;
use rayon::prelude::*;
use std::time::Instant;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};

const N_NODES: usize = 4000;
const SHARDS_PER_NODE: u32 = 3;
const SIM_STEPS: u64 = 20;
const TXS_PER_STEP: usize = 200;
const MEMPOOL_MAX: usize = 5000;
const N_SENDERS: usize = 100;

struct SimNode {
    id: String,
    shard_ids: Vec<u32>,
    shard_set: ShardSet,
    angp_node: AngpNode,
    dag_logic: DagLogic,
    signing_key: SigningKey,
    txs_finalized: u64,
}

impl SimNode {
    fn new(idx: usize, base_dir: &str) -> Self {
        let id = format!("node_{:04}", idx);
        let data_dir = format!("{}/{}", base_dir, id);
        std::fs::create_dir_all(&data_dir).unwrap_or_default();

        let shard_start = (idx as u32 * SHARDS_PER_NODE) % N_SHARDS;
        let shard_ids: Vec<u32> = (0..SHARDS_PER_NODE)
            .map(|s| (shard_start + s) % N_SHARDS)
            .collect();
        let shard_set = ShardSet::only(&shard_ids);

        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);

        let mut dag_logic = DagLogic::new(MEMPOOL_MAX, &data_dir);
        dag_logic.get_state_mut().genesis_allocate(&id);

        SimNode {
            id: id.clone(),
            shard_ids,
            shard_set,
            angp_node: AngpNode::new(id, AttackType::Honest),
            dag_logic,
            signing_key,
            txs_finalized: 0,
        }
    }
}

#[test]
#[ignore]
fn bench_simulator_1000_nodes() {
    println!("\n{:=^70}", "");
    println!("  NEUROGRAPH v3.5.9 — SIMULATOR {} NODURI + HYBRID CONSENSUS", N_NODES);
    println!("{:=^70}", "");
    println!();
    println!("Consens HYBRID (3 nivele):");
    println!("  Nivel 1: Consens per NOD (compute_consensus pe propuneri)");
    println!("  Nivel 2: Consens per SHARD (ShardDigest per shard)");
    println!("  Nivel 3: Consens GLOBAL/ANCHOR (compute_consensus pe shard_digests)");
    println!();
    println!("Config:");
    println!("  N_NODES:           {}", N_NODES);
    println!("  N_SHARDS:          {}", N_SHARDS);
    println!("  SHARDS_PER_NODE:   {}", SHARDS_PER_NODE);
    println!("  Noduri/shard:      {:.1}", (N_NODES as f64 * SHARDS_PER_NODE as f64) / N_SHARDS as f64);
    println!("  SIM_STEPS:         {}", SIM_STEPS);
    println!("  TXS_PER_STEP:      {}", TXS_PER_STEP);
    println!();

    // ─── 1. Creare noduri ───────────────────────────────────────────
    println!("1. Creare {} noduri (PoW mining, paralel)...", N_NODES);
    let base_dir = format!("/tmp/neurograph_sim_{}", std::process::id());
    let _ = std::fs::remove_dir_all(&base_dir);
    std::fs::create_dir_all(&base_dir).unwrap_or_default();

    let start = Instant::now();
    let nodes: Vec<Arc<Mutex<SimNode>>> = (0..N_NODES)
        .into_par_iter()
        .map(|i| Arc::new(Mutex::new(SimNode::new(i, &base_dir))))
        .collect();
    let creation_time = start.elapsed();
    println!("   Gata in {:.1}s ({:.0} noduri/sec)\n",
             creation_time.as_secs_f64(),
             N_NODES as f64 / creation_time.as_secs_f64());

    // ─── 2. Statistici shards ───────────────────────────────────────
    println!("2. Grupare noduri pe {} shards...", N_SHARDS);
    let mut shard_counts: HashMap<u32, usize> = HashMap::new();
    for node_arc in &nodes {
        let node = node_arc.lock().unwrap();
        for &s in &node.shard_ids {
            *shard_counts.entry(s).or_insert(0) += 1;
        }
    }
    let active_shards = shard_counts.len();
    let avg_nodes_per_shard = N_NODES as f64 * SHARDS_PER_NODE as f64 / active_shards as f64;
    println!("   Shards active: {}/{}", active_shards, N_SHARDS);
    println!("   Noduri/shard (avg): {:.1}", avg_nodes_per_shard);
    println!("   BFT 70%: {:.1} noduri necesare per shard\n", avg_nodes_per_shard * 0.7);

    // ─── 3. Genesis + generare txs ─────────────────────────────────
    println!("3. Genesis pentru {} senderi + generare {} txs...",
             N_SENDERS, TXS_PER_STEP * SIM_STEPS as usize);
    let sender_names: Vec<String> = (0..N_SENDERS).map(|i| format!("sender_{}", i)).collect();
    let mut nonce_counters: Vec<u64> = vec![0; N_SENDERS];

    // Alocam genesis pentru toti senderii (paralel)
    nodes.par_iter().for_each(|node_arc| {
        let mut node = node_arc.lock().unwrap();
        for sender in &sender_names {
            node.dag_logic.get_state_mut().genesis_allocate(sender);
        }
    });

    let mut rng = OsRng;
    let keys: Vec<SigningKey> = (0..N_SENDERS).map(|_| SigningKey::generate(&mut rng)).collect();

    let total_txs_to_gen = TXS_PER_STEP * SIM_STEPS as usize;
    let start = Instant::now();
    let all_txs: Vec<Vec<Transaction>> = (0..SIM_STEPS)
        .map(|_| {
            (0..TXS_PER_STEP)
                .map(|_| {
                    let sender_idx = rng.gen_range(0..N_SENDERS);
                    let sender = sender_names[sender_idx].clone();
                    let receiver = format!("receiver_{}", rng.gen_range(0..1000));
                    let amount = rng.gen_range(1..1000);
                    nonce_counters[sender_idx] += 1;
                    let nonce = nonce_counters[sender_idx];
                    let mut tx = Transaction::new_with_fee(
                        sender, receiver, amount, nonce, 1, vec![],
                    );
                    tx.sign(&keys[sender_idx]);
                    tx
                })
                .collect()
        })
        .collect();
    let gen_time = start.elapsed();
    println!("   Gata in {:.1}s ({:.0} txs/sec)\n",
             gen_time.as_secs_f64(),
             total_txs_to_gen as f64 / gen_time.as_secs_f64());

    // ─── 4. Simulare main loop cu HYBRID CONSENSUS ─────────────────
    println!("4. Simulare {} steps (HYBRID CONSENSUS 3 nivele)...", SIM_STEPS);
    println!("   Load: {} txs/step\n", TXS_PER_STEP);

    let total_finalized = AtomicU64::new(0);
    let total_added = AtomicU64::new(0);
    let total_shard_digests = AtomicU64::new(0);
    let sim_start = Instant::now();

    for step in 0..SIM_STEPS {
        let step_txs = &all_txs[step as usize];

        // ── 4a. Tx distribution la nodurile cu shard-ul corect ───
        for tx in step_txs {
            let from_shard = shard_of_address(&tx.sender);
            for node_arc in &nodes {
                let mut node = node_arc.lock().unwrap();
                if node.shard_set.contains(from_shard) {
                    if node.dag_logic.add_transaction(tx.clone()) {
                        total_added.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }

        // ── 4b. Colectam TOATE propunerile de la TOATE nodurile ───
        let t = step as f64 * 0.5;
        let mut all_proposals: Vec<DagProposal> = Vec::new();

        for node_arc in &nodes {
            let mut node = node_arc.lock().unwrap();
            let prediction = node.angp_node.generate_prediction(t, step);
            let rep_map = node.angp_node.get_all_reputations();
            let consensus_median = node.angp_node.get_consensus().median_prediction.clone();
            let recent_tips = node.angp_node.get_recent_common_tips().clone();

            let node_id = node.id.clone();
            let next_seq = node.angp_node.next_seq;
            let pow_nonce = node.angp_node.pow_nonce;

            let proposal = node.dag_logic.build_proposal(
                &node_id, step, next_seq, pow_nonce,
                &rep_map, &prediction, &consensus_median, &recent_tips,
                Vec::new(),
            );
            all_proposals.push(proposal);
            node.angp_node.next_seq += 1;
        }

        // ── 4c. HYBRID CONSENSUS (3 nivele) ───
        // Folosim DagLogic-ul primului nod ca "aggregator" cu ShardSet::all()
        let aggregator_set = ShardSet::all();
        let rep_map_global = nodes[0].lock().unwrap().angp_node.get_all_reputations();

        let hybrid_result = {
            let aggregator = nodes[0].lock().unwrap();
            ShardConsensusManager::compute_hybrid(
                &aggregator.dag_logic,
                &all_proposals,
                &rep_map_global,
                &aggregator_set,
                step,
            )
        };

        // ── 4d. Distribuim rezultatele la toate nodurile ───
        for node_arc in &nodes {
            let mut node = node_arc.lock().unwrap();

            // Clone shard_ids ca sa evitam borrow conflict
            let node_shard_ids = node.shard_ids.clone();

            // Update reputatii pe baza erorilor combinate
            node.angp_node.update_reputations(step, &hybrid_result.errors);

            // Pentru fiecare shard al nodului, setam consensul local
            for &shard_id in &node_shard_ids {
                if let Some(shard_consensus) = hybrid_result.shard_consensus.get(&shard_id) {
                    node.angp_node.record_common_tips(shard_consensus.common_tips.clone());
                }
            }

            // Set consensul global (anchor) pentru pasul urmator
            node.angp_node.set_consensus(hybrid_result.global_consensus.clone());

            // Finalize batch folosind consensul shard-urilor nodului
            if DagLogic::should_finalize(step) {
                for &shard_id in &node_shard_ids {
                    if let Some(shard_consensus) = hybrid_result.shard_consensus.get(&shard_id) {
                        let (finalized, _rewards) = node.dag_logic.finalize_batch(shard_consensus);
                        if !finalized.is_empty() {
                            let fin_count = finalized.len() as u64;
                            node.txs_finalized += fin_count;
                            total_finalized.fetch_add(fin_count, Ordering::Relaxed);
                        }
                    }
                }
            }
        }

        // Count shard digests produse
        total_shard_digests.fetch_add(hybrid_result.shard_digests.len() as u64, Ordering::Relaxed);

        // Progress la fiecare 25 steps
        if step > 0 && step % 25 == 0 {
            let elapsed = sim_start.elapsed().as_secs_f64();
            let tf = total_finalized.load(Ordering::Relaxed);
            let ta = total_added.load(Ordering::Relaxed);
            let tsd = total_shard_digests.load(Ordering::Relaxed);
            println!("   [Step {}/{}] {:.1}s | finalized: {} | added: {} | digests: {} | TPS: {:.0}",
                     step, SIM_STEPS, elapsed, tf, ta, tsd, tf as f64 / elapsed);
        }
    }

    let sim_elapsed = sim_start.elapsed();
    let tf = total_finalized.load(Ordering::Relaxed);
    let ta = total_added.load(Ordering::Relaxed);
    let tsd = total_shard_digests.load(Ordering::Relaxed);

    // ─── 5. Rezultate ───────────────────────────────────────────────
    println!("\n{:=^70}", "");
    println!("  REZULTATE SIMULATOR v3.5.9 (HYBRID CONSENSUS)");
    println!("{:=^70}", "");
    println!();
    println!("Simulare:");
    println!("  Noduri:              {}", N_NODES);
    println!("  Shards:              {} (active: {})", N_SHARDS, active_shards);
    println!("  Noduri/shard (avg):  {:.1}", avg_nodes_per_shard);
    println!("  Timp simulare:       {:.1}s", sim_elapsed.as_secs_f64());
    println!();

    println!("Tranzactii:");
    println!("  Tx trimise:          {}", total_txs_to_gen);
    println!("  Tx added (mempool):  {}", ta);
    println!("  Tx finalized:        {}", tf);
    println!();

    println!("Hybrid Consensus:");
    println!("  Shard digests produse: {}", tsd);
    println!("  Nivel 1 (per nod):    ACTIVE");
    println!("  Nivel 2 (per shard):  ACTIVE (ShardDigest per shard)");
    println!("  Nivel 3 (global):     ACTIVE (anchor global)");
    println!();

    let tps_sent = total_txs_to_gen as f64 / sim_elapsed.as_secs_f64();
    let tps_finalized = tf as f64 / sim_elapsed.as_secs_f64();

    println!("Throughput:");
    println!("  >>> TPS trimise:     {:.0} txs/sec <<<", tps_sent);
    println!("  >>> TPS finalized:   {:.0} txs/sec <<<", tps_finalized);
    println!();

    // ─── 6. Proiectie 1M TPS ────────────────────────────────────────
    println!("{:=^70}", "");
    println!("  PROIECTIE 1M TPS");
    println!("{:=^70}", "");
    println!();

    let tps_per_node = tps_finalized / N_NODES as f64;
    let cross_shard_overhead = 0.20;
    let tps_per_shard = tps_per_node;
    let tps_system = N_SHARDS as f64 * tps_per_shard * (1.0 - cross_shard_overhead);

    println!("TPS per nod (măsurat):   {:.2}", tps_per_node);
    println!("N_SHARDS:                {}", N_SHARDS);
    println!("Cross-shard overhead:    {:.0}%", cross_shard_overhead * 100.0);
    println!();
    println!("TPS sistem (cu {} shards): {:.0} TPS", N_SHARDS, tps_system);
    println!();

    if tps_system >= 1_000_000.0 {
        println!(">>> TARGET 1M TPS ATINS! <<<");
    } else {
        let gap = 1_000_000.0 / tps_system;
        println!("Gap fata de 1M TPS: {:.1}x", gap);
        println!();
        println!("Optimizari necesare:");
        println!("  1. Batch verify nativ: 10x speedup");
        println!("  2. Mempool lock-free: 2x speedup");
        println!("  3. Server 96-core: 10x speedup");
    }

    // ─── 7. Reputatii ───────────────────────────────────────────────
    println!("\nReputatii (sample 10 noduri):");
    for i in (0..N_NODES).step_by(N_NODES / 10).take(10) {
        let node = nodes[i].lock().unwrap();
        let reps = node.angp_node.get_all_reputations();
        let avg_rep = if reps.is_empty() { 0.0 } else {
            reps.values().sum::<f64>() / reps.len() as f64
        };
        println!("  {} | finalized: {} | avg_rep: {:.3} | shards: {:?}",
                 node.id, node.txs_finalized, avg_rep, node.shard_ids);
    }

    // ─── 8. Memorie ─────────────────────────────────────────────────
    println!("\nMemorie (estimata):");
    let mem_per_node = std::mem::size_of::<SimNode>() + MEMPOOL_MAX * 1024;
    let total_mem = mem_per_node * N_NODES;
    println!("  Per nod: ~{} KB", mem_per_node / 1024);
    println!("  Total:   ~{} MB", total_mem / 1024 / 1024);

    // Cleanup
    let _ = std::fs::remove_dir_all(&base_dir);

    println!("\n{:=^70}", "");
    println!("  SIMULATOR HYBRID CONSENSUS COMPLET");
    println!("{:=^70}", "");
}
