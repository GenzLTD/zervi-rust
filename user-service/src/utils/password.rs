use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

use super::error::{AppError, Result};

/// 哈希密码
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| AppError::PasswordError(e.to_string()))
}

/// 空串表示还没设过密码。不能把它当成「空密码的哈希」去比对。
pub fn password_is_unset(password_hash: &str) -> bool {
    password_hash.trim().is_empty()
}

/// 验证密码
pub fn verify_password(password: &str, password_hash: &str) -> Result<bool> {
    let parsed_hash =
        PasswordHash::new(password_hash).map_err(|e| AppError::PasswordError(e.to_string()))?;

    let argon2 = Argon2::default();

    Ok(argon2
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hashing_and_verification() {
        let password = "test_password_123";
        let hashed = hash_password(password).unwrap();

        assert!(verify_password(password, &hashed).unwrap());
        assert!(!verify_password("wrong_password", &hashed).unwrap());
    }

    #[test]
    fn empty_hash_is_unset_and_does_not_match_empty_password() {
        assert!(password_is_unset(""));
        assert!(password_is_unset("   "));
        assert!(!password_is_unset("$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA"));
        assert!(verify_password("", "").is_err());
    }
}
