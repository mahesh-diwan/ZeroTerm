//! Crypto utilities for E2E encrypted sync

pub struct CryptoKey(pub [u8; 32]);

impl CryptoKey {
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        Self(key)
    }
}
