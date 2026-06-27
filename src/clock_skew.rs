//! v3.5.1 — Clock Skew Checker (pasiv, mainnet-prep)
//!
//! Scop: pregătește terenul pentru mainnet fără a schimba protocolul de consens.
//! În stadiul curent, checker-ul:
//!   - măsoară diferența dintre timestamp-ul unei tranzacții recepționate și
//!     clock-ul local al nodului
//!   - doar LOGHEAZĂ/WARNAZĂ când skew > MAX_SKEW_MS — nu respinge tx
//!   - expune un API simplu pentru viitoare stare "strict mode" (mainnet)
//!
//! IMPORTANT: acest modul NU atinge `Transaction::new()` (păstrează
//! `SystemTime::now()` pentru backward-compat). Doar observă și raportează.
//!
//! Metrice utile pentru mainnet tuning:
//!   - distribution skews per peer (P50/P95/P99)
//!   - counting warnings per interval
//!   - tracking "worst offender" peers

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Prag implicit: 10 minute (generos pentru rețea cu NTP sincregulat).
/// Mainnet final: 500ms (NTP-tuned). Testnet: 2000ms.
/// v3.5.5: Crescut de la 2000ms la 600000ms (10 min) pentru a evita false positives
/// pe Windows cu clock nesincronizat NTP.
pub const DEFAULT_MAX_SKEW_MS: i64 = 600_000;  // 10 minute

/// Prag peste care considerăm skew "grav" (1 oră) — posibil nod offline sincronizat greșit.
/// v3.5.5: Crescut de la 5000ms la 3600000ms (1 oră) pentru a evita spam de SEVERE warnings.
pub const SEVERE_SKEW_MS: i64 = 3_600_000;  // 1 oră

#[derive(Debug, Default, Clone)]
pub struct SkewStats {
    /// Total tranzacții verificate.
    pub total_checked: u64,
    /// Tranzacții cu skew > MAX_SKEW_MS (warning).
    pub warnings: u64,
    /// Tranzacții cu skew > SEVERE_SKEW_MS (severe).
    pub severe: u64,
    /// Skew-ul maxim observat (ms, pozitiv = tx în viitor, negativ = tx în trecut).
    pub max_skew_ms: i64,
    /// Suma skews (pentru calcul mediu).
    pub sum_skew_ms: i64,
}

impl SkewStats {
    pub fn mean_skew_ms(&self) -> f64 {
        if self.total_checked == 0 { 0.0 }
        else { self.sum_skew_ms as f64 / self.total_checked as f64 }
    }
}

/// Checker pasiv: observă skew, nu respinge.
pub struct ClockSkewChecker {
    max_skew_ms: i64,
    severe_skew_ms: i64,
    /// Statistici per peer (sender string), pentru a identifica noduri problematice.
    per_peer: Mutex<HashMap<String, SkewStats>>,
    /// Statistici globale.
    global: Mutex<SkewStats>,
}

impl ClockSkewChecker {
    pub fn new(max_skew_ms: i64) -> Self {
        // v3.5.5: severe_skew_ms = max_skew_ms * 5 (sau min 5s)
        let severe_skew_ms = (max_skew_ms * 5).max(5_000);
        Self {
            max_skew_ms,
            severe_skew_ms,
            per_peer: Mutex::new(HashMap::new()),
            global: Mutex::new(SkewStats::default()),
        }
    }

    /// v3.5.5: Constructor custom cu praguri explicite pentru ambele niveluri.
    pub fn with_thresholds(max_skew_ms: i64, severe_skew_ms: i64) -> Self {
        Self {
            max_skew_ms,
            severe_skew_ms,
            per_peer: Mutex::new(HashMap::new()),
            global: Mutex::new(SkewStats::default()),
        }
    }

    pub fn with_default() -> Self {
        // v3.5.5: Folosim pragurile default mari (10 min / 1 oră)
        Self::with_thresholds(DEFAULT_MAX_SKEW_MS, SEVERE_SKEW_MS)
    }

