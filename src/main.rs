mod config;
mod data;
mod draw;
mod encode;
mod fmt;
mod layout;
mod render;
mod scene;
mod text;
mod timeline;
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
    /// Render the video(s) for a week folder into <dir>/out/
    Render {
        dir: PathBuf,
        #[arg(long, value_enum, default_value = "main")]
        format: FormatArg,
        /// Half resolution, 30 fps, for quick checks
        #[arg(long)]
        preview: bool,
    },
    /// Render out/thumb.png (1280x720)
    Thumb {
        dir: PathBuf,
        /// Year to show (default: last year in the data)
        #[arg(long)]
        year: Option<f64>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum FormatArg {
    Main,
    Short,
    Both,
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
        Cmd::Render { dir, format, preview } => {
            use timeline::Format;
            let formats: &[Format] = match format {
                FormatArg::Main => &[Format::Main],
                FormatArg::Short => &[Format::Short],
                FormatArg::Both => &[Format::Main, Format::Short],
            };
            for &f in formats {
                render::render(&dir, f, preview)?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Thumb { dir, year } => {
            render::thumb(&dir, year)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
