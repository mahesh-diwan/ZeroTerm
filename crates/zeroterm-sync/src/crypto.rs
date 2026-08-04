use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

#[derive(Clone, Copy)]
pub struct CryptoKey(pub [u8; 32]);

impl CryptoKey {
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        Self(key)
    }

    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.0));
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, data)
            .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
        let mut result = nonce.to_vec();
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
        if data.len() < 12 {
            anyhow::bail!("data too short");
        }
        let (nonce_bytes, ciphertext) = data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.0));
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("decrypt: {e}"))?;
        Ok(plaintext)
    }
}
