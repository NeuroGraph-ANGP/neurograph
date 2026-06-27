# Release v3.5.12 — "honest0 lives"

**Release date:** June 27, 2026

## 🎉 Major Fix: Self-Observation Bug

This release fixes a critical bug present since v1.0 where the first honest node in any deployment would collapse to reputation 0.000 — even without any attackers present.

### Root Cause

In `node.rs`, the function `get_all_reputations()` iterated only over `received_proposals.keys()`. But each node never added its own proposal to `received_proposals` (the gossip simulation excluded self with `if dp.sender != node.id`). As a result:

- `honest0` never appeared in its own `rep_map`
- `compute_consensus()` treated `honest0` with default reputation 0
- `update_reputations()` never updated `honest0`'s own reputation
- All other 9 nodes saw `honest0` correctly (rep = 0.998), but `honest0` saw itself as nonexistent

### Fix

Three coordinated changes:

1. **`node.rs::get_all_reputations()`** — now includes `self.id` in the returned map
2. **`node.rs::update_reputations()`** — now includes `self.id` in `active_senders`
3. **`main.rs`** — now calls `add_own_proposal()` after `build_proposal()`

## 📊 Test Results

| Metric | v3.5.11 (buggy) | v3.5.12 (fixed) |
|---|---|---|
| `honest_only` (10 honest, 0 attackers) | 9/10 (honest0 dead) | **10/10** ✓ |
| `sim_attack_resistance` (10h vs 5a) | 9/10 | **10/10** (rep=1.000) ✓ |
| `bft_threshold` (4 scenarios) | 3/4 passed | **4/4 passed** ✓ |
| `stress_limits` (39 scenarios) | 9/10 honest (honest0 dead) | **10/10 honest in ALL** ✓ |
| Previously failing `bft_test_2_adaptive_50pct` | FAILED | **PASSED** ✓ |

**Total: 127 scenarios, 10/10 honest survival in all.**

## 🚀 Performance (Unchanged from v3.5.11)

| Benchmark | Value | Hardware |
|---|---|---|
| Ed25519 signature verification | 96,081 sigs/sec | 4 cores, rayon parallel |
| Pipeline throughput (sig + mempool) | 30,269 TPS | 4 cores |
| Full-protocol simulator (1K nodes) | 13,108 TPS | 4 cores, 961 shards |
| Real TCP network (5 nodes) | ~130 TPS | 4 cores |
| Projected (96-core + optimizations) | 2,421,000 TPS | c5.metal |

## 📚 Publications

This release is documented in:
- **Whitepaper v3.5.12** (48 pages, PDF) — `docs/whitepaper.pdf`
- **Zenodo DOI:** 10.5281/zenodo.XXXXXXX (publish after upload)

## 📦 Download

### Source Code

- `neurograph_v3.5.12_final.zip` (280 KB) — complete source code
- `neurograph_all_tests_v3.5.12.zip` (54 KB) — all 74 test functions

### Build Instructions

```bash
# Prerequisites: Rust 1.96+
git clone https://github.com/YOUR_USERNAME/neurograph.git
cd neurograph
cargo build --release

# Run tests
cargo test --release --test honest_only -- --nocapture --ignored honest_only_no_attackers
# Expected: "Honest alive: 10/10" ✓
```

## 🔄 Migration from v3.5.11

If you have v3.5.11 deployed:

1. Pull v3.5.12: `git pull origin main && git checkout v3.5.12`
2. Rebuild: `cargo build --release`
3. Restart your nodes — no data migration required (state format unchanged)

## 🙏 Acknowledgments

Bug discovered through systematic stress testing with the new `honest_only_no_attackers` test scenario. Thanks to the iterative development methodology that surfaced this issue.

## 📋 Full Changelog

- **Fixed:** `honest0` self-observation bug (node.rs + main.rs)
- **Added:** `honest_only.rs` test (verifies the fix)
- **Updated:** Version to 3.5.12 in Cargo.toml
- **Updated:** Whitepaper Section 7.6 documents the fix
- **Updated:** All test tables in whitepaper reflect 10/10 results

---

**Full Changelog:** https://github.com/YOUR_USERNAME/neurograph/compare/v3.5.11...v3.5.12
