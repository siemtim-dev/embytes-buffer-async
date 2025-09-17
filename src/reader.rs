use core::{cell::Cell, ops::Deref, pin::Pin, slice::from_raw_parts, task::{Context, Poll}};

use crate::{mutex::Mutex, AsyncBuffer, BufferError};

pub trait BufferRead {
    
    /// Ready from the currently available data in the [`BufferReader`].
    /// The readable part of the buffer is passed to `f`. 
    /// `f` returns the number of bytes read and a result that is passed back to the caller.
    fn read_slice<F, U>(&self, f: F) -> Result<U, BufferError> where F: FnOnce(&[u8]) -> (usize, U);

    fn read_slice_async<F, U>(&self, f: F) -> impl Future<Output = Result<U, BufferError>> where F: FnMut(&[u8]) -> ReadSliceAsyncResult<U>;

    fn pull(&self, buf: &mut[u8]) -> impl Future<Output = Result<(), BufferError>>;

    fn wait_for_new_data<'b>(&'b self) -> impl Future<Output = Result<(), BufferError>> + Send;

    fn try_reset(&self) -> Result<(), BufferError>;

    fn lock(&self) -> impl Future<Output = impl RLock>;

    fn len(&self) -> usize;
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
            if let Some(readable) = readable {
                match f(readable) {
                    ReadSliceAsyncResult::Wait => {
                        inner.add_read_waker(cx);
                        Poll::Pending
                    },
                    ReadSliceAsyncResult::Ready(bytes_read, result) => Poll::Ready(
                        inner.read_commit(bytes_read).map(|_| result)
                    ),
                }
            } else {
                inner.add_read_waker(cx);
                Poll::Pending
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

            if let Some(src) = src {
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
            } else {
                inner.add_read_waker(cx);
                Poll::Pending
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
            match readable {
                Some(readable) if readable.len() >= buf.len() => {
                    let readable = &readable[..buf.len()];
                    buf.copy_from_slice(readable);
                    inner.read_commit(buf.len()).unwrap();
                    Poll::Ready(Ok(()))
                },
                _ => {
                    inner.add_read_waker(cx);
                    Poll::Pending
                }
            }
        })
    }
}

impl <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]> + Send> BufferRead for BufferReader<'a, C, T> {

    fn read_slice<F, U>(&self, f: F) -> Result<U, BufferError> where F: FnOnce(&[u8]) -> (usize, U){
        self.buffer.inner.lock_mut(|inner|{
            if let Some(readable) = inner.readable_data() {
                let (bytes_read, result) = f(readable);
                inner.read_commit(bytes_read)
                    .map(|_| result )
            } else {
                Err(BufferError::Locked)
            }
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
    
    fn try_reset(&self) -> Result<(), BufferError> {
        self.buffer.inner.lock_mut(|inner| inner.try_reset())
    }

    fn wait_for_new_data<'b>(&'b self) -> impl Future<Output = Result<(), BufferError>> + Send {
        self.buffer.inner.lock(|inner|{
            NewDataFuture{
                reader: self,
                old_len: inner.len()
            }
        })
    }
    
    fn lock(&self) -> impl Future<Output = impl RLock> {
        ReadLockFuture {
            reader: self
        }
    }
    
    fn len(&self) -> usize {
        self.buffer.inner.lock(|inner| inner.len())
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
        self.reader.buffer.inner.lock_mut(|inner| {
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

pub trait RLock: Deref<Target = [u8]> + Send {
    fn set_bytes_read(&self, bytes_read: usize) -> Result<(), BufferError>;
    fn wait_for_new_data(self) -> impl Future<Output = Result<(), BufferError>> + Send;
}

pub struct ReadLock<'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    reader: &'b BufferReader<'a, C, T>,
    data: *const u8,
    len: usize,
    bytes_read: Cell<usize>
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> ReadLock<'a, 'b, C, T> {
    fn new(reader: &'b BufferReader<'a, C, T>, data: *const u8, len: usize,) -> Self {
        Self {
            data: data,
            reader: reader,
            len: len,
            bytes_read: Cell::new(0)
        }
    }
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]> + Send> RLock for ReadLock<'a, 'b, C, T> {
    fn set_bytes_read(&self, bytes_read: usize) -> Result<(), BufferError> {
        if bytes_read > self.len {
            Err(BufferError::NoData)
        } else {
            self.bytes_read.set(bytes_read);
            Ok(())
        }
    }

    fn wait_for_new_data(self) -> impl Future<Output = Result<(), BufferError>> + Send {
        let old_len = self.reader.buffer.inner.lock_mut(|inner| {
            inner.read_commit(self.bytes_read.get()).unwrap();
            self.bytes_read.set(0);
            inner.len()
        });

        NewDataFuture{
            reader: self.reader,
            old_len: old_len
        }
    }
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Deref for ReadLock<'a, 'b, C, T> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        unsafe {
            from_raw_parts(self.data, self.len)
        }
    }
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Drop for ReadLock<'a, 'b, C, T> {
    fn drop(&mut self) {
        self.reader.buffer.inner.lock_mut(|inner| {
            inner.read_commit(self.bytes_read.get()).unwrap();
            unsafe { inner.read_unlock() };
        });
    }
}

unsafe impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Send for ReadLock<'a, 'b, C, T> {}

pub struct ReadLockFuture <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    reader: &'b BufferReader<'a, C, T>
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Unpin for ReadLockFuture<'a, 'b, C, T> {}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Future for ReadLockFuture<'a, 'b, C, T> {
    type Output = ReadLock<'a, 'b, C, T>;

    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.reader.buffer.inner.lock_mut(|inner| {
            inner.poll_read_lock(cx).map(|(data, len)| ReadLock::new(self.reader, data, len))
        })
    }
}