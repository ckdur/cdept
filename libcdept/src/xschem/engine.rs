//! Client side of the xschem helper: starts `cdept-xschem`, sends it requests and reads
//! the replies. One [`Engine`] is one helper process, i.e. one xschem.

use std::io::BufReader;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use eyre::{Result, WrapErr, bail, eyre};

use super::protocol::{self, Message, PROTOCOL_VERSION, Request};
use super::{setup, tcl};
use crate::Project;

/// File name of the helper program.
pub const HELPER_NAME: &str = "cdept-xschem";
/// Environment variable naming the helper program explicitly.
pub const HELPER_ENV: &str = "CDEPT_XSCHEM";

#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// The helper program. Found automatically when `None` (see [`Engine::find_helper`]).
    pub helper: Option<PathBuf>,
    /// Start-up script (Tcl), see [`super::setup::startup_script`].
    pub startup: Option<String>,
    /// Working folder of the helper. xschem resolves relative names against it.
    pub cwd: Option<PathBuf>,
    /// How long to wait for xschem to start.
    pub startup_timeout: Duration,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            helper: None,
            startup: None,
            cwd: None,
            startup_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetlistFormat {
    Spice,
    Spectre,
    Verilog,
    Vhdl,
    Tedax,
}

impl NetlistFormat {
    /// xschem's name for the format (`xschem set netlist_type …`).
    pub fn name(self) -> &'static str {
        match self {
            NetlistFormat::Spice => "spice",
            NetlistFormat::Spectre => "spectre",
            NetlistFormat::Verilog => "verilog",
            NetlistFormat::Vhdl => "vhdl",
            NetlistFormat::Tedax => "tedax",
        }
    }

    /// Usual file suffix, without the dot.
    pub fn suffix(self) -> &'static str {
        match self {
            NetlistFormat::Spice => "spice",
            NetlistFormat::Spectre => "scs",
            NetlistFormat::Verilog => "v",
            NetlistFormat::Vhdl => "vhdl",
            NetlistFormat::Tedax => "tdx",
        }
    }
}

/// One instance placed in a schematic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// Instance name, e.g. `M1`.
    pub name: String,
    /// The symbol as the schematic refers to it, e.g. `sg13g2_pr/sg13_lv_nmos.sym`.
    pub symbol: String,
    /// The symbol's `type` attribute, e.g. `subcircuit`, `nmos`, `label`.
    pub kind: String,
}

/// A running xschem helper.
pub struct Engine {
    child: Child,
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
    xschem_version: String,
    /// Private folder holding the socket and the start-up file. Removed on drop.
    runtime_dir: PathBuf,
}

