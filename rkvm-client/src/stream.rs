use crate::client::Error;

use async_trait::async_trait;
use rkvm_net::{Update, message::Message};
use std::pin::Pin;
use std::io;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufStream};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc::Sender};
use tokio_rustls::client::TlsStream;

#[cfg(target_os = "windows")]
use tokio::net::windows::named_pipe::NamedPipeClient;

pub enum RkvmStream {
    Tcp(BufStream<TlsStream<TcpStream>>),

    #[cfg(target_os = "windows")]
    Pipe(BufStream<NamedPipeClient>),
}

impl AsyncRead for RkvmStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {

        match self.get_mut() {
            RkvmStream::Tcp(stream) =>
                Pin::new(stream).poll_read(cx, buf),

            #[cfg(target_os = "windows")]
            RkvmStream::Pipe(stream) =>
                Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for RkvmStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {

        match self.get_mut() {
            RkvmStream::Tcp(stream) =>
                Pin::new(stream).poll_write(cx, buf),

            #[cfg(target_os = "windows")]
            RkvmStream::Pipe(stream) =>
                Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {

        match self.get_mut() {
            RkvmStream::Tcp(stream) =>
                Pin::new(stream).poll_flush(cx),

            #[cfg(target_os = "windows")]
            RkvmStream::Pipe(stream) =>
                Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {

        match self.get_mut() {
            RkvmStream::Tcp(stream) =>
                Pin::new(stream).poll_shutdown(cx),

            #[cfg(target_os = "windows")]
            RkvmStream::Pipe(stream) =>
                Pin::new(stream).poll_shutdown(cx),
        }
    }
}

#[async_trait]
pub trait RkvmWriter {
    async fn send(&mut self, update: Update) -> Result<(), Error>;
}

#[async_trait]
impl<W> RkvmWriter for W
where
    W: AsyncWrite + Unpin + Send,
{
    async fn send(&mut self, update: Update) -> Result<(), Error> {
        update.encode(self).await?;
        self.flush().await?;
        Ok(())
    }
}

pub struct SenderWriter {
    tx: Sender<Update>,
}

#[async_trait]
impl RkvmWriter for SenderWriter {
    async fn send(&mut self, update: Update) -> Result<(), Error> {
        self.tx.send(update).await.map_err(|e| io::Error::other(e.to_string()))?;
        Ok(())
    }
}

pub struct LockWriter<W> {
    w: Arc<Mutex<W>>
}

impl<W> LockWriter<W>  {
    pub fn new(w: Arc<Mutex<W>>) -> Self {
        LockWriter { w: w }
    }
}

#[async_trait]
impl<W> RkvmWriter for LockWriter<W>
where
    W: RkvmWriter + Send {
    async fn send(&mut self, update: Update) -> Result<(), Error> {
        self.w.lock().await.send(update).await
    }
}
