//! Hand-written memcached ASCII protocol parser
//!
//! Two-phase parsing:
//! 1. Parse command line (up to \r\n)
//! 2. For storage commands, read data block

use crate::ProtocolError;
use crate::protocol::command::{Command, MAX_KEY_LENGTH, is_valid_key};
use std::borrow::Cow;

/// Case-insensitive command comparison (avoids allocation from to_ascii_lowercase)
#[inline]
fn cmd_eq(cmd: &[u8], expected: &[u8]) -> bool {
    cmd.len() == expected.len()
        && cmd
            .iter()
            .zip(expected.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
}

/// Result of parsing
#[derive(Debug)]
pub enum ParseResult<'a> {
    /// Command fully parsed
    Complete(Command<'a>, usize),
    /// Need more data to complete parsing
    NeedMoreData,
    /// Parse error
    Error(ProtocolError),
}

/// Parser state for handling storage commands that need data
#[derive(Debug, Clone)]
pub struct PendingStorageCommand {
    pub key: Vec<u8>,
    pub flags: u32,
    pub exptime: u64,
    pub bytes: usize,
    pub noreply: bool,
    pub command_line_end: usize,
}

/// Parse a memcached command from a buffer
pub fn parse(buf: &[u8]) -> ParseResult<'_> {
    // Find the end of the command line
    let line_end = match find_crlf(buf) {
        Some(pos) => pos,
        None => return ParseResult::NeedMoreData,
    };

    let line = &buf[..line_end];

    // Parse the command name
    let mut parts = line.split(|&b| b == b' ');
    let cmd_name = match parts.next() {
        Some(name) if !name.is_empty() => name,
        _ => return ParseResult::Error(ProtocolError::InvalidCommand("empty command".to_string())),
    };

    // Match command (case-insensitive, no allocation)
    if cmd_eq(cmd_name, b"get") {
        parse_get(parts, line_end + 2)
    } else if cmd_eq(cmd_name, b"set") {
        parse_set(parts, buf, line_end)
    } else if cmd_eq(cmd_name, b"delete") {
        parse_delete(parts, line_end + 2)
    } else if cmd_eq(cmd_name, b"version") {
        ParseResult::Complete(Command::Version, line_end + 2)
    } else if cmd_eq(cmd_name, b"quit") {
        ParseResult::Complete(Command::Quit, line_end + 2)
    } else {
        ParseResult::Error(ProtocolError::InvalidCommand(
            String::from_utf8_lossy(cmd_name).to_string(),
        ))
    }
}

/// Continue parsing a storage command after receiving data block
pub fn parse_storage_data<'a>(buf: &'a [u8], pending: &PendingStorageCommand) -> ParseResult<'a> {
    // Need: command_line_end + data_bytes + 2 (for \r\n after data)
    let data_start = pending.command_line_end + 2;
    let data_end = data_start + pending.bytes;
    let total_needed = data_end + 2; // +2 for trailing \r\n

    if buf.len() < total_needed {
        return ParseResult::NeedMoreData;
    }

    // Verify trailing \r\n
    if buf[data_end] != b'\r' || buf[data_end + 1] != b'\n' {
        return ParseResult::Error(ProtocolError::UnexpectedData);
    }

    let data = Cow::Borrowed(&buf[data_start..data_end]);
    let key = Cow::Owned(pending.key.clone());

    let cmd = Command::Set {
        key,
        flags: pending.flags,
        exptime: pending.exptime,
        data,
        noreply: pending.noreply,
    };

    ParseResult::Complete(cmd, total_needed)
}

/// Find \r\n in buffer
fn find_crlf(buf: &[u8]) -> Option<usize> {
    (0..buf.len().saturating_sub(1)).find(|&i| buf[i] == b'\r' && buf[i + 1] == b'\n')
}

/// Parse get command
fn parse_get<'a>(
    mut parts: impl Iterator<Item = &'a [u8]>,
    consumed: usize,
) -> ParseResult<'a> {
    let mut keys = Vec::new();

    for part in parts.by_ref() {
        if part.is_empty() {
            continue;
        }
        if !is_valid_key(part) {
            if part.len() > MAX_KEY_LENGTH {
                return ParseResult::Error(ProtocolError::KeyTooLong);
            }
            return ParseResult::Error(ProtocolError::InvalidKey(
                String::from_utf8_lossy(part).to_string(),
            ));
        }
        keys.push(Cow::Borrowed(part));
    }

    if keys.is_empty() {
        return ParseResult::Error(ProtocolError::InvalidCommand(
            "get requires at least one key".to_string(),
        ));
    }

    ParseResult::Complete(Command::Get { keys }, consumed)
}

