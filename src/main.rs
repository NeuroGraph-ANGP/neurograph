//! v3.5.2 — Binar demo resincronizat cu API-ul curent din lib.
//!
//! Acest binar demonstrează un nod ANGP complet:
//!   1. Pornește TCP server pentru gossip (predicții + tranzacții)
//!   2. Rulează main loop: generează predicție → build proposal → gossip →
//!      compute consensus → update reputații → finalize batch
//!   3. La recepție tx: verifică semnătură + verifică clock skew (pasiv)
//!
//! NOTĂ: main.rs a fost resincronizat în v3.5.2 — versiunea anterioară folosea
//! API-uri șterse (add_remote_prediction, process_vote, finalize_based_on_median,
//! received_predictions, get_last_prediction). Acum folosește API-ul real:
//!   - AngpNode::generate_prediction()
//!   - AngpNode::add_remote_proposal()
//!   - DagLogic::build_proposal()
//!   - DagLogic::compute_consensus()
//!   - DagLogic::finalize_batch()
//!   - clock_skew::ClockSkewChecker (v3.5.1)

use neurograph::config::*;
use neurograph::utils::*;
use neurograph::attack::AttackType;
use neurograph::node::AngpNode;
use neurograph::network::{
    PredictionMessage, TransactionMessage,
    send_to_peer, send_transaction_to_peer,
    send_report, start_tcp_server,
};
use neurograph::dag_logic::DagLogic;
use neurograph::app::App;
use neurograph::transaction::Transaction;
use neurograph::clock_skew::{ClockSkewChecker, SkewVerdict};
use neurograph::sharding::{ShardSet, shard_of_address};

