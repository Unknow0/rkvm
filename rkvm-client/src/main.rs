mod client;
mod config;
mod stream;
mod tls;

use clap::Parser;
use client::{Error, init_tracing, init_config};
use rkvm_input::writer::Writer;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use stream::RkvmStream;
use tokio::io::{split, BufStream};
use tokio::signal;
use tokio::time::{Duration, sleep};

#[cfg(target_os="windows")]
use tokio::net::windows::named_pipe::ClientOptions;
#[cfg(target_os="windows")]
use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;


#[derive(Parser)]
#[structopt(name = "rkvm-client", about = "The rkvm client application")]
struct Args {
    #[clap(help = "Path to configuration file")]
    config_path: PathBuf,
    #[clap(long, default_value = "info", help = "log filter")]
    log_level: String,
    #[clap(long, help = "output file for the logs")]
    log_file: Option<PathBuf>,
    #[cfg(target_os="windows")]
    #[clap(long, help = "internal use")]
    pipe: bool,
}

async fn process_args_default(args: Args) -> Result<RkvmStream,Error> {
    let config = init_config(&args.config_path).await?;
    let connector = tls::configure(&config.certificate).await?;
    client::init_stream(&config.server.hostname, config.server.port, &connector, &config.password).await
}
#[cfg(not(target_os="windows"))]
async fn process_args(args: Args) -> Result<RkvmStream,Error> {
    process_args_default(args).await
}

#[cfg(target_os="windows")]
async fn process_args(args: Args) -> Result<RkvmStream,Error> {
    if args.pipe {
        tracing::info!("pipe mode {:?}", args.config_path);
        let pipe = loop {
            match ClientOptions::new().open(&args.config_path) {
                Ok(client) => break client,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => (),
                Err(e) => return Err(Error::Io(e)),
            }

            sleep(Duration::from_millis(50)).await;
        };
        Ok(RkvmStream::Pipe(BufStream::with_capacity(1024, 1024, pipe)))
    }else {
        process_args_default(args).await
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    init_tracing(&args.log_level, &args.log_file);

    let stream = match process_args(args).await {
        Ok(stream) => stream,
        Err(e) => {
            tracing::error!("Failed to open stream {}", e);
            return ExitCode::FAILURE;
        }
    };

    let writers = Rc::new(RefCell::new(HashMap::<usize, Writer>::new()));
    let update = |update| async {
        client::handler(&mut writers.borrow_mut(), update).await
    };

    let (r,w) = split(stream);
    tokio::select! {
        result = client::run(r, w, update) => {
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
