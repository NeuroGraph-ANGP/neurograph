//! Integration tests pentru v2.2: Wallet, ConflictResolver, RateLimiter, Snapshot.
//!
//! Aceste teste NU schimbă protocolul ANGP. Verifică doar că noile module
//! funcționează corect izolat.

use neurograph::wallet::Wallet;
use neurograph::conflict::{ConflictResolver, ConflictStatus};
use neurograph::rate_limit::{RateLimiter, RateLimitResult};
use neurograph::snapshot::SnapshotResponse;
use neurograph::transaction::Transaction;
use neurograph::ledger::Ledger;
use neurograph::state::StateManager;

mod common;

#[test]
fn test_wallet_persistence() {
    let tmp = common::TempDir::new("wallet_persistence");
    let wallet1 = Wallet::load_or_create(tmp.path(), "alice").expect("create wallet");
    let pub_key_1 = wallet1.public_key_hex.clone();

    // Re-încărcăm din același director — trebuie să obținem aceeași cheie
    let wallet2 = Wallet::load_or_create(tmp.path(), "alice").expect("load wallet");
    assert_eq!(wallet2.public_key_hex, pub_key_1,
        "Wallet must persist identity across restarts");
}

#[test]
fn test_wallet_unique_identities() {
    let tmp1 = common::TempDir::new("wallet_unique_1");
    let tmp2 = common::TempDir::new("wallet_unique_2");
    let w1 = Wallet::load_or_create(tmp1.path(), "alice").unwrap();
    let w2 = Wallet::load_or_create(tmp2.path(), "bob").unwrap();
    assert_ne!(w1.public_key_hex, w2.public_key_hex,
        "Two different wallets must have different public keys");
}

#[test]
fn test_conflict_first_seen_then_conflict() {
    let mut resolver = ConflictResolver::new();

    // Prima tx cu (sender="alice", nonce=1)
    let tx1 = Transaction::new_with_nonce(
        "alice".to_string(), "bob".to_string(), 100, 1, vec![],
    );
    let status1 = resolver.observe(&tx1, "observer_node", 0);
    assert!(matches!(status1, ConflictStatus::FirstSeen));

    // A doua tx cu același (sender, nonce) dar conținut diferit → conflict
    let tx2 = Transaction::new_with_nonce(
        "alice".to_string(), "carol".to_string(), 200, 1, vec![],
    );
    let status2 = resolver.observe(&tx2, "observer_node", 0);
    match status2 {
        ConflictStatus::Conflict(vote) => {
            assert_eq!(vote.sender, "alice");
            assert_eq!(vote.nonce, 1);
            // Votul favorizează candidata cu cel mai mic hash (regulă deterministă)
            let expected = std::cmp::min(tx1.hash, tx2.hash);
            assert_eq!(vote.favored_tx, expected);
        }
        _ => panic!("Expected Conflict, got {:?}", status2),
    }
}

#[test]
fn test_conflict_duplicate_ignored() {
    let mut resolver = ConflictResolver::new();
    let tx = Transaction::new_with_nonce(
        "alice".to_string(), "bob".to_string(), 100, 1, vec![],
    );
    let _ = resolver.observe(&tx, "observer", 0);
    let status = resolver.observe(&tx, "observer", 0);
    assert!(matches!(status, ConflictStatus::Duplicate),
        "Re-observing the same tx must return Duplicate");
}

