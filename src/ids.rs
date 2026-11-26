//! Identifiants aléatoires, empreintes et hachage des mots de passe.

use argon2::Argon2;
use sha2::{Digest, Sha256};

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("générateur aléatoire du système indisponible");
    buf
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

/// Jeton secret (session, API) : 32 octets aléatoires en hexadécimal.
pub fn random_token() -> String {
    hex(&random_bytes::<32>())
}

/// Clé publique d'un projet, lisible dans le code des pages.
pub fn random_public_key() -> String {
    hex(&random_bytes::<12>())
}

/// Identifiant aléatoire positif sur 63 bits (compatible BIGINT).
pub fn random_id() -> i64 {
    (u64::from_be_bytes(random_bytes::<8>()) >> 1) as i64
}

pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        // Longueur en préfixe : deux découpages différents ne donnent jamais la même empreinte.
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(hasher.finalize().as_slice());
    out
}

/// Empreinte ramenée à un entier positif sur 63 bits.
pub fn hash_to_id(parts: &[&[u8]]) -> i64 {
    let digest = sha256(parts);
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    (u64::from_be_bytes(first) >> 1) as i64
}

const ARGON_PREFIX: &str = "argon2id";

/// Hache un mot de passe (Argon2id, paramètres par défaut de la bibliothèque).
/// Coûteux : à appeler hors du fil d'exécution asynchrone.
pub fn hash_password(password: &str) -> String {
    let salt = random_bytes::<16>();
    let mut out = [0u8; 32];
    Argon2::default().hash_password_into(password.as_bytes(), &salt, &mut out).expect("paramètres Argon2 valides");
    format!("{ARGON_PREFIX}${}${}", hex(&salt), hex(&out))
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let mut parts = stored.split('$');
    let (Some(ARGON_PREFIX), Some(salt), Some(hash), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let (Some(salt), Some(expected)) = (unhex(salt), unhex(hash)) else {
        return false;
    };
    let mut out = vec![0u8; expected.len()];
    if expected.is_empty() || Argon2::default().hash_password_into(password.as_bytes(), &salt, &mut out).is_err() {
        return false;
    }
    constant_time_eq(&out, &expected)
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let stored = hash_password("correct horse");
        assert!(verify_password("correct horse", &stored));
        assert!(!verify_password("correct horsf", &stored));
        assert!(!verify_password("correct horse", "argon2id$zz$zz"));
        assert!(!verify_password("x", ""));
        assert_ne!(stored, hash_password("correct horse"), "le sel change à chaque hachage");
    }

    #[test]
    fn hashing_is_unambiguous_and_positive() {
        assert_ne!(sha256(&[b"ab", b"c"]), sha256(&[b"a", b"bc"]));
        for i in 0..200u32 {
            assert!(hash_to_id(&[&i.to_be_bytes()]) >= 0);
            assert!(random_id() >= 0);
        }
        assert_eq!(random_token().len(), 64);
        assert_eq!(hex(&[0, 255, 16]), "00ff10");
        assert_eq!(unhex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(unhex("0g"), None);
    }
}
