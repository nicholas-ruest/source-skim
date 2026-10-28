use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::de::DeserializeOwned;
use source_skim::{compile, render, verify, Bundle, Manifest, Policy};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile a deterministic selection manifest.
    Compile {
        /// Source bundle JSON file, or - for stdin.
        #[arg(short, long)]
        input: PathBuf,
        /// Maximum total upstream token estimate.
        #[arg(short, long)]
        budget: u32,
        /// Minimum number of distinct sources to preserve.
        #[arg(long, default_value_t = 1)]
        min_sources: usize,
        /// Output manifest path; stdout when omitted.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Verify a manifest against its exact source bundle.
    Verify {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        manifest: PathBuf,
    },
    /// Verify and render selected text with source/range markers.
    Render {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        manifest: PathBuf,
        /// Rendered context path; stdout when omitted.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Compile {
            input,
            budget,
            min_sources,
            output,
        } => {
            let bundle: Bundle = read_json(&input)?;
            let manifest = compile(
                &bundle,
                Policy {
                    token_budget: budget,
                    min_sources,
                },
            )?;
            let text = serde_json::to_string_pretty(&manifest)? + "\n";
            write_output(output.as_deref(), &text)?;
        }
        Command::Verify { input, manifest } => {
            let bundle: Bundle = read_json(&input)?;
            let manifest: Manifest = read_json(&manifest)?;
            let result = verify(&bundle, &manifest)?;
            println!(
                "verified {} spans, {} tokens, {} sources",
                result.spans, result.total_tokens, result.sources_covered
            );
        }
        Command::Render {
            input,
            manifest,
            output,
        } => {
            let bundle: Bundle = read_json(&input)?;
            let manifest: Manifest = read_json(&manifest)?;
            let text = render(&bundle, &manifest)?;
            write_output(output.as_deref(), &text)?;
        }
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let mut text = String::new();
    if path == Path::new("-") {
        io::stdin()
            .read_to_string(&mut text)
            .context("reading JSON from stdin")?;
    } else {
        text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    }
    serde_json::from_str(&text).with_context(|| format!("parsing JSON from {}", path.display()))
}

fn write_output(path: Option<&Path>, text: &str) -> Result<()> {
    if let Some(path) = path {
        fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    } else {
        print!("{text}");
    }
    Ok(())
}
