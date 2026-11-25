use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Owned UTF-8 secret material with an explicit plaintext access boundary.
///
/// The transparent serde representation intentionally remains a JSON string so
/// existing configuration, redb documents, backups, and sync payloads stay
/// byte/field compatible at the schema boundary.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    pub fn expose_secret_mut(&mut self) -> &mut String {
        &mut self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_secret(mut self) -> String {
        std::mem::take(&mut self.0)
    }
}

impl From<String> for SecretString {
    fn from(secret: String) -> Self {
        Self(secret)
    }
}

impl From<&str> for SecretString {
    fn from(secret: &str) -> Self {
        Self(secret.to_owned())
    }
}

impl PartialEq<str> for SecretString {
    fn eq(&self, other: &str) -> bool {
        self.expose_secret() == other
    }
}

impl PartialEq<&str> for SecretString {
    fn eq(&self, other: &&str) -> bool {
        self.expose_secret() == *other
    }
}

impl PartialEq<String> for SecretString {
    fn eq(&self, other: &String) -> bool {
        self.expose_secret() == other
    }
}

impl std::ops::Deref for SecretString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.expose_secret()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString(<redacted>)")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}