/// Parse set command
fn parse_set<'a>(
    mut parts: impl Iterator<Item = &'a [u8]>,
    buf: &'a [u8],
    line_end: usize,
) -> ParseResult<'a> {
    // <key> <flags> <exptime> <bytes> [noreply]
    let key = match parts.next() {
        Some(k) if !k.is_empty() => k,
        _ => return ParseResult::Error(ProtocolError::InvalidCommand("missing key".to_string())),
    };

    if !is_valid_key(key) {
        if key.len() > MAX_KEY_LENGTH {
            return ParseResult::Error(ProtocolError::KeyTooLong);
        }
        return ParseResult::Error(ProtocolError::InvalidKey(
            String::from_utf8_lossy(key).to_string(),
        ));
    }

    let flags = match parts.next().and_then(parse_u32) {
        Some(f) => f,
        None => return ParseResult::Error(ProtocolError::InvalidFlags),
    };

    let exptime = match parts.next().and_then(parse_u64) {
        Some(e) => e,
        None => return ParseResult::Error(ProtocolError::InvalidExptime),
    };

    let bytes = match parts.next().and_then(parse_usize) {
        Some(b) => b,
        None => return ParseResult::Error(ProtocolError::InvalidBytesLength),
    };

    let noreply = parts.next().map(|s| s == b"noreply").unwrap_or(false);

    // Check if we have enough data for the data block
    let data_start = line_end + 2;
    let data_end = data_start + bytes;
    let total_needed = data_end + 2;

    if buf.len() < total_needed {
        return ParseResult::NeedMoreData;
    }

    // Verify trailing \r\n
    if buf[data_end] != b'\r' || buf[data_end + 1] != b'\n' {
        return ParseResult::Error(ProtocolError::UnexpectedData);
    }

    let data = Cow::Borrowed(&buf[data_start..data_end]);
    let key = Cow::Borrowed(key);

    let cmd = Command::Set {
        key,
        flags,
        exptime,
        data,
        noreply,
    };

    ParseResult::Complete(cmd, total_needed)
}

/// Parse pending storage command line (for partial reads)
pub fn parse_storage_command_line(
    buf: &[u8],
) -> Result<Option<PendingStorageCommand>, ProtocolError> {
    let line_end = match find_crlf(buf) {
        Some(pos) => pos,
        None => return Ok(None),
    };

    let line = &buf[..line_end];
    let mut parts = line.split(|&b| b == b' ');

    let cmd_name = match parts.next() {
        Some(name) if !name.is_empty() => name,
        _ => return Err(ProtocolError::InvalidCommand("empty command".to_string())),
    };

    // Only handle set command (case-insensitive, no allocation)
    if !cmd_eq(cmd_name, b"set") {
        return Ok(None);
    }

    let key = match parts.next() {
        Some(k) if !k.is_empty() => k,
        _ => return Err(ProtocolError::InvalidCommand("missing key".to_string())),
    };

    if !is_valid_key(key) {
        if key.len() > MAX_KEY_LENGTH {
            return Err(ProtocolError::KeyTooLong);
        }
        return Err(ProtocolError::InvalidKey(
            String::from_utf8_lossy(key).to_string(),
        ));
    }

    let flags = parts
        .next()
        .and_then(parse_u32)
        .ok_or(ProtocolError::InvalidFlags)?;

    let exptime = parts
        .next()
        .and_then(parse_u64)
        .ok_or(ProtocolError::InvalidExptime)?;

    let bytes = parts
        .next()
        .and_then(parse_usize)
        .ok_or(ProtocolError::InvalidBytesLength)?;

    let noreply = parts.next().map(|s| s == b"noreply").unwrap_or(false);

    Ok(Some(PendingStorageCommand {
        key: key.to_vec(),
        flags,
        exptime,
        bytes,
        noreply,
        command_line_end: line_end,
    }))
}

