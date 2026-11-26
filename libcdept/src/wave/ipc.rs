//! The client of UberWave's IPC (protocol 1): a Unix domain socket, one UTF-8 JSON object
//! per line in both directions. A request is `{"id":N,"cmd":"name",...args}`, the answer
//! `{"id":N,"ok":true,...}` or `{"id":N,"ok":false,"error":"..."}`. Unsolicited lines
//! (`{"event":"cursor",...}` after `subscribe`) are kept in [`Client::take_events`].
//!
//! Only what CDePT needs is typed; [`Client::request`] sends any command.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eyre::{Result, WrapErr, bail, eyre};
use serde_json::{Map, Value, json};

/// The protocol version this client speaks.
pub const PROTOCOL: u64 = 1;
/// How long to wait for the answer to a request by default.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest line accepted from the viewer (a big `list_signals` answer).
const MAX_LINE: usize = 256 * 1024 * 1024;

/// What `hello` answers.
#[derive(Debug, Clone, PartialEq)]
pub struct Hello {
    pub app: String,
    pub version: String,
    pub protocol: u64,
    pub features: Vec<String>,
}

/// One signal of `list_signals`.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub name: String,
    pub file: String,
    pub unit: String,
    pub complex: bool,
}

/// A file of the session (`list_files`).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionFile {
    pub tag: String,
    pub path: String,
    pub analyses: Vec<String>,
}

/// Data pushed as a new trace (`add_trace`).
#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    pub name: String,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub unit: Option<String>,
    pub x_unit: Option<String>,
    /// Where it goes.
    pub pane: PaneSel,
    /// X on a log scale (a Bode plot).
    pub x_log: bool,
    /// An eye diagram: `(period, offset)`.
    pub fold: Option<(f64, f64)>,
}

/// The pane a signal or trace goes to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PaneSel {
    /// The viewer chooses (the active pane; for a trace of that name, its pane).
    #[default]
    Default,
    /// A new pane.
    New,
    /// This pane, counting from 0.
    Index(u32),
}

impl PaneSel {
    /// The protocol's form: a number, `"new"`, or nothing.
    fn json(self) -> Option<Value> {
        match self {
            PaneSel::Default => None,
            PaneSel::New => Some(json!("new")),
            PaneSel::Index(i) => Some(json!(i)),
        }
    }
}

impl Trace {
    pub fn new(name: impl Into<String>, x: Vec<f64>, y: Vec<f64>) -> Trace {
        Trace {
            name: name.into(),
            x,
            y,
            unit: None,
            x_unit: None,
            pane: PaneSel::Default,
            x_log: false,
            fold: None,
        }
    }
}

/// A connection to a running viewer.
pub struct Client {
    stream: UnixStream,
    buf: Vec<u8>,
    next_id: i64,
    timeout: Duration,
    events: Vec<Value>,
}

impl Client {
    /// Connect once.
    pub fn connect_once(socket: &Path) -> Result<Client> {
        let stream = UnixStream::connect(socket)
            .wrap_err_with(|| format!("cannot connect to {}", socket.display()))?;
        Ok(Client {
            stream,
            buf: Vec::new(),
            next_id: 1,
            timeout: REPLY_TIMEOUT,
            events: Vec::new(),
        })
    }

