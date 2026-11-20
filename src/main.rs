mod config;
mod data;
#[cfg(test)]
mod testutil;
mod validate;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(version, about = "Bar chart race video renderer")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check a week folder's data.csv and video.toml
    Validate { dir: PathBuf },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    match cli.cmd {
        Cmd::Validate { dir } => {
            let r = validate::run(&dir)?;
            println!("items: {}", r.items);
            println!("years: {}–{}", r.first_year, r.last_year);
            println!("estimated: {:.1}%", r.estimated_share * 100.0);
            println!("reviewed: {}", if r.reviewed { "yes" } else { "no" });
            for w in &r.warnings {
                println!("warning: {w}");
            }
            for e in &r.errors {
                println!("error: {e}");
            }
            Ok(if r.errors.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
    }
}
