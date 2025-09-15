use core::{pin::Pin, task::{Context, Poll}};

use crate::{mutex::Mutex, AsyncBuffer, BufferError};

pub trait BufferRead {
    
    /// Ready from the currently available data in the [`BufferReader`].
    /// The readable part of the buffer is passed to `f`. 
    /// `f` returns the number of bytes read and a result that is passed back to the caller.
    fn read_slice<F, U>(&self, f: F) -> Result<U, BufferError> where F: FnOnce(&[u8]) -> (usize, U);

    fn read_slice_async<F, U>(&self, f: F) -> impl Future<Output = Result<U, BufferError>> where F: FnMut(&[u8]) -> ReadSliceAsyncResult<U>;

    fn pull(&self, buf: &mut[u8]) -> impl Future<Output = Result<(), BufferError>>;

    fn wait_for_new_data<'b>(&'b self) -> impl Future<Output = Result<(), BufferError>>;

    fn reset(&self);
}

/// A type to read from an [`AsyncBuffer`]
pub struct BufferReader<'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    buffer: &'a AsyncBuffer<C, T>
}

pub enum ReadSliceAsyncResult<T> {
    Wait,
    Ready(usize, T),
}

impl <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> BufferReader<'a, C, T> {

    pub(super) fn new(buffer: &'a AsyncBuffer<C, T>) -> Self {
        Self {
            buffer: buffer
        }
    }  

    /// A poll method to implement a `ReadSliceAsync`-[`Future`]
    /// The readable part of the buffer is passed to `f`. 
    /// `f` returns [`ReadSliceAsyncResult::Wait`] if the function should wait for new data.
    /// `f` returns [`ReadSliceAsyncResult::Ready`] the number of bytes read and a result that is passed back to the caller.
    fn poll_read_slice<F, U>(&self, f: &mut F, cx:&mut Context<'_> ) -> Poll<Result<U, BufferError>> 
    where F: FnMut(&[u8]) -> ReadSliceAsyncResult<U> {
        self.buffer.inner.lock_mut(|inner| {
            let readable = inner.readable_data();
            match f(readable) {
                ReadSliceAsyncResult::Wait => {
                    inner.add_read_waker(cx);
                    Poll::Pending
                },
                ReadSliceAsyncResult::Ready(bytes_read, result) => Poll::Ready(
                    inner.read_commit(bytes_read).map(|_| result)
                ),
            }
        })
    }

    /// Base function for implementing readers like [`embedded_io::Read`]
    /// Returns the number of bytes read from the buffer to the provided slice
    /// 
    /// # Errors
    /// 
    /// [`BufferError::ProvidedSliceEmpty`] if the provided slice is empty
    /// [`BufferError::NoData`] if there ae no bytes to read
    fn read_base(&self, buf: &mut[u8]) -> Result<usize, BufferError> {
        if buf.is_empty() {
            return Err(BufferError::ProvidedSliceEmpty);
        }

        self.buffer.inner.lock_mut(|inner|{
            let src = inner.readable_data();

            if src.is_empty() {
                return Err(BufferError::NoData);
            }
            else if src.len() > buf.len() {
                buf.copy_from_slice(&src[0..buf.len()]);
                inner.read_commit(buf.len()).unwrap();
                Ok(buf.len())
            } else {
                let buf = &mut buf[0..src.len()];
                buf.copy_from_slice(src);
                let bytes_read = src.len();
                inner.read_commit(bytes_read).unwrap();
                Ok(bytes_read)
            }
        })
    }

    /// Base function to implement async read functionality like [`embedded_io_async::Read`]
    fn poll_read(&self, buf: &mut[u8], cx: &mut Context<'_>) -> Poll<Result<usize, BufferError>>{
        if buf.is_empty() {
            return Poll::Ready(Err(BufferError::ProvidedSliceEmpty));
        }

        self.buffer.inner.lock_mut(|inner|{
            let src = inner.readable_data();

            if src.is_empty() {
                inner.add_read_waker(cx);
                Poll::Pending
            }
            else if src.len() > buf.len() {
                buf.copy_from_slice(&src[0..buf.len()]);
                inner.read_commit(buf.len()).unwrap();

                Poll::Ready(Ok(buf.len()))
            } else {
                let buf = &mut buf[0..src.len()];
                buf.copy_from_slice(src);
                let bytes_read = src.len();
                inner.read_commit(bytes_read).unwrap();

                Poll::Ready(Ok(bytes_read))
            }
        })
    }

    /// Base poll function to implement a pull future
    /// a pull future wait until `buf.len()` bytes are available
    fn poll_pull(&self, buf: &mut[u8], cx: &mut Context<'_>) -> Poll<Result<(), BufferError>> {
        self.buffer.inner.lock_mut(|inner|{
            if inner.capacity() < buf.len() {
                return Poll::Ready(Err(BufferError::NoCapacity));
            }
            
            let readable = inner.readable_data();
            if readable.len() >= buf.len() {
                let readable = &readable[..buf.len()];
                buf.copy_from_slice(readable);
                inner.read_commit(buf.len()).unwrap();
                Poll::Ready(Ok(()))
            } else {
                // println!("poll_pull: waiting: {} bytes readable but {} bytes required", readable.len(), buf.len());
                inner.add_read_waker(cx);
                Poll::Pending
            }
        })
    }
}

