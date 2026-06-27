use std::collections::{HashMap, VecDeque};
use crate::config::{BOOTSTRAP_STEPS, GRACE_PERIOD,
    REPUTATION_FLOOR_THRESHOLD, REPUTATION_FLOOR_MIN_STEPS,
    REPUTATION_FLOOR_VALUE, REPUTATION_FLOOR_GRACE,
};

// ─── Parametri (whitepaper) ────────────────────────────────────────
pub const ALPHA_FAST: f64 = 0.5;
pub const ALPHA_SLOW: f64 = 0.02;
pub const POSITIVE_THRESHOLD: f64 = 1.2;
pub const CERT_EXPONENT: f64 = 4.27;
pub const TEMPORAL_MIN_GOOD: usize = 10;
pub const TEMPORAL_CAP_FACTOR: f64 = 0.5;
pub const BAD_FREQ_WINDOW: usize = 50;
pub const BAD_FREQ_THRESHOLD: f64 = 0.4;
pub const BAD_FREQ_PENALTY: f64 = 0.95;
pub const ANY_NEGATIVE_WINDOW: usize = 50;
pub const ANY_NEGATIVE_LIMIT: usize = 15;
pub const ANY_NEGATIVE_PENALTY: f64 = 0.7;
pub const ALTERNATING_WINDOW: usize = 20;
pub const ALTERNATING_THRESHOLD: usize = 12;
pub const ALTERNATING_PENALTY: f64 = 0.95;
pub const SHORT_WINDOW: usize = 10;
pub const LONG_WINDOW: usize = 50;
pub const CHANGEPOINT_THRESHOLD: f64 = 1.5;
pub const HYBRID_PENALTY: f64 = 0.95;
pub const FAULT_THRESHOLD: f64 = 0.5;
pub const FAULT_WINDOW: u64 = 2000;
pub const FAULT_PENALTY: f64 = 0.995;
pub const VIOLATION_THRESHOLD: f64 = 0.5;
pub const VIOLATION_WINDOW: u64 = 2000;
pub const LARGE_ERROR_THRESHOLD: f64 = 0.6;
pub const LARGE_ERROR_LIMIT: usize = 10;
pub const LARGE_ERROR_PENALTY: f64 = 0.92;
pub const IMMEDIATE_LARGE_ERROR_PENALTY: f64 = 0.98;
pub const LONG_ERROR_WINDOW: usize = 50;
pub const NEGATIVE_THRESHOLD: f64 = 0.6;
pub const CONSECUTIVE_BAD_LIMIT: usize = 5;
pub const CONSECUTIVE_BAD_PENALTY: f64 = 0.9;
pub const STRIKE_DECAY: f64 = 0.999;
pub const STRIKE_PENALTY_FACTOR: f64 = 0.3;
pub const MAX_STEP_DECAY: f64 = 0.95;

pub struct ReputationEngine {
    rep_fast: HashMap<String, f64>,
    rep_slow: HashMap<String, f64>,
    spike_count: HashMap<String, usize>,
    cert_history: HashMap<String, VecDeque<f64>>,
    last_step: HashMap<String, u64>,
    large_error_flags: HashMap<String, VecDeque<usize>>,
    any_negative_flags: HashMap<String, VecDeque<usize>>,
    cert_sequence: HashMap<String, VecDeque<usize>>,
    violation_counter: HashMap<String, usize>,
    large_error_timestamps: HashMap<String, VecDeque<u64>>,
    fault_timestamps: HashMap<String, VecDeque<u64>>,
    reputation_cache: HashMap<String, f64>,
    short_error_history: HashMap<String, VecDeque<f64>>,
    long_error_history: HashMap<String, VecDeque<f64>>,
    outlier_count: HashMap<String, usize>,
    consecutive_bad: HashMap<String, usize>,
    strike: HashMap<String, f64>,
    first_seen_step: HashMap<String, u64>,
    bad_freq_flags: HashMap<String, VecDeque<usize>>,
    // v3.3: Reputation floor — protejează honest nodes de death spiral
    above_threshold_streak: HashMap<String, usize>,
    floor_active_until: HashMap<String, u64>,
}

