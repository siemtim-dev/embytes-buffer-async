
use core::{pin::Pin, task::{Context, Poll}};

use crate::{mutex::Mutex, AsyncBuffer, BufferError};

pub enum WriteSliceAsyncResult {
    Wait,
    Ready(usize),
}

pub struct BufferWriter <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    buffer: &'a AsyncBuffer<C, T>
}
 
impl <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> BufferWriter<'a, C, T> {

    pub(crate) fn new(buffer: &'a AsyncBuffer<C, T>) -> Self {
        Self {
            buffer: buffer
        }
    }

    /// Base function for implementing writers like [`embedded_io::Write`]
    /// Returns the number of bytes writen to the buffer from the provided slice
    /// 
    /// # Errors
    /// 
    /// [`BufferError::ProvidedSliceEmpty`] if the provided slice is empty
    /// [`BufferError::NoCapacity`] if the buffer has no capacity after calling [`Buffer::shift`]
    fn write_base(&mut self, buf: &[u8]) -> Result<usize, BufferError> {
        if buf.is_empty() {
            return Err(BufferError::ProvidedSliceEmpty);
        }

        self.buffer.inner.lock_mut(|inner|{
            let cap = inner.maybe_shift();

            if cap == 0 {
                return Err(BufferError::NoCapacity);
            }

            let tgt = inner.writeable_data();
            
            if cap < buf.len() {
                tgt.copy_from_slice(&buf[0..cap]);
                inner.write_commit(cap)?;
                Ok(cap)
            } else {
                let tgt = &mut tgt[0..buf.len()];
                tgt.copy_from_slice(buf);
                inner.write_commit(cap)?;
                Ok(buf.len())
            }
        })
    }

    fn poll_write(&self, buf: &[u8], cx: &mut Context<'_>) -> Poll<Result<usize, BufferError>> {
        if buf.is_empty() {
            return Poll::Ready(Err(BufferError::ProvidedSliceEmpty));
        }

        self.buffer.inner.lock_mut(|inner|{
            let cap = match inner.poll_ensure_capacity(cx, 1) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(n)) => n,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
            };

            let tgt = inner.writeable_data();
            
            if cap < buf.len() {
                tgt.copy_from_slice(&buf[0..cap]);
                inner.write_commit(cap).unwrap();
                Poll::Ready(Ok(cap))
            } else {
                let tgt = &mut tgt[0..buf.len()];
                tgt.copy_from_slice(buf);
                inner.write_commit(buf.len()).unwrap();
                Poll::Ready(Ok(buf.len()))
            }
        })
    }

    pub fn push(&self, data: &[u8]) -> Result<(), BufferError> {
        self.buffer.inner.lock_mut(|inner| {
            if inner.has_capacity(data.len()) {
                let tgt = inner.writeable_data();
                let tgt = &mut tgt[..data.len()];
                tgt.copy_from_slice(data);
                inner.write_commit(data.len())
                    .expect("must not throw because capacity is checked before");
                Ok(())
            } else {
                Err(BufferError::NoCapacity)
            }
        })
    }

    pub fn push_async<'b, 'c>(&'b self, data: &'c [u8]) -> PushFuture<'a, 'b, 'c, C, T> {
        PushFuture {
            writer: self,
            data: data
        }
    } 

    fn poll_push(&self, data: &[u8], cx: &mut Context<'_>) -> Poll<Result<(), BufferError>> {
        self.buffer.inner.lock_mut(|inner| {
            if data.len() > inner.capacity() {
                return Poll::Ready(Err(BufferError::NoCapacity));
            }
            
            inner.shift();

            let writeable = inner.writeable_data();
            if writeable.len() < data.len() {
                // println!("poll_push: waiting: {} bytes writeable but {} bytes of data", writeable.len(), data.len());
                inner.add_write_waker(cx);
                Poll::Pending
            } else {
                writeable[..data.len()].copy_from_slice(data);
                inner.write_commit(data.len()).unwrap();
                Poll::Ready(Ok(()))
            }
        })
    }

    pub fn write_slice<F>(&self, f: F) -> Result<(), BufferError> where F: FnOnce(&mut [u8]) -> usize {
        self.buffer.inner.lock_mut(|inner| {
            inner.shift();
            let writeable = inner.writeable_data();
            let bytes_written = f(writeable);
            if bytes_written > writeable.len() {
                Err(BufferError::NoCapacity)
            } else {
                inner.write_commit(bytes_written).unwrap();
                Ok(())
            }
        })
    }

    pub fn write_slice_async<F>(&self, f: F) -> impl Future<Output = Result<(), BufferError>> where F: FnMut(&mut [u8]) -> WriteSliceAsyncResult{
        WriteSliceAsyncFuture{
            writer: self,
            f: f
        }
    }

    fn poll_write_slice<F>(&self, f: &mut F, cx: &mut Context<'_>) -> Poll<Result<(), BufferError>> where F: FnMut(&mut [u8]) -> WriteSliceAsyncResult {
        self.buffer.inner.lock_mut(|inner| {
            inner.shift();
            let writeable = inner.writeable_data();
            match f(writeable) {
                WriteSliceAsyncResult::Wait => if writeable.len() < inner.capacity() {
                        inner.add_write_waker(cx);
                        Poll::Pending
                    } else {
                        Poll::Ready(Err(BufferError::NoCapacity))
                    },
                WriteSliceAsyncResult::Ready(bytes_written) if bytes_written > writeable.len() => {
                    Poll::Ready(Err(BufferError::NoCapacity))
                },
                WriteSliceAsyncResult::Ready(bytes_written) => {
                    inner.write_commit(bytes_written).unwrap();
                    Poll::Ready(Ok(()))
                }
            }
        })
    }

    pub fn await_capacity<'b>(&'b self, expected_capacity: usize) -> CapacityFuture<'a, 'b, C, T> {
        CapacityFuture {
            writer: self,
            expected_capacity: expected_capacity
        }
    }

    pub fn reset(&self) {
        self.buffer.inner.lock_mut(|inner| inner.reset());
    }
}

