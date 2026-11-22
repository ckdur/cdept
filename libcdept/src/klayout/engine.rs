//! Client side of the KLayout helper: starts `klayout` with the helper script, sends it
//! calls and reads the replies. One [`Engine`] is one helper process.

use std::ffi::OsString;
use std::io::BufReader;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use eyre::{Result, WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::protocol::{self, Message, PROTOCOL_VERSION, Request, method};
use super::setup;
use crate::layout::RefTable;
use crate::layout::refs::SaveReport;
use crate::{Format, Library, Project};

#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// The `klayout` binary. Found automatically when `None` ([`setup::find_klayout`]).
    pub klayout: Option<PathBuf>,
    /// The helper folder. Found automatically when `None` ([`setup::find_helper_dir`]).
    pub helper_dir: Option<PathBuf>,
    /// Working folder of the helper.
    pub cwd: Option<PathBuf>,
    /// Extra environment, e.g. `KLAYOUT_PATH`, `PDK_ROOT` ([`setup::project_env`]).
    pub env: Vec<(OsString, OsString)>,
    /// Let the helper draw: it sends frames (see [`super::Session`]). Off for exports
    /// and previews.
    pub display: bool,
    /// How long to wait for KLayout to start.
    pub startup_timeout: Duration,
}

impl EngineOptions {
    /// Options for an engine set up for `project`: its PDK and `[klayout]` settings,
    /// relative names resolved against the project folder.
    pub fn for_project(project: &Project) -> EngineOptions {
        EngineOptions {
            cwd: Some(project.root.clone()),
            env: setup::project_env(project),
            ..Default::default()
        }
    }

    /// `technology` and `layer_properties` for `open`, from the environment.
    pub(crate) fn open_params(&self, refs: &RefTable, editable: bool) -> Value {
        let mut params = setup::open_extras(&self.env);
        params.insert(
            "refs".into(),
            serde_json::to_value(refs).unwrap_or(Value::Null),
        );
        params.insert("editable".into(), editable.into());
        Value::Object(params)
    }
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            klayout: None,
            helper_dir: None,
            cwd: None,
            env: Vec::new(),
            display: false,
            startup_timeout: Duration::from_secs(60),
        }
    }
}

/// What `open` reports.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OpenInfo {
    #[serde(default)]
    pub top: String,
    #[serde(default)]
    pub dbu: f64,
    /// Ghost cells that could not be resolved (drawn as red X).
    #[serde(default)]
    pub missing: Vec<String>,
    /// Cells loaded from other views.
    #[serde(default)]
    pub external: Vec<String>,
}

/// The process behind a connection. Dropping it ends the process and removes its
/// private folder.
pub(crate) struct Process {
    child: Child,
    /// A second handle on the socket, to say goodbye with.
    socket: UnixStream,
    /// Private folder holding the socket and the frame files.
    runtime_dir: PathBuf,
}

impl Process {
    /// Explain a broken connection; the second value is the exit code when the helper
    /// ended by itself.
    pub(crate) fn explain(&mut self, error: &std::io::Error) -> (eyre::Report, Option<i32>) {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return (
                        eyre!("the KLayout helper stopped ({status})"),
                        status.code(),
                    );
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => return (eyre!("lost contact with the KLayout helper: {error}"), None),
            }
        }
    }

    fn wait(&mut self) -> std::io::Result<()> {
        self.child.wait().map(|_| ())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let goodbye = Request::Shutdown { id: u64::MAX };
        if protocol::write_message(&mut self.socket, &goodbye).is_ok() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline && matches!(self.child.try_wait(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.runtime_dir);
    }
}

/// A started helper: the socket in both directions and the process behind it.
pub(crate) struct Connection {
    pub(crate) reader: BufReader<UnixStream>,
    pub(crate) writer: UnixStream,
    pub(crate) process: Process,
    pub(crate) klayout_version: String,
}

