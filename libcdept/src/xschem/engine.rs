//! Client side of the xschem helper: starts `cdept-xschem`, sends it requests and reads
//! the replies. One [`Engine`] is one helper process, i.e. one xschem.

use std::ffi::OsString;
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
use crate::{Format, Library, LibraryLayout, Project};

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
    /// Extra environment variables for the helper, e.g. `PDK_ROOT`.
    pub env: Vec<(OsString, OsString)>,
    /// Let xschem draw: it gets a window whose pixels arrive as frames (see
    /// [`super::Session`]). Off for netlisting and queries.
    pub display: bool,
    /// How long to wait for xschem to start.
    pub startup_timeout: Duration,
}

impl EngineOptions {
    /// Options for an engine set up for `project`: its libraries are on xschem's
    /// symbol search path and relative names resolve against the project folder.
    pub fn for_project(project: &Project) -> EngineOptions {
        EngineOptions {
            startup: Some(setup::startup_script(project)),
            cwd: Some(project.root.clone()),
            ..Default::default()
        }
    }
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            helper: None,
            startup: None,
            cwd: None,
            env: Vec::new(),
            display: false,
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

/// A schematic and what is placed in it, as part of a [`Hierarchy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchyCell {
    /// The schematic file.
    pub schematic: PathBuf,
    /// File name without extension, which is how xschem names the subcircuit.
    pub name: String,
    pub instances: Vec<HierarchyInstance>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchyInstance {
    pub instance: Instance,
    /// Index (in [`Hierarchy::cells`]) of the schematic this instance descends into.
    /// `None` for primitives and for subcircuits whose schematic is missing.
    pub child: Option<usize>,
}

/// The design hierarchy below a schematic. Every schematic appears once, however
/// often it is used; `cells[0]` is the top.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hierarchy {
    pub cells: Vec<HierarchyCell>,
}

/// The helper process and what belongs to it. Dropping it ends the process: it asks
/// first (so xschem removes its temporary folders), then insists, and removes the
/// private folder.
pub(crate) struct Process {
    child: Child,
    /// A second handle on the socket, to say goodbye with.
    socket: UnixStream,
    /// Private folder holding the socket, the start-up file and the frame files.
    runtime_dir: PathBuf,
}

impl Process {
    /// Explain a broken connection: most often the helper has exited. The second
    /// value is its exit code when it ended by itself.
    pub(crate) fn explain(&mut self, error: &std::io::Error) -> (eyre::Report, Option<i32>) {
        // The socket closes a moment before the process is gone: give it that moment.
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return (eyre!("the xschem helper stopped ({status})"), status.code());
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => return (eyre!("lost contact with the xschem helper: {error}"), None),
            }
        }
    }

    fn wait(&mut self) -> std::io::Result<()> {
        self.child.wait().map(|_| ())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // All of this is a no-op when the process already ended.
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
    pub(crate) xschem_version: String,
}

impl Connection {
    /// Start the helper and wait until xschem is ready.
    pub(crate) fn start(options: &EngineOptions) -> Result<Connection> {
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
        let result = Connection::start_in(&helper, options, &runtime_dir);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&runtime_dir);
        }
        result
    }

    fn start_in(helper: &Path, options: &EngineOptions, runtime_dir: &Path) -> Result<Connection> {
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
        if options.display {
            command.arg("--display").arg("--frame-dir").arg(runtime_dir);
        }
        if let Some(cwd) = &options.cwd {
            command.current_dir(cwd);
        }
        command.envs(options.env.iter().map(|(name, value)| (name, value)));
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
        // From here on `connection` cleans up after itself when dropped.
        let mut connection = Connection {
            reader: BufReader::new(stream.try_clone()?),
            process: Process {
                child,
                socket: stream.try_clone()?,
                runtime_dir: runtime_dir.to_path_buf(),
            },
            writer: stream,
            xschem_version: String::new(),
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
            Message::Ready { protocol, xschem } => {
                if protocol != PROTOCOL_VERSION {
                    bail!(
                        "{} speaks protocol {protocol}, this program speaks {PROTOCOL_VERSION}; \
                         rebuild the helper",
                        helper.display()
                    );
                }
                connection.xschem_version = xschem;
            }
            Message::Failed { error } => {
                bail!("xschem did not start: {error}");
            }
            other => {
                bail!("unexpected first message from the helper: {other:?}");
            }
        }
        connection.writer.set_read_timeout(None)?;
        Ok(connection)
    }
}

