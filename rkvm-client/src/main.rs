#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod client;
mod config;
mod tls;

use clap::Parser;
use config::Config;
use std::time::Duration;
use std::path::PathBuf;
use std::fs::OpenOptions;
use std::io::{stdout, BufWriter};
use std::process::ExitCode;
use tokio::{fs, signal};
use tokio::time::sleep;
use tokio_rustls::TlsConnector;
use tracing_subscriber::{fmt,Registry,EnvFilter};
use tracing_subscriber::prelude::*;

#[derive(Parser)]
#[structopt(name = "rkvm-client", about = "The rkvm client application")]
struct Args {
    #[clap(help = "Path to configuration file")]
    config_path: PathBuf,
    #[clap(long, default_value = "info", help = "log filter")]
    log_level: String,
    /// Optional log file
    #[clap(long, help = "output file for the logs")]
    log_file: Option<PathBuf>,
}

async fn main_loop(config: &Config, connector: &TlsConnector) -> ExitCode {
     tokio::select! {
        result = client::run(&config.server.hostname, config.server.port, connector, &config.password) => {
            if let Err(err) = result {
                tracing::error!("Error: {}", err);
                return ExitCode::FAILURE;
            }
        }
        // This is needed to properly clean libevdev stuff up.
        result = signal::ctrl_c() => {
            if let Err(err) = result {
                tracing::error!("Error setting up signal handler: {}", err);
                return ExitCode::FAILURE;
            }

            tracing::info!("Exiting on signal");
        }
    }

    ExitCode::SUCCESS
}

fn init_tracing(log_level: &String, log_file: &Option<PathBuf>) {
    let filter = EnvFilter::new(log_level);
    if let Some(path) = log_file {
        let file = OpenOptions::new().create(true).append(true).open(path).unwrap();
        let fmt_layer = fmt::layer().with_writer(move || BufWriter::new(file.try_clone().unwrap())).without_time();
        let registry = Registry::default().with(filter).with(fmt_layer);
        tracing::subscriber::set_global_default(registry).unwrap();
    } else {
        let fmt_layer = fmt::layer().with_writer(stdout).without_time();
        let registry = Registry::default().with(filter).with(fmt_layer);
        tracing::subscriber::set_global_default(registry).unwrap();
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    init_tracing(&args.log_level, &args.log_file);

    let config = match fs::read_to_string(&args.config_path).await {
        Ok(config) => config,
        Err(err) => {
            tracing::error!("Error reading config: {}", err);
            return ExitCode::FAILURE;
        }
    };

    let config = match toml::from_str::<Config>(&config) {
        Ok(config) => config,
        Err(err) => {
            tracing::error!("Error parsing config: {}", err);
            return ExitCode::FAILURE;
        }
    };

    let connector = match tls::configure(&config.certificate).await {
        Ok(connector) => connector,
        Err(err) => {
            tracing::error!("Error configuring TLS: {}", err);
            return ExitCode::FAILURE;
        }
    };
    
    match config.reconnect_delay.map(Duration::from_secs) {
        None => main_loop(&config, &connector).await,
        Some(reconnect_delay) => {
            loop {
                let code = main_loop(&config, &connector).await;
                if code == ExitCode::SUCCESS {
                    return code;
                }
                sleep(reconnect_delay).await;
            }
        }
    }
}
