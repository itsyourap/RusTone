//! Phase 0 spike: drive pjproject from Rust, register to the JF router,
//! place a call, and check the Contact headers that went out.

mod config;
mod contact_check;
mod pj;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::config::Config;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Register, optionally call or wait for calls, unregister, then check Contacts.
    Run {
        /// Account and settings file.
        #[arg(short, long, default_value = "rustone-cli.toml")]
        config: PathBuf,
        /// Number or SIP URI to call after registering.
        #[arg(long)]
        call: Option<String>,
        /// Seconds to stay in the call once it's answered.
        #[arg(long, default_value_t = 15)]
        duration: u64,
        /// Seconds to wait for incoming calls after registering.
        #[arg(long, default_value_t = 0)]
        listen: u64,
        /// Answer incoming calls automatically (otherwise they just ring).
        #[arg(long)]
        answer: bool,
        /// Seconds to wait for registration.
        #[arg(long, default_value_t = 20)]
        reg_timeout: u64,
        /// Echo the full pjsua log (including SIP messages) to stderr.
        #[arg(long)]
        sip: bool,
    },
    /// Check the Contact headers in an existing pjsua log .
    CheckLog { file: PathBuf },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Returns whether the Contact check passed.
fn run(cli: Cli) -> Result<bool> {
    match cli.command {
        Command::CheckLog { file } => {
            let bytes = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            Ok(print_contact_check(&String::from_utf8_lossy(&bytes)))
        }
        Command::Run { config, call, duration, listen, answer, reg_timeout, sip } => {
            let cfg = Config::load(&config)?;
            let stop = Arc::new(AtomicBool::new(false));
            let handler_stop = stop.clone();
            ctrlc::set_handler(move || handler_stop.store(true, Ordering::Relaxed))?;
            let opts = pj::Options {
                call,
                duration: Duration::from_secs(duration),
                listen: Duration::from_secs(listen),
                answer,
                reg_timeout: Duration::from_secs(reg_timeout),
                echo_sip: sip,
            };
            // Check the Contacts even if the session failed part-way.
            let (result, log) = pj::run(&cfg, &opts, &stop);
            let passed = print_contact_check(&log);
            result.map(|()| passed)
        }
    }
}

fn print_contact_check(log: &str) -> bool {
    let report = contact_check::check_log(log);
    println!("\nContact check:\n{report}");
    report.ok()
}
