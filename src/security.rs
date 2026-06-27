use sha2::{Sha512_256, Digest};

pub fn verify_pow(nonce: u64, id: &str, difficulty: usize) -> bool {
    let input = format!("{}{}", id, nonce);
    let hash = Sha512_256::digest(input.as_bytes());
    let hash_hex = format!("{:x}", hash);
    hash_hex.chars().take(difficulty).all(|c| c == '0')
}

pub fn mine_pow(id: &str, difficulty: usize) -> u64 {
    let mut nonce = 0u64;
    while !verify_pow(nonce, id, difficulty) {
        nonce += 1;
        if nonce % 50000 == 0 {
            println!("[{}] Mining PoW... nonce: {}", id, nonce);
        }
    }
    println!("[{}] PoW found! nonce: {}", id, nonce);
    nonce
}
