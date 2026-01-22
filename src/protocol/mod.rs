//! Memcached ASCII protocol implementation

pub mod command;
pub mod parser;
pub mod response;

pub use command::{Command, MAX_KEY_LENGTH};
pub use parser::{
    ParseResult, PendingStorageCommand, parse, parse_storage_command_line, parse_storage_data,
};
pub use response::ResponseWriter;
