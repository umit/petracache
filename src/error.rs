//! Error types for PetraCache

use thiserror::Error;

/// Main error type for PetraCache
#[derive(Error, Debug)]
pub enum PetraCacheError {
    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),
}

/// Protocol parsing errors
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("Invalid command: {0}")]
    InvalidCommand(String),

    #[error("Invalid key: {0}")]
    InvalidKey(String),

    #[error("Invalid value: {0}")]
    InvalidValue(String),

    #[error("Invalid flags")]
    InvalidFlags,

    #[error("Invalid exptime")]
    InvalidExptime,

    #[error("Invalid bytes length")]
    InvalidBytesLength,

    #[error("Invalid numeric value")]
    InvalidNumericValue,

    #[error("Key too long (max 250 bytes)")]
    KeyTooLong,

    #[error("Value too large")]
    ValueTooLarge,

    #[error("Unexpected data")]
    UnexpectedData,

    #[error("Incomplete command")]
    IncompleteCommand,
}

/// Storage layer errors
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("RocksDB error: {0}")]
    RocksDb(#[from] rust_rocksdb::Error),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Value encoding error: {0}")]
    Encoding(String),

    #[error("Value decoding error: {0}")]
    Decoding(String),

    #[error("Key not found")]
    NotFound,

    #[error("Key already exists")]
    AlreadyExists,

    #[error("Not a numeric value")]
    NotNumeric,

    #[error("Numeric overflow")]
    NumericOverflow,

    #[error("Numeric underflow")]
    NumericUnderflow,
}

pub type Result<T> = std::result::Result<T, PetraCacheError>;
