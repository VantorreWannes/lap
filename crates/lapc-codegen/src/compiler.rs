use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

const RUNTIME_SOURCE: &str = include_str!("../../../runtime/lap_runtime.c");

pub struct Compiler {
    program: String,
    prefix: Vec<String>,
}

impl Compiler {
    fn new(program: &str, prefix: &[&str]) -> Self {
        Self {
            program: String::from(program),
            prefix: prefix.iter().map(|word| String::from(*word)).collect(),
        }
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.prefix);
        command
    }

    pub fn name(&self) -> String {
        let mut words = vec![self.program.clone()];
        words.extend(self.prefix.iter().cloned());
        words.join(" ")
    }
}

fn candidates() -> Vec<Compiler> {
    vec![
        Compiler::new("cc", &[]),
        Compiler::new("gcc", &[]),
        Compiler::new("clang", &[]),
        Compiler::new("zig", &["cc"]),
    ]
}

static PROBE_NUMBER: AtomicUsize = AtomicUsize::new(0);

fn probe_path(name: &str) -> PathBuf {
    let number = PROBE_NUMBER.fetch_add(1, Ordering::Relaxed);
    env::temp_dir().join(format!("lapc-probe-{}-{number}-{name}", std::process::id()))
}

fn builds_runtime(compiler: &Compiler) -> bool {
    let source = probe_path("probe.c");
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

pub fn find_compiler() -> Result<Compiler, String> {
    if let Ok(configured) = env::var("CC") {
        return Ok(Compiler::new(&configured, &[]));
    }
    let candidates = candidates();
    let names = candidates
        .iter()
        .map(Compiler::name)
        .collect::<Vec<_>>()
        .join(", ");
    candidates.into_iter().find(builds_runtime).ok_or_else(|| {
        format!(
            "no C compiler that can build the runtime was found (tried {names}); set CC to choose one"
        )
    })
}