use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::thread;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use rand::Rng;
use flume;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut port = 0u16;
    let mut node_name = format!("node{}", rand::thread_rng().gen::<u16>());
    let mut attack_type = AttackType::Honest;
    let mut known_peers: Vec<SocketAddr> = Vec::new();
    // v3.5.1: enable/disable clock skew checker (default ON, pasiv)
    let mut skew_checker_enabled = true;
    // v3.5.11: Sharding configuration. None = legacy (all shards), Some = subset.
    let mut shard_ids: Vec<u32> = Vec::new();
    let mut sharding_enabled = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --port requires a value");
                    std::process::exit(1);
                }
                port = args[i + 1].parse()?;
                i += 2;
            }
            "--name" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --name requires a value");
                    std::process::exit(1);
                }
                node_name = args[i + 1].clone();
                i += 2;
            }
            "--attack-type" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --attack-type requires a value");
                    std::process::exit(1);
                }
                attack_type = AttackType::from_str(&args[i + 1]);
                i += 2;
            }
            "--peer" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --peer requires a value");
                    std::process::exit(1);
                }
                known_peers.push(args[i + 1].parse()?);
                i += 2;
            }
            "--no-skew-check" => {
                skew_checker_enabled = false;
                i += 1;
            }
            // v3.5.11: Sharding activation. Format: --shards 0,1,2,3
            "--shards" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --shards requires a value (e.g. --shards 0,1,2)");
                    std::process::exit(1);
                }
                sharding_enabled = true;
                shard_ids = args[i + 1]
                    .split(',')
                    .filter_map(|s| s.trim().parse::<u32>().ok())
                    .collect();
                if shard_ids.is_empty() {
                    eprintln!("Error: --shards value could not be parsed. Use format: 0,1,2");
                    std::process::exit(1);
                }
                // Validare: shard IDs trebuie să fie < N_SHARDS
                for &sid in &shard_ids {
                    if sid >= neurograph::config::N_SHARDS {
                        eprintln!("Error: shard id {} is out of range (max {})",
                                 sid, neurograph::config::N_SHARDS - 1);
                        std::process::exit(1);
                    }
                }
                i += 2;
            }
            _ => {
                eprintln!("Warning: unknown argument '{}'", args[i]);
                i += 1;
            }
        }
    }

    if port == 0 {
        eprintln!("Usage: --port <PORT> --attack-type <TYPE> [--name <NAME>] [--peer <IP:PORT> ...] [--no-skew-check] [--shards 0,1,2]");
        eprintln!("Attack types: honest, coordinated, random, gaussian, flipflop, sleeper, drift, outlier, adaptive, clone, sybil");
        eprintln!("Sharding: --shards 0,1,2 (proceseaza doar shards 0,1,2 din {})", neurograph::config::N_SHARDS);
        std::process::exit(1);
    }

    // v3.5.11: Construire ShardSet. Daca --shards nu e specificat, legacy mode (toate shards).
    let shard_set: ShardSet = if sharding_enabled {
        ShardSet::only(&shard_ids)
    } else {
        ShardSet::all()
    };

    let data_dir = format!("./data/{}", node_name);
    let dag_logic = Arc::new(Mutex::new(DagLogic::new(1000, &data_dir)));

    // v3.5.1: Clock skew checker — pasiv, doar log/warn, nu respinge
    let skew_checker = Arc::new(ClockSkewChecker::with_default());

    // Channels pentru gossip și CLI
    let (gossip_tx, gossip_rx) = flume::unbounded::<Transaction>();
    let (cli_line_tx, cli_line_rx) = flume::unbounded::<String>();

    let app = App::new(dag_logic.clone(), node_name.clone(), gossip_tx.clone());
    let exit_signal = app.exit_signal.clone();

    let mut angp_node = AngpNode::new(node_name.clone(), attack_type.clone());

    let (tx_pred, rx_pred) = flume::unbounded::<PredictionMessage>();
    let (tx_tx, rx_tx) = flume::unbounded::<TransactionMessage>();

    let tx_pred_clone = tx_pred.clone();
    let tx_tx_clone = tx_tx.clone();
    let _server_handle = thread::spawn(move || {
        if let Err(e) = start_tcp_server(port, tx_pred_clone, tx_tx_clone) {
            eprintln!("TCP server error: {}", e);
        }
    });

    thread::sleep(Duration::from_millis(100));

    println!("Node {} started on port {}, attack type: {:?} (ANGP v3.5.11, PoW SHA-512/256 diff {})",
             node_name, port, attack_type, POW_DIFFICULTY);
    println!("Clock skew checker: {} (threshold={}ms, severe={}ms)",
             if skew_checker_enabled { "ENABLED (passive)" } else { "DISABLED" },
             neurograph::clock_skew::DEFAULT_MAX_SKEW_MS,
             neurograph::clock_skew::SEVERE_SKEW_MS);
    // v3.5.11: Afisare configuratie sharding
    if shard_set.is_sharded() {
        let shards_list = shard_set.list().unwrap_or_default();
        println!("Sharding: ENABLED — processing {} of {} shards: {:?}",
                 shards_list.len(), neurograph::config::N_SHARDS, shards_list);
        let my_shard = shard_of_address(&node_name);
        println!("  My node '{}' is in shard {} ({} processed)",
                 node_name, my_shard,
                 if shard_set.contains(my_shard) { "IS" } else { "NOT" });
    } else {
        println!("Sharding: DISABLED (legacy mode — processing all {} shards)",
                 neurograph::config::N_SHARDS);
    }

    let mut step = 0u64;
    let _start_time = Instant::now();

    // stdin reader thread
    let exit_for_reader = exit_signal.clone();
    thread::spawn(move || {
        loop {
            if let Ok(true) = exit_for_reader.lock().map(|e| *e) {
                break;
            }
            let mut input = String::new();
            match std::io::stdin().read_line(&mut input) {
                Ok(0) | Err(_) => break,
                Ok(_) => { let _ = cli_line_tx.send(input); }
            }
        }
    });

    // CLI processor thread
    let cli_handle = thread::spawn(move || {
        let mut app_clone = app;
        app_clone.run_cli(cli_line_rx);
    });

    // MAIN LOOP
    loop {
        // ─── 1. Procesare mesaje de predicții de la peers ─────────────
        while let Ok(msg) = rx_pred.try_recv() {
            let pred_arr = vec_to_array(&msg.prediction);
            let rep_map: HashMap<String, f64> = HashMap::new();
            let recent_tips = angp_node.get_recent_common_tips().clone();
            let consensus_median = angp_node.get_consensus().median_prediction.clone();
            let proposal = dag_logic.lock().unwrap().build_proposal(
                &msg.sender, msg.step, msg.seq, msg.nonce,
                &rep_map, &pred_arr, &consensus_median, &recent_tips,
                Vec::new(),
            );
            angp_node.add_remote_proposal(proposal);
        }

        // ─── 2. Procesare mesaje de tranzacții (gossip) ───────────────
        while let Ok(msg) = rx_tx.try_recv() {
            let tx = msg.transaction;

            // v3.5.11: Sharding filter — procesam doar tx-uri din shards asignate.
            // Daca sender nu e intr-un shard al nostru, skip (alt nod il va procesa).
            // ATENTIE: aceasta filtrare e INAINDE de verificarea semnaturii,
            // ca sa economisim CPU pe tx-uri pe care oricum nu le procesam.
            // Dar clock skew checker ruleaza oricum (pentru monitorizare globala).
            if !shard_set.processes_tx(&tx) {
                // Tx nu e pentru shards noastre — skip silently (rețeaua gossip-uiește tot).
                continue;
            }

            // v3.5.1: Clock skew check — PASIV, doar warn
            if skew_checker_enabled {
                let verdict = skew_checker.check(tx.timestamp, &msg.sender);
                match verdict {
                    SkewVerdict::Warning => {
                        println!("[{}] WARN skew from {}: tx_ts={} (over threshold)",
                                 node_name, msg.sender, tx.timestamp);
                    }
                    SkewVerdict::Severe => {
                        println!("[{}] SEVERE skew from {}: tx_ts={} (possible desync)",
                                 node_name, msg.sender, tx.timestamp);
                    }
                    SkewVerdict::Ok => {}
                }
            }

            // Verificare semnătură
            if !tx.verify_signature() {
                println!("[{}] Transaction rejected from {}: invalid signature", node_name, msg.sender);
                continue;
            }

            // Adăugare în DAG
            let mut logic = dag_logic.lock().unwrap();
            if logic.add_transaction_from_gossip(tx.clone()) {
                drop(logic);
                // Re-gossip către peers cunoscuți
                let gossip_msg = TransactionMessage {
                    sender: node_name.clone(),
                    transaction: tx,
                };
                for peer in &known_peers {
                    for retry in 0..3 {
                        send_transaction_to_peer(*peer, &gossip_msg);
                        if retry < 2 {
                            thread::sleep(Duration::from_millis(50));
                        }
                    }
                }
            }
        }

        // ─── 3. Procesare tranzacții locale (din CLI) ─────────────────
        while let Ok(tx) = gossip_rx.try_recv() {
            // v3.5.11: Sharding filter pentru tx-uri locale.
            // Daca sender nu e intr-un shard al nostru, nu trimitem la peers
            // (nu e responsabilitatea noastra sa gossip-uim tx-uri pe care nu le procesam).
            // Dar daca e intra-shard (sender si receiver in acelasi shard al nostru), OK.
            // Daca e cross-shard, trimitem oricum — alt nod cu shard-ul receiver va procesa.
            if shard_set.is_sharded() && !shard_set.processes_tx(&tx) {
                let from_shard = shard_of_address(&tx.sender);
                let to_shard = shard_of_address(&tx.receiver);
                println!("[{}] Skip local tx: from_shard={} not in our shards (to_shard={})",
                         node_name, from_shard, to_shard);
                continue;
            }

            let gossip_msg = TransactionMessage {
                sender: node_name.clone(),
                transaction: tx,
            };
            for peer in &known_peers {
                send_transaction_to_peer(*peer, &gossip_msg);
            }
        }

        // ─── 4. Generare predicție neurală pentru pasul curent ────────
        let t = step as f64 * 0.5;
        let prediction = angp_node.generate_prediction(t, step);

        // ─── 5. Build proposal local ──────────────────────────────────
        let rep_map = angp_node.get_all_reputations();
        let consensus_median = angp_node.get_consensus().median_prediction.clone();
        let recent_tips = angp_node.get_recent_common_tips().clone();
        // v3.5.2: Build proposal local (pentru tracking intern, nu trimitem direct
        // — prediction e deja trimisă ca PredictionMessage către peers)
        // v3.5.12 FIX: Adăugăm propria propunere la received_proposals ca să se observe pe sine.
        let _local_proposal = {
            let mut logic = dag_logic.lock().unwrap();
            let proposal = logic.build_proposal(
                &node_name, step, angp_node.next_seq, angp_node.pow_nonce,
                &rep_map, &prediction, &consensus_median, &recent_tips,
                Vec::new(),
            );
            // v3.5.12: auto-observare — nodul își vede propria propunere
            angp_node.add_own_proposal(proposal.clone());
            proposal
        };

        // Trimite proposal-ul către peers
        let msg = PredictionMessage {
            sender: angp_node.id.clone(),
            step,
            seq: angp_node.next_seq,
            nonce: angp_node.pow_nonce,
            prediction: array_to_vec(&prediction),
        };
        for peer in &known_peers {
            send_to_peer(*peer, &msg);
        }
        angp_node.next_seq += 1;

        // ─── 6. Compute consensus emergent ────────────────────────────
        let active_proposals = angp_node.collect_active_proposals(step);
        let (new_consensus, errors) = {
            let logic = dag_logic.lock().unwrap();
            logic.compute_consensus(&active_proposals, &rep_map)
        };

        // ─── 7. Update reputații pe baza erorilor ─────────────────────
        angp_node.update_reputations(step, &errors);

        // ─── 8. Record common tips pentru momentum (v2.4 #3) ──────────
        angp_node.record_common_tips(new_consensus.common_tips.clone());

        // ─── 9. Set consensus (pentru pasul următor) ──────────────────
        angp_node.set_consensus(new_consensus.clone());

        // ─── 10. Finalize batch dacă e momentul ────────────────────────
        if DagLogic::should_finalize(step) {
            let mut logic = dag_logic.lock().unwrap();
            let (finalized, rewards) = logic.finalize_batch(&new_consensus);
            if !finalized.is_empty() {
                println!("Finalized {} transactions (rewards={} milliANGP)",
                         finalized.len(), rewards);
                for hash in finalized.iter().take(5) {
                    if let Some(tx) = logic.get_ledger().finalized.get(hash) {
                        println!("  - {}", tx);
                    }
                }
            }
        }

        // ─── 11. Raportare periodică (la 500 pași) ────────────────────
        if step > 0 && step % 500 == 0 {
            println!("\n[Step {}] Status:", step);
            let senders: Vec<String> = {
                let mut s: Vec<String> = angp_node.get_all_reputations().keys().cloned().collect();
                s.sort();
                s
            };

            let consensus_vec = &new_consensus.median_prediction;

            let (ledger_count, mempool_count) = {
                let logic = dag_logic.lock().unwrap();
                (logic.get_ledger_len(), logic.get_mempool_len())
            };

            println!("Total finalized transactions: {}", ledger_count);
            println!("Mempool size: {}", mempool_count);

            println!("\nPer-node details:");
            println!("{:<15} {:<8} {:<10} {:<12} {:<12}",
                     "Node", "Rep", "Messages", "Distance", "Status");

            let mut reputations_map: HashMap<String, f64> = HashMap::new();
            for sender in &senders {
                let rep = angp_node.get_reputation(sender).unwrap_or(0.0);
                let msg_count = angp_node.get_message_count(sender);
                let dist = if let Some(prop) = angp_node.get_last_proposal(sender) {
                    let pred = vec_to_array(&prop.prediction);
                    // v3.5.3 FIX: Verificam shape ca sa evitam panic daca consensus_vec e empty.
                    if pred.len() == consensus_vec.len() {
                        let err = norm(&(pred - consensus_vec));
                        format!("{:.4}", err)
                    } else {
                        "N/A".to_string()
                    }
                } else {
                    "N/A".to_string()
                };
                let status = if rep > 0.7 { "OK" } else if rep > 0.3 { "WARN" } else { "BAD" };
                println!("{:<15} {:<8.3} {:<10} {:<12} {:<12}",
                         sender, rep, msg_count, dist, status);
                reputations_map.insert(sender.clone(), rep);
            }

            // Trimite raport la observer (dacă e pornit)
            send_report(step, &reputations_map, "127.0.0.1:20001");

            // v3.5.1: Raport clock skew
            if skew_checker_enabled {
                let skew_stats = skew_checker.global_stats();
                println!("\nClock Skew Stats:");
                println!("  Total checked: {}", skew_stats.total_checked);
                println!("  Warnings: {} (severe: {})", skew_stats.warnings, skew_stats.severe);
                println!("  Max skew: {}ms | Mean: {:.2}ms",
                         skew_stats.max_skew_ms, skew_stats.mean_skew_ms());
                let offenders = skew_checker.top_offenders(3);
                if !offenders.is_empty() {
                    println!("  Top offenders:");
                    for (peer, stats) in offenders {
                        println!("    - {}: {} warnings (max {}ms)",
                                 peer, stats.warnings, stats.max_skew_ms);
                    }
                }
            }

            println!("System status:");
            println!("  - AdaptiveDag nodes: {}", angp_node.dag_node_count());
            println!("  - Last adaptive α: {:.4}", angp_node.last_adaptive_alpha);
        }

        step += 1;
        if step >= 5000 {
            let final_reputations = angp_node.get_all_reputations();
            send_report(step, &final_reputations, "127.0.0.1:20001");
            println!("\nSimulation finished after {} steps", step);
            break;
        }

        // v3.5.5: Folosim LOOP_SLEEP_MS din config (10ms = 100 pasi/sec, era 50ms)
        thread::sleep(Duration::from_millis(neurograph::config::LOOP_SLEEP_MS));
    }

    // Signal CLI to exit
    *exit_signal.lock().unwrap() = true;
    cli_handle.join().unwrap();

    Ok(())
}
