# Contributing to NeuroGraph (ANGP)

Thanks for your interest in contributing! This document covers the basics.

## 🚀 Quick Start for Contributors

1. **Fork** the repository
2. **Clone** your fork: `git clone https://github.com/YOUR_USERNAME/neurograph.git`
3. **Create a branch**: `git checkout -b feature/my-feature`
4. **Build**: `cargo build --release`
5. **Test**: `cargo test --release -- --nocapture --ignored`
6. **Commit**: `git commit -m "feat: add my feature"`
7. **Push**: `git push origin feature/my-feature`
8. **Open Pull Request**

## 🧪 Testing Requirements

All PRs must pass:

```bash
# Unit tests (34 functions)
cargo test --release --lib

# Honest-only test (must be 10/10)
cargo test --release --test honest_only -- --nocapture --ignored honest_only_no_attackers

# BFT threshold (4 scenarios)
cargo test --release --test bft_threshold -- --nocapture --ignored

# Stress tests (39 scenarios, must be 10/10 honest in all)
cargo test --release --test stress_limits -- --nocapture --ignored
```

**No PR is merged if any honest node dies in any scenario.**

## 🎯 Areas Needing Help

### High Priority

- **Lock-free mempool** — replace `Mutex<HashMap>` with `DashMap` in `src/mempool.rs`
- **libp2p networking** — implement GossipSub + Kademlia in `src/p2p.rs`
- **Snapshot sync** — implement `SnapshotRequest`/`SnapshotResponse` in `src/snapshot.rs`
- **Dynamic shard sizing** — integrate `optimal_shards()` into `src/config.rs`

### Medium Priority

- **Formal proofs** — Coq/TLA+ for the 3 theorems in whitepaper
- **GPU acceleration** — CUDA for batch Ed25519 verification
- **Wallet CLI** — command-line wallet for end users
- **Block explorer** — web UI for transaction browsing

### Low Priority

- **Bridge Ethereum** — cross-chain bridge
- **SDK** — JavaScript, Python bindings
- **Mobile wallet** — iOS/Android

## 📝 Code Style

- Follow Rust standard formatting (`cargo fmt`)
- Use `#![warn(clippy::all)]` in modules
- Document public functions with `///` comments
- Add `#[cfg(test)]` modules for new functionality
- Use descriptive variable names (avoid `x`, `y` except for coordinates)

## 🐛 Reporting Bugs

Open a [GitHub Issue](https://github.com/YOUR_USERNAME/neurograph/issues) with:

1. **Version** (e.g., v3.5.12)
2. **Platform** (OS, Rust version, CPU cores)
3. **Steps to reproduce**
4. **Expected vs actual behavior**
5. **Logs** (use `--nocapture` flag)

## 💬 Discussions

For questions, ideas, or general discussion, use [GitHub Discussions](https://github.com/YOUR_USERNAME/neurograph/discussions).

## 📜 License

By contributing, you agree that your contributions will be licensed under the MIT License.