    /// Verifică o tranzacție recepționată. Returnează skew-ul observat (ms).
    ///
    /// NOTĂ: Nu respinge tx — doar actualizează statistici și (în viitor) emite log.
    /// Sketch logging: în producție vom folosi `tracing::warn!`. Pentru acum
    /// returnează `SkewVerdict` ca caller-ul să decidă log/reject/collect.
    pub fn check(&self, tx_timestamp_secs: u64, sender: &str) -> SkewVerdict {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let tx_secs = tx_timestamp_secs as i64;
        let skew_ms = (now_secs - tx_secs) * 1_000;

        let verdict = if skew_ms.abs() > self.severe_skew_ms {
            SkewVerdict::Severe
        } else if skew_ms.abs() > self.max_skew_ms {
            SkewVerdict::Warning
        } else {
            SkewVerdict::Ok
        };

        // Update global stats
        {
            let mut g = self.global.lock().unwrap();
            g.total_checked += 1;
            g.sum_skew_ms += skew_ms;
            if skew_ms.abs() > g.max_skew_ms.abs() {
                g.max_skew_ms = skew_ms;
            }
            match verdict {
                SkewVerdict::Warning => g.warnings += 1,
                SkewVerdict::Severe => {
                    g.warnings += 1;
                    g.severe += 1;
                }
                SkewVerdict::Ok => {}
            }
        }

        // Update per-peer stats
        {
            let mut peers = self.per_peer.lock().unwrap();
            let stats = peers.entry(sender.to_string()).or_default();
            stats.total_checked += 1;
            stats.sum_skew_ms += skew_ms;
            if skew_ms.abs() > stats.max_skew_ms.abs() {
                stats.max_skew_ms = skew_ms;
            }
            match verdict {
                SkewVerdict::Warning => stats.warnings += 1,
                SkewVerdict::Severe => {
                    stats.warnings += 1;
                    stats.severe += 1;
                }
                SkewVerdict::Ok => {}
            }
        }

        verdict
    }

    /// Snapshot statisticilor globale.
    pub fn global_stats(&self) -> SkewStats {
        self.global.lock().unwrap().clone()
    }

    /// Snapshot statisticilor pentru un peer.
    pub fn peer_stats(&self, sender: &str) -> Option<SkewStats> {
        self.per_peer.lock().unwrap().get(sender).cloned()
    }

    /// Top N cei mai apropiați de prag (pentru diagnostic mainnet).
    pub fn top_offenders(&self, n: usize) -> Vec<(String, SkewStats)> {
        let peers = self.per_peer.lock().unwrap();
        let mut all: Vec<(String, SkewStats)> = peers
            .iter()
            .filter(|(_, s)| s.warnings > 0)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        all.sort_by(|a, b| b.1.warnings.cmp(&a.1.warnings));
        all.truncate(n);
        all
    }

