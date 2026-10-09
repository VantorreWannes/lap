use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use lapc_tokenizer::Token;

#[derive(Parser)]
#[command(version, about = "Tokenize a Lap source file")]
struct Arguments {
    file: PathBuf,
    #[arg(short, long, value_enum, default_value = "json")]
    format: Format,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Json,
    Pretty,
    Text,
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match run(&arguments) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: &Arguments) -> Result<String> {
    let path = &arguments.file;
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let tokens = lapc_tokenizer::lex(&source)
        .with_context(|| format!("failed to tokenize {}", path.display()))?;
    render(&tokens, arguments.format)
}

fn render(tokens: &[Token], format: Format) -> Result<String> {
    Ok(match format {
        Format::Json => serde_json::to_string(tokens)?,
        Format::Pretty => serde_json::to_string_pretty(tokens)?,
        Format::Text => tokens
            .iter()
            .map(|token| {
                format!(
                    "{}..{} {}",
                    token.span().start(),
                    token.span().end(),
                    token.kind()
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    })
}
