use ndarray::Array1;
use rayon::prelude::*;
use sha2::{Sha512_256, Digest};
use crate::config::DIM;
use crate::transaction::Hash;

pub fn norm(v: &Array1<f64>) -> f64 {
    v.dot(v).sqrt()
}

pub fn vec_to_array(v: &[f64]) -> Array1<f64> {
    let mut arr = Array1::zeros(DIM);
    for i in 0..DIM.min(v.len()) {
        arr[i] = v[i];
    }
    arr
}

pub fn array_to_vec(arr: &Array1<f64>) -> Vec<f64> {
    arr.iter().copied().collect()
}

/// Mediană per-componentă, paralelizată cu Rayon.
pub fn median_arrays(arrays: &[Array1<f64>]) -> Array1<f64> {
    let n = arrays.len();
    if n == 0 { return Array1::zeros(DIM); }
    if n == 1 { return arrays[0].clone(); }
    let result: Vec<f64> = (0..DIM)
        .into_par_iter()
        .map(|i| {
            let mut values: Vec<f64> = arrays.iter().map(|arr| arr[i]).collect();
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            if n % 2 == 0 { (values[n / 2 - 1] + values[n / 2]) / 2.0 }
            else { values[n / 2] }
        })
        .collect();
    Array1::from(result)
}

pub fn weighted_median_arrays(arrays: &[Array1<f64>], weights: &[f64]) -> Array1<f64> {
    assert_eq!(arrays.len(), weights.len());
    let n = arrays.len();
    if n == 0 { return Array1::zeros(DIM); }
    let total_w: f64 = weights.iter().sum();
    if total_w <= 0.0 { return Array1::zeros(DIM); }
    let result: Vec<f64> = (0..DIM)
        .into_par_iter()
        .map(|i| {
            let mut pairs: Vec<(f64, f64)> = arrays.iter().zip(weights.iter())
                .map(|(arr, w)| (arr[i], *w)).collect();
            pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let half = total_w / 2.0;
            let mut cum = 0.0;
            let mut last_val = 0.0;
            for (val, w) in &pairs {
                last_val = *val;
                cum += *w;
                if cum >= half { return last_val; }
            }
            last_val
        })
        .collect();
    Array1::from(result)
}

pub fn prediction_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f64>()
        .sqrt()
}

pub fn euclidean(a: &Array1<f64>, b: &Array1<f64>) -> f64 {
    let diff = a - b;
    diff.dot(&diff).sqrt()
}

/// Hash SHA-512/256 (truncated 512 to 256 bits) — v2.5.
///
/// Aceasta este criptografic mai puternică decât SHA-256 pur (mai puțin vulnerabil
/// la anumite atacuri de length extension) dar produce tot 32 bytes, deci `Hash = [u8; 32]`
/// rămâne compatibil cu tot codul existent.
pub fn canonical_hash<T: serde::Serialize>(t: &T) -> Hash {
    let bytes = bincode::serialize(t).unwrap_or_default();
    let mut hasher = Sha512_256::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    let mut h = [0u8; 32];
    h.copy_from_slice(&result);
    h
}

/// Sortează un slice de hash-uri și le concatenează într-un singur hash (pentru "set hash").
pub fn hash_set_hash(hashes: &[Hash]) -> Hash {
    let mut sorted: Vec<Hash> = hashes.to_vec();
    sorted.sort();
    let mut hasher = Sha512_256::new();
    for h in &sorted {
        hasher.update(h);
    }
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Counts how many items appear in both vecs (set intersection size).
pub fn intersection_count<T: PartialEq + Clone>(a: &[T], b: &[T]) -> usize {
    a.iter().filter(|x| b.contains(x)).count()
}
