use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};

use crate::error::PiError;

/// Strips leading and trailing ASCII whitespace bytes.
fn trim_ascii_bytes(mut s: &[u8]) -> &[u8] {
    while let Some((first, rest)) = s.split_first() {
        if first.is_ascii_whitespace() {
            s = rest;
        } else {
            break;
        }
    }
    while let Some((last, rest)) = s.split_last() {
        if last.is_ascii_whitespace() {
            s = rest;
        } else {
            break;
        }
    }
    s
}

/// Strict LF-framed JSONL reader.
///
/// Reads records separated strictly by byte `0x0A` (`\n`), stripping an optional
/// preceding `0x0D` (`\r`). It does **not** split on Unicode line/paragraph separators
/// (`U+2028` and `U+2029`), preserving protocol integrity when JSON strings contain
/// such characters.
pub struct JsonLinesReader<R> {
    reader: BufReader<R>,
    buffer: Vec<u8>,
}

impl<R: AsyncRead + Unpin> JsonLinesReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            buffer: Vec::with_capacity(1024),
        }
    }

    /// Reads the next JSON record from the underlying stream.
    ///
    /// Returns `Ok(Some(record))` if a record was parsed,
    /// `Ok(None)` if the end of the stream was reached,
    /// or `Err(PiError)` on I/O or JSON parsing error.
    pub async fn read_record<T: DeserializeOwned>(&mut self) -> Result<Option<T>, PiError> {
        loop {
            self.buffer.clear();
            let bytes_read = self.reader.read_until(b'\n', &mut self.buffer).await?;
            if bytes_read == 0 {
                return Ok(None);
            }

            let mut slice = self.buffer.as_slice();
            if slice.ends_with(b"\n") {
                slice = &slice[..slice.len() - 1];
            }
            if slice.ends_with(b"\r") {
                slice = &slice[..slice.len() - 1];
            }

            let trimmed = trim_ascii_bytes(slice);
            if trimmed.is_empty() {
                // Skip empty lines and continue
                continue;
            }

            let record: T = serde_json::from_slice(trimmed)?;
            return Ok(Some(record));
        }
    }

    /// Unwraps the inner reader.
    pub fn into_inner(self) -> R {
        self.reader.into_inner()
    }
}

/// Asynchronous writer that frames JSON records with LF (`\n`) and flushes immediately.
pub struct JsonLinesWriter<W> {
    writer: BufWriter<W>,
}

impl<W: AsyncWrite + Unpin> JsonLinesWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer: BufWriter::new(writer),
        }
    }

    /// Serializes a record into a single JSON line, appends LF, and flushes.
    pub async fn write_record<T: Serialize>(&mut self, record: &T) -> Result<(), PiError> {
        let mut bytes = serde_json::to_vec(record)?;
        bytes.push(b'\n');
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Flushes any pending output.
    pub async fn flush(&mut self) -> Result<(), PiError> {
        self.writer.flush().await?;
        Ok(())
    }

    /// Unwraps the inner writer.
    pub fn into_inner(self) -> W {
        self.writer.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tokio::io::duplex;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct TestMessage {
        id: String,
        content: String,
    }

    #[tokio::test]
    async fn test_roundtrip_standard() {
        let (client, server) = duplex(1024);
        let mut writer = JsonLinesWriter::new(client);
        let mut reader = JsonLinesReader::new(server);

        let msg = TestMessage {
            id: "msg-1".into(),
            content: "Hello, world!".into(),
        };

        writer.write_record(&msg).await.unwrap();
        let received: Option<TestMessage> = reader.read_record().await.unwrap();

        assert_eq!(received, Some(msg));
    }

    #[tokio::test]
    async fn test_unicode_line_separators_not_split() {
        // U+2028 is \u{2028} (E2 80 A8)
        // U+2029 is \u{2029} (E2 80 A9)
        let (client, server) = duplex(1024);
        let mut writer = JsonLinesWriter::new(client);
        let mut reader = JsonLinesReader::new(server);

        let special_content = format!("Before{}Middle{}After", '\u{2028}', '\u{2029}');
        let msg = TestMessage {
            id: "msg-unicode".into(),
            content: special_content.clone(),
        };

        writer.write_record(&msg).await.unwrap();
        let received: Option<TestMessage> = reader.read_record().await.unwrap();

        assert_eq!(received.unwrap().content, special_content);
    }

    #[tokio::test]
    async fn test_crlf_and_empty_lines() {
        let (mut client, server) = duplex(1024);
        let mut reader = JsonLinesReader::new(server);

        tokio::spawn(async move {
            let data = b"\r\n\r\n{\"id\":\"1\",\"content\":\"first\"}\r\n\n{\"id\":\"2\",\"content\":\"second\"}\n\r\n";
            client.write_all(data).await.unwrap();
            client.flush().await.unwrap();
        });

        let r1: Option<TestMessage> = reader.read_record().await.unwrap();
        assert_eq!(r1.unwrap().id, "1");

        let r2: Option<TestMessage> = reader.read_record().await.unwrap();
        assert_eq!(r2.unwrap().id, "2");

        let r3: Option<TestMessage> = reader.read_record().await.unwrap();
        assert_eq!(r3, None);
    }
}
