//! Rate Limiting — anti-DoS per peer.
//!
//! Protejează nodurile oneste de atacuri de tip spam/flooding.
//! Fără rate limiting, un atacator (sau un nod buggy) ar putea inunda un nod onest
//! cu mii de mesaje/secundă, consumând CPU pe verificări de semnături.
//!
//! Strategie: token bucket per peer.
//!   - Fiecare peer are un bucket cu capacitate `BURST_SIZE` tokeni.
//!   - Bucket-ul se reîncarcă cu `REFILL_RATE` tokeni/secundă.
//!   - Fiecare mesaj consumă 1 token.
//!   - Dacă bucket-ul e gol → mesajul e dropped (și peer-ul poate fi penalizat
//!     în reputație pentru "message flood").
//!
//! IMPORTANT: Acest modul NU schimbă protocolul. Doar filtrează mesajele la intrare,
//! înainte de a ajunge la procesarea de consens.

use std::collections::HashMap;
use std::time::Instant;

/// Configurare rate limiting.
/// v2.5: Crescute pentru a suporta loop de 10ms (era 50ms).
/// La 100 props/sec + txs, 200 burst / 200/s sustained e potrivit.
pub const BURST_SIZE: u32 = 500;        // v2.5: 100 → 500 (pentru loop 10ms)
pub const REFILL_RATE_PER_SEC: f64 = 250.0; // v2.5: 50 → 250 (5× mai mult pentru loop 10ms)
pub const MIN_TOKENS_FOR_PENALTY: f64 = 0.0;

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new() -> Self {
        TokenBucket {
            tokens: BURST_SIZE as f64,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * REFILL_RATE_PER_SEC).min(BURST_SIZE as f64);
        self.last_refill = now;
    }

    /// Încearcă să consume 1 token. Returnează true dacă a reușit.
    fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Token-ii disponibili (pentru logging).
    fn current_tokens(&self) -> f64 {
        self.tokens
    }
}

pub struct RateLimiter {
    buckets: HashMap<String, TokenBucket>,
    /// Counter de mesaje dropped per peer (pentru logging / penalizare).
    dropped_count: HashMap<String, u64>,
}

#[derive(Debug, Clone, Copy)]
pub enum RateLimitResult {
    Allowed,
    /// Mesaj dropped din cauza rate limit-ului.
    Denied { current_tokens: f64 },
}

impl RateLimiter {
    pub fn new() -> Self {
        RateLimiter {
            buckets: HashMap::new(),
            dropped_count: HashMap::new(),
        }
    }

    /// Verifică dacă un mesaj de la `peer` poate fi procesat.
    pub fn check(&mut self, peer: &str) -> RateLimitResult {
        let bucket = self.buckets.entry(peer.to_string()).or_insert_with(TokenBucket::new);
        if bucket.try_consume() {
            RateLimitResult::Allowed
        } else {
            *self.dropped_count.entry(peer.to_string()).or_insert(0) += 1;
            RateLimitResult::Denied { current_tokens: bucket.current_tokens() }
        }
    }

    /// Returnează numărul de mesaje dropped per peer (pentru logging).
    pub fn dropped_per_peer(&self) -> &HashMap<String, u64> {
        &self.dropped_count
    }

    /// Curăță entries vechi (apelat ocazional pentru a preveni memory leak).
    pub fn cleanup(&mut self) {
        // Păstrăm doar peer-ii cu activitate recentă (ultima refill < 60s)
        let cutoff = Instant::now() - std::time::Duration::from_secs(60);
        self.buckets.retain(|_, bucket| bucket.last_refill > cutoff);
    }
}

impl Default for RateLimiter {
    fn default() -> Self { Self::new() }
}
