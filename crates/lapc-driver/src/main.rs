use std::fs;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use lapc_driver::{DriverError, build, check};

#[derive(Parser)]
#[command(name = "lapc", version, about = "A compiler for Lap")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Check a program")]
    Check {
        #[arg(required = true, help = "The Lap files, appended in the order given")]
        files: Vec<PathBuf>,
    },
    #[command(about = "Build a program into an executable")]
    Build {
        #[arg(required = true, help = "The Lap files, appended in the order given")]
        files: Vec<PathBuf>,
        #[arg(short, long, help = "The output executable")]
        output: PathBuf,
    },
}

fn main() {
    let arguments = Arguments::parse();
    if let Err(error) = run(arguments) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> Result<(), DriverError> {
    match arguments.command {
        Command::Check { files } => {
            let source = read_sources(&files)?;
            check(&source)?;
            println!("ok");
            Ok(())
        }
        Command::Build { files, output } => {
            let source = read_sources(&files)?;
            let program = check(&source)?;
            build(&program, &output)
        }
    }
}

fn read_sources(paths: &[PathBuf]) -> Result<String, DriverError> {
    let mut source = String::new();
    for path in paths {
        let text = fs::read_to_string(path).map_err(|error| DriverError::Read {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        source.push_str(&text);
        source.push('\n');
    }
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::io::Write;
    use std::path::Path;
    use std::process::{Command as ProcessCommand, Stdio};

    use lapc_codegen::find_compiler;

    const PRELUDE: &str =
        "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\n";

    const DRAIN: &str = "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\n\
         BIT.ZERO = NAND(BIT.ONE, BIT.ONE)\n\
         BYTE = [BIT, BIT, BIT, BIT, BIT, BIT, BIT, BIT]\n\
         BYTE.ZERO = [BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO]\n\
         BYTE.ONE = [BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO]\n\
         BYTE.TWO = [BIT.ZERO, BIT.ONE, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO, BIT.ZERO]\n\
         OP.STREAM.READ = [BYTE.ONE, BYTE.TWO]\n\
         STREAM.IN = [BYTE.ZERO, BYTE.ZERO, BYTE.ZERO, BYTE.ZERO, BYTE.ZERO, BYTE.ZERO, BYTE.ZERO, BYTE.ZERO]\n\
         read.byte = () [BIT, BYTE] { [status, byte] = EXTERN(OP.STREAM.READ, STREAM.IN)\n BRANCH (status) { [status, byte] } { [status, BYTE.ZERO] } }\n\
         drain = () BIT { [status, byte] = read.byte()\n BRANCH (status) { drain() } { BIT.ZERO } }\n\
         main = () BIT { drain() }\n";

    fn test_directory(name: &str) -> PathBuf {
        let directory = env::temp_dir().join(format!("lapc-test-{}-{name}", std::process::id()));
        fs::create_dir_all(&directory).expect("the test directory is created");
        directory
    }

    fn write_sources(directory: &Path, sources: &[&str]) -> Vec<PathBuf> {
        sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let path = directory.join(format!("source{index}.lap"));
                fs::write(&path, source).expect("the source is written");
                path
            })
            .collect()
    }

    fn c_compiler_available() -> bool {
        let available = find_compiler().is_ok();
        if !available {
            eprintln!("skipping: no C compiler is available");
        }
        available
    }

    fn build_sources(name: &str, sources: &[&str]) -> PathBuf {
        let directory = test_directory(name);
        let files = write_sources(&directory, sources);
        let output = directory.join(format!("program{}", std::env::consts::EXE_SUFFIX));
        run(Arguments {
            command: Command::Build {
                files,
                output: output.clone(),
            },
        })
        .expect("the driver builds the program");
        output
    }

    fn run_program(path: &Path, input: &[u8]) -> (i32, Vec<u8>) {
        let mut child = ProcessCommand::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("the program runs");
        let mut standard_input = child
            .stdin
            .take()
            .expect("the program has a standard input");
        let input = input.to_vec();
        let writer = std::thread::spawn(move || {
            let _ = standard_input.write_all(&input);
        });
        let output = child.wait_with_output().expect("the program finishes");
        writer.join().expect("the writer finishes");
        (
            output
                .status
                .code()
                .expect("the program exits with a status"),
            output.stdout,
        )
    }

    fn build_and_run_with_input(name: &str, sources: &[&str], input: &[u8]) -> (i32, Vec<u8>) {
        let output = build_sources(name, sources);
        run_program(&output, input)
    }

    fn build_and_run(name: &str, sources: &[&str]) -> i32 {
        let (code, _) = build_and_run_with_input(name, sources, &[]);
        code
    }

    fn build_one_and_run(name: &str, source: &str) -> i32 {
        build_and_run(name, &[source])
    }

    #[test]
    fn a_program_returning_zero_exits_with_zero() {
        if !c_compiler_available() {
            return;
        }
        let code = build_one_and_run("zero", &format!("{PRELUDE}main = () BIT {{ BIT.ZERO }}\n"));
        assert_eq!(code, 0);
    }

    #[test]
    fn a_program_returning_one_exits_with_one() {
        if !c_compiler_available() {
            return;
        }
        let code = build_one_and_run("one", &format!("{PRELUDE}main = () BIT {{ BIT.ONE }}\n"));
        assert_eq!(code, 1);
    }

    #[test]
    fn a_program_with_a_function_and_a_reference_runs() {
        if !c_compiler_available() {
            return;
        }
        let code = build_one_and_run(
            "toggle",
            &format!(
                "{PRELUDE}bit.not = (a: BIT) BIT {{ NAND(a, a) }}\n\
                 bit.toggle = (target: *BIT) [] {{ target = bit.not(target) }}\n\
                 main = () BIT {{ state: BIT = BIT.ZERO\n bit.toggle(*state)\n state }}\n"
            ),
        );
        assert_eq!(code, 1);
    }

    #[test]
    fn a_program_with_a_branch_and_a_collection_runs() {
        if !c_compiler_available() {
            return;
        }
        let code = build_one_and_run(
            "branch",
            &format!(
                "{PRELUDE}U2 = [BIT, BIT]\n\
                 u2.select = (flag: BIT, when.one: U2, when.zero: U2) U2 {{ BRANCH (flag) {{ when.one }} {{ when.zero }} }}\n\
                 main = () BIT {{ low: U2 = [BIT.ZERO, BIT.ONE]\n high: U2 = [BIT.ONE, BIT.ONE]\n chosen: U2 = u2.select(BIT.ONE, low, high)\n [first, second] = chosen\n second }}\n"
            ),
        );
        assert_eq!(code, 1);
    }

    #[test]
    fn a_nested_destructuring_runs() {
        if !c_compiler_available() {
            return;
        }
        let code = build_one_and_run(
            "nested",
            &format!(
                "{PRELUDE}NESTED = [[BIT, BIT], BIT]\n\
                 main = () BIT {{ bundle: NESTED = [[BIT.ZERO, BIT.ONE], BIT.ONE]\n [[first, second], third] = bundle\n second }}\n"
            ),
        );
        assert_eq!(code, 1);
    }

    #[test]
    fn the_readme_example_core_runs() {
        if !c_compiler_available() {
            return;
        }
        let code = build_one_and_run(
            "readme",
            &format!(
                "{PRELUDE}U2 = [BIT, BIT]\n\
                 PAIR = [BIT, BIT]\n\
                 TUPLE = [BIT, U2, PAIR]\n\
                 bit.not = (a: BIT) BIT {{ NAND(a, a) }}\n\
                 bit.toggle = (target: *BIT) [] {{ target = bit.not(target) }}\n\
                 bit.toggle.twice = (target: *BIT) [] {{ bit.toggle(*target)\n bit.toggle(*target) }}\n\
                 u2.select = (flag: BIT, when.one: U2, when.zero: U2) U2 {{ BRANCH (flag) {{ when.one }} {{ when.zero }} }}\n\
                 main = () BIT {{ state: BIT = BIT.ZERO\n alias: *BIT = *state\n alias = BIT.ONE\n bit.toggle.twice(*state)\n low: U2 = [BIT.ZERO, BIT.ONE]\n high: U2 = [BIT.ONE, BIT.ONE]\n chosen: U2 = u2.select(state, low, high)\n paired: PAIR = chosen\n bundle: TUPLE = [state, chosen, paired]\n [head, *middle, last] = bundle\n head }}\n"
            ),
        );
        assert_eq!(code, 1);
    }

    #[test]
    fn files_are_appended_in_order() {
        if !c_compiler_available() {
            return;
        }
        let code = build_and_run(
            "order",
            &[
                "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\nBIT.ZERO = NAND(BIT.ONE, BIT.ONE)\n",
                "main = () BIT { BIT.ONE }\n",
            ],
        );
        assert_eq!(code, 1);
    }

    #[test]
    fn a_use_before_the_definition_fails() {
        let directory = test_directory("reverse");
        let files = write_sources(
            &directory,
            &[
                "main = () BIT { BIT.ONE }\n",
                "BIT.ONE = NAND(BIT, NAND(BIT, BIT))\n",
            ],
        );
        let result = run(Arguments {
            command: Command::Build {
                files,
                output: directory.join("program"),
            },
        });
        assert!(result.is_err(), "the use precedes the definition");
    }

    #[test]
    fn the_check_command_accepts_a_valid_program() {
        let directory = test_directory("check");
        let source = format!("{PRELUDE}main = () BIT {{ BIT.ZERO }}\n");
        let files = write_sources(&directory, &[&source]);
        run(Arguments {
            command: Command::Check { files },
        })
        .expect("the driver checks the program");
    }

    #[test]
    fn an_invalid_program_fails_the_check() {
        let directory = test_directory("invalid");
        let source = String::from("main = () BIT { BRANCH (BIT) { BIT.ZERO } { BIT.ZERO } }\n");
        let files = write_sources(&directory, &[&source]);
        let result = run(Arguments {
            command: Command::Check { files },
        });
        assert!(result.is_err(), "the condition is indeterminate");
    }

    #[test]
    fn a_missing_file_fails() {
        let result = run(Arguments {
            command: Command::Check {
                files: vec![PathBuf::from("/nonexistent.lap")],
            },
        });
        assert!(result.is_err());
    }

    #[test]
    fn the_help_command_succeeds() {
        match Arguments::try_parse_from(["lapc", "--help"]) {
            Err(error) => assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp),
            Ok(_) => panic!("help is not a run"),
        }
    }

    #[test]
    fn the_runtime_test_passes() {
        if !c_compiler_available() {
            return;
        }
        let compiler = find_compiler().expect("the compiler is found");
        let directory = test_directory("runtime");
        let binary = directory.join(format!("lap_runtime_test{}", std::env::consts::EXE_SUFFIX));
        let status = compiler
            .command()
            .arg("-std=c17")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg("../../runtime/lap_runtime.c")
            .arg("../../runtime/lap_runtime_test.c")
            .arg("-o")
            .arg(&binary)
            .status()
            .expect("the C compiler runs");
        assert!(status.success(), "the runtime test compiles");
        let status = ProcessCommand::new(&binary)
            .status()
            .expect("the runtime test runs");
        assert!(status.success(), "the runtime test passes");
    }

    #[test]
    fn a_deep_tail_recursion_runs() {
        if !c_compiler_available() {
            return;
        }
        let input = vec![b'x'; 300_000];
        let (code, output) = build_and_run_with_input("drain", &[DRAIN], &input);
        assert_eq!(code, 0);
        assert!(output.is_empty());
    }

    #[test]
    fn an_erased_function_declared_with_an_empty_collection_runs() {
        if !c_compiler_available() {
            return;
        }
        let code = build_one_and_run(
            "erased",
            &format!(
                "{PRELUDE}intrinsic.bit.not = []\nmain = () BIT {{ intrinsic.bit.not(BIT.ZERO) }}\n"
            ),
        );
        assert_eq!(code, 1);
    }
}