impl ReputationEngine {
    pub fn new() -> Self {
        Self {
            rep_fast: HashMap::new(), rep_slow: HashMap::new(),
            spike_count: HashMap::new(), cert_history: HashMap::new(),
            last_step: HashMap::new(), large_error_flags: HashMap::new(),
            any_negative_flags: HashMap::new(), cert_sequence: HashMap::new(),
            violation_counter: HashMap::new(), large_error_timestamps: HashMap::new(),
            fault_timestamps: HashMap::new(), reputation_cache: HashMap::new(),
            short_error_history: HashMap::new(), long_error_history: HashMap::new(),
            outlier_count: HashMap::new(), consecutive_bad: HashMap::new(),
            strike: HashMap::new(), first_seen_step: HashMap::new(),
            bad_freq_flags: HashMap::new(),
            above_threshold_streak: HashMap::new(),
            floor_active_until: HashMap::new(),
        }
    }

    fn continuous_certificate(&self, err: f64) -> f64 {
        if err >= POSITIVE_THRESHOLD { 0.0 }
        else {
            let ratio = err / POSITIVE_THRESHOLD;
            1.0 - ratio.powf(CERT_EXPONENT)
        }
    }

    fn is_warmup(&self, target: &str, step: u64) -> bool {
        let first_seen = *self.first_seen_step.get(target).unwrap_or(&0);
        step < first_seen || step < BOOTSTRAP_STEPS
    }

    pub fn apply_replay_penalty(&mut self, target: &str, step: u64) {
        if self.is_warmup(target, step) { return; }
        if let Some(rep) = self.reputation_cache.get_mut(target) {
            *rep *= 0.3; if *rep < 0.0 { *rep = 0.0; }
        }
        if let Some(fast) = self.rep_fast.get_mut(target) { *fast *= 0.3; }
        if let Some(slow) = self.rep_slow.get_mut(target) { *slow *= 0.3; }
        *self.strike.entry(target.to_string()).or_insert(0.0) += 10.0;
    }

