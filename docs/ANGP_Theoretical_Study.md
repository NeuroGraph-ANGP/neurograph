# ANGP: A Decentralized Reputation-Based Consensus Protocol for Directed Acyclic Graphs

## Source
- **Zenodo DOI:** https://doi.org/10.5281/zenodo.20966117
- **Record URL:** https://zenodo.org/records/20966117
- **Publication date:** June 2026
- **Author:** A. Toth

## Abstract

The Adaptive Neural Gossip Protocol (ANGP) is a fully decentralized, asynchronous consensus mechanism designed for Directed Acyclic Graph (DAG) based distributed ledgers. Unlike classical Byzantine Fault Tolerant (BFT) systems that rely on leader election or quorums, ANGP uses:

- A median-based consensus computed from predictions exchanged via gossip.
- A continuous reputation engine that distinguishes honest nodes from Byzantine attackers, including coordinated collusion, rare attacks, sensor faults, and network impairments.
- A lightweight Proof-of-Work (PoW) layer (SHA-512/256) to prevent Sybil identity floods, while keeping the core protocol free of staking or token-based governance.

ANGP tolerates up to 44% coordinated attackers and 66% uncoordinated attackers with no degradation in honest node safety. It operates asynchronously, requires no global time synchronization, and self-heals under packet loss and network delays. This document provides the complete architectural blueprint, component specifications, security analysis, and integration guidelines for building a production-grade DAG based cryptocurrency or distributed application on top of ANGP.

## Relation to NeuroGraph

This theoretical study provides the architectural blueprint and security analysis that underpins the NeuroGraph (ANGP) implementation. The NeuroGraph prototype (v3.5.13) is the empirical validation of the concepts described in this document.
