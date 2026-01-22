//! Memcached ASCII protocol command types

use std::borrow::Cow;

/// Maximum key length (memcached spec)
pub const MAX_KEY_LENGTH: usize = 250;

/// Parsed memcached command
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command<'a> {
    /// get <key>*
    Get { keys: Vec<Cow<'a, [u8]>> },

    /// set <key> <flags> <exptime> <bytes> [noreply]
    Set {
        key: Cow<'a, [u8]>,
        flags: u32,
        exptime: u64,
        data: Cow<'a, [u8]>,
        noreply: bool,
    },

    /// delete <key> [noreply]
    Delete { key: Cow<'a, [u8]>, noreply: bool },

    /// quit
    Quit,
}

impl<'a> Command<'a> {
    /// Returns true if this command should not send a response
    pub fn is_noreply(&self) -> bool {
        match self {
            Command::Set { noreply, .. } | Command::Delete { noreply, .. } => *noreply,
            _ => false,
        }
    }
}

/// Check if a key is valid
pub fn is_valid_key(key: &[u8]) -> bool {
    if key.is_empty() || key.len() > MAX_KEY_LENGTH {
        return false;
    }
    // Keys cannot contain control characters or whitespace
    key.iter().all(|&b| b > 32 && b < 127)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_key() {
        assert!(is_valid_key(b"valid_key"));
        assert!(is_valid_key(b"key-with-dashes"));
        assert!(is_valid_key(b"key:with:colons"));
        assert!(!is_valid_key(b""));
        assert!(!is_valid_key(b"key with space"));
        assert!(!is_valid_key(b"key\twith\ttab"));
        assert!(!is_valid_key(&[b'a'; 251])); // Too long
    }

    #[test]
    fn test_is_noreply() {
        let cmd = Command::Set {
            key: Cow::Borrowed(b"key"),
            flags: 0,
            exptime: 0,
            data: Cow::Borrowed(b"data"),
            noreply: true,
        };
        assert!(cmd.is_noreply());

        let cmd = Command::Get {
            keys: vec![Cow::Borrowed(b"key" as &[u8])],
        };
        assert!(!cmd.is_noreply());
    }
}