    pub fn update_reputation(&mut self, target: &str, final_err: f64, step: u64) -> f64 {
        if !self.first_seen_step.contains_key(target) {
            self.first_seen_step.insert(target.to_string(), step);
        }
        if self.is_warmup(target, step) {
            let rep = 0.5 + 0.5 * ((step as f64) / (BOOTSTRAP_STEPS as f64)).min(1.0);
            self.reputation_cache.insert(target.to_string(), rep);
            self.rep_fast.insert(target.to_string(), rep);
            self.rep_slow.insert(target.to_string(), rep);
            self.last_step.insert(target.to_string(), step);
            return rep;
        }
        if let Some(&last) = self.last_step.get(target) {
            if step > last && step - last > GRACE_PERIOD {
                self.rep_fast.entry(target.to_string()).and_modify(|r| *r *= 0.9);
                self.rep_slow.entry(target.to_string()).and_modify(|r| *r *= 0.98);
            }
        }
        self.last_step.insert(target.to_string(), step);

        let cert_val = self.continuous_certificate(final_err);
        let hist = self.cert_history.entry(target.to_string()).or_insert_with(VecDeque::new);
        hist.push_back(cert_val);
        if hist.len() > 1000 { hist.pop_front(); }
        let spike_detected = if hist.len() >= TEMPORAL_MIN_GOOD + 1 {
            let recent: Vec<f64> = hist.iter().rev().skip(1).take(TEMPORAL_MIN_GOOD).cloned().collect();
            cert_val < 0.3 && recent.iter().all(|&v| v > 0.7)
        } else { false };
        if spike_detected {
            *self.spike_count.entry(target.to_string()).or_insert(0) += 1;
        }

        let fast = self.rep_fast.entry(target.to_string()).or_insert(0.5);
        *fast = ALPHA_FAST * cert_val + (1.0 - ALPHA_FAST) * (*fast);
        let slow = self.rep_slow.entry(target.to_string()).or_insert(0.5);
        *slow = ALPHA_SLOW * cert_val + (1.0 - ALPHA_SLOW) * (*slow);
        let mut rep = (*fast).min(*slow);

        let spike_cnt = *self.spike_count.get(target).unwrap_or(&0);
        if spike_cnt > 0 {
            rep = rep.min(TEMPORAL_CAP_FACTOR / (1.0 + spike_cnt as f64));
        }

        if final_err > LARGE_ERROR_THRESHOLD {
            *self.strike.entry(target.to_string()).or_insert(0.0) += 0.5;
        }
        let strike = self.strike.entry(target.to_string()).or_insert(0.0);
        *strike *= STRIKE_DECAY;
        if *strike > 0.01 { rep *= (-STRIKE_PENALTY_FACTOR * *strike).exp(); }
        if final_err > LARGE_ERROR_THRESHOLD { rep *= IMMEDIATE_LARGE_ERROR_PENALTY; }

        let short_err = self.short_error_history.entry(target.to_string()).or_insert_with(VecDeque::new);
        short_err.push_back(final_err);
        if short_err.len() > SHORT_WINDOW { short_err.pop_front(); }
        let long_err = self.long_error_history.entry(target.to_string()).or_insert_with(VecDeque::new);
        long_err.push_back(final_err);
        if long_err.len() > LONG_WINDOW { long_err.pop_front(); }
        if short_err.len() >= SHORT_WINDOW && long_err.len() >= LONG_WINDOW {
            let sm = short_err.iter().sum::<f64>() / short_err.len() as f64;
            let lm = long_err.iter().sum::<f64>() / long_err.len() as f64;
            if sm > lm + CHANGEPOINT_THRESHOLD { rep *= HYBRID_PENALTY; }
        }

        let lec = self.large_error_flags.entry(target.to_string()).or_insert_with(VecDeque::new);
        lec.push_back(if final_err > LARGE_ERROR_THRESHOLD { 1 } else { 0 });
        if lec.len() > LONG_ERROR_WINDOW { lec.pop_front(); }
        if lec.len() == LONG_ERROR_WINDOW {
            if lec.iter().sum::<usize>() > LARGE_ERROR_LIMIT { rep *= LARGE_ERROR_PENALTY; }
        }

        let bad_counter = self.consecutive_bad.entry(target.to_string()).or_insert(0);
        if final_err > NEGATIVE_THRESHOLD {
            *bad_counter += 1;
            if *bad_counter > CONSECUTIVE_BAD_LIMIT { rep *= CONSECUTIVE_BAD_PENALTY; }
        } else { *bad_counter = 0; }

        let is_neg = if final_err > NEGATIVE_THRESHOLD { 1 } else { 0 };
        let flags = self.bad_freq_flags.entry(target.to_string()).or_insert_with(VecDeque::new);
        flags.push_back(is_neg);
        if flags.len() > BAD_FREQ_WINDOW { flags.pop_front(); }
        if flags.len() >= BAD_FREQ_WINDOW {
            let bf = flags.iter().sum::<usize>() as f64 / BAD_FREQ_WINDOW as f64;
            if bf > BAD_FREQ_THRESHOLD { rep *= BAD_FREQ_PENALTY; }
        }

        let nf = self.any_negative_flags.entry(target.to_string()).or_insert_with(VecDeque::new);
        nf.push_back(is_neg);
        if nf.len() > ANY_NEGATIVE_WINDOW { nf.pop_front(); }
        if nf.len() >= ANY_NEGATIVE_WINDOW {
            if nf.iter().sum::<usize>() >= ANY_NEGATIVE_LIMIT { rep *= ANY_NEGATIVE_PENALTY; }
        }

        let is_good = if final_err <= POSITIVE_THRESHOLD { 1 } else { 0 };
        let seq = self.cert_sequence.entry(target.to_string()).or_insert_with(VecDeque::new);
        seq.push_back(is_good);
        if seq.len() > ALTERNATING_WINDOW { seq.pop_front(); }
        if seq.len() >= ALTERNATING_WINDOW && seq.len() >= 2 {
            let transitions = (1..seq.len()).filter(|&i| seq[i] != seq[i-1]).count();
            if transitions > ALTERNATING_THRESHOLD { rep *= ALTERNATING_PENALTY; }
        }

        if final_err > LARGE_ERROR_THRESHOLD {
            let ts = self.large_error_timestamps.entry(target.to_string()).or_insert_with(VecDeque::new);
            ts.push_back(step);
            if ts.len() > 20 { ts.pop_front(); }
            if ts.len() >= 5 {
                let intervals: Vec<u64> = ts.iter().zip(ts.iter().skip(1)).map(|(a, b)| b - a).collect();
                let mean = intervals.iter().sum::<u64>() as f64 / intervals.len() as f64;
                let variance = intervals.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / intervals.len() as f64;
                let cv = variance.sqrt() / mean;
                if mean > 200.0 && cv < 0.5 { rep *= 0.6; }
            }
        }

        let vc = self.violation_counter.entry(target.to_string()).or_insert(0);
        if cert_val < VIOLATION_THRESHOLD { *vc += 1; } else { *vc = 0; }
        if *vc >= VIOLATION_WINDOW as usize { rep = 0.0; }

        if cert_val < FAULT_THRESHOLD {
            let fc = self.fault_timestamps.entry(target.to_string()).or_insert_with(VecDeque::new);
            fc.push_back(step);
            let limit = step.saturating_sub(FAULT_WINDOW);
            while fc.front().map_or(false, |&t| t < limit) { fc.pop_front(); }
            if fc.len() >= FAULT_WINDOW as usize { rep *= FAULT_PENALTY; }
        }

        if rep < 0.0 { rep = 0.0; }
        if rep > 1.0 { rep = 1.0; }

        // v3.3: Reputation Floor — protejează honest nodes de death spiral
        //
        // Floor-ul se activează în 2 cazuri:
        // 1. Un nod a fost deasupra REPUTATION_FLOOR_THRESHOLD pentru
        //    REPUTATION_FLOOR_MIN_STEPS pași → floor activ pentru REPUTATION_FLOOR_GRACE
        // 2. v3.3b: În timpul bootstrap (step < BOOTSTRAP_STEPS + 100),
        //    floor-ul e activ implicit pentru toate nodurile (le dăm timp
        //    să se stabilizeze)
        let streak = self.above_threshold_streak.entry(target.to_string()).or_insert(0);
        if rep >= REPUTATION_FLOOR_THRESHOLD {
            *streak += 1;
        } else {
            *streak = 0;
        }

        let _floor_until = self.floor_active_until.get(target).copied().unwrap_or(0);
        if *streak >= REPUTATION_FLOOR_MIN_STEPS {
            self.floor_active_until.insert(target.to_string(), step + REPUTATION_FLOOR_GRACE);
        }

        // Aplicăm floor-ul dacă e activ SAU dacă suntem în bootstrap extins
        let current_floor_until = self.floor_active_until.get(target).copied().unwrap_or(0);
        let in_extended_bootstrap = step < BOOTSTRAP_STEPS + 200;
        if (step < current_floor_until || in_extended_bootstrap) && rep < REPUTATION_FLOOR_VALUE {
            rep = REPUTATION_FLOOR_VALUE;
        }

        self.reputation_cache.insert(target.to_string(), rep);
        rep
    }

    pub fn get_reputation(&self, target: &str) -> Option<f64> {
        self.reputation_cache.get(target).copied()
    }

    pub fn mark_outlier(&mut self, target: &str, step: u64) {
        if self.is_warmup(target, step) { return; }
        let cnt = self.outlier_count.entry(target.to_string()).or_insert(0);
        *cnt += 1;
        if *cnt >= 3 {
            if let Some(rep) = self.reputation_cache.get_mut(target) {
                *rep *= 0.8; if *rep < 0.0 { *rep = 0.0; }
            }
        }
    }
}

impl Default for ReputationEngine {
    fn default() -> Self { Self::new() }
}