impl <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> BufferRead for BufferReader<'a, C, T> {

    fn read_slice<F, U>(&self, f: F) -> Result<U, BufferError> where F: FnOnce(&[u8]) -> (usize, U){
        self.buffer.inner.lock_mut(|inner|{
            let readable = inner.readable_data();
            let (bytes_read, result) = f(readable);
            inner.read_commit(bytes_read)
                .map(|_| result )
        })
    }
    
    fn read_slice_async<F, U>(&self, f: F) -> impl Future<Output = Result<U, BufferError>> 
    where F: FnMut(&[u8]) -> ReadSliceAsyncResult<U> {
        ReadSliceAsyncFuture{
            reader: self,
            f
        }
    }
    
    fn pull(&self, buf: &mut[u8]) -> impl Future<Output = Result<(), BufferError>> {
        PullDataFuture{
            reader: self,
            buf: buf
        }
    }
    
    fn reset(&self) {
        self.buffer.inner.lock_mut(|inner| inner.reset());
    }

    fn wait_for_new_data<'b>(&'b self) -> impl Future<Output = Result<(), BufferError>> {
        self.buffer.inner.lock(|inner|{
            NewDataFuture{
                reader: self,
                old_len: inner.len()
            }
        })
    }
}

pub struct ReadSliceAsyncFuture<'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>, F, U> where F: FnMut(&[u8]) -> ReadSliceAsyncResult<U> {
    reader: &'b BufferReader<'a, C, T>,
    f: F
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>, F, U> Unpin for ReadSliceAsyncFuture<'a, 'b, C, T, F, U> where F: FnMut(&[u8]) -> ReadSliceAsyncResult<U> {}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>, F, U> Future for ReadSliceAsyncFuture<'a, 'b, C, T, F, U> where F: FnMut(&[u8]) -> ReadSliceAsyncResult<U> {
    type Output = Result<U, BufferError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.reader.poll_read_slice(&mut self.f, cx)
    }
}

pub struct NewDataFuture <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    reader: &'b BufferReader<'a, C, T>,
    old_len: usize
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Unpin for NewDataFuture<'a, 'b, C, T> {}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Future for NewDataFuture<'a, 'b, C, T> {
    type Output = Result<(), BufferError>;

    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.reader.buffer.inner.lock_mut(|inner|{
            if self.old_len < inner.len() {
                Poll::Ready(Ok(()))
            } else if inner.capacity() <= self.old_len {
                Poll::Ready(Err(BufferError::NoCapacity))
            } else {
                inner.add_read_waker(cx);
                Poll::Pending
            }
        })
    }
}

pub struct PullDataFuture <'a, 'b, 'c, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    reader: &'b BufferReader<'a, C, T>,
    buf: &'c mut [u8]
}

impl <'a, 'b, 'c, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Unpin for PullDataFuture<'a, 'b, 'c, C, T> {}

impl <'a, 'b, 'c, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Future for PullDataFuture<'a, 'b, 'c, C, T> {
    type Output = Result<(), BufferError>;

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.reader.poll_pull(self.buf, cx)
    }
}

#[cfg(feature = "embedded")]
impl <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> embedded_io::ErrorType for BufferReader<'a, C, T> {
    type Error = embedded_io::ErrorKind;
}

#[cfg(feature = "embedded")]
pub struct EmbeddedReadFuture <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    reader: &'a BufferReader<'a, C, T>,
    buf: &'b mut [u8]
}

#[cfg(feature = "embedded")]
impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Unpin for EmbeddedReadFuture<'a, 'b, C, T> {}

#[cfg(feature = "embedded")]
impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Future for EmbeddedReadFuture<'a, 'b, C, T> {
    type Output = Result<usize, embedded_io::ErrorKind>;

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.reader.poll_read(&mut self.buf, cx)
            .map(|err| match err {
                Ok(n) => Ok(n),
                Err(BufferError::ProvidedSliceEmpty) => Ok(0),
                Err(err) => panic!("unexpected err returned from poll_read(): {}", err)
            })
    }
}

#[cfg(feature = "embedded")]
impl <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> embedded_io_async::Read for BufferReader<'a, C, T> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let f = EmbeddedReadFuture{
            reader: self,
            buf: buf
        };
        f.await
    }
}

#[cfg(feature = "std")]
impl <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> std::io::Read for BufferReader<'a, C, T> {
    
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::io::ErrorKind;

        match self.read_base(buf) {
            Ok(n) => Ok(n),
            Err(BufferError::ProvidedSliceEmpty) => Ok(0),
            Err(BufferError::NoData) => Err(ErrorKind::WouldBlock.into()),
            Err(e) => {
                panic!("unexpected error reading from buffer: {}", e);
            }
        }
    }
}