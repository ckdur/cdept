//! Messages between CDePT and the `cdept-xschem` helper.
//!
//! Transport: one Unix socket, UTF-8, one JSON object per line. CDePT listens and the
//! helper connects (its `--connect <path>` argument). After connecting the helper sends
//! [`Message::Ready`] or [`Message::Failed`]; from then on every [`Request`] gets
//! exactly one [`Message::Reply`] with the same `id`.

use std::io::{self, BufRead, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Bumped on incompatible changes. Both sides must agree.
pub const PROTOCOL_VERSION: u32 = 1;

/// CDePT → helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Evaluate a Tcl script in xschem's interpreter, at global level.
    Tcl { id: u64, script: String },
    /// Reply, then exit.
    Shutdown { id: u64 },
}

/// Helper → CDePT.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    /// xschem started. `xschem` is its version string.
    Ready { protocol: u32, xschem: String },
    /// xschem could not start; the helper exits after this.
    Failed { error: String },
    /// Answer to a request. `value` is the result, or the error text when `ok` is false.
    Reply { id: u64, ok: bool, value: String },
}

/// Write one message as a line and flush.
pub fn write_message<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()
}

/// Read one message. `Ok(None)` when the other side closed the connection.
pub fn read_message<T: DeserializeOwned>(reader: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(&line)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_single_lines() {
        let request = Request::Tcl {
            id: 7,
            script: "puts {a\nb}".into(),
        };
        let mut wire = Vec::new();
        write_message(&mut wire, &request).unwrap();
        let text = String::from_utf8(wire.clone()).unwrap();
        assert_eq!(text.matches('\n').count(), 1, "newlines must be escaped");
        assert!(text.contains(r#""type":"tcl""#));

        let back: Option<Request> = read_message(&mut wire.as_slice()).unwrap();
        assert_eq!(back, Some(request));
    }

    #[test]
    fn end_of_stream_is_none() {
        let back: Option<Message> = read_message(&mut &b""[..]).unwrap();
        assert_eq!(back, None);
    }

    #[test]
    fn garbage_is_an_error() {
        let result: io::Result<Option<Message>> = read_message(&mut &b"not json\n"[..]);
        assert!(result.is_err());
    }
}
