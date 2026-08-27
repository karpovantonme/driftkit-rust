//! driftkit: finds where a project's declarations and its behaviour disagree.
//!
//! One binary, one check per subcommand, one contract for all of them. The
//! Python kit this comes from grew to twenty-five separate tools that had
//! quietly drifted apart in their own flags and JSON keys; that is the reason
//! for the single contract in `core`, and for the conformance rules baked
//! into the shape of this file.

use driftkit::env;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "driftkit",
    version,
    about = "Finds where a project's declarations and its behaviour disagree"
)]
struct Cli {
    #[command(subcommand)]
    check: Check,
}

#[derive(Subcommand)]
enum Check {
    /// The example env file against what the code actually reads
    Env {
        /// Project directory
        #[arg(long)]
        dir: PathBuf,
        /// Also list declared-but-unread names (soft, and noisy)
        #[arg(long)]
        plain: bool,
        /// Write findings to JSON
        #[arg(long)]
        json: Option<PathBuf>,
        /// List what was dismissed and what went unmatched
        #[arg(short, long)]
        verbose: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.check {
        Check::Env {
            dir,
            plain,
            json,
            verbose,
        } => {
            let mut report = env::Report::default();
            env::analyse(&dir, &mut report, plain);
            env::print_report(&report, verbose);

            if let Some(path) = json {
                match serde_json::to_string_pretty(&report.findings) {
                    Ok(text) => {
                        if let Err(e) = std::fs::write(&path, text) {
                            eprintln!("could not write {}: {e}", path.display());
                            return ExitCode::from(2);
                        }
                    }
                    Err(e) => {
                        eprintln!("could not encode findings: {e}");
                        return ExitCode::from(2);
                    }
                }
            }

            // Exit code is 1 if and only if there is at least one hard
            // finding, so the tool works inside a shell `if`.
            if report.hard() > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}