/// A running xschem helper, used one request at a time: send, wait for the reply.
/// For a schematic shown in the GUI see [`super::Session`].
pub struct Engine {
    connection: Connection,
    next_id: u64,
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
        Ok(Engine {
            connection: Connection::start(&options)?,
            next_id: 1,
        })
    }

    /// Start an engine set up for `project`: its libraries are on xschem's symbol
    /// search path and relative names resolve against the project folder.
    pub fn for_project(project: &Project) -> Result<Engine> {
        Engine::start(EngineOptions::for_project(project))
    }

    /// Version of the xschem built into the helper.
    pub fn xschem_version(&self) -> &str {
        &self.connection.xschem_version
    }

    /// Evaluate a Tcl script in xschem and return its result.
    pub fn tcl(&mut self, script: &str) -> Result<String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&Request::Tcl {
            id,
            script: script.to_string(),
        })?;
        loop {
            match self.read()? {
                Message::Reply { id: got, ok, value } if got == id => {
                    return if ok {
                        Ok(value)
                    } else {
                        Err(eyre!("{}", value.trim_end()))
                    };
                }
                // Frames and widget updates are for a `Session`; nobody shows them here.
                Message::Frame { .. } | Message::Damage { .. } | Message::Ui { .. } => {}
                other => {
                    bail!("unexpected message from the helper: {other:?}");
                }
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

    /// Walk the hierarchy below `top`: which schematics it uses, through which
    /// instances. Loads every schematic in turn, so the current design afterwards is
    /// the last one visited.
    pub fn hierarchy(&mut self, top: &Path) -> Result<Hierarchy> {
        // Name, symbol, type and (for subcircuits) the schematic of every instance.
        const LIST: &str = "set cdept_r {}; set cdept_i 0\n\
            foreach {cdept_n cdept_s cdept_t} [xschem instance_list] {\n\
              set cdept_f {}\n\
              if {$cdept_t eq {subcircuit}} { set cdept_f [xschem get_sch_from_sym $cdept_i] }\n\
              lappend cdept_r $cdept_n $cdept_s $cdept_t $cdept_f\n\
              incr cdept_i\n\
            }\n\
            set cdept_r";

        let key = |file: &Path| file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        let cell = |schematic: PathBuf| HierarchyCell {
            name: schematic
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            schematic,
            instances: Vec::new(),
        };
        let top = absolute(top)?;
        let mut hierarchy = Hierarchy {
            cells: vec![cell(top.clone())],
        };
        let mut index = std::collections::HashMap::from([(key(&top), 0usize)]);
        let mut next = 0;
        while next < hierarchy.cells.len() {
            let schematic = hierarchy.cells[next].schematic.clone();
            self.load(&schematic)?;
            let list = self.tcl(LIST)?;
            let items = tcl::parse_list(&list).map_err(|e| eyre!("instance list: {e}"))?;
            let mut instances = Vec::new();
            for [name, symbol, kind, file] in items.as_chunks::<4>().0 {
                let file = Path::new(file);
                let child = (!file.as_os_str().is_empty() && file.is_file()).then(|| {
                    *index.entry(key(file)).or_insert_with(|| {
                        hierarchy.cells.push(cell(file.to_path_buf()));
                        hierarchy.cells.len() - 1
                    })
                });
                instances.push(HierarchyInstance {
                    instance: Instance {
                        name: name.clone(),
                        symbol: symbol.clone(),
                        kind: kind.clone(),
                    },
                    child,
                });
            }
            hierarchy.cells[next].instances = instances;
            next += 1;
        }
        Ok(hierarchy)
    }

    /// Netlist the schematic of `cell` into a view of the same cell and return the
    /// file written: `<cell>/netlist/<cell>.<suffix>`, or `<cell>.<suffix>` in a flat
    /// library.
    pub fn netlist_view(
        &mut self,
        library: &Library,
        cell: &str,
        format: NetlistFormat,
    ) -> Result<PathBuf> {
        library.writable()?;
        let schematic = cell_schematic(library, cell)?;
        let file = format!("{cell}.{}", format.suffix());
        let output = match library.layout {
            LibraryLayout::Cells => library.path.join(cell).join("netlist").join(file),
            LibraryLayout::Flat => library.path.join(file),
        };
        self.load(&schematic)?;
        self.netlist(format, &output)?;
        Ok(output)
    }

    /// Make the symbol of `cell` from the pins of its schematic, as xschem's "Make
    /// symbol from schematic" does, and return the file written:
    /// `<cell>/symbol/<cell>.sym` with a `schematic=` attribute in a library with view
    /// folders, `<cell>.sym` next to the schematic in a flat one.
    ///
    /// An existing symbol is replaced only with `overwrite`: its drawing is lost.
    pub fn make_symbol(
        &mut self,
        library: &Library,
        cell: &str,
        overwrite: bool,
    ) -> Result<PathBuf> {
        library.writable()?;
        let schematic = cell_schematic(library, cell)?;
        let schematic = tcl::quote(&schematic.to_string_lossy());
        let symbol = PathBuf::from(self.tcl(&format!("cdept::symbol_file {schematic}"))?);
        if symbol.exists() && !overwrite {
            return Err(eyre!(
                "{} exists already; it is only replaced on request (--force)",
                symbol.display()
            ));
        }
        // `yes`: do not ask before replacing a symbol.
        self.tcl(&format!("make_symbol {schematic} yes"))?;
        if !symbol.is_file() {
            return Err(eyre!("xschem did not write {}", symbol.display()));
        }
        Ok(symbol)
    }

    /// Make the schematic of `cell` from the pins of its symbol, as xschem's "Make
    /// schematic from symbol" does (an input, output or inout pin for each pin), and
    /// return the file written: the one the symbol's `schematic=` attribute names
    /// (normally `<cell>/schematic/<cell>.sch`) in a library with view folders,
    /// `<cell>.sch` next to the symbol in a flat one.
    ///
    /// An existing schematic is replaced only with `overwrite`: its contents are lost.
    pub fn make_schematic(
        &mut self,
        library: &Library,
        cell: &str,
        overwrite: bool,
    ) -> Result<PathBuf> {
        library.writable()?;
        let symbol = cell_symbol(library, cell)?;
        let symbol = tcl::quote(&symbol.to_string_lossy());
        let schematic = self.tcl(&format!("cdept::schematic_file {symbol}"))?;
        if schematic.is_empty() {
            return Err(eyre!(
                "cannot tell where the schematic of cell '{cell}' goes: see the \
                 schematic= attribute of its symbol"
            ));
        }
        let schematic = PathBuf::from(schematic);
        if schematic.exists() && !overwrite {
            return Err(eyre!(
                "{} exists already; it is only replaced on request (--force)",
                schematic.display()
            ));
        }
        // `yes`: do not ask before replacing a schematic.
        self.tcl(&format!("cdept::make_schematic {symbol} yes"))?;
        if !schematic.is_file() {
            return Err(eyre!("xschem did not write {}", schematic.display()));
        }
        Ok(schematic)
    }

    /// Ask the helper to exit and wait for it.
    pub fn shutdown(mut self) -> Result<()> {
        let id = self.next_id;
        self.send(&Request::Shutdown { id })?;
        // The reply may or may not arrive before the process ends; both are fine.
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

/// A file path as one Tcl word.
pub(crate) fn word(path: &Path) -> String {
    tcl::quote(&path.to_string_lossy())
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

/// The xschem schematic of `cell`: the view named `schematic`, else the first one.
fn cell_schematic(library: &Library, cell: &str) -> Result<PathBuf> {
    cell_view(library, cell, Format::XschemSchematic, "schematic")
}

/// The xschem symbol of `cell`: the view named `symbol`, else the first one.
fn cell_symbol(library: &Library, cell: &str) -> Result<PathBuf> {
    cell_view(library, cell, Format::XschemSymbol, "symbol")
}

fn cell_view(library: &Library, cell: &str, format: Format, name: &str) -> Result<PathBuf> {
    library
        .cell(cell)
        .and_then(|c| {
            let mut views = c.entries.iter().filter(|e| e.format == Some(format));
            views
                .clone()
                .find(|e| e.name == name)
                .or_else(|| views.next())
        })
        .and_then(|entry| entry.main_file.clone())
        .ok_or_else(|| {
            eyre!(
                "cell '{cell}' of library '{}' has no xschem {name}",
                library.name
            )
        })
}
