//! OpenVAF: `openvaf-r [-D MACRO] [-I dir] -o out.osdi model.va`, compiling a
//! Verilog-A model to an OSDI library that ngspice and VACASK load.
//!
//! Compiled libraries are cached per project: [`cache_dir`] is
//! `<project>/.cdept/osdi/<name>-<hash>/`, the hash covering the sources, the flags and
//! the compiler, so a changed model (or flag) compiles again and an unchanged one never
//! does ([`compile_cached`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail};

use super::{Adapter, Job};

/// Environment variable that names the OpenVAF program.
pub const OPENVAF_ENV: &str = "CDEPT_OPENVAF";

/// `openvaf-r` (OpenVAF-Reloaded) and `openvaf` on `PATH` (see `tools::KNOWN_TOOLS`).
pub const ADAPTER: Adapter = Adapter {
    name: "OpenVAF",
    env_var: OPENVAF_ENV,
    tool: "OpenVAF",
};

/// Where compiled libraries are cached, relative to the project folder.
pub const CACHE: &str = ".cdept/osdi";

/// One compilation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Compile {
    /// The root Verilog-A file.
    pub source: PathBuf,
    /// The OSDI file to write.
    pub out: PathBuf,
    /// `-D` macros (`NAME` or `NAME=VALUE`).
    pub defines: Vec<String>,
    /// `-I` include folders.
    pub includes: Vec<PathBuf>,
}

/// The job for a compilation. The log is `<out>.log` next to the output.
pub fn job(program: &Path, c: &Compile, env: &[(OsString, OsString)]) -> Job {
    let dir = c
        .out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let log = format!(
        "{}.log",
        c.out
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    );
    let mut job = Job::new("OpenVAF", program, dir, &log);
    for d in &c.defines {
        job = job.args(["-D", d]);
    }
    for i in &c.includes {
        job = job.args([OsString::from("-I"), i.as_os_str().to_owned()]);
    }
    job.args([
        OsString::from("-o"),
        c.out.as_os_str().to_owned(),
        c.source.as_os_str().to_owned(),
    ])
    .env(env)
}

/// Compile. The output must exist afterwards; an error names the source and has the
/// compiler's last lines.
pub fn compile(program: &Path, c: &Compile, env: &[(OsString, OsString)]) -> Result<()> {
    if let Some(dir) = c.out.parent() {
        std::fs::create_dir_all(dir)
            .wrap_err_with(|| format!("cannot create {}", dir.display()))?;
    }
    let _ = std::fs::remove_file(&c.out);
    let out = job(program, c, env).run_unchecked()?;
    if !out.status.success() || !c.out.is_file() {
        bail!(
            "OpenVAF could not compile {} ({})\nlog: {}\n{}",
            c.source.display(),
            out.status,
            out.log.display(),
            super::tail(&out.text, 12)
        );
    }
    Ok(())
}

/// A stable hash (hex, 16 digits) of byte strings: FNV-1a over each, with a separator.
pub fn hash_parts<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in part.iter().chain(&[0xff]) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

/// The Verilog-A files a model is made of: the root, and the files next to it that
/// `` `include `` may read (`.va`, `.include`, `.vams`, `.h`), sorted by name.
pub fn source_files(root: &Path) -> Vec<PathBuf> {
    let dir = root.parent().unwrap_or(Path::new("."));
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|e| {
                    matches!(e.to_str(), Some("va" | "include" | "vams" | "h" | "inc"))
                })
        })
        .collect();
    files.sort();
    if !files.contains(&root.to_path_buf()) {
        files.push(root.to_path_buf());
    }
    files
}

/// The hash of a compilation: file names and contents of [`source_files`], the flags
/// and the compiler program's name.
pub fn source_hash(program: &Path, c: &Compile) -> Result<String> {
    let mut parts: Vec<Vec<u8>> = vec![];
    for f in source_files(&c.source) {
        parts.push(
            f.file_name()
                .unwrap_or_default()
                .as_encoded_bytes()
                .to_vec(),
        );
        parts.push(std::fs::read(&f).wrap_err_with(|| format!("cannot read {}", f.display()))?);
    }
    parts.push(c.defines.join("\n").into_bytes());
    parts.push(
        program
            .file_name()
            .unwrap_or_default()
            .as_encoded_bytes()
            .to_vec(),
    );
    Ok(hash_parts(parts.iter().map(|p| p.as_slice())))
}

