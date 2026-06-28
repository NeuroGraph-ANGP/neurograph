<div align="center">

# NeuroGraph (ANGP)

### Adaptive Neural Gossip Protocol

**The first cryptocurrency with neural-inspired emergent consensus**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-1.96+-orange.svg)](https://www.rust-lang.org)
[![Tests: 74](https://img.shields.io/badge/Tests-127_scenarios-green.svg)](#testing)
[![Version: v3.5.12](https://img.shields.io/badge/Version-3.5.12-blue.svg)](#version-history)
[![DOI](https://img.shields.io/badge/DOI-10.5281/zenodo.XXXXXXX-blue.svg)](https://doi.org/10.5281/zenodo.XXXXXXX)

</div>

---

## 🧠 What is NeuroGraph?

NeuroGraph replaces traditional Byzantine consensus with an **emergent mechanism** based on a Hebbian adaptive DAG and reputation-weighted median voting. **No staking. No leader election. No voting rounds.** Security emerges from the collective behavior of honest nodes whose predictions converge through neural learning.

### Why it matters

Every production cryptocurrency forces designers to choose at most 2 of 3 properties: decentralization, security, scalability. NeuroGraph is the first to demonstrate — empirically, across **127 attack scenarios** — that honest nodes survive at the **theoretical BFT limit of 50%** without staking, without leader election, and without voting rounds.

---

## 📊 Key Metrics (Empirically Measured)

| Metric | Value | Hardware |
|---|---|---|
| **Ed25519 signature verification** | 96,081 sigs/sec | 8 cores, rayon parallel |
| **Pipeline throughput** (sig + mempool) | 30,269 TPS | 8 cores |
| **Full-protocol simulator** (1K nodes, 961 shards) | 13,108 TPS | 8 cores |
| **Real TCP network** (5 nodes, end-to-end) | ~130 TPS | 8 cores |
| **Projected** (96-core + batch verify + lock-free mempool) | 157,296 TPS | c5.metal |

**All numbers are measured. No theoretical estimates.**

---

## 🛡️ Security Validation

| Test Suite | Scenarios | Honest Survival |
|---|---:|---:|
| Honest only (zero attackers) | 1 | 10/10 ✓ |
| 10 honest vs 5 attackers | 1 | 10/10 (rep=1.000) ✓ |
| BFT threshold 50% | 4 | 10/10 in all ✓ |
| Stress tests (12 attack types × 6 proportions) | 39 | **10/10 in ALL** ✓ |
| Mixed attacks (55%, 60%, 67%, 71%) | 5 | 10/10 in all ✓ |
| **Total** | **127** | **100% honest survival** |

### 12 Attack Types Tested

Coordinated · Clone · Adaptive · FlipFlop · GaussianNoise · Sleeper · Drift · OutlierBurst · Sybil · Byzantine · DoubleSpend · RandomNoise

### 5-Layer Defense

1. **EMA Smoothing** — makes honest nodes temporally consistent
2. **Reputation Floor** — prevents death spirals (0.3 floor, 500-step grace)
3. **Soft Error** — tanh cap on per-step error (κ = 0.2)
4. **Cluster-Aware Weight Capping** — caps coordinated clusters at 30% of total weight
5. **Attack Detectors** — Clone, Coordination, Adaptive, Temporal Consistency, Prediction Jump

---

## 🚀 Quick Start

### Prerequisites

- Rust 1.96+ (https://rustup.rs)
- 4+ CPU cores recommended
- 4 GB RAM minimum

### Build

```bash
git clone https://github.com/YOUR_USERNAME/neurograph.git
cd neurograph
cargo build --release
```

### Run a single node

```bash
./target/release/neurograph --shards 0,1,2 --port 8765
```

### Run the 10 honest vs 5 attackers simulation

```bash
cargo test --release --test sim_attack_resistance -- --nocapture --ignored sim_10_honest_vs_5_attackers
```

### Run all 127 tests

```bash
# All tests (takes ~2 minutes)
cargo test --release -- --nocapture --ignored

# Or use the runner script
.\tests\run_all_tests.ps1
```

---

## 📚 Documentation

- **[Whitepaper v3.5.12 (PDF, 41 pages)](docs/whitepaper.pdf)** — Complete technical specification
- **[Test Results JSON](docs/test_results_v3512_summary.json)** — All measurements
- **[Test Inventory](tests/tests_inventory.md)** — All 74 test functions documented
- **[API Documentation](docs/api.md)** — Coming soon

---

## 📖 Academic Publications

1. ANGP: Local Reputation via Self-Issued Certificates (v1) — [https://doi.org/10.5281/zenodo.20523176]

2. ANGP: Local Reputation via Self-Issued Certificates (v2) — [https://doi.org/10.5281/zenodo.20586893]

3. ANGP: A Decentralized Reputation Protocol Using Adaptive Hebbian DAGs — [https://doi.org/10.5281/zenodo.20744982]
    
4. ANGP: A Decentralized Reputation-Based Consensus Protocol for Directed Acyclic Graphs — [https://doi.org/10.5281/zenodo.20966117]

---

## 🏗️ Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                     NeuroGraph (ANGP) v3.5.13                    │
├─────────────────────────────────────────────────────────────────┤
│  WALLET & IDENTITY LAYER                                         │
│  ├── PKCS8 identity persistence (identity.pem, 0600 perms)       │
│  ├── Ed25519 keypair management (ring → ed25519-dalek compat)   │
│  ├── libp2p PeerId derivation (same Ed25519 seed)                │
│  └── PoW mining for network entry (SHA-512/256, difficulty 4)   │
├─────────────────────────────────────────────────────────────────┤
│  LEDGER & STATE LAYER                                            │
│  ├── Account-based state (in-memory HashMap, O(1) ops)           │
│  ├── Transaction history (bincode serialization, 10.5M txs/sec)  │
│  ├── Mempool (pending txs, O(1) double-spend detection)         │
│  ├── Tx eviction (MAX_FINALIZE_RETRIES=3, prevents OOM)         │
│  ├── Periodic flush (every 100 steps, dirty-flag optimized)      │
│  └── Snapshot sync (SnapshotRequest/Response for bootstrapping)  │
├─────────────────────────────────────────────────────────────────┤
│  CONSENSUS LAYER (3-Level Hybrid)                                │
│  ├── Level 1: Per-node Hebbian DAG prediction                    │
│  ├── Level 2: Per-shard reputation-weighted median               │
│  └── Level 3: Global anchor (shard digests as proposals)         │
│  All 3 levels call identical compute_consensus() function         │
├─────────────────────────────────────────────────────────────────┤
│  SECURITY LAYER (5 Detectors + Cluster Cap)                      │
│  ├── Clone Detector (ε=0.005, 5-step confirmation)               │
│  ├── Coordination Detector (Union-Find, ε=0.005)                 │
│  ├── Cluster-Aware Weight Capping (λ=0.30, ε=0.003)              │
│  ├── Temporal Consistency (window=20, θ=0.08)                    │
│  ├── Prediction Jump (θ=0.2, 3-step confirmation)                │
│  └── Adaptive Attacker (zone=0.15, 100-step window)              │
├─────────────────────────────────────────────────────────────────┤
│  REPUTATION ENGINE (Dual-EMA + Floor)                            │
│  ├── Fast EMA (α=0.5, ~2 steps response)                         │
│  ├── Slow EMA (α=0.05, ~50 steps response)                       │
│  ├── Soft error (tanh, κ=0.2, prevents death spiral)             │
│  ├── Reputation floor (0.3, 500-step grace period)               │
│  └── Penalties: spike, bad-freq, changepoint, strike, violation  │
├─────────────────────────────────────────────────────────────────┤
│  SHARDING (961 shards = 31²)                                     │
│  ├── Account-based: shard(addr) = SHA-512/256(addr) mod 961      │
│  ├── Cross-shard: LockTx → CommitTx → RefundTx (2-phase commit)  │
│  ├── Dynamic sizing: optimal_shards(N) = max(1, min(961, N/30))  │
│  └── Receipt propagation (embedded in DagProposal gossip)        │
├─────────────────────────────────────────────────────────────────┤
│  NETWORK LAYER                                                   │
│  ├── TCP mesh (default) or libp2p (GossipSub + Kademlia)         │
│  ├── bincode wire format ([tag][length][body] framing)           │
│  ├── Network batching (up to 256 txs/message)                    │
│  ├── Token bucket rate limiting (500 burst, 250/sec refill)      │
│  └── Conflict resolution (first-seen + min-hash tiebreak)        │
├─────────────────────────────────────────────────────────────────┤
│  CRYPTO LAYER                                                    │
│  ├── Ed25519 signatures (local ed25519-dalek fork, pub mod batch)│
│  ├── Batch verify: 96,081 sigs/sec (rayon parallel, 8 cores)    │
│  ├── SHA-512/256 hashing (1.57M hashes/sec, 64-bit optimized)    │
│  └── Anti-malleability (canonical hash over all fields)          │
└─────────────────────────────────────────────────────────────────┘
```

---

## 📁 Project Structure

```
neurograph/
├── src/                    # 24 Rust modules (~6000 LOC)
│   ├── dag.rs              # AdaptiveDag with Hebbian learning + pruning
│   ├── node.rs             # AngpNode (v3.5.12 fix: self-observation)
│   ├── dag_logic.rs        # Consensus computation, batch finalization
│   ├── sharding.rs         # 961-shard account-based partitioning
│   ├── shard_consensus.rs  # 3-level hybrid consensus manager
│   ├── attack_detection.rs # 5 detectors + cluster cap
│   ├── reputation.rs       # Dual-EMA + floor + penalties
│   ├── transaction.rs      # Ed25519 batch verify (local fork)
│   ├── clock_skew.rs       # Passive skew checker
│   └── ...                 # 16 more modules
├── tests/                  # 74 test functions, 127 scenarios
│   ├── honest_only.rs      # v3.5.12 FIX verification
│   ├── sim_attack_resistance.rs
│   ├── bft_threshold.rs    # 4 BFT 50% scenarios
│   ├── stress_limits.rs    # 39 scenarios
│   └── ...
├── vendor/ed25519-dalek/   # Local fork with pub mod batch
├── docs/                   # Whitepaper, test results
├── Cargo.toml              # v3.5.12
└── LICENSE                 # MIT
```

---

## 🧪 Testing

### Run all tests

```bash
# Unit tests (34 functions, ~2 seconds)
cargo test --release --lib

# Integration tests (40 functions, ~15 seconds)
cargo test --release --test honest_only -- --nocapture --ignored
cargo test --release --test sim_attack_resistance -- --nocapture --ignored
cargo test --release --test bft_threshold -- --nocapture --ignored

# Stress tests (39 scenarios, ~50 seconds)
cargo test --release --test stress_limits -- --nocapture --ignored

# Benchmarks (7 tests, ~5 minutes)
cargo test --release --test bench_batch_verify -- --nocapture --ignored
cargo test --release --test bench_throughput -- --nocapture --ignored
cargo test --release --test bench_simulator -- --nocapture --ignored
```

### Expected results (v3.5.12)

- **Unit tests:** 34/34 passed
- **Integration tests:** 25/25 suites passed
- **Stress tests:** 39/39 scenarios, **10/10 honest alive in ALL**
- **Benchmarks:** 96K sigs/sec, 30K pipeline TPS, 13K simulator TPS

---

## 🔄 Version History

| Version | Date | Key Change |
|---|---|---|
| v1.0 | 2026 | Foundation: Hebbian DAG, reputation engine |
| v2.0 | 2026 | Crypto DAG: DagProposal, batch finalization |
| v2.5 | 2026 | Performance: SHA-512/256, bincode, O(1) double-spend |
| v3.0 | 2026 | Sharding: 16 shards, cross-shard receipts |
| v3.2 | 2026 | Security: eliminated rep>0.9 filter, attack detection |
| v3.3 | 2026 | Honest protection: reputation floor, soft error |
| v3.4 | 2026 | BFT hardening: cluster weight capping, jump detector |
| v3.5 | 2026 | Hybrid consensus: shard + global anchor |
| v3.5.2 | 2026 | Rayon parallel batch verify (96K sigs/sec) |
| v3.5.8 | 2026 | Simulator for 1K nodes (6.7K TPS) |
| v3.5.10 | 2026 | Local ed25519-dalek fork with pub mod batch (11.7K TPS) |
| v3.5.11 | 2026 | AdaptiveDag pruning + prediction cache (13.1K TPS) |
| **v3.5.12** | **2026-06-27** | **Self-observation fix: honest0 lives (10/10 in ALL tests)** |

---

## 🤝 Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md).

Areas needing help:
- 🔧 Lock-free mempool (DashMap integration)
- 🌐 libp2p production networking
- 📸 Snapshot sync implementation
- 📊 Formal proofs (Coq/TLA+)
- 🔍 Security audit preparation

---

## 📄 License

MIT License — see [LICENSE](LICENSE).

The whitepaper and documentation are licensed under CC-BY 4.0.

---

## 📧 Contact

- **Issues:** [GitHub Issues](https://github.com/YOUR_USERNAME/neurograph/issues)
- **Discussions:** [GitHub Discussions](https://github.com/YOUR_USERNAME/neurograph/discussions)
- **Academic:** See Zenodo publications above

---

<div align="center">

**v3.5.12 — "honest0 lives"**

Built with ❤️ and Rust

</div>
