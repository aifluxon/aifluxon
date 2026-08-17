use base64::{engine::general_purpose, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub fn generate_pkce() -> (String, String) {
    let mut verifier_bytes = [0_u8; 64];
    rand::thread_rng().fill_bytes(&mut verifier_bytes);
    let verifier = general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge = general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub fn generate_state() -> String {
    let mut state = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut state);
    general_purpose::URL_SAFE_NO_PAD.encode(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_verifier() {
        let (verifier, challenge) = generate_pkce();
        let expected = general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
        assert!(verifier.len() >= 43);
        assert_eq!(challenge.len(), 43);
        assert!(!verifier.contains('='));
        assert!(!challenge.contains('='));
    }

    #[test]
    fn pkce_verifier_is_random_per_attempt() {
        assert_ne!(generate_pkce().0, generate_pkce().0);
        assert_ne!(generate_state(), generate_state());
    }
}