impl Engine {
    /// Where the helper program is: `$CDEPT_XSCHEM`, next to the running program, or
    /// on `PATH`.
    pub fn find_helper() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os(HELPER_ENV).filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(path));
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            let sibling = dir.join(HELPER_NAME);
            if sibling.is_file() {
                return Some(sibling);
            }
        }
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join(HELPER_NAME))
            .find(|candidate| candidate.is_file())
    }

    /// Start the helper and wait until xschem is ready.
    pub fn start(options: EngineOptions) -> Result<Engine> {
        let helper = options
            .helper
            .clone()
            .or_else(Engine::find_helper)
            .ok_or_else(|| {
                eyre!(
                    "the xschem helper ({HELPER_NAME}) was not found. Build it with \
                 `cargo build -p {HELPER_NAME}`, or set ${HELPER_ENV} to its path"
                )
            })?;

        let runtime_dir = create_runtime_dir()?;
        let result = Engine::start_in(&helper, &options, &runtime_dir);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&runtime_dir);
        }
        result
    }

    fn start_in(helper: &Path, options: &EngineOptions, runtime_dir: &Path) -> Result<Engine> {
        let socket = runtime_dir.join("socket");
        let listener = UnixListener::bind(&socket)
            .wrap_err_with(|| format!("cannot create the socket {}", socket.display()))?;

        let mut command = Command::new(helper);
        command.arg("--connect").arg(&socket);
        if let Some(script) = &options.startup {
            let rcfile = runtime_dir.join("xschemrc");
            std::fs::write(&rcfile, script)
                .wrap_err_with(|| format!("cannot write {}", rcfile.display()))?;
            command.arg("--rcfile").arg(&rcfile);
        }
        if let Some(cwd) = &options.cwd {
            command.current_dir(cwd);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .wrap_err_with(|| format!("cannot start {}", helper.display()))?;
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
        let mut engine = Engine {
            child,
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            next_id: 1,
            xschem_version: String::new(),
            runtime_dir: runtime_dir.to_path_buf(),
        };

        // From here on `engine` cleans up after itself when dropped.
        match engine.read()? {
            Message::Ready { protocol, xschem } => {
                if protocol != PROTOCOL_VERSION {
                    bail!(
                        "{} speaks protocol {protocol}, this program speaks {PROTOCOL_VERSION}; \
                         rebuild the helper",
                        helper.display()
                    );
                }
                engine.xschem_version = xschem;
            }
            Message::Failed { error } => {
                bail!("xschem did not start: {error}");
            }
            other => {
                bail!("unexpected first message from the helper: {other:?}");
            }
        }
        engine.writer.set_read_timeout(None)?;
        Ok(engine)
    }

    /// Start an engine set up for `project`: its libraries are on xschem's symbol
    /// search path and relative names resolve against the project folder.
    pub fn for_project(project: &Project) -> Result<Engine> {
        Engine::start(EngineOptions {
            startup: Some(setup::startup_script(project)),
            cwd: Some(project.root.clone()),
            ..Default::default()
        })
    }

    /// Version of the xschem built into the helper.
    pub fn xschem_version(&self) -> &str {
        &self.xschem_version
    }

    /// Evaluate a Tcl script in xschem and return its result.
    pub fn tcl(&mut self, script: &str) -> Result<String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&Request::Tcl {
            id,
            script: script.to_string(),
        })?;
        match self.read()? {
            Message::Reply { id: got, ok, value } if got == id => {
                if ok {
                    Ok(value)
                } else {
                    Err(eyre!("{}", value.trim_end()))
                }
            }
            other => {
                bail!("unexpected message from the helper: {other:?}");
            }
        }
    }

    /// Load a schematic or symbol file as the current design.
    pub fn load(&mut self, file: &Path) -> Result<()> {
        // xschem silently starts an empty design when the file is missing.
        if !file.is_file() {
            bail!("no such file: {}", file.display());
        }
        let file = absolute(file)?;
        self.tcl(&format!("xschem load {}", word(&file)))?;
        let loaded = self.tcl("xschem get schname")?;
        // xschem tidies the name (`a/../b` becomes `b`), so compare the files.
        let same = match (Path::new(&loaded).canonicalize(), file.canonicalize()) {
            (Ok(loaded), Ok(file)) => loaded == file,
            _ => Path::new(&loaded) == file,
        };
        if !same {
            bail!("xschem loaded {loaded} instead of {}", file.display());
        }
        Ok(())
    }

    /// Write the netlist of the current design, with its hierarchy, to `output`.
    pub fn netlist(&mut self, format: NetlistFormat, output: &Path) -> Result<()> {
        let output = absolute(output)?;
        if let Some(dir) = output.parent() {
            std::fs::create_dir_all(dir)
                .wrap_err_with(|| format!("cannot create {}", dir.display()))?;
        }
        // A stale file must not pass for a new netlist.
        if output.exists() {
            std::fs::remove_file(&output)
                .wrap_err_with(|| format!("cannot replace {}", output.display()))?;
        }
        self.tcl(&format!("xschem set netlist_type {}", format.name()))?;
        let status = self.tcl(&format!("xschem netlist -noalert {}", word(&output)))?;
        if !output.is_file() {
            bail!("xschem wrote no netlist (status {status})");
        }
        if status.trim() != "0" {
            bail!(
                "xschem reported netlisting errors (status {status}); see {} and \
                 xschem's messages in the log",
                output.display()
            );
        }
        Ok(())
    }

    /// Draw the current design, zoomed to fit, into an SVG file.
    pub fn export_svg(&mut self, output: &Path) -> Result<()> {
        let output = absolute(output)?;
        if output.exists() {
            std::fs::remove_file(&output)
                .wrap_err_with(|| format!("cannot replace {}", output.display()))?;
        }
        self.tcl(&format!("xschem print svg {} 0 0", word(&output)))?;
        if !output.is_file() {
            bail!("xschem wrote no SVG file");
        }
        Ok(())
    }

    /// The instances placed in the current design (one level, no descent).
    pub fn instances(&mut self) -> Result<Vec<Instance>> {
        let list = self.tcl("xschem instance_list")?;
        let items = tcl::parse_list(&list).map_err(|e| eyre!("instance list: {e}"))?;
        if items.len() % 3 != 0 {
            bail!(
                "instance list has {} items, not a multiple of 3",
                items.len()
            );
        }
        let mut items = items.into_iter();
        let mut instances = Vec::new();
        while let (Some(name), Some(symbol), Some(kind)) =
            (items.next(), items.next(), items.next())
        {
            instances.push(Instance { name, symbol, kind });
        }
        Ok(instances)
    }

    /// Ask the helper to exit and wait for it.
    pub fn shutdown(mut self) -> Result<()> {
        let id = self.next_id;
        self.send(&Request::Shutdown { id })?;
        // The reply may or may not arrive before the process ends; both are fine.
        let _ = self.read();
        self.child.wait()?;
        Ok(())
    }

    fn send(&mut self, request: &Request) -> Result<()> {
        protocol::write_message(&mut self.writer, request)
            .map_err(|error| self.connection_error(error))
    }

    fn read(&mut self) -> Result<Message> {
        match protocol::read_message(&mut self.reader) {
            Ok(Some(message)) => Ok(message),
            Ok(None) => Err(self.connection_error(std::io::ErrorKind::UnexpectedEof.into())),
            Err(error) => Err(self.connection_error(error)),
        }
    }

    /// Explain a broken connection: most often the helper has exited.
    fn connection_error(&mut self, error: std::io::Error) -> eyre::Report {
        // The socket closes a moment before the process is gone: give it that moment.
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return eyre!("the xschem helper stopped ({status})"),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => return eyre!("lost contact with the xschem helper: {error}"),
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Ask first, so xschem removes its temporary folders; then insist. All of this
        // is a no-op when the process already ended (after `shutdown`).
        let id = self.next_id;
        if protocol::write_message(&mut self.writer, &Request::Shutdown { id }).is_ok() {
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

/// A file path as one Tcl word.
fn word(path: &Path) -> String {
    tcl::quote(&path.to_string_lossy())
}

fn absolute(path: &Path) -> Result<PathBuf> {
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
        "cdept-xschem-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = base.join(name);
    // Fails if the name exists, so nobody else's folder is ever reused.
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
            Err(error) => return Err(error).wrap_err("waiting for the xschem helper"),
        }
        if let Some(status) = child.try_wait()? {
            bail!("the xschem helper stopped before connecting ({status})");
        }
        if start.elapsed() > timeout {
            bail!("the xschem helper did not connect within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Pass what xschem prints on to the log (target `xschem`, level debug), line by line.
fn forward_output(child: &mut Child) {
    fn forward(stream: impl std::io::Read + Send + 'static) {
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if !line.trim().is_empty() {
                    tracing::debug!(target: "xschem", "{line}");
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
