//! A command-line check of a SIP account through pjproject: register,
//! place or answer a call, and show the SIP traffic.

mod config;
mod pj;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
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
    /// Register, optionally call or wait for calls, then unregister.
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
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
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
            pj::run(&cfg, &opts, &stop)
        }
    }
}
