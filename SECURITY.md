# Security Policy

## 🔒 Supported Versions

| Version | Supported | Status |
|---|---|---|
| v3.5.12 | ✅ | Current release |
| v3.5.11 | ❌ | Contains honest0 bug (fixed in v3.5.12) |
| < v3.5.11 | ❌ | Upgrade required |

## 🐛 Reporting a Vulnerability

**DO NOT open a public GitHub issue for security vulnerabilities.**

Instead, please email: `security@neurograph.io` (replace with your email)

Include:
1. Description of the vulnerability
2. Steps to reproduce
3. Potential impact
4. Suggested fix (if any)

### Response Timeline

- **Acknowledgment:** within 48 hours
- **Initial assessment:** within 7 days
- **Fix or mitigation:** within 30 days (severity-dependent)
- **Public disclosure:** after fix is released, coordinated with reporter

## 🏆 Recognition

Vulnerability reporters will be credited in:
- The next release notes
- The whitepaper acknowledgments section
- A dedicated security hall of fame (coming soon)

## 🔐 Security Measures

NeuroGraph implements defense-in-depth:

1. **Ed25519 signatures** (not ECDSA — post-quantum resistant curve)
2. **SHA-512/256 hashing** (64-bit optimized, length-extension resistant)
3. **5-layer attack detection** (Clone, Coordination, Adaptive, Temporal, Jump)
4. **Cluster-aware weight capping** (prevents coordinated domination)
5. **Reputation floor** (prevents death spirals)
6. **O(1) double-spend detection** (nonce history lookup)
7. **PoW entry** (Sybil resistance without staking)

## 🚨 Known Limitations

- **Cluster detection complexity:** O(N²) per shard — mitigated by sharding
- **Cross-shard latency:** 2-phase commit adds ~2× latency
- **Bootstrap period:** 350 steps required for new nodes
- **Single-machine testing:** all results from 4-core machine (production validation pending)

See whitepaper Section 13.4 for full limitations list.
