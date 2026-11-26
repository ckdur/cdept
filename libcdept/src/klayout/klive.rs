//! klive: a KLayout window that runs the klive salt package listens on TCP port 8082
//! of this machine and opens (or reloads) a layout sent to it as JSON
//! (`{"gds": "<file>"}`). CDePT uses it as the first "open layout in KLayout" path
//! (docs/KLAYOUT.md): the file goes to the KLayout that already runs.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use eyre::{Result, WrapErr, eyre};

/// klive's port.
pub const PORT: u16 = 8082;

/// The JSON klive reads for `file`.
pub fn message(file: &Path) -> String {
    serde_json::json!({ "gds": file.to_string_lossy() }).to_string()
}

/// Send `file` to a running KLayout with klive. An error when nothing listens (no
/// KLayout with klive runs) or the connection fails.
pub fn send(file: &Path) -> Result<String> {
    send_to(&SocketAddr::from(([127, 0, 0, 1], PORT)), file)
}

/// [`send`] to another address (tests).
pub fn send_to(addr: &SocketAddr, file: &Path) -> Result<String> {
    let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
    let mut stream = TcpStream::connect_timeout(addr, Duration::from_millis(300))
        .map_err(|e| eyre!("no KLayout with klive answers on {addr}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream
        .write_all(message(&file).as_bytes())
        .wrap_err("cannot send to klive")?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    // klive answers with a short JSON line (its version); some versions say nothing.
    let mut answer = Vec::new();
    let _ = stream.take(4096).read_to_end(&mut answer);
    Ok(String::from_utf8_lossy(&answer).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_is_sent_as_json_and_nobody_listening_is_an_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut got = String::new();
            s.read_to_string(&mut got).unwrap();
            s.write_all(b"{\"version\": \"0.4\"}\n").unwrap();
            got
        });
        let answer = send_to(&addr, Path::new("/p/a b.gds")).unwrap();
        assert!(answer.contains("version"));
        let got: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
        assert_eq!(got["gds"], "/p/a b.gds");
        // A port nobody listens on.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = free.local_addr().unwrap();
        drop(free);
        assert!(send_to(&addr, Path::new("/x.gds")).is_err());
    }
}