/// Parse delete command
/// Format: delete <key> [exptime] [noreply]\r\n
/// exptime is parsed but ignored (for mcrouter compatibility)
fn parse_delete<'a>(mut parts: impl Iterator<Item = &'a [u8]>, consumed: usize) -> ParseResult<'a> {
    let key = match parts.next() {
        Some(k) if !k.is_empty() => k,
        _ => {
            return ParseResult::Error(ProtocolError::InvalidCommand(
                "delete requires a key".to_string(),
            ));
        }
    };

    if !is_valid_key(key) {
        if key.len() > MAX_KEY_LENGTH {
            return ParseResult::Error(ProtocolError::KeyTooLong);
        }
        return ParseResult::Error(ProtocolError::InvalidKey(
            String::from_utf8_lossy(key).to_string(),
        ));
    }

    // Parse optional exptime and noreply
    // Format: [exptime] [noreply] where exptime is a number
    let mut noreply = false;
    for part in parts {
        if part.is_empty() {
            continue;
        }
        if part == b"noreply" {
            noreply = true;
        }
        // If it's a number, it's exptime - we parse but ignore it
        // (memcached delete exptime is deprecated anyway)
    }

    ParseResult::Complete(
        Command::Delete {
            key: Cow::Borrowed(key),
            noreply,
        },
        consumed,
    )
}

/// Parse bytes as u32
fn parse_u32(bytes: &[u8]) -> Option<u32> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Parse bytes as u64
fn parse_u64(bytes: &[u8]) -> Option<u64> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Parse bytes as usize
fn parse_usize(bytes: &[u8]) -> Option<usize> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_get() {
        let buf = b"get foo bar baz\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Get { keys }, consumed) => {
                assert_eq!(keys.len(), 3);
                assert_eq!(keys[0].as_ref(), b"foo");
                assert_eq!(keys[1].as_ref(), b"bar");
                assert_eq!(keys[2].as_ref(), b"baz");
                assert_eq!(consumed, buf.len());
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_set() {
        let buf = b"set mykey 42 3600 5\r\nhello\r\n";
        match parse(buf) {
            ParseResult::Complete(
                Command::Set {
                    key,
                    flags,
                    exptime,
                    data,
                    noreply,
                },
                consumed,
            ) => {
                assert_eq!(key.as_ref(), b"mykey");
                assert_eq!(flags, 42);
                assert_eq!(exptime, 3600);
                assert_eq!(data.as_ref(), b"hello");
                assert!(!noreply);
                assert_eq!(consumed, buf.len());
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_set_noreply() {
        let buf = b"set mykey 0 0 3 noreply\r\nfoo\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Set { noreply, .. }, _) => {
                assert!(noreply);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_delete() {
        let buf = b"delete mykey\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Delete { key, noreply }, _) => {
                assert_eq!(key.as_ref(), b"mykey");
                assert!(!noreply);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_delete_noreply() {
        let buf = b"delete mykey noreply\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Delete { noreply, .. }, _) => {
                assert!(noreply);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_delete_with_exptime() {
        // mcrouter format: delete <key> <exptime>\r\n
        let buf = b"delete mykey 0\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Delete { key, noreply }, _) => {
                assert_eq!(key.as_ref(), b"mykey");
                assert!(!noreply);
            }
            other => panic!("unexpected: {:?}", other),
        }

        // delete <key> <exptime> noreply\r\n
        let buf = b"delete mykey 300 noreply\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Delete { key, noreply }, _) => {
                assert_eq!(key.as_ref(), b"mykey");
                assert!(noreply);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_quit() {
        let buf = b"quit\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Quit, _) => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_version() {
        let buf = b"version\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Version, consumed) => {
                assert_eq!(consumed, buf.len());
            }
            other => panic!("unexpected: {:?}", other),
        }

        // Case insensitive
        let buf = b"VERSION\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Version, _) => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_need_more_data() {
        let buf = b"get foo";
        match parse(buf) {
            ParseResult::NeedMoreData => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_invalid_command() {
        let buf = b"invalid\r\n";
        match parse(buf) {
            ParseResult::Error(ProtocolError::InvalidCommand(_)) => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_parse_key_too_long() {
        let long_key = vec![b'a'; 251];
        let mut buf = b"get ".to_vec();
        buf.extend_from_slice(&long_key);
        buf.extend_from_slice(b"\r\n");

        match parse(&buf) {
            ParseResult::Error(ProtocolError::KeyTooLong) => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_case_insensitive_commands() {
        let buf = b"GET foo\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Get { .. }, _) => {}
            other => panic!("unexpected: {:?}", other),
        }

        let buf = b"SET mykey 0 0 3\r\nbar\r\n";
        match parse(buf) {
            ParseResult::Complete(Command::Set { .. }, _) => {}
            other => panic!("unexpected: {:?}", other),
        }
    }
}
