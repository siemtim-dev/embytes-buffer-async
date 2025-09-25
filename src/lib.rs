#![cfg_attr(not(feature = "std"), no_std)]



use thiserror::Error;

use crate::{inner::BufferInner, mutex::{Mutex, MutexImpl}};

pub(crate) mod mutex;

mod reader;
pub use reader::*;

mod writer;
pub use writer::*;

mod inner;
pub use inner::*;

#[cfg(feature = "embedded")]
pub mod pipe;

// #[cfg(test)]
/// Just for tests
pub mod testutils;

/// Error enum 
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BufferError {

    #[error("Error writing to buffer: no remaining capacity")]
    NoCapacity,

    #[error("The provided slice to read from or write to has a len = 0")]
    ProvidedSliceEmpty,

    #[error("Error reading from buffer: no remaining data")]
    NoData,

    #[error("The requested operation has failed bacause a resource is locked")]
    Locked,
}

pub trait BufferSource: AsRef<[u8]> + AsMut<[u8]> + Send {}

impl <T> BufferSource for T where T: AsRef<[u8]> + AsMut<[u8]> + Send {}

/// An async Buffer implementation that can be concurrently read from and written to
pub struct AsyncBuffer <const C: usize, T: BufferSource> {
    pub(crate) inner: MutexImpl<BufferInner<C, T>>
}

impl <const C: usize, T: BufferSource> AsyncBuffer<C, T> {

    /// Creates a new [`AsyncBuffer`] from a provided source.
    /// the souce must have a non zero length.
    pub fn new(source: T) -> Self {
        assert!(source.as_ref().len() > 0);
        Self {
            inner: MutexImpl::new(BufferInner::new(source))
        }
    }
}

impl <const C: usize, const N: usize> AsyncBuffer<C, [u8; N]> {

    pub fn new_stack() -> Self {
        Self::new([0; N])
    }
    
}

impl <const C: usize, T: BufferSource>  Buffer for AsyncBuffer<C, T> {
    /// Creates a [`BufferReader`] to read from the async buffer.
    /// It is not crecommended to have more than one reader.
    fn create_reader<'a>(&'a self) -> impl BufferRead + 'a {
        BufferReader::new(self)
    }

    /// Creates a [`BufferWriter`] to write to the async buffer.
    /// It is not crecommended to have more than one writer.
    fn create_writer<'a>(&'a self) -> impl writer::BufferWrite + 'a {
        BufferWriter::new(self)
    }

    fn lock<'a>(&'a self) -> impl Future<Output = impl inner::RWLock + 'a> {
        ReadWriteLockFuture::new(self)
    }
}

pub trait Buffer {
    /// Creates a [`BufferRead`] to read from the async buffer.
    /// It is not crecommended to have more than one reader.
    fn create_reader<'a>(&'a self) -> impl BufferRead + 'a;

    /// Creates a [`BufferWrite`] to write to the async buffer.
    /// It is not crecommended to have more than one writer.
    fn create_writer<'a>(&'a self) -> impl BufferWrite + 'a;

    fn lock<'a>(&'a self) -> impl Future<Output = impl RWLock + 'a>;
}
