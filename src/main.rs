//! CLI for sigmaos-rebuild.

use clap::Parser;
use sigmaos_rebuild::{current_kernel, save_records, RebuildError, RebuildPlan};
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(
    name = "sigmaos-rebuild",
    about = "SigmaOS system rebuild orchestrator"
)]
struct Cli {
    /// Dry run: print steps without executing.
    #[arg(long)]
    dry_run: bool,
    /// Save execution records to this JSON file.
    #[arg(long)]
    record: Option<PathBuf>,
}

fn run(cmd: &[String]) -> Result<(i32, String), RebuildError> {
    let (prog, args) = cmd
        .split_first()
        .ok_or_else(|| RebuildError::Validation("empty command".into()))?;
    let out = Command::new(prog).args(args).output()?;
    let tail = String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .take(5)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Ok((out.status.code().unwrap_or(-1), tail))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let cli = Cli::parse();
    let kernel = current_kernel();
    let plan = RebuildPlan::standard(&kernel);

    if cli.dry_run {
        print!("{}", plan.describe());
    } else {
        println!("{}", plan.describe());
        eprintln!("Executing rebuild for kernel {kernel}...");
    }

    let records = plan.execute(cli.dry_run, run)?;
    if let Some(path) = cli.record {
        save_records(&records, &path)?;
        eprintln!("records saved: {}", path.display());
    }
    Ok(())
}
