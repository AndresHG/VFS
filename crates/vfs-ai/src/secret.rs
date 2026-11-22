//! The redacting secret type for API keys (VFS-28 R2).

use std::fmt;

use crate::keys::KeyError;

/// Printed instead of a key by `Debug` and `Display` (VFS-28 R2).
pub const REDACTED: &str = "«redacted»";

/// A provider API key. It cannot be printed, serialized or read outside this crate: `Debug`
/// and `Display` print «redacted», and only the provider calls and the key store see the text
/// (VFS-28 R2). The bytes are zeroed on drop.
pub struct ApiKey(String);

impl ApiKey {
    /// Takes the key as typed. Surrounding whitespace is dropped; an empty key, inner
    /// whitespace or control characters, or more than 1,000 characters are rejected.
    pub fn new(text: impl Into<String>) -> Result<ApiKey, KeyError> {
        let text = text.into();
        let trimmed = text.trim();
        let ok = !trimmed.is_empty()
            && trimmed.len() <= 1_000
            && trimmed.chars().all(|c| c.is_ascii_graphic());
        let key = ApiKey(trimmed.to_owned());
        drop(ApiKey(text));
        if ok {
            Ok(key)
        } else {
            Err(KeyError::Invalid)
        }
    }

    /// The last 4 characters, the only part ever shown (R1: «termina en …a1b2»).
    pub fn last4(&self) -> String {
        let start = self.0.len().saturating_sub(4);
        self.0[start..].to_owned()
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl Clone for ApiKey {
    fn clone(&self) -> Self {
        ApiKey(self.0.clone())
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl Drop for ApiKey {
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8, and the string is not used afterwards.
        unsafe { self.0.as_bytes_mut() }.fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-28 AC-1 (R2): `Debug` and `Display` never print the key.
    #[test]
    fn debug_and_display_are_redacted() {
        let key = ApiKey::new("  sk-ant-test-1234abcd \n").unwrap();
        assert_eq!(format!("{key:?}"), "«redacted»");
        assert_eq!(format!("{key}"), "«redacted»");
        assert_eq!(format!("{:?}", Some(key.clone())), "Some(«redacted»)");
        assert_eq!(key.expose(), "sk-ant-test-1234abcd");
        assert_eq!(key.last4(), "abcd");
    }

    #[test]
    fn rejects_empty_and_spaced_keys() {
        assert!(ApiKey::new("   ").is_err());
        assert!(ApiKey::new("sk ant").is_err());
        assert!(ApiKey::new("sk-ñ").is_err());
        assert!(ApiKey::new("x".repeat(1_001)).is_err());
    }
}
