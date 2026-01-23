//! Memcached ASCII protocol response builder

use bytes::BytesMut;
use itoa::Buffer;

/// Response writer for memcached ASCII protocol
pub struct ResponseWriter {
    buf: BytesMut,
}

impl ResponseWriter {
    /// Create a new response writer with the given capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: BytesMut::with_capacity(capacity),
        }
    }

    /// Get the internal buffer
    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    /// Take the buffer, leaving an empty buffer in its place
    pub fn take(&mut self) -> BytesMut {
        std::mem::take(&mut self.buf)
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Returns true if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Write a VALUE line for get response
    /// Format: VALUE <key> <flags> <bytes>\r\n<data>\r\n
    pub fn value(&mut self, key: &[u8], flags: u32, data: &[u8]) {
        let mut itoa_buf = Buffer::new();
        self.buf.extend_from_slice(b"VALUE ");
        self.buf.extend_from_slice(key);
        self.buf.extend_from_slice(b" ");
        self.buf
            .extend_from_slice(itoa_buf.format(flags).as_bytes());
        self.buf.extend_from_slice(b" ");
        self.buf
            .extend_from_slice(itoa_buf.format(data.len()).as_bytes());
        self.buf.extend_from_slice(b"\r\n");
        self.buf.extend_from_slice(data);
        self.buf.extend_from_slice(b"\r\n");
    }

    /// Write END to terminate get response
    pub fn end(&mut self) {
        self.buf.extend_from_slice(b"END\r\n");
    }

    /// Write STORED response
    pub fn stored(&mut self) {
        self.buf.extend_from_slice(b"STORED\r\n");
    }

    /// Write NOT_FOUND response
    pub fn not_found(&mut self) {
        self.buf.extend_from_slice(b"NOT_FOUND\r\n");
    }

    /// Write DELETED response
    pub fn deleted(&mut self) {
        self.buf.extend_from_slice(b"DELETED\r\n");
    }

    /// Write VERSION response
    /// Format: VERSION <version_string>\r\n
    /// Used by mcrouter for health checks (TKO recovery probes)
    pub fn version(&mut self, version: &str) {
        self.buf.extend_from_slice(b"VERSION ");
        self.buf.extend_from_slice(version.as_bytes());
        self.buf.extend_from_slice(b"\r\n");
    }

    /// Write CLIENT_ERROR response
    pub fn client_error(&mut self, message: &str) {
        self.buf.extend_from_slice(b"CLIENT_ERROR ");
        self.buf.extend_from_slice(message.as_bytes());
        self.buf.extend_from_slice(b"\r\n");
    }

    /// Write SERVER_ERROR response
    pub fn server_error(&mut self, message: &str) {
        self.buf.extend_from_slice(b"SERVER_ERROR ");
        self.buf.extend_from_slice(message.as_bytes());
        self.buf.extend_from_slice(b"\r\n");
    }
}

impl Default for ResponseWriter {
    fn default() -> Self {
        Self::new(4096)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value() {
        let mut writer = ResponseWriter::new(256);
        writer.value(b"mykey", 42, b"hello");
        assert_eq!(writer.buffer(), b"VALUE mykey 42 5\r\nhello\r\n");
    }

    #[test]
    fn test_get_response() {
        let mut writer = ResponseWriter::new(256);
        writer.value(b"key1", 0, b"value1");
        writer.value(b"key2", 1, b"value2");
        writer.end();

        let expected = b"VALUE key1 0 6\r\nvalue1\r\nVALUE key2 1 6\r\nvalue2\r\nEND\r\n";
        assert_eq!(writer.buffer(), &expected[..]);
    }

    #[test]
    fn test_simple_responses() {
        let mut writer = ResponseWriter::new(256);

        writer.stored();
        assert_eq!(writer.take().as_ref(), b"STORED\r\n");

        writer.deleted();
        assert_eq!(writer.take().as_ref(), b"DELETED\r\n");

        writer.not_found();
        assert_eq!(writer.take().as_ref(), b"NOT_FOUND\r\n");
    }

    #[test]
    fn test_errors() {
        let mut writer = ResponseWriter::new(256);

        writer.client_error("bad command line format");
        assert_eq!(
            writer.take().as_ref(),
            b"CLIENT_ERROR bad command line format\r\n"
        );

        writer.server_error("out of memory");
        assert_eq!(writer.take().as_ref(), b"SERVER_ERROR out of memory\r\n");
    }

    #[test]
    fn test_version() {
        let mut writer = ResponseWriter::new(256);
        writer.version("rocksproxy 0.1.0");
        assert_eq!(writer.buffer(), b"VERSION rocksproxy 0.1.0\r\n");
    }
}
