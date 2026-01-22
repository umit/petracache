//! Memcached ASCII protocol command types

use std::borrow::Cow;

/// Maximum key length (memcached spec)
pub const MAX_KEY_LENGTH: usize = 250;

/// Parsed memcached command
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command<'a> {
    /// get <key>*
    Get { keys: Vec<Cow<'a, [u8]>> },

    /// gets <key>* (with CAS - we don't support CAS, but accept the command)
    Gets { keys: Vec<Cow<'a, [u8]>> },

    /// set <key> <flags> <exptime> <bytes> [noreply]
    Set {
        key: Cow<'a, [u8]>,
        flags: u32,
        exptime: u64,
        data: Cow<'a, [u8]>,
        noreply: bool,
    },

    /// add <key> <flags> <exptime> <bytes> [noreply]
    Add {
        key: Cow<'a, [u8]>,
        flags: u32,
        exptime: u64,
        data: Cow<'a, [u8]>,
        noreply: bool,
    },

    /// replace <key> <flags> <exptime> <bytes> [noreply]
    Replace {
        key: Cow<'a, [u8]>,
        flags: u32,
        exptime: u64,
        data: Cow<'a, [u8]>,
        noreply: bool,
    },

    /// delete <key> [noreply]
    Delete { key: Cow<'a, [u8]>, noreply: bool },

    /// incr <key> <value> [noreply]
    Incr {
        key: Cow<'a, [u8]>,
        value: u64,
        noreply: bool,
    },

    /// decr <key> <value> [noreply]
    Decr {
        key: Cow<'a, [u8]>,
        value: u64,
        noreply: bool,
    },

    /// touch <key> <exptime> [noreply]
    Touch {
        key: Cow<'a, [u8]>,
        exptime: u64,
        noreply: bool,
    },

    /// flush_all [delay] [noreply]
    FlushAll { delay: u64, noreply: bool },

    /// version
    Version,

    /// stats [args]
    Stats { args: Option<Cow<'a, [u8]>> },

    /// quit
    Quit,
}

impl<'a> Command<'a> {
    /// Returns true if this command should not send a response
    pub fn is_noreply(&self) -> bool {
        match self {
            Command::Set { noreply, .. }
            | Command::Add { noreply, .. }
            | Command::Replace { noreply, .. }
            | Command::Delete { noreply, .. }
            | Command::Incr { noreply, .. }
            | Command::Decr { noreply, .. }
            | Command::Touch { noreply, .. }
            | Command::FlushAll { noreply, .. } => *noreply,
            _ => false,
        }
    }

    /// Convert to owned command (for async processing)
    pub fn into_owned(self) -> Command<'static> {
        match self {
            Command::Get { keys } => Command::Get {
                keys: keys
                    .into_iter()
                    .map(|k| Cow::Owned(k.into_owned()))
                    .collect(),
            },
            Command::Gets { keys } => Command::Gets {
                keys: keys
                    .into_iter()
                    .map(|k| Cow::Owned(k.into_owned()))
                    .collect(),
            },
            Command::Set {
                key,
                flags,
                exptime,
                data,
                noreply,
            } => Command::Set {
                key: Cow::Owned(key.into_owned()),
                flags,
                exptime,
                data: Cow::Owned(data.into_owned()),
                noreply,
            },
            Command::Add {
                key,
                flags,
                exptime,
                data,
                noreply,
            } => Command::Add {
                key: Cow::Owned(key.into_owned()),
                flags,
                exptime,
                data: Cow::Owned(data.into_owned()),
                noreply,
            },
            Command::Replace {
                key,
                flags,
                exptime,
                data,
                noreply,
            } => Command::Replace {
                key: Cow::Owned(key.into_owned()),
                flags,
                exptime,
                data: Cow::Owned(data.into_owned()),
                noreply,
            },
            Command::Delete { key, noreply } => Command::Delete {
                key: Cow::Owned(key.into_owned()),
                noreply,
            },
            Command::Incr {
                key,
                value,
                noreply,
            } => Command::Incr {
                key: Cow::Owned(key.into_owned()),
                value,
                noreply,
            },
            Command::Decr {
                key,
                value,
                noreply,
            } => Command::Decr {
                key: Cow::Owned(key.into_owned()),
                value,
                noreply,
            },
            Command::Touch {
                key,
                exptime,
                noreply,
            } => Command::Touch {
                key: Cow::Owned(key.into_owned()),
                exptime,
                noreply,
            },
            Command::FlushAll { delay, noreply } => Command::FlushAll { delay, noreply },
            Command::Version => Command::Version,
            Command::Stats { args } => Command::Stats {
                args: args.map(|a| Cow::Owned(a.into_owned())),
            },
            Command::Quit => Command::Quit,
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