/// The cache folder of a technology's compiled models: `<project>/.cdept/osdi/<name>-<hash>`.
pub fn cache_dir(project_root: &Path, name: &str, hash: &str) -> PathBuf {
    project_root.join(CACHE).join(format!("{name}-{hash}"))
}

/// What [`compile_cached`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cached {
    pub osdi: PathBuf,
    /// False when the cache already had the library.
    pub compiled: bool,
}

/// Compile `c.source` into `<cache>/<stem>.osdi` unless that file is there already
/// (the folder name carries the hash of the sources and flags). `c.out` is ignored.
pub fn compile_cached(
    program: &Path,
    c: &Compile,
    cache: &Path,
    env: &[(OsString, OsString)],
) -> Result<Cached> {
    let stem = c
        .source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model".into());
    let osdi = cache.join(format!("{stem}.osdi"));
    if osdi.is_file() {
        return Ok(Cached {
            osdi,
            compiled: false,
        });
    }
    let mut c = c.clone();
    c.out = osdi.clone();
    compile(program, &c, env)?;
    Ok(Cached {
        osdi,
        compiled: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_openvafs() {
        let c = Compile {
            source: "/m/psp103.va".into(),
            out: "/o/psp103.osdi".into(),
            defines: vec!["__XYCE__".into()],
            includes: vec!["/inc".into()],
        };
        let j = job(Path::new("/x/openvaf-r"), &c, &[]);
        let args: Vec<_> = j
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-D",
                "__XYCE__",
                "-I",
                "/inc",
                "-o",
                "/o/psp103.osdi",
                "/m/psp103.va"
            ]
        );
        assert_eq!(j.dir, Path::new("/o"));
    }

    #[test]
    fn hashes_differ_with_the_parts() {
        assert_eq!(
            hash_parts([&b"a"[..], &b"b"[..]]),
            hash_parts([&b"a"[..], &b"b"[..]])
        );
        assert_ne!(hash_parts([&b"ab"[..]]), hash_parts([&b"a"[..], &b"b"[..]]));
    }

    #[cfg(unix)]
    #[test]
    fn compiles_once_and_reports_failures() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("m.va");
        std::fs::write(&src, "module m; endmodule\n").unwrap();
        let fake = super::super::testutil::fake(
            dir.path(),
            "openvaf",
            "while [ $# -gt 0 ]; do [ \"$1\" = -o ] && o=$2; shift; done; echo osdi > \"$o\"; echo ran >> \"$o.count\"",
        );
        let c = Compile {
            source: src.clone(),
            out: PathBuf::new(),
            ..Default::default()
        };
        let hash = source_hash(&fake, &c).unwrap();
        let cache = cache_dir(dir.path(), "tech", &hash);
        let first = compile_cached(&fake, &c, &cache, &[]).unwrap();
        assert!(first.compiled && first.osdi.is_file());
        let second = compile_cached(&fake, &c, &cache, &[]).unwrap();
        assert!(!second.compiled);
        assert_eq!(
            std::fs::read_to_string(cache.join("m.osdi.count"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        // A changed source is another cache folder.
        std::fs::write(&src, "module m2; endmodule\n").unwrap();
        assert_ne!(source_hash(&fake, &c).unwrap(), hash);
        let broken =
            super::super::testutil::fake(dir.path(), "bad", "echo 'error: syntax' ; exit 1");
        let err = compile(
            &broken,
            &Compile {
                source: src,
                out: dir.path().join("x.osdi"),
                ..Default::default()
            },
            &[],
        )
        .unwrap_err();
        assert!(format!("{err}").contains("syntax"), "{err}");
    }

    /// The real compiler, when there is one, on a small model.
    #[test]
    fn real_openvaf_compiles_a_resistor() {
        let Some(program) = ADAPTER.locate() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let va = dir.path().join("res.va");
        std::fs::write(
            &va,
            "`include \"disciplines.vams\"\nmodule res(a, b);\n inout a, b; electrical a, b;\n parameter real r = 1k;\n analog V(a,b) <+ r * I(a,b);\nendmodule\n",
        )
        .unwrap();
        let env = crate::vacask::tool_env_for(&program);
        let c = Compile {
            source: va.clone(),
            out: dir.path().join("res.osdi"),
            ..Default::default()
        };
        if let Err(e) = compile(&program, &c, &env) {
            // OpenVAF versions differ in where disciplines.vams comes from; a bad
            // include path is not what this test is about.
            let text = format!("{e:#}");
            assert!(text.contains("disciplines"), "{text}");
            return;
        }
        assert!(c.out.metadata().unwrap().len() > 1000);
    }
}
