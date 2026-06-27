use crate::transaction::Transaction;
use crate::dag_logic::DagLogic;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use flume;

pub struct App {
    pub dag_logic: Arc<Mutex<DagLogic>>,
    pub exit_signal: Arc<Mutex<bool>>,
    /// v3.1: ed25519-dalek SigningKey (înlocuiește ring::Ed25519KeyPair).
    pub signing_key: SigningKey,
    pub node_id: String,
    gossip_tx: flume::Sender<Transaction>,
}

impl App {
    pub fn new(
        dag_logic: Arc<Mutex<DagLogic>>,
        node_id: String,
        gossip_tx: flume::Sender<Transaction>,
    ) -> Self {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        Self::new_with_wallet(dag_logic, node_id, gossip_tx, signing_key)
    }

    /// v3.1: Constructor cu SigningKey persistent (de la Wallet).
    pub fn new_with_wallet(
        dag_logic: Arc<Mutex<DagLogic>>,
        node_id: String,
        gossip_tx: flume::Sender<Transaction>,
        signing_key: SigningKey,
    ) -> Self {
        App {
            dag_logic,
            exit_signal: Arc::new(Mutex::new(false)),
            signing_key,
            node_id,
            gossip_tx,
        }
    }

    /// Submit o tranzacție cu nonce secvențial + fee implicit.
    pub fn submit_transaction(&mut self, receiver: String, amount: u64) -> bool {
        self.submit_transaction_with_fee(receiver, amount, crate::config::DEFAULT_TX_FEE_MILLI)
    }

    pub fn submit_transaction_with_fee(&mut self, receiver: String, amount: u64, fee: u64) -> bool {
        let sender = self.node_id.clone();
        let mut logic = self.dag_logic.lock().unwrap();

        // Determină nonce secvențial
        let next_nonce = {
            let mempool_last = logic.get_mempool().get_last_nonce(&sender).unwrap_or(0);
            let ledger_last = logic.get_ledger().last_finalized_nonce(&sender).unwrap_or(0);
            mempool_last.max(ledger_last) + 1
        };

        let parents = logic.select_parents(3);
        let mut tx = Transaction::new_with_fee(
            sender, receiver, amount, next_nonce, fee, parents,
        );
        tx.sign(&self.signing_key);

        if !tx.verify_signature() {
            println!("[App] BUG: signed tx failed verification");
            return false;
        }

        let tx_for_gossip = tx.clone();
        if logic.add_transaction(tx) {
            let _ = self.gossip_tx.send(tx_for_gossip);
            true
        } else {
            false
        }
    }

    /// Etapa 3: Alocare genesis pentru un cont (doar pentru demo/testare).
    pub fn genesis_allocate(&self, addr: &str) {
        let mut logic = self.dag_logic.lock().unwrap();
        logic.get_state_mut().genesis_allocate(addr);
        println!("[App] Genesis allocation: {} → {} milliANGP", addr, crate::config::GENESIS_BALANCE_MILLI);
    }

    pub fn print_status(&self) {
        let logic = self.dag_logic.lock().unwrap();
        println!("─── NeuroGraph Status ───");
        println!("  Mempool: {} txs", logic.get_mempool_len());
        println!("  Ledger: {} finalized entries", logic.get_ledger_len());
        println!("  State: {} accounts", logic.get_state().len());
        let tips = logic.get_ledger().get_tips();
        println!("  Tips: {}", tips.len());
        println!("  Total fees collected: {} milliANGP", logic.get_ledger().total_fees_collected());

        println!("\nTop balances:");
        for (addr, bal) in logic.get_state().top_balances(5) {
            println!("  {} → {} milliANGP ({}.{:03} ANGP)",
                addr, bal, bal / 1000, bal % 1000);
        }
    }

    pub fn run_cli(&mut self, line_rx: flume::Receiver<String>) {
        println!("NeuroGraph CLI v2.0 — Commands:");
        println!("  tx <receiver> <amount> [fee]    - send a transaction (amounts in milliANGP)");
        println!("  genesis <addr>                  - allocate genesis balance to addr");
        println!("  balance <addr>                  - show balance of addr");
        println!("  status                          - show system status");
        println!("  exit                            - stop the node");
        println!();

        loop {
            if let Ok(true) = self.exit_signal.lock().map(|e| *e) { break; }
            match line_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(input) => {
                    let parts: Vec<&str> = input.trim().split_whitespace().collect();
                    if parts.is_empty() { continue; }
                    match parts[0] {
                        "tx" => {
                            if parts.len() < 3 {
                                println!("Format: tx <receiver> <amount> [fee]");
                                continue;
                            }
                            let receiver = parts[1].to_string();
                            let amount: u64 = match parts[2].parse() {
                                Ok(v) => v, Err(_) => { println!("Invalid amount"); continue; }
                            };
                            let fee: u64 = parts.get(3)
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(crate::config::DEFAULT_TX_FEE_MILLI);
                            if self.submit_transaction_with_fee(receiver, amount, fee) {
                                println!("✓ Transaction added to mempool (fee={} milliANGP)", fee);
                            } else {
                                println!("✗ Transaction rejected (invalid sig / double-spend / insufficient balance / mempool full)");
                            }
                        }
                        "genesis" => {
                            if parts.len() < 2 { println!("Format: genesis <addr>"); continue; }
                            self.genesis_allocate(parts[1]);
                        }
                        "balance" => {
                            if parts.len() < 2 { println!("Format: balance <addr>"); continue; }
                            let logic = self.dag_logic.lock().unwrap();
                            let bal = logic.get_state().get_balance(parts[1]);
                            println!("{} → {} milliANGP ({}.{:03} ANGP)",
                                parts[1], bal, bal / 1000, bal % 1000);
                        }
                        "status" => self.print_status(),
                        "exit" | "quit" => {
                            println!("Exiting...");
                            let _ = self.exit_signal.lock().map(|mut e| *e = true);
                            break;
                        }
                        _ => println!("Unknown command. Use: tx, genesis, balance, status, exit"),
                    }
                }
                Err(flume::RecvTimeoutError::Timeout) => continue,
                Err(flume::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    pub fn stop(&self) {
        let _ = self.exit_signal.lock().map(|mut e| *e = true);
    }
}
