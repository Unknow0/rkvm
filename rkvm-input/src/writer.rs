use crate::abs::{AbsAxis, AbsInfo};
use crate::event::Event;
use crate::key::Key;
use crate::rel::RelAxis;

use std::ffi::CString;
use std::io::Error;

#[cfg(target_os = "windows")]
pub use {crate::windows::writer::WriterWindows as Writer, crate::windows::writer::WriterWindowsBuilder as WriterBuilder};
#[cfg(target_os = "linux")]
pub use {crate::linux::writer::WriterLinux as Writer, crate::linux::writer::WriterLinuxBuilder as WriterBuilder};

pub trait WriterPlatform {
    type Builder: WriterBuilderPlatform;

    fn builder() -> Result<Self::Builder, Error>;

    fn write<'a>(&'a mut self, event: &'a Event) -> impl std::future::Future<Output = Result<(), Error>> + Send + 'a;
}

pub trait WriterBuilderPlatform: Sized {
    type Writer: WriterPlatform;

    fn name(self, name: &CString) -> Self;

    fn vendor(self, value: u16) -> Self;

    fn product(self, value: u16) -> Self;

    fn version(self, value: u16) -> Self;

    fn rel<T: IntoIterator<Item = RelAxis>>(self, items: T) -> Result<Self, Error>;

    fn abs<T: IntoIterator<Item = (AbsAxis, AbsInfo)>>(self, items: T) -> Result<Self, Error>;

    fn key<T: IntoIterator<Item = Key>>(self, items: T) -> Result<Self, Error>;

    fn delay(self, value: Option<i32>) -> Result<Self, Error>;

    fn period(self, value: Option<i32>) -> Result<Self, Error>;

    fn build(self) -> impl std::future::Future<Output = Result<Self::Writer, Error>> + Send;
}
