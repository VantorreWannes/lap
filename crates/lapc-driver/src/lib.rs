use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use lapc_check::check_program;
use lapc_codegen::emit_object;
use lapc_erase::erase_intrinsics;
use lapc_parse::parse_program;

pub const RUNTIME_SOURCE: &str = include_str!("../../../runtime/lap_runtime.c");
pub const ENTRY_SOURCE: &str = include_str!("../../../runtime/lap_entry.c");

pub fn check(source: &str) -> Result<lapc_ir::Program, DriverError> {
    let program = parse_program(source).map_err(|error| DriverError::Parse {
        message: format!("{error:?}"),
    })?;
    let program = check_program(&program).map_err(|error| DriverError::Check {
        message: format!("{error:?}"),
    })?;
    Ok(erase_intrinsics(program))
}

pub struct Compiler {
    program: String,
    prefix: Vec<String>,
}

impl Compiler {
    fn new(program: &str, prefix: &[&str]) -> Self {
        Compiler {
            program: String::from(program),
            prefix: prefix.iter().map(|word| String::from(*word)).collect(),
        }
    }

    pub fn command(&self) -> ProcessCommand {
        let mut command = ProcessCommand::new(&self.program);
        command.args(&self.prefix);
        command
    }

    fn name(&self) -> String {
        let mut words = vec![self.program.clone()];
        words.extend(self.prefix.iter().cloned());
        words.join(" ")
    }
}

fn compiler_candidates() -> Vec<Compiler> {
    vec![
        Compiler::new("cc", &[]),
        Compiler::new("gcc", &[]),
        Compiler::new("clang", &[]),
        Compiler::new("zig", &["cc"]),
    ]
}

static TEMPORARY_NUMBER: AtomicUsize = AtomicUsize::new(0);

pub fn temporary_path(name: &str) -> PathBuf {
    let number = TEMPORARY_NUMBER.fetch_add(1, Ordering::Relaxed);
    env::temp_dir().join(format!("lapc-{}-{number}-{name}", std::process::id()))
}

fn compiler_builds_runtime(compiler: &Compiler) -> bool {
    let source = temporary_path("probe.c");
    let object = source.with_extension("o");
    if fs::write(&source, RUNTIME_SOURCE).is_err() {
        return false;
    }
    let status = compiler
        .command()
        .arg("-std=c17")
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = fs::remove_file(&source);
    let _ = fs::remove_file(&object);
    matches!(status, Ok(status) if status.success())
}

pub fn find_compiler() -> Result<Compiler, DriverError> {
    if let Ok(configured) = env::var("CC") {
        return Ok(Compiler::new(&configured, &[]));
    }
    let candidates = compiler_candidates();
    let names = candidates
        .iter()
        .map(Compiler::name)
        .collect::<Vec<_>>()
        .join(", ");
    candidates
        .into_iter()
        .find(compiler_builds_runtime)
        .ok_or(DriverError::Compiler {
            message: format!(
                "no C compiler that can build the runtime was found (tried {names}); set CC to choose one"
            ),
        })
}

pub fn build(program: &lapc_ir::Program, output: &Path) -> Result<(), DriverError> {
    let object = emit_object(program).map_err(|error| DriverError::Codegen {
        message: format!("{error:?}"),
    })?;
    let directory = temporary_path("build");
    fs::create_dir_all(&directory).map_err(|error| DriverError::Write {
        path: directory.display().to_string(),
        message: error.to_string(),
    })?;
    let result = link(&directory, &object, output);
    let _ = fs::remove_dir_all(&directory);
    result
}

fn link(directory: &Path, object: &[u8], output: &Path) -> Result<(), DriverError> {
    let object_path = directory.join("program.o");
    let runtime_path = directory.join("lap_runtime.c");
    let entry_path = directory.join("lap_entry.c");
    write(&object_path, object)?;
    write(&runtime_path, RUNTIME_SOURCE.as_bytes())?;
    write(&entry_path, ENTRY_SOURCE.as_bytes())?;
    let compiler = find_compiler()?;
    let mut command = compiler.command();
    command.arg("-std=c17").arg("-O2").arg("-Wl,-S");
    if cfg!(target_os = "linux") {
        command.arg("-no-pie");
    }
    let status = command
        .arg("-o")
        .arg(output)
        .arg(&object_path)
        .arg(&runtime_path)
        .arg(&entry_path)
        .status()
        .map_err(|error| DriverError::Link {
            message: error.to_string(),
        })?;
    if !status.success() {
        return Err(DriverError::Link {
            message: String::from("the linker failed"),
        });
    }
    Ok(())
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), DriverError> {
    fs::write(path, bytes).map_err(|error| DriverError::Write {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

#[derive(Debug)]
pub enum DriverError {
    Read { path: String, message: String },
    Write { path: String, message: String },
    Parse { message: String },
    Check { message: String },
    Codegen { message: String },
    Compiler { message: String },
    Link { message: String },
}

impl fmt::Display for DriverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriverError::Read { path, message } => {
                write!(formatter, "cannot read {path}: {message}")
            }
            DriverError::Write { path, message } => {
                write!(formatter, "cannot write {path}: {message}")
            }
            DriverError::Parse { message } => write!(formatter, "parse error: {message}"),
            DriverError::Check { message } => write!(formatter, "check error: {message}"),
            DriverError::Codegen { message } => write!(formatter, "codegen error: {message}"),
            DriverError::Compiler { message } => write!(formatter, "compiler error: {message}"),
            DriverError::Link { message } => write!(formatter, "link error: {message}"),
        }
    }
}
