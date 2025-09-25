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

/// An async Buffer implementation that can be concurrently read from and written to
pub struct AsyncBuffer <const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    pub(crate) inner: MutexImpl<BufferInner<C, T>>
}

impl <const C: usize, T: AsRef<[u8]> + AsMut<[u8]>>  AsyncBuffer<C, T> {

    /// Creates a new [`AsyncBuffer`] from a provided source.
    /// the souce must have a non zero length.
    pub fn new(source: T) -> Self {
        assert!(source.as_ref().len() > 0);
        Self {
            inner: MutexImpl::new(BufferInner::new(source))
        }
    }

    /// Creates a [`BufferReader`] to read from the async buffer.
    /// It is not crecommended to have more than one reader.
    pub fn create_reader<'a>(&'a self) -> BufferReader<'a, C, T> {
        BufferReader::new(self)
    }

    /// Creates a [`BufferWriter`] to write to the async buffer.
    /// It is not crecommended to have more than one writer.
    pub fn create_writer<'a>(&'a self) -> BufferWriter<'a, C, T> {
        BufferWriter::new(self)
    }

    pub fn lock<'a>(&'a self) -> impl Future<Output = ReadWriteLock<'a, C, T>> {
        ReadWriteLockFuture::new(self)
    }
}

impl <const C: usize, const N: usize>  AsyncBuffer<C, [u8; N]> {

    pub fn new_stack() -> Self {
        Self::new([0; N])
    }
    
}