impl Connection {
    /// Start the helper and wait until KLayout is ready.
    pub(crate) fn start(options: &EngineOptions) -> Result<Connection> {
        let klayout = options
            .klayout
            .clone()
            .or_else(setup::find_klayout)
            .ok_or_else(|| {
                eyre!(
                    "KLayout was not found. Build it with cdept-klayout/build.sh, install it, \
                     or set ${} to the klayout binary",
                    setup::KLAYOUT_ENV
                )
            })?;
        let helper_dir = options
            .helper_dir
            .clone()
            .or_else(setup::find_helper_dir)
            .ok_or_else(|| {
                eyre!(
                    "the KLayout helper script ({}) was not found; set ${} to its folder",
                    setup::MAIN_SCRIPT,
                    setup::HELPER_ENV
                )
            })?;
        let runtime_dir = create_runtime_dir()?;
        let result = Connection::start_in(&klayout, &helper_dir, options, &runtime_dir);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&runtime_dir);
        }
        result
    }

    fn start_in(
        klayout: &Path,
        helper_dir: &Path,
        options: &EngineOptions,
        runtime_dir: &Path,
    ) -> Result<Connection> {
        let socket = runtime_dir.join("socket");
        let listener = UnixListener::bind(&socket)
            .wrap_err_with(|| format!("cannot create the socket {}", socket.display()))?;

        let mut command = Command::new(klayout);
        command.args(setup::helper_args(
            helper_dir,
            &socket,
            runtime_dir,
            options.display,
        ));
        if let Some(cwd) = &options.cwd {
            command.current_dir(cwd);
        }
        command.envs(setup::helper_env(klayout));
        command.envs(options.env.iter().map(|(name, value)| (name, value)));
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .wrap_err_with(|| format!("cannot start {}", klayout.display()))?;
        forward_output(&mut child);

        let stream = match accept(&listener, &mut child, options.startup_timeout) {
            Ok(stream) => stream,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        stream.set_read_timeout(Some(options.startup_timeout))?;
        let mut connection = Connection {
            reader: BufReader::new(stream.try_clone()?),
            process: Process {
                child,
                socket: stream.try_clone()?,
                runtime_dir: runtime_dir.to_path_buf(),
            },
            writer: stream,
            klayout_version: String::new(),
        };

        let first = match protocol::read_message(&mut connection.reader) {
            Ok(Some(message)) => message,
            Ok(None) => {
                let eof = std::io::ErrorKind::UnexpectedEof.into();
                return Err(connection.process.explain(&eof).0);
            }
            Err(error) => return Err(connection.process.explain(&error).0),
        };
        match first {
            Message::Ready { protocol, klayout } => {
                if protocol != PROTOCOL_VERSION {
                    bail!(
                        "the KLayout helper in {} speaks protocol {protocol}, this program \
                         speaks {PROTOCOL_VERSION}; install matching versions",
                        helper_dir.display()
                    );
                }
                connection.klayout_version = klayout;
            }
            Message::Failed { error } => {
                bail!("KLayout did not start: {error}");
            }
            other => {
                bail!("unexpected first message from the KLayout helper: {other:?}");
            }
        }
        connection.writer.set_read_timeout(None)?;
        Ok(connection)
    }
}

/// A running KLayout helper used one call at a time: send, wait for the reply. For a
/// layout shown in the GUI see [`super::Session`].
pub struct Engine {
    connection: Connection,
    options: EngineOptions,
    next_id: u64,
}

impl Engine {
    /// Start the helper and wait until KLayout is ready.
    pub fn start(options: EngineOptions) -> Result<Engine> {
        Ok(Engine {
            connection: Connection::start(&options)?,
            options,
            next_id: 1,
        })
    }

    /// Start an engine set up for `project`.
    pub fn for_project(project: &Project) -> Result<Engine> {
        Engine::start(EngineOptions::for_project(project))
    }

    /// `KLayout 0.30.12`.
    pub fn klayout_version(&self) -> &str {
        &self.connection.klayout_version
    }

    /// Call a helper method (docs/KLAYOUT.md §4) and wait for its answer. Events and
    /// frames arriving meanwhile are dropped.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&Request::Call {
            id,
            method: method.to_string(),
            params,
        })?;
        loop {
            match self.read()? {
                Message::Reply {
                    id: got,
                    ok,
                    value,
                    error,
                } if got == id => {
                    return if ok {
                        Ok(value)
                    } else {
                        Err(eyre!("{method}: {}", error.trim_end()))
                    };
                }
                Message::Event {
                    event: protocol::Event::Notice { level, text },
                } => tracing::debug!(target: "klayout", "{level:?}: {text}"),
                _ => {}
            }
        }
    }

    /// Open the root of `refs` with everything it references.
    pub fn open(&mut self, refs: &RefTable, editable: bool) -> Result<OpenInfo> {
        let params = self.options.open_params(refs, editable);
        let value = self.call(method::OPEN, params)?;
        Ok(serde_json::from_value(value).unwrap_or_default())
    }

    /// Save the open layout (to `file`, default: where it was opened from): its own
    /// cells, references as ghost cells. The report says what the sidecar should hold
    /// ([`crate::layout::refs::update_after_save`]).
    pub fn save(&mut self, file: Option<&Path>) -> Result<SaveReport> {
        let params = match file {
            Some(file) => json!({ "file": absolute(file)? }),
            None => json!({}),
        };
        let value = self.call(method::SAVE, params)?;
        serde_json::from_value(value).wrap_err("the helper's save report is not understood")
    }

    /// Draw the root of `refs` into a PNG file of `width` × `height` pixels.
    pub fn render_png(
        &mut self,
        refs: &RefTable,
        width: u32,
        height: u32,
        out: &Path,
    ) -> Result<()> {
        self.render_cell_png(refs, None, width, height, out)
    }

    /// [`Engine::render_png`] of one cell of the root file: `cell` names a cell of a
    /// file that holds several (a standard-cell GDS, see [`crate::Entry::member`]);
    /// `None` draws the file's top cell.
    pub fn render_cell_png(
        &mut self,
        refs: &RefTable,
        cell: Option<&str>,
        width: u32,
        height: u32,
        out: &Path,
    ) -> Result<()> {
        let mut params = self.options.open_params(refs, false);
        if let (Value::Object(map), Some(cell)) = (&mut params, cell) {
            map.insert("cell".into(), cell.into());
        }
        if let Value::Object(map) = &mut params {
            map.insert("width".into(), width.into());
            map.insert("height".into(), height.into());
            map.insert("out".into(), json!(absolute(out)?));
        }
        self.call(method::RENDER, params)?;
        Ok(())
    }

    /// Write the root of `refs` with every reference filled in (GDS or OASIS, by the
    /// suffix of `out`).
    pub fn export(&mut self, refs: &RefTable, out: &Path) -> Result<()> {
        let mut params = self.options.open_params(refs, false);
        if let Value::Object(map) = &mut params {
            map.insert("out".into(), json!(absolute(out)?));
        }
        self.call(method::EXPORT, params)?;
        Ok(())
    }

    /// `{klayout, technology, file, top, cells, dbu}`.
    pub fn info(&mut self) -> Result<Value> {
        self.call(method::INFO, json!({}))
    }

    /// Run Python in the helper; the `repr` of the last expression.
    pub fn python(&mut self, code: &str) -> Result<String> {
        let value = self.call(method::PYTHON, json!({ "code": code }))?;
        Ok(match value {
            Value::String(text) => text,
            Value::Null => String::new(),
            other => other.to_string(),
        })
    }

    /// Ask the helper to exit and wait for it.
    pub fn shutdown(mut self) -> Result<()> {
        let id = self.next_id;
        self.send(&Request::Shutdown { id })?;
        let _ = self.read();
        self.connection.process.wait()?;
        Ok(())
    }

    fn send(&mut self, request: &Request) -> Result<()> {
        protocol::write_message(&mut self.connection.writer, request)
            .map_err(|error| self.connection.process.explain(&error).0)
    }

    fn read(&mut self) -> Result<Message> {
        match protocol::read_message(&mut self.connection.reader) {
            Ok(Some(message)) => Ok(message),
            Ok(None) => {
                let eof = std::io::ErrorKind::UnexpectedEof.into();
                Err(self.connection.process.explain(&eof).0)
            }
            Err(error) => Err(self.connection.process.explain(&error).0),
        }
    }
}

