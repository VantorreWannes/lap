use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use lapc_codegen::{Compiler, find_compiler};
use lapc_driver::{build, check, temporary_path};

const REPETITIONS: usize = 5;

struct Measurement {
    nanoseconds: u64,
    result: Vec<u8>,
}

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let directory = programs_directory();
    let compiler = find_compiler()?;
    println!(
        "{:<16} {:>12} {:>12} {:>8}",
        "benchmark", "lap ns", "c ns", "lap/c"
    );
    for name in benchmark_names(&directory)? {
        let lap = build_lap(&directory, &name)?;
        let c = build_c(&directory, &name, &compiler)?;
        let lap_measurement = measure(&lap)?;
        let c_measurement = measure(&c)?;
        if lap_measurement.result != c_measurement.result {
            return Err(format!("{name} disagrees with its C reference"));
        }
        println!(
            "{:<16} {:>12} {:>12} {:>8.2}",
            name,
            lap_measurement.nanoseconds,
            c_measurement.nanoseconds,
            lap_measurement.nanoseconds as f64 / c_measurement.nanoseconds as f64
        );
    }
    Ok(())
}

fn programs_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("programs")
}

fn benchmark_names(directory: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("lap") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        if stem == "lib" {
            continue;
        }
        names.push(String::from(stem));
    }
    names.sort();
    Ok(names)
}

fn build_lap(directory: &Path, name: &str) -> Result<PathBuf, String> {
    let mut source = read(&directory.join("lib.lap"))?;
    source.push('\n');
    source.push_str(&read(&directory.join(format!("{name}.lap")))?);
    let program = check(&source).map_err(|error| error.to_string())?;
    let output = temporary_path(&format!("{name}{}", env::consts::EXE_SUFFIX));
    build(&program, &output).map_err(|error| error.to_string())?;
    Ok(output)
}

fn build_c(directory: &Path, name: &str, compiler: &Compiler) -> Result<PathBuf, String> {
    let output = temporary_path(&format!("{name}-c{}", env::consts::EXE_SUFFIX));
    let status = compiler
        .command()
        .arg("-std=c17")
        .arg("-O2")
        .arg("-I")
        .arg(directory)
        .arg(directory.join(format!("{name}.c")))
        .arg("-o")
        .arg(&output)
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("the C build of {name} failed"));
    }
    Ok(output)
}

fn measure(path: &Path) -> Result<Measurement, String> {
    let mut best: Option<Measurement> = None;
    for _ in 0..REPETITIONS {
        let output = Command::new(path)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!("{} exited with a failure", path.display()));
        }
        if output.stdout.len() < 8 {
            return Err(format!("{} wrote no measurement", path.display()));
        }
        let mut word = [0u8; 8];
        word.copy_from_slice(&output.stdout[..8]);
        let measurement = Measurement {
            nanoseconds: u64::from_le_bytes(word),
            result: output.stdout[8..].to_vec(),
        };
        if best
            .as_ref()
            .is_none_or(|best| measurement.nanoseconds < best.nanoseconds)
        {
            best = Some(measurement);
        }
    }
    best.ok_or_else(|| format!("{} was never measured", path.display()))
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}
