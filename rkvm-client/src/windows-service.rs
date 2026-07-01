#![cfg(target_os="windows")]
mod client;
mod config;
mod connection;
mod windows_daemon;

use crate::client::{init_tracing, Error, RkvmWriter};
use crate::windows_daemon::{writer::ClientWriter, client_process::ClientProcess, stream::LockWriter};

use rkvm_input::windows::writer::WriterWindows;
use rkvm_net::Update;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{WriteHalf, split};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::mpsc::{channel, Receiver};
use tokio::time::interval;
use tracing::Instrument;
use windows_service::define_windows_service;
use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,ServiceType};
use windows_service::service_control_handler::{register, ServiceControlHandlerResult};
use windows_service::service_dispatcher;

const SERVICE_NAME: &str = "RkvmService";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;
const SERVICE_LOG: &str = r"C:\ProgramData\rkvm\rkvm-service.log";
const SERVICE_CFG: &str = r"C:\ProgramData\rkvm\client.toml";

enum ServiceEvent {
    Stop,
    Restart,
}

define_windows_service!(ffi_service_main, service_main);

fn main() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

fn service_main(_arguments: Vec<OsString>) {
    init_tracing(&"info".to_string(), &Some(PathBuf::from(SERVICE_LOG)));
    tracing::info!("Starting service");
    if let Err(e) = run_service() {
        let _ = std::fs::write(r"C:\Windows\Temp\rkvm-service.log", format!("Service error: {e:#?}"));
        tracing::error!("Service error: {:?}", e);
    }
}

fn run_service() -> windows_service::Result<()> {
    let (tx, rx) = channel(4);
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop => {
                tracing::info!("Service stop requested");
                let _ = tx.try_send(ServiceEvent::Stop);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::SessionChange(change) => {
                tracing::info!("Session change: {:?}", change);
                let _ = tx.try_send(ServiceEvent::Restart);
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = register(SERVICE_NAME, event_handler)?;

    let status = ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP
            | ServiceControlAccept::SESSION_CHANGE,
        exit_code: ServiceExitCode::NO_ERROR,
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };
    if let Err(e) = status_handle.set_service_status(status) {
        tracing::error!("Failed to start service {:?}", e);
        return Err(e);
    }

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    match rt.block_on(process_loop(rx)) {
          Ok(_) => {
            tracing::info!("Service stopped normally");

            status_handle.set_service_status(ServiceStatus {
                service_type: SERVICE_TYPE,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            })?;
        }
        Err(e) => {
            tracing::error!(error = ?e, "Service crashed");

            status_handle.set_service_status(ServiceStatus {
                service_type: SERVICE_TYPE,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::Win32(1),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            })?;
        }
    }
    Ok(())
}
async fn process_loop(mut rx: Receiver<ServiceEvent>) -> Result<(), Error> {
    loop {
        match process(&mut rx).await {
            Ok(_) => break Ok(()),
            Err(e) => {
                tracing::error!(?e, "run_once crashed, restarting");
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn process(rx: &mut Receiver<ServiceEvent>) -> Result<(), Error> {
    tracing::info!("Service started");

    let mut cl:ClientProcess = ClientProcess::new().await?;

    let stream = connection::init_stream(SERVICE_CFG).await?;
    let (mut stream_r, mut stream_w) = split(stream);

    let srv_update = WriterWindows::new(ClientWriter::new(cl.writer()));

    let ping_w = cl.writer();
    let mut restart_w = cl.writer();
    let span_server = tracing::info_span!("run", stream="server");
    let span_client = tracing::info_span!("run", stream="client");
    tokio::select! {
        res = client::run(&mut stream_r, &mut stream_w, srv_update).instrument(span_server) => res,
        res = cl.run().instrument(span_client) => res,
        res = ping_sender(ping_w) => res,
        _ = async {
            loop {
                match rx.recv().await {
                    Some(ServiceEvent::Stop) => {
                        tracing::info!("Service requested stop");
                        break ();
                    }
                    Some(ServiceEvent::Restart) => {
                        tracing::info!("Service restarting");
                        let _ = restart_w.send(Update::Stop);
                    }
                    _ => {}
                }
            }
        } => Ok(())
    }
}

async fn ping_sender(mut w: LockWriter<WriteHalf<NamedPipeServer>>) -> Result<(),Error> {
    let mut interval = interval(rkvm_net::PING_INTERVAL);
    interval.tick().await;
    loop {
        interval.tick().await;
        w.send(Update::Ping).await?
    }
}
