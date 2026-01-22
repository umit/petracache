//! Value encoding/decoding for RocksDB storage
//!
//! Binary format: [8 bytes: expire_at][4 bytes: flags][N bytes: data]
//!
//! TTL Rules (memcached-compatible):
//! - 0 = never expire
//! - <= 2592000 (30 days) = relative seconds from now
//! - > 2592000 = absolute Unix timestamp

use crate::StorageError;
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum relative TTL value (30 days in seconds)
const MAX_RELATIVE_TTL: u64 = 2592000;

/// Stored value with metadata
#[derive(Debug, Clone)]
pub struct StoredValue {
    /// Expiration timestamp (0 = never expire)
    pub expire_at: u64,
    /// Memcached flags
    pub flags: u32,
    /// Actual data
    pub data: Vec<u8>,
}

impl StoredValue {
    /// Create a new stored value
    pub fn new(flags: u32, exptime: u64, data: Vec<u8>) -> Self {
        let expire_at = calculate_expire_at(exptime);
        Self {
            expire_at,
            flags,
            data,
        }
    }

    /// Create a stored value with a pre-calculated expire_at timestamp
    pub fn with_expire_at(flags: u32, expire_at: u64, data: Vec<u8>) -> Self {
        Self {
            expire_at,
            flags,
            data,
        }
    }

    /// Encode the value to bytes for storage
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(12 + self.data.len());
        buf.extend_from_slice(&self.expire_at.to_le_bytes());
        buf.extend_from_slice(&self.flags.to_le_bytes());
        buf.extend_from_slice(&self.data);
        buf
    }

    /// Decode a stored value from bytes
    pub fn decode(bytes: &[u8]) -> Result<Self, StorageError> {
        if bytes.len() < 12 {
            return Err(StorageError::Decoding(
                "Value too short to decode".to_string(),
            ));
        }

        let expire_at = u64::from_le_bytes(
            bytes[0..8]
                .try_into()
                .map_err(|_| StorageError::Decoding("Invalid expire_at".to_string()))?,
        );

        let flags = u32::from_le_bytes(
            bytes[8..12]
                .try_into()
                .map_err(|_| StorageError::Decoding("Invalid flags".to_string()))?,
        );

        let data = bytes[12..].to_vec();

        Ok(Self {
            expire_at,
            flags,
            data,
        })
    }

    /// Check if the value has expired
    pub fn is_expired(&self) -> bool {
        if self.expire_at == 0 {
            return false;
        }
        current_timestamp() >= self.expire_at
    }

    /// Update the expiration time
    pub fn touch(&mut self, exptime: u64) {
        self.expire_at = calculate_expire_at(exptime);
    }

    /// Get the data as a numeric value for incr/decr
    pub fn as_u64(&self) -> Result<u64, StorageError> {
        let s = std::str::from_utf8(&self.data).map_err(|_| StorageError::NotNumeric)?;
        s.trim()
            .parse::<u64>()
            .map_err(|_| StorageError::NotNumeric)
    }

    /// Set the data from a numeric value
    pub fn set_numeric(&mut self, value: u64) {
        self.data = value.to_string().into_bytes();
    }
}

/// Calculate the absolute expiration timestamp from memcached exptime
pub fn calculate_expire_at(exptime: u64) -> u64 {
    if exptime == 0 {
        0 // Never expire
    } else if exptime <= MAX_RELATIVE_TTL {
        // Relative time: add to current timestamp
        current_timestamp() + exptime
    } else {
        // Absolute Unix timestamp
        exptime
    }
}

/// Get the current Unix timestamp
pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode() {
        let value = StoredValue::with_expire_at(42, 1234567890, b"hello".to_vec());
        let encoded = value.encode();
        let decoded = StoredValue::decode(&encoded).unwrap();

        assert_eq!(decoded.expire_at, 1234567890);
        assert_eq!(decoded.flags, 42);
        assert_eq!(decoded.data, b"hello");
    }

    #[test]
    fn test_never_expire() {
        let value = StoredValue::new(0, 0, b"data".to_vec());
        assert_eq!(value.expire_at, 0);
        assert!(!value.is_expired());
    }

    #[test]
    fn test_relative_ttl() {
        let now = current_timestamp();
        let value = StoredValue::new(0, 60, b"data".to_vec());
        // Allow 1 second tolerance
        assert!(value.expire_at >= now + 59 && value.expire_at <= now + 61);
    }

    #[test]
    fn test_absolute_timestamp() {
        let future = current_timestamp() + 3000000;
        let value = StoredValue::new(0, future, b"data".to_vec());
        assert_eq!(value.expire_at, future);
    }

    #[test]
    fn test_expired() {
        let value = StoredValue::with_expire_at(0, 1, b"data".to_vec());
        assert!(value.is_expired());
    }

    #[test]
    fn test_numeric_value() {
        let mut value = StoredValue::with_expire_at(0, 0, b"123".to_vec());
        assert_eq!(value.as_u64().unwrap(), 123);

        value.set_numeric(456);
        assert_eq!(value.data, b"456");
        assert_eq!(value.as_u64().unwrap(), 456);
    }

    #[test]
    fn test_invalid_numeric() {
        let value = StoredValue::with_expire_at(0, 0, b"hello".to_vec());
        assert!(value.as_u64().is_err());
    }

    #[test]
    fn test_decode_too_short() {
        let result = StoredValue::decode(&[0, 1, 2]);
        assert!(result.is_err());
    }
}
