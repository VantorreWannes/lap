use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use lapc_check::check_program;
use lapc_codegen::{emit_c, find_compiler};
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

static TEMPORARY_NUMBER: AtomicUsize = AtomicUsize::new(0);

pub fn temporary_path(name: &str) -> PathBuf {
    let number = TEMPORARY_NUMBER.fetch_add(1, Ordering::Relaxed);
    env::temp_dir().join(format!("lapc-{}-{number}-{name}", std::process::id()))
}

pub fn build(program: &lapc_ir::Program, output: &Path) -> Result<(), DriverError> {
    let directory = temporary_path("build");
    fs::create_dir_all(&directory).map_err(|error| DriverError::Write {
        path: directory.display().to_string(),
        message: error.to_string(),
    })?;
    let result = emit_c(program)
        .map_err(|error| DriverError::Codegen {
            message: format!("{error:?}"),
        })
        .and_then(|source| {
            if let Ok(path) = env::var("LAPC_DUMP_C") {
                let _ = fs::write(path, &source);
            }
            link(&directory, &source, output)
        });
    let _ = fs::remove_dir_all(&directory);
    result
}

fn link(directory: &Path, program: &str, output: &Path) -> Result<(), DriverError> {
    let program_path = directory.join("program.c");
    let runtime_path = directory.join("lap_runtime.c");
    let entry_path = directory.join("lap_entry.c");
    write(&program_path, program.as_bytes())?;
    write(&runtime_path, RUNTIME_SOURCE.as_bytes())?;
    write(&entry_path, ENTRY_SOURCE.as_bytes())?;
    let compiler = find_compiler().map_err(|message| DriverError::Compiler { message })?;
    let mut command = compiler.command();
    command.arg("-std=c17").arg("-O2");
    let status = command
        .arg("-o")
        .arg(output)
        .arg(&program_path)
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