pub struct WriteSliceAsyncFuture<'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>, F> where F: FnMut(&mut [u8]) -> WriteSliceAsyncResult {
    writer: &'b BufferWriter<'a, C, T>,
    f: F
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>, F> Unpin for WriteSliceAsyncFuture<'a, 'b, C, T, F> where F: FnMut(&mut [u8]) -> WriteSliceAsyncResult {}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>, F> Future for WriteSliceAsyncFuture<'a, 'b, C, T, F> where F: FnMut(&mut [u8]) -> WriteSliceAsyncResult {
    type Output = Result<(), BufferError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.writer.poll_write_slice(&mut self.f, cx)
    }
}

#[cfg(feature = "embedded")]
impl <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> embedded_io::ErrorType for BufferWriter<'a, C, T> {
    type Error = embedded_io::ErrorKind;
}

#[cfg(feature = "embedded")]
struct EmbeddedWriteFuture<'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    writer: &'a BufferWriter<'a, C, T>,
    buf: &'b [u8]
}

#[cfg(feature = "embedded")]
impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Unpin for EmbeddedWriteFuture<'a, 'b, C, T> {}

#[cfg(feature = "embedded")]
impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Future for EmbeddedWriteFuture<'a, 'b, C, T> {
    type Output = Result<usize, embedded_io::ErrorKind>;
    
    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.writer.poll_write(self.buf, cx)
            .map(|result| match result {
                Ok(n) => Ok(n),
                Err(BufferError::ProvidedSliceEmpty) => Ok(0),
                Err(err) => panic!("unexpected err returned from poll_write(): {}", err)
            })
    }
}

#[cfg(feature = "embedded")]
impl <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> embedded_io_async::Write for BufferWriter<'a, C, T> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let f = EmbeddedWriteFuture{
            writer: self,
            buf: buf
        };

        f.await
    }
}

#[cfg(feature = "std")]
impl <'a, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> std::io::Write for BufferWriter<'a, C, T> {

    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use std::io::ErrorKind;
        match self.write_base(buf) {
            Ok(n) => Ok(n),
            Err(BufferError::ProvidedSliceEmpty) => Ok(0),
            Err(BufferError::NoCapacity) => Err(ErrorKind::WouldBlock.into()),
            Err(e) => {
                panic!("unexpected error writing to buffer: {}", e);
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct CapacityFuture <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    writer: &'b BufferWriter<'a, C, T>,
    expected_capacity: usize
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Unpin for CapacityFuture<'a, 'b, C, T> {}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Future for CapacityFuture<'a, 'b, C, T> {
    type Output = Result<(), BufferError>;

    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.writer.buffer.inner.lock_mut(|inner|{
            inner.poll_ensure_capacity(cx, self.expected_capacity)
                .map_ok(|_| ())
        })
    }
}

pub struct PushFuture <'a, 'b, 'c, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    writer: &'b BufferWriter<'a, C, T>,
    data: &'c [u8]
}

impl <'a, 'b, 'c, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Unpin for PushFuture<'a, 'b, 'c, C, T> {}

impl <'a, 'b, 'c, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> Future for PushFuture<'a, 'b, 'c, C, T> {
    type Output = Result<(), BufferError>;

    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.writer.poll_push(self.data, cx)
    }
}