    /// Connect, trying again every 50 ms until `timeout` (the viewer needs a moment to
    /// create its socket after it was started).
    pub fn connect(socket: &Path, timeout: Duration) -> Result<Client> {
        let end = Instant::now() + timeout;
        loop {
            match Client::connect_once(socket) {
                Ok(c) => return Ok(c),
                Err(e) if Instant::now() >= end => {
                    return Err(e.wrap_err(format!(
                        "the viewer did not accept a connection on {} within {:.0} s",
                        socket.display(),
                        timeout.as_secs_f64()
                    )));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    /// The time to wait for each answer.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Events received while waiting for answers, oldest first (and forgotten).
    pub fn take_events(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.events)
    }

    /// Send a command and wait for its answer. `args` is a JSON object (or null) whose
    /// members go next to `id` and `cmd`. An `ok:false` answer is an error carrying the
    /// viewer's text. Returns the answer object.
    pub fn request(&mut self, cmd: &str, args: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let mut obj = match args {
            Value::Object(m) => m,
            Value::Null => Map::new(),
            other => return Err(eyre!("arguments of '{cmd}' must be an object, not {other}")),
        };
        obj.insert("id".into(), json!(id));
        obj.insert("cmd".into(), json!(cmd));
        let mut line = serde_json::to_vec(&Value::Object(obj))?;
        line.push(b'\n');
        let deadline = Instant::now() + self.timeout;
        self.stream.set_write_timeout(Some(self.timeout)).ok();
        self.stream
            .write_all(&line)
            .wrap_err_with(|| format!("cannot send '{cmd}' to the viewer"))?;
        loop {
            let reply = self.read_line(deadline, cmd)?;
            let v: Value = serde_json::from_slice(&reply).wrap_err_with(|| {
                format!(
                    "the viewer's answer to '{cmd}' is not JSON: {}",
                    preview(&reply)
                )
            })?;
            if v.get("event").is_some() && v.get("id").is_none() {
                self.events.push(v);
                continue;
            }
            match v.get("id").and_then(Value::as_i64) {
                Some(got) if got == id => {}
                // The answer to an earlier request that timed out.
                Some(got) if got < id => continue,
                _ => {
                    return Err(eyre!(
                        "the viewer's answer to '{cmd}' has the wrong id: {}",
                        preview(&reply)
                    ));
                }
            }
            return match v.get("ok").and_then(Value::as_bool) {
                Some(true) => Ok(v),
                Some(false) => {
                    let why = v
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("no reason given");
                    Err(eyre!("UberWave refused '{cmd}': {why}"))
                }
                None => Err(eyre!(
                    "the viewer's answer to '{cmd}' has no ok: {}",
                    preview(&reply)
                )),
            };
        }
    }

    /// One line (without the newline), waiting until `deadline`.
    fn read_line(&mut self, deadline: Instant, cmd: &str) -> Result<Vec<u8>> {
        loop {
            if let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=i).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if line.is_empty() {
                    continue;
                }
                return Ok(line);
            }
            if self.buf.len() > MAX_LINE {
                bail!("the viewer sent a line over {} MB", MAX_LINE >> 20);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                bail!(
                    "the viewer did not answer '{cmd}' within {:.0} s",
                    self.timeout.as_secs_f64()
                );
            }
            self.stream.set_read_timeout(Some(left)).ok();
            let mut chunk = [0u8; 64 * 1024];
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    return Err(eyre!(
                        "the viewer closed the connection{}",
                        if self.buf.is_empty() {
                            ""
                        } else {
                            " in the middle of an answer"
                        }
                    ));
                }
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e).wrap_err("cannot read from the viewer"),
            }
        }
    }

    pub fn hello(&mut self) -> Result<Hello> {
        let v = self.request("hello", Value::Null)?;
        let protocol = v.get("protocol").and_then(Value::as_u64).unwrap_or(0);
        if protocol != PROTOCOL {
            bail!("the viewer speaks IPC protocol {protocol}, CDePT {PROTOCOL}");
        }
        Ok(Hello {
            app: text(&v, "app"),
            version: text(&v, "version"),
            protocol,
            features: strings(v.get("features")),
        })
    }

    /// Replace the session by one file.
    pub fn open(&mut self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        self.request("open", file_args(path, analysis, tag))
            .map(drop)
    }

    /// Add a file to the session.
    pub fn import(&mut self, path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Result<()> {
        self.request("import", file_args(path, analysis, tag))
            .map(drop)
    }

    pub fn list_files(&mut self) -> Result<Vec<SessionFile>> {
        let v = self.request("list_files", Value::Null)?;
        Ok(v.get("files")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|f| SessionFile {
                        tag: text(f, "tag"),
                        path: text(f, "path"),
                        analyses: strings(f.get("analyses")),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The signals, optionally of one file (tag) and matching a filter. The flag says the
    /// viewer cut the list.
    pub fn list_signals(
        &mut self,
        file: Option<&str>,
        filter: Option<&str>,
    ) -> Result<(Vec<Signal>, bool)> {
        let mut args = Map::new();
        if let Some(f) = file {
            args.insert("file".into(), json!(f));
        }
        if let Some(f) = filter {
            args.insert("filter".into(), json!(f));
        }
        let v = self.request("list_signals", Value::Object(args))?;
        let signals = v
            .get("signals")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|s| Signal {
                        name: text(s, "name"),
                        file: text(s, "file"),
                        unit: text(s, "unit"),
                        complex: s.get("complex").and_then(Value::as_bool).unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok((
            signals,
            v.get("truncated").and_then(Value::as_bool).unwrap_or(false),
        ))
    }

    /// Show signals (`name` or `tag:name`) in a pane (a number, or `"new"`). Returns the
    /// pane they went to, when the viewer says it.
    pub fn show(&mut self, signals: &[String], pane: PaneSel) -> Result<Option<u64>> {
        let mut args = Map::new();
        args.insert("signals".into(), json!(signals));
        if let Some(p) = pane.json() {
            args.insert("pane".into(), p);
        }
        let v = self.request("show", Value::Object(args))?;
        Ok(v.get("pane").and_then(Value::as_u64))
    }

    pub fn hide(&mut self, signals: &[String]) -> Result<()> {
        self.request("hide", json!({ "signals": signals }))
            .map(drop)
    }

    pub fn set_analysis(&mut self, name: &str) -> Result<()> {
        self.request("set_analysis", json!({ "name": name }))
            .map(drop)
    }

    pub fn set_cursors(&mut self, a: Option<f64>, b: Option<f64>) -> Result<()> {
        let mut args = Map::new();
        if let Some(a) = a {
            args.insert("a".into(), json!(a));
        }
        if let Some(b) = b {
            args.insert("b".into(), json!(b));
        }
        self.request("set_cursors", Value::Object(args)).map(drop)
    }

    /// The raw answer: `{a, b, values:{sig:{a,b}}}`.
    pub fn get_cursors(&mut self) -> Result<Value> {
        self.request("get_cursors", Value::Null)
    }

    pub fn set_range(&mut self, range: Option<(f64, f64)>) -> Result<()> {
        let args = match range {
            Some((x0, x1)) => json!({ "x0": x0, "x1": x1 }),
            None => json!({ "fit": true }),
        };
        self.request("set_range", args).map(drop)
    }

    /// Push data as a new trace. Points whose x or y is not finite are left out (JSON has
    /// no way to say them).
    pub fn add_trace(&mut self, trace: &Trace) -> Result<()> {
        if trace.x.len() != trace.y.len() {
            bail!(
                "trace '{}': {} x values but {} y values",
                trace.name,
                trace.x.len(),
                trace.y.len()
            );
        }
        let (mut x, mut y) = (
            Vec::with_capacity(trace.x.len()),
            Vec::with_capacity(trace.y.len()),
        );
        for (a, b) in trace.x.iter().zip(&trace.y) {
            if a.is_finite() && b.is_finite() {
                x.push(*a);
                y.push(*b);
            }
        }
        let mut args = Map::new();
        args.insert("name".into(), json!(trace.name));
        args.insert("x".into(), json!(x));
        args.insert("y".into(), json!(y));
        if let Some(u) = &trace.unit {
            args.insert("unit".into(), json!(u));
        }
        if let Some(u) = &trace.x_unit {
            args.insert("x_unit".into(), json!(u));
        }
        if let Some(p) = trace.pane.json() {
            args.insert("pane".into(), p);
        }
        if trace.x_log {
            args.insert("x_log".into(), json!(true));
        }
        if let Some((period, offset)) = trace.fold {
            args.insert("fold_period".into(), json!(period));
            args.insert("fold_offset".into(), json!(offset));
        }
        self.request("add_trace", Value::Object(args)).map(drop)
    }

    pub fn remove_trace(&mut self, name: &str) -> Result<()> {
        self.request("remove_trace", json!({ "name": name }))
            .map(drop)
    }

    pub fn subscribe(&mut self, events: &[&str]) -> Result<()> {
        self.request("subscribe", json!({ "events": events }))
            .map(drop)
    }

    /// Ask the viewer to close. The answer may not come before it exits: a closed
    /// connection after the request counts as done.
    pub fn quit(&mut self) -> Result<()> {
        match self.request("quit", Value::Null) {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("closed the connection") => Ok(()),
            Err(e) => Err(e),
        }
    }
}

fn file_args(path: &Path, analysis: Option<&str>, tag: Option<&str>) -> Value {
    let mut args = Map::new();
    args.insert("path".into(), json!(path.to_string_lossy()));
    if let Some(a) = analysis {
        args.insert("analysis".into(), json!(a));
    }
    if let Some(t) = tag {
        args.insert("tag".into(), json!(t));
    }
    Value::Object(args)
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn preview(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(&bytes[..bytes.len().min(120)]);
    if bytes.len() > 120 {
        format!("{s}...")
    } else {
        s.into_owned()
    }
}

/// FNV-1a, 64 bits: stable between processes and builds (a socket name must be).
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

/// The socket of the viewer of one run: under `$XDG_RUNTIME_DIR` when it exists (per
/// user, private), else the temporary folder. The name is a hash of the project folder
/// and the run (`library/cell/view`), short enough for the 108 bytes of a socket path.
/// A second "open in UberWave" of the same run finds the viewer of the first.
pub fn socket_path(project_root: &Path, run: &str) -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let root = std::path::absolute(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    base.join(format!(
        "cdept-uw-{:016x}.sock",
        fnv(&format!("{}\n{run}", root.display()))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;

    /// A server in a thread: `handler` gets each request line and returns the lines to
    /// send back (none: no answer); `false` from `keep` closes the connection after.
    fn serve(
        dir: &Path,
        mut handler: impl FnMut(Value) -> Vec<String> + Send + 'static,
    ) -> (PathBuf, std::thread::JoinHandle<()>) {
        let sock = dir.join("s.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let h = std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            let mut out = conn.try_clone().unwrap();
            for line in BufReader::new(conn).lines() {
                let Ok(line) = line else { return };
                let v: Value = serde_json::from_str(&line).unwrap();
                for reply in handler(v) {
                    if reply == "<close>" {
                        return;
                    }
                    if out.write_all(reply.as_bytes()).is_err() {
                        return;
                    }
                }
            }
        });
        (sock, h)
    }

    fn ok(req: &Value, extra: Value) -> String {
        let mut o = extra.as_object().cloned().unwrap_or_default();
        o.insert("id".into(), req["id"].clone());
        o.insert("ok".into(), json!(true));
        format!("{}\n", Value::Object(o))
    }

    #[test]
    fn ids_count_up_and_hello_is_typed() {
        let tmp = tempfile::tempdir().unwrap();
        let (sock, h) = serve(tmp.path(), |r| match r["cmd"].as_str().unwrap() {
            "hello" => vec![ok(
                &r,
                json!({"app":"UberWaveViewer","version":"1.2","protocol":1,"features":["add_trace"]}),
            )],
            _ => vec![ok(&r, json!({"echo": r["id"]}))],
        });
        let mut c = Client::connect(&sock, Duration::from_secs(2)).unwrap();
        let hello = c.hello().unwrap();
        assert_eq!(
            (hello.app.as_str(), hello.protocol, hello.features.clone()),
            ("UberWaveViewer", 1, vec!["add_trace".to_string()])
        );
        let second = c.request("ping", Value::Null).unwrap();
        assert_eq!(second["echo"], 2);
        assert_eq!(c.request("ping", json!({"a": 1})).unwrap()["echo"], 3);
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn an_error_answer_carries_the_text_and_a_wrong_protocol_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (sock, h) = serve(tmp.path(), |r| match r["cmd"].as_str().unwrap() {
            "hello" => vec![ok(&r, json!({"app":"x","version":"9","protocol":2}))],
            _ => vec![format!(
                "{}\n",
                json!({"id": r["id"], "ok": false, "error": "unknown command 'frob'"})
            )],
        });
        let mut c = Client::connect_once(&sock).unwrap();
        let e = c.hello().unwrap_err().to_string();
        assert!(e.contains("protocol 2"), "{e}");
        let e = c.request("frob", Value::Null).unwrap_err().to_string();
        assert!(
            e.contains("refused 'frob'") && e.contains("unknown command 'frob'"),
            "{e}"
        );
        assert!(c.request("x", json!(5)).is_err());
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn a_big_trace_goes_through_and_non_finite_points_are_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let (sock, h) = serve(tmp.path(), |r| {
            let n = r["x"].as_array().map_or(0, Vec::len);
            assert_eq!(n, r["y"].as_array().unwrap().len());
            vec![ok(&r, json!({"points": n}))]
        });
        let mut c = Client::connect_once(&sock).unwrap();
        let n = 400_000;
        let x: Vec<f64> = (0..n).map(|i| i as f64 * 1e-9).collect();
        let mut y: Vec<f64> = x.iter().map(|t| (t * 1e6).sin()).collect();
        y[7] = f64::NAN;
        y[9] = f64::INFINITY;
        c.add_trace(&Trace::new("big", x, y)).unwrap();
        // The answer holds the count: ask again through request to read it.
        let r = c
            .request("add_trace", json!({"name":"s","x":[0.0,1.0],"y":[1.0,2.0]}))
            .unwrap();
        assert_eq!(r["points"], 2);
        assert!(c.add_trace(&Trace::new("bad", vec![1.0], vec![])).is_err());
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn events_are_kept_and_stale_answers_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let (sock, h) = serve(tmp.path(), |r| {
            vec![
                "{\"event\":\"cursor\",\"a\":1.5,\"b\":null,\"values\":{}}\n".to_string(),
                "{\"id\":0,\"ok\":true}\n".to_string(),
                ok(&r, json!({})),
            ]
        });
        let mut c = Client::connect_once(&sock).unwrap();
        c.subscribe(&["cursor"]).unwrap();
        let ev = c.take_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["a"], 1.5);
        assert!(c.take_events().is_empty());
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn a_reply_in_pieces_and_a_truncated_reply() {
        let tmp = tempfile::tempdir().unwrap();
        let mut n = 0;
        let (sock, h) = serve(tmp.path(), move |r| {
            n += 1;
            if n == 1 {
                let full = ok(&r, json!({"files": []}));
                let (a, b) = full.split_at(5);
                // Two pieces: handled by the line buffer.
                std::thread::sleep(Duration::from_millis(1));
                vec![a.to_string(), b.to_string()]
            } else {
                // Half a line, then the connection closes.
                vec!["{\"id\":2,\"ok\":tr".to_string(), "<close>".to_string()]
            }
        });
        let mut c = Client::connect_once(&sock).unwrap();
        assert!(c.list_files().unwrap().is_empty());
        let e = c
            .request("list_files", Value::Null)
            .unwrap_err()
            .to_string();
        assert!(e.contains("closed the connection in the middle"), "{e}");
        h.join().unwrap();
    }

    #[test]
    fn garbage_and_missing_ok_are_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mut n = 0;
        let (sock, h) = serve(tmp.path(), move |r| {
            n += 1;
            match n {
                1 => vec!["not json\n".to_string()],
                2 => vec![format!("{}\n", json!({"id": r["id"]}))],
                _ => vec![format!("{}\n", json!({"id": 99, "ok": true}))],
            }
        });
        let mut c = Client::connect_once(&sock).unwrap();
        assert!(
            c.request("a", Value::Null)
                .unwrap_err()
                .to_string()
                .contains("not JSON")
        );
        assert!(
            c.request("a", Value::Null)
                .unwrap_err()
                .to_string()
                .contains("no ok")
        );
        assert!(
            c.request("a", Value::Null)
                .unwrap_err()
                .to_string()
                .contains("wrong id")
        );
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn a_silent_server_times_out() {
        let tmp = tempfile::tempdir().unwrap();
        let (sock, h) = serve(tmp.path(), |_| vec![]);
        let mut c = Client::connect_once(&sock).unwrap();
        c.set_timeout(Duration::from_millis(200));
        let t = Instant::now();
        let e = c.request("hello", Value::Null).unwrap_err().to_string();
        assert!(e.contains("did not answer 'hello'"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(3));
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn connect_retries_until_the_socket_appears_and_gives_up() {
        let tmp = tempfile::tempdir().unwrap();
        let sock = tmp.path().join("late.sock");
        let t = Instant::now();
        let e = Client::connect(&sock, Duration::from_millis(150))
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("did not accept a connection"), "{e}");
        assert!(t.elapsed() >= Duration::from_millis(150));
        let s2 = sock.clone();
        let late = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            let l = UnixListener::bind(&s2).unwrap();
            let _ = l.accept();
        });
        Client::connect(&sock, Duration::from_secs(5)).unwrap();
        late.join().unwrap();
    }

    #[test]
    fn typed_helpers_build_the_documented_requests() {
        let tmp = tempfile::tempdir().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let log = seen.clone();
        let (sock, h) = serve(tmp.path(), move |r| {
            log.lock().unwrap().push(r.clone());
            match r["cmd"].as_str().unwrap() {
                "list_signals" => vec![ok(
                    &r,
                    json!({"signals":[{"name":"v(out)","file":"tt","unit":"V","complex":false}],"truncated":true}),
                )],
                "quit" => vec!["<close>".to_string()],
                _ => vec![ok(&r, json!({}))],
            }
        });
        let mut c = Client::connect_once(&sock).unwrap();
        c.open(Path::new("/a.raw"), Some("tran"), Some("tt"))
            .unwrap();
        c.import(Path::new("/b.raw"), None, Some("ff")).unwrap();
        let (sigs, cut) = c.list_signals(Some("tt"), Some("out")).unwrap();
        assert!(cut && sigs[0].name == "v(out)" && !sigs[0].complex);
        c.show(&["tt:v(out)".to_string()], PaneSel::New).unwrap();
        c.hide(&["v(out)".to_string()]).unwrap();
        c.set_analysis("tran").unwrap();
        c.set_cursors(Some(1e-9), None).unwrap();
        c.set_range(None).unwrap();
        c.set_range(Some((0.0, 1.0))).unwrap();
        c.remove_trace("t").unwrap();
        c.quit().unwrap();
        drop(c);
        h.join().unwrap();
        let seen = seen.lock().unwrap();
        let cmds: Vec<&str> = seen.iter().map(|r| r["cmd"].as_str().unwrap()).collect();
        assert_eq!(
            cmds,
            [
                "open",
                "import",
                "list_signals",
                "show",
                "hide",
                "set_analysis",
                "set_cursors",
                "set_range",
                "set_range",
                "remove_trace",
                "quit"
            ]
        );
        assert_eq!(seen[0]["analysis"], "tran");
        assert_eq!(seen[1].get("analysis"), None);
        assert_eq!(seen[3]["pane"], "new");
        assert_eq!(seen[7]["fit"], true);
        assert_eq!(seen[8]["x1"], 1.0);
    }

    #[test]
    fn show_is_sent_per_file_into_one_pane_and_waits_for_the_load() {
        use crate::wave::session::ViewFile;
        use crate::wave::show::{Show, ShowRequest, apply};
        let tmp = tempfile::tempdir().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let log = seen.clone();
        let mut empty_answers = 1;
        let (sock, h) = serve(tmp.path(), move |r| {
            log.lock().unwrap().push(r.clone());
            match r["cmd"].as_str().unwrap() {
                // The first file is not loaded at the first ask.
                "list_signals" if empty_answers > 0 => {
                    empty_answers -= 1;
                    vec![ok(&r, json!({"signals": []}))]
                }
                "list_signals" => vec![ok(
                    &r,
                    json!({"signals":[
                        {"name":"time","file":r["file"],"unit":"s","complex":false},
                        {"name":"v(vdd)","file":r["file"],"unit":"V","complex":false},
                        {"name":"v(out)","file":r["file"],"unit":"V","complex":false}]}),
                )],
                "show" => vec![ok(&r, json!({"pane": 3}))],
                _ => vec![ok(&r, json!({}))],
            }
        });
        let mut c = Client::connect_once(&sock).unwrap();
        let files: Vec<ViewFile> = ["a", "b"]
            .iter()
            .map(|t| ViewFile {
                tag: t.to_string(),
                raw: PathBuf::from(format!("/{t}.raw")),
            })
            .collect();
        let done = apply(
            &mut c,
            &files,
            &ShowRequest::default(),
            Duration::from_secs(5),
        );
        assert_eq!(done.shown, ["a:v(out)", "b:v(out)"]);
        assert!(done.notes.is_empty(), "{:?}", done.notes);
        // Off sends nothing; a file never loaded is a note.
        let off = apply(&mut c, &files, &ShowRequest::off(), Duration::from_secs(1));
        assert_eq!(off, Default::default());
        assert_eq!(ShowRequest::off().show, Show::Off);
        drop(c);
        h.join().unwrap();
        let seen = seen.lock().unwrap();
        let shows: Vec<&Value> = seen.iter().filter(|r| r["cmd"] == "show").collect();
        assert_eq!(shows.len(), 2);
        assert_eq!(shows[0]["pane"], "new");
        assert_eq!(shows[0]["signals"], json!(["a:v(out)"]));
        assert_eq!(shows[1]["pane"], 3);
        // list_signals asks for one file by tag; the first answer was empty and asked again.
        let asks = seen.iter().filter(|r| r["cmd"] == "list_signals").count();
        assert_eq!(asks, 3);
        assert_eq!(seen[0]["file"], "a");
    }

    #[test]
    fn socket_names_are_short_stable_and_per_run() {
        let a = socket_path(Path::new("/p/proj"), "cells/tb/testbench");
        assert_eq!(a, socket_path(Path::new("/p/proj"), "cells/tb/testbench"));
        assert_ne!(a, socket_path(Path::new("/p/proj"), "cells/tb/other"));
        assert_ne!(a, socket_path(Path::new("/p/other"), "cells/tb/testbench"));
        assert!(a.as_os_str().len() < 100, "{}", a.display());
    }
}