#[test]
fn test_conflict_resolution_weighted_by_reputation() {
    let mut resolver = ConflictResolver::new();
    let tx_a = Transaction::new_with_nonce("alice".to_string(), "bob".to_string(), 100, 1, vec![]);
    let tx_b = Transaction::new_with_nonce("alice".to_string(), "carol".to_string(), 200, 1, vec![]);

    // Observăm ambele tx-uri (al doilea declanșează conflict)
    let _ = resolver.observe(&tx_a, "node_x", 0);
    let _ = resolver.observe(&tx_b, "node_x", 0);

    // 3 noduri votează pentru tx_a (cu reputații mari), 1 pentru tx_b
    use neurograph::conflict::ConflictVote;
    resolver.record_vote(ConflictVote {
        voter: "voter1".to_string(), sender: "alice".to_string(),
        nonce: 1, favored_tx: tx_a.hash, step: 0,
    });
    resolver.record_vote(ConflictVote {
        voter: "voter2".to_string(), sender: "alice".to_string(),
        nonce: 1, favored_tx: tx_a.hash, step: 0,
    });
    resolver.record_vote(ConflictVote {
        voter: "voter3".to_string(), sender: "alice".to_string(),
        nonce: 1, favored_tx: tx_b.hash, step: 0,
    });

    let mut reps = std::collections::HashMap::new();
    reps.insert("voter1".to_string(), 0.95);
    reps.insert("voter2".to_string(), 0.95);
    reps.insert("voter3".to_string(), 0.10);  // vot cu reputație mică

    // Fereastra de vot = 20 pași → forțăm expirarea
    let winner = resolver.resolve("alice", 1, 25, &reps);
    assert!(winner.is_some(), "Conflict should resolve after window expires");
    let winner = winner.unwrap();
    // tx_a ar trebui să câștige (2 voturi cu rep mare vs 1 cu rep mică)
    assert_eq!(winner, tx_a.hash,
        "Winner should be tx_a (higher weighted vote count)");
}

#[test]
fn test_rate_limiter_allows_burst() {
    let mut rl = RateLimiter::new();
    // Burst = 500 mesaje; primele 500 ar trebui permise
    for i in 0..500 {
        let result = rl.check(&format!("peer_{}", i % 5));
        assert!(matches!(result, RateLimitResult::Allowed),
            "Message {} should be allowed within burst", i);
    }
}

#[test]
fn test_rate_limiter_denies_after_burst() {
    let mut rl = RateLimiter::new();
    // Trimitem 750 de mesaje de la același peer (burst = 500 în v2.5)
    let mut allowed = 0;
    let mut denied = 0;
    for _ in 0..750 {
        match rl.check("spammer") {
            RateLimitResult::Allowed => allowed += 1,
            RateLimitResult::Denied { .. } => denied += 1,
        }
    }
    assert_eq!(allowed, 500, "Burst should allow exactly 500 messages (v2.5)");
    assert_eq!(denied, 250, "Messages over burst should be denied");
}

#[test]
fn test_rate_limiter_recovers_after_time() {
    use std::thread;
    use std::time::Duration;
    let mut rl = RateLimiter::new();
    // Epuizăm burst-ul
    for _ in 0..500 {
        let _ = rl.check("peer");
    }
    // Toate respinse imediat
    assert!(matches!(rl.check("peer"), RateLimitResult::Denied { .. }));
    // Așteptăm 100ms — la 250 tok/s, ar trebui să recuperăm ~25 tokeni
    thread::sleep(Duration::from_millis(100));
    let result = rl.check("peer");
    assert!(matches!(result, RateLimitResult::Allowed),
        "Rate limiter should allow messages after refill");
}

#[test]
fn test_snapshot_build_and_apply() {
    let tmp = common::TempDir::new("snapshot_test");

    // Cream un ledger + state cu câteva date
    let ledger_path = format!("{}/ledger.json", tmp.path());
    let state_path = format!("{}/state.json", tmp.path());
    let mut ledger = Ledger::new(ledger_path);
    let mut state = StateManager::new(state_path);

    // Adăugăm o tranzacție
    let tx = Transaction::new_with_nonce(
        "alice".to_string(), "bob".to_string(), 100, 1, vec![],
    );
    ledger.add(tx);
    state.genesis_allocate("alice");

    // Construim snapshot-ul
    let snap = SnapshotResponse::build("responder_node", 42, &ledger, &state);
    assert_eq!(snap.last_step, 42);
    assert_eq!(snap.ledger.entries.len(), 1);
    assert_eq!(snap.state.balances.len(), 1);

    // Aplicăm pe un alt ledger gol
    let tmp2 = common::TempDir::new("snapshot_apply");
    let ledger2_path = format!("{}/ledger.json", tmp2.path());
    let state2_path = format!("{}/state.json", tmp2.path());
    let mut ledger2 = Ledger::new(ledger2_path);
    let mut state2 = StateManager::new(state2_path);
    assert_eq!(ledger2.len(), 0);

    let (entries, accounts) = snap.apply(&mut ledger2, &mut state2);
    assert_eq!(entries, 1, "Snapshot should apply 1 entry");
    assert_eq!(accounts, 1, "Snapshot should apply 1 account");
    assert_eq!(ledger2.len(), 1);
    assert_eq!(state2.get_balance("alice"), 1_000_000);
}