    /// Reset statistici (pentru tests).
    pub fn reset(&self) {
        *self.global.lock().unwrap() = SkewStats::default();
        self.per_peer.lock().unwrap().clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkewVerdict {
    /// Skew sub prag — totul OK.
    Ok,
    /// Skew peste MAX_SKEW_MS — log warning, nu respinge.
    Warning,
    /// Skew peste SEVERE_SKEW_MS — posibil nod desincronizat grav.
    Severe,
}

impl SkewVerdict {
    pub fn is_problem(&self) -> bool {
        matches!(self, SkewVerdict::Warning | SkewVerdict::Severe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn test_recent_tx_is_ok() {
        let checker = ClockSkewChecker::with_default();
        let ts = now_secs();
        let verdict = checker.check(ts, "alice");
        assert_eq!(verdict, SkewVerdict::Ok);
        let stats = checker.global_stats();
        assert_eq!(stats.total_checked, 1);
        assert_eq!(stats.warnings, 0);
    }

    #[test]
    fn test_old_tx_triggers_warning() {
        // v3.5.5: Folosim prag custom mic ca sa testam logica (nu pragurile default mari)
        let checker = ClockSkewChecker::new(100);  // 100ms prag, 5s severe implicit
        // 10 secunde în trecut — peste SEVERE_SKEW_MS
        let ts = now_secs().saturating_sub(10);
        let verdict = checker.check(ts, "bob");
        assert_eq!(verdict, SkewVerdict::Severe);
        let stats = checker.global_stats();
        assert_eq!(stats.warnings, 1);
        assert_eq!(stats.severe, 1);
    }

    #[test]
    fn test_future_tx_triggers_warning() {
        // v3.5.5: Prag custom mic pentru testare
        let checker = ClockSkewChecker::new(100);
        // 10 secunde în viitor — posibil attacker cu clock skewat
        let ts = now_secs().saturating_add(10);
        let verdict = checker.check(ts, "mallory");
        assert_eq!(verdict, SkewVerdict::Severe);
    }

    #[test]
    fn test_per_peer_tracking() {
        // v3.5.5: Prag custom mic pentru testare
        let checker = ClockSkewChecker::new(100);
        // Alice trimite 3 txs curate
        for _ in 0..3 {
            checker.check(now_secs(), "alice");
        }
        // Bob trimite 2 txs skewed
        for _ in 0..2 {
            checker.check(now_secs().saturating_sub(10), "bob");
        }

        let alice = checker.peer_stats("alice").unwrap();
        assert_eq!(alice.total_checked, 3);
        assert_eq!(alice.warnings, 0);

        let bob = checker.peer_stats("bob").unwrap();
        assert_eq!(bob.total_checked, 2);
        assert_eq!(bob.warnings, 2);
        assert_eq!(bob.severe, 2);
    }

    #[test]
    fn test_top_offenders_orders_by_warnings() {
        // v3.5.5: Prag custom mic pentru testare
        let checker = ClockSkewChecker::new(100);
        // Carol: 5 warnings, Dave: 2 warnings, Eve: 0 warnings
        for _ in 0..5 {
            checker.check(now_secs().saturating_sub(10), "carol");
        }
        for _ in 0..2 {
            checker.check(now_secs().saturating_sub(10), "dave");
        }
        for _ in 0..10 {
            checker.check(now_secs(), "eve");
        }

        let offenders = checker.top_offenders(3);
        assert_eq!(offenders.len(), 2); // Eve nu are warnings
        assert_eq!(offenders[0].0, "carol");
        assert_eq!(offenders[0].1.warnings, 5);
        assert_eq!(offenders[1].0, "dave");
        assert_eq!(offenders[1].1.warnings, 2);
    }

    #[test]
    fn test_custom_threshold() {
        // Prag custom foarte strict: 100ms
        let checker = ClockSkewChecker::new(100);
        // O tx cu 1s în trecut — peste 100ms dar sub 5s severe
        let ts = now_secs().saturating_sub(1);
        let verdict = checker.check(ts, "frank");
        assert_eq!(verdict, SkewVerdict::Warning);
    }

    #[test]
    fn test_reset_clears_stats() {
        // v3.5.5: Prag custom mic pentru testare
        let checker = ClockSkewChecker::new(100);
        checker.check(now_secs().saturating_sub(10), "alice");
        assert_eq!(checker.global_stats().warnings, 1);
        checker.reset();
        assert_eq!(checker.global_stats().warnings, 0);
        assert_eq!(checker.global_stats().total_checked, 0);
    }

    #[test]
    fn test_mean_skew_calculation() {
        // v3.5.5: Prag custom mic pentru testare
        let checker = ClockSkewChecker::new(100);
        // 2 txs curate (skew ~0), 1 tx cu skew +10s (tx în trecut = skew pozitiv)
        checker.check(now_secs(), "alice");
        checker.check(now_secs(), "alice");
        checker.check(now_secs().saturating_sub(10), "alice");
        let stats = checker.global_stats();
        assert_eq!(stats.total_checked, 3);
        // Media ar trebui să fie în jur de +3333ms (cu toleranță pentru runtime)
        // Convenție: skew > 0 = tx în trecut (clock local înaintea clock-ului tx)
        let mean = stats.mean_skew_ms();
        assert!(mean > 1000.0, "Mean skew should be positive (tx in past): got {}", mean);
    }
}
