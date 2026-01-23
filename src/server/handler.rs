//! Command handlers for memcached protocol commands

use super::Server;
use crate::protocol::{Command, ResponseWriter};
use crate::storage::StoredValue;
use std::sync::Arc;

/// Execute a parsed command
pub fn execute(server: &Arc<Server>, cmd: Command<'_>, response: &mut ResponseWriter) {
    match cmd {
        Command::Get { keys } => {
            server.metrics.cmd_get.inc();
            handle_get(server, keys, response);
        }
        Command::Set {
            key,
            flags,
            exptime,
            data,
            ..
        } => {
            server.metrics.cmd_set.inc();
            handle_set(server, &key, flags, exptime, &data, response);
        }
        Command::Delete { key, .. } => {
            server.metrics.cmd_delete.inc();
            handle_delete(server, &key, response);
        }
        Command::Version => {
            handle_version(response);
        }
        Command::Quit => {
            // Handled in connection loop
        }
    }
}

/// Handle VERSION command (used by mcrouter for health checks)
fn handle_version(response: &mut ResponseWriter) {
    response.version(concat!("petracache ", env!("CARGO_PKG_VERSION")));
}

/// Handle GET command
fn handle_get(
    server: &Arc<Server>,
    keys: Vec<std::borrow::Cow<'_, [u8]>>,
    response: &mut ResponseWriter,
) {
    if keys.len() == 1 {
        // Fast path - single key (most common case)
        match server.storage.get(&keys[0]) {
            Ok(Some(value)) => {
                server.metrics.get_hits.inc();
                response.value(&keys[0], value.flags, &value.data);
            }
            Ok(None) => {
                server.metrics.get_misses.inc();
            }
            Err(e) => {
                server.metrics.storage_errors.inc();
                response.server_error(&e.to_string());
                return;
            }
        }
    } else {
        // Multi-key path
        let keys_vec: Vec<Vec<u8>> = keys.iter().map(|k| k.to_vec()).collect();
        match server.storage.get_multi(&keys_vec) {
            Ok(results) => {
                for (key, value_opt) in results {
                    if let Some(value) = value_opt {
                        server.metrics.get_hits.inc();
                        response.value(&key, value.flags, &value.data);
                    } else {
                        server.metrics.get_misses.inc();
                    }
                }
            }
            Err(e) => {
                server.metrics.storage_errors.inc();
                response.server_error(&e.to_string());
                return;
            }
        }
    }
    response.end();
}

/// Handle SET command
fn handle_set(
    server: &Arc<Server>,
    key: &[u8],
    flags: u32,
    exptime: u64,
    data: &[u8],
    response: &mut ResponseWriter,
) {
    let value = StoredValue::new(flags, exptime, data.to_vec());
    match server.storage.set(key, value) {
        Ok(()) => response.stored(),
        Err(e) => {
            server.metrics.storage_errors.inc();
            response.server_error(&e.to_string());
        }
    }
}

/// Handle DELETE command
fn handle_delete(server: &Arc<Server>, key: &[u8], response: &mut ResponseWriter) {
    match server.storage.delete(key) {
        Ok(true) => response.deleted(),
        Ok(false) => response.not_found(),
        Err(e) => {
            server.metrics.storage_errors.inc();
            response.server_error(&e.to_string());
        }
    }
}
