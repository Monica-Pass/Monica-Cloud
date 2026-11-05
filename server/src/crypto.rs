//! Server-side primitives only: password hashing, tokens and digests.
//! Vault encryption happens exclusively on clients.

use std::sync::LazyLock;

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::error::{ApiResult, Error};

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

/// URL-safe random token with `N` bytes of entropy.
pub fn random_token<const N: usize>() -> String {
    BASE64URL_NOPAD.encode(&random_bytes::<N>())
}

pub fn sha256_hex(data: &[u8]) -> String {
    HEXLOWER.encode(&Sha256::digest(data))
}

/// Argon2id with the crate defaults (19 MiB, t=2, p=1), the OWASP baseline.
fn hash_password(secret: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .expect("argon2 hashing with default parameters cannot fail")
        .to_string()
}

fn verify_password(secret: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default().verify_password(secret.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// Each Argon2id run takes 19 MiB and a core for tens of milliseconds. At most this many run at
/// once, so a burst of logins queues up instead of exhausting memory or the blocking pool.
static HASHING: LazyLock<Semaphore> = LazyLock::new(|| {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    Semaphore::new(cores.clamp(2, 16))
});

/// Run password hashing work on the blocking pool, within the [`HASHING`] limit. Never call it
/// inside `Db::run`: that would hold a pooled database connection for the whole computation.
async fn hashing<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> ApiResult<T> {
    let _permit = HASHING.acquire().await.map_err(|_| Error::Internal("password hashing is unavailable".into()))?;
    Ok(tokio::task::spawn_blocking(work).await?)
}

/// Argon2id hash (PHC string) of an authHash or Send passwordHash.
pub async fn hash_secret(secret: String) -> ApiResult<String> {
    hashing(move || hash_password(&secret)).await
}

pub async fn verify_secret(secret: String, phc: String) -> ApiResult<bool> {
    hashing(move || verify_password(&secret, &phc)).await
}

/// Burn the same time as a real verification so missing accounts are not observable.
pub async fn dummy_verify() -> ApiResult<()> {
    static DUMMY: LazyLock<String> = LazyLock::new(|| hash_password("monica-cloud-dummy"));
    hashing(|| {
        let _ = verify_password("not-the-password", &DUMMY);
    })
    .await
}

/// Checks that an authHash / send passwordHash is base64 of exactly 32 bytes.
pub fn is_valid_auth_hash(value: &str) -> bool {
    data_encoding::BASE64.decode(value.as_bytes()).map(|b| b.len() == 32).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn argon2_round_trip() {
        let phc = hash_secret("secret".into()).await.unwrap();
        assert!(verify_secret("secret".into(), phc.clone()).await.unwrap());
        assert!(!verify_secret("other".into(), phc).await.unwrap());
        dummy_verify().await.unwrap();
    }

    #[test]
    fn auth_hash_shape() {
        assert!(is_valid_auth_hash(&data_encoding::BASE64.encode(&[7u8; 32])));
        assert!(!is_valid_auth_hash(&data_encoding::BASE64.encode(&[7u8; 31])));
        assert!(!is_valid_auth_hash("password"));
    }
}