/// The layout file of `cell`: the view named `layout`, else the first GDS or OASIS
/// view.
pub fn cell_layout(library: &Library, cell: &str) -> Result<PathBuf> {
    library
        .cell(cell)
        .and_then(|c| {
            let mut views = c
                .entries
                .iter()
                .filter(|e| matches!(e.format, Some(Format::Gds | Format::Oasis)));
            views
                .clone()
                .find(|e| e.name == "layout")
                .or_else(|| views.next())
        })
        .and_then(|entry| entry.main_file.clone())
        .ok_or_else(|| {
            eyre!(
                "cell '{cell}' of library '{}' has no GDS or OASIS layout",
                library.name
            )
        })
}

pub(crate) fn absolute(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).wrap_err_with(|| format!("bad path {}", path.display()))
}

/// A new folder only this user can enter: under `$XDG_RUNTIME_DIR` when there is one,
/// else under the temporary folder.
fn create_runtime_dir() -> Result<PathBuf> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let name = format!(
        "cdept-klayout-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = base.join(name);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    Ok(dir)
}

/// Wait for the helper to connect, giving up if it exits or takes too long.
fn accept(listener: &UnixListener, child: &mut Child, timeout: Duration) -> Result<UnixStream> {
    listener.set_nonblocking(true)?;
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error).wrap_err("waiting for the KLayout helper"),
        }
        if let Some(status) = child.try_wait()? {
            bail!(
                "the KLayout helper stopped before connecting ({status}); run with -v for its output"
            );
        }
        if start.elapsed() > timeout {
            bail!("the KLayout helper did not connect within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Pass what KLayout prints on to the log (target `klayout`, level debug).
fn forward_output(child: &mut Child) {
    fn forward(stream: impl std::io::Read + Send + 'static) {
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if !line.trim().is_empty() {
                    tracing::debug!(target: "klayout", "{line}");
                }
            }
        });
    }
    if let Some(stdout) = child.stdout.take() {
        forward(stdout);
    }
    if let Some(stderr) = child.stderr.take() {
        forward(stderr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_carries_refs_and_technology() {
        let options = EngineOptions {
            env: vec![(setup::TECH_ENV.into(), "sg13g2".into())],
            ..Default::default()
        };
        let refs = RefTable {
            root: "/a.gds".into(),
            ..Default::default()
        };
        let params = options.open_params(&refs, true);
        assert_eq!(params["editable"], true);
        assert_eq!(params["technology"], "sg13g2");
        assert_eq!(params["refs"]["root"], "/a.gds");
    }

    #[test]
    fn a_missing_klayout_is_explained() {
        let options = EngineOptions {
            klayout: Some("/nonexistent/klayout".into()),
            helper_dir: Some("/nonexistent".into()),
            ..Default::default()
        };
        let error = Engine::start(options).err().unwrap();
        assert!(format!("{error:#}").contains("cannot start"), "{error:#}");
    }
}
