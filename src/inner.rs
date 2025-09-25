use core::{ops::{Deref, DerefMut}, slice::from_raw_parts_mut, task::{Context, Poll}};

use crate::{mutex::Mutex, wakers::WakerRegistration, AsyncBuffer, BufferError, BufferSource, WLock};


/// This struct contains the inner state of the buffer
pub(crate) struct BufferInner <const C: usize, T: BufferSource> {
    
    /// The index where to start read operations
    read_position: usize,

    /// The index where to start write operations
    write_position: usize,

    /// The underlying memory
    source: T,

    /// wakers registered from waiting readers that wait for new data
    read_wakers: WakerRegistration<C>,

    /// wakers registered from waiting writers that wait for new space to write to
    write_wakers: WakerRegistration<C>,

    read_loked: bool,

    write_locked: bool
    
}

impl <const C: usize, T: BufferSource> BufferInner<C, T> {
    pub(super) fn new(source: T) -> Self {
        Self {
            read_position: 0,
            write_position: 0,
            source: source,
            
            read_wakers: WakerRegistration::new(),
            write_wakers: WakerRegistration::new(),

            read_loked: false,
            write_locked: false
        }
    }

    pub(crate) fn try_reset(&mut self) -> Result<(), BufferError> {
        if self.read_loked || self.write_locked {
            Err(BufferError::Locked)
        } else {
            self.reset();
            Ok(())
        }
    }

    fn reset(&mut self) {
        self.read_position = 0;
        self.write_position = 0;
        self.wake_readers();
        self.wake_writers();
    }

    /// Function used for unsafe reset
    /// Only self when a read write lock is aquired
    unsafe fn reset_unchecked(&mut self) {
        self.reset();
    }

    pub(crate) fn poll_shift(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if self.read_loked || self.write_locked {
            self.add_write_waker(cx);
            Poll::Pending
        } else {
            unsafe { self.shift() };
            Poll::Ready(())
        }
    }

    /// Shifs all elemts to the left to reuse dead capacity.
    /// returns the new capacity
    unsafe fn shift(&mut self) {
        if self.read_position > 0 {
            self.source.as_mut().rotate_left(self.read_position);
            self.write_position -= self.read_position;
            self.read_position = 0;
        }
    }

    pub(crate) fn try_shift(&mut self) -> Result<(), BufferError> {
        if self.read_loked || self.write_locked {
            Err(BufferError::Locked)
        } else {
            unsafe { self.shift() };
            Ok(())
        }
    }

    pub(crate) fn remaining_capacity(&self) -> usize {
        self.capacity() - self.write_position
    }

    /// Returns the total capacity of the underlying source
    pub(crate) fn capacity(&self) -> usize {
        self.source.as_ref().len()
    }

    /// Performs a poll which is ready when at least `required_capaciyt` is writeable
    /// returns an error if `required_capaciyt > self.capacity()`
    /// 
    /// This function is only for write operations because it registers a write waker.
    pub(crate) fn poll_ensure_capacity(&mut self, cx: &mut Context<'_>, required_capaciyt: usize) -> Poll<Result<usize, BufferError>> {
        if self.write_locked {
            self.add_write_waker(cx);
            return Poll::Pending;
        }
        
        if self.remaining_capacity() >= required_capaciyt {
            return Poll::Ready(Ok(self.remaining_capacity()))
        }

        if self.capacity() < required_capaciyt {
            return Poll::Ready(Err(BufferError::NoCapacity));
        }

        if let Poll::Pending = self.poll_shift(cx) {
            return Poll::Pending;
        }

        if self.remaining_capacity() >= required_capaciyt {
            Poll::Ready(Ok(self.remaining_capacity()))
        } else {
            self.add_write_waker(cx);
            Poll::Pending
        }
    }

    

    pub(crate) fn has_capacity(&self, required_capaciyt: usize) -> bool {
        self.remaining_capacity() >= required_capaciyt
    }

    /// Returns the has remaining capacity of this [`BufferInner<C, T>`].
    // pub(crate) fn has_remaining_capacity(&self) -> bool {
    //     self.remaining_capacity() > 0
    // }

    /// returns the length of the readable data
    pub(crate) fn len(&self) -> usize {
        self.write_position - self.read_position
    }

    pub(crate) fn add_write_waker(&mut self, cx: &mut Context<'_>) {
        self.write_wakers.register(cx.waker());
    }

    pub(crate) fn wake_writers(&mut self) {
        self.write_wakers.wake();
    }

    pub(crate) fn add_read_waker(&mut self, cx: &mut Context<'_>) {
        self.read_wakers.register(cx.waker());
    }

    pub(crate) fn wake_readers(&mut self) {
        self.read_wakers.wake();
    }

    /// Returns the readable data as a slice
    pub(crate) fn readable_data(&self) -> Option<&[u8]> {
        if self.read_loked {
            None
        } else {
            Some(&self.source.as_ref()[self.read_position..self.write_position])
        }
    }

    /// Only self when a read write lock is aquired
    // pub(crate) unsafe fn readable_data_unchecked(&self) -> &[u8] {
    //     &self.source.as_ref()[self.read_position..self.write_position]
    // }

    /// Only self when a read write lock is aquired
    pub(crate) unsafe fn writeable_data_unchecked(&mut self) -> &mut [u8] {
        &mut self.source.as_mut()[self.write_position..]
    }

    pub(crate) fn writeable_data(&mut self) -> Option<&mut [u8]> {
        if self.write_locked {
            None
        } else {
            Some(&mut self.source.as_mut()[self.write_position..])
        }
    }

    pub(crate) fn write_commit(&mut self, bytes_written: usize) -> Result<(), BufferError> {
        if bytes_written > self.remaining_capacity() {
            Err(BufferError::NoCapacity)
        } else {
            self.write_position += bytes_written;
            self.wake_readers();
            Ok(())
        }
    }

    pub(crate) fn read_commit(&mut self, bytes_read: usize) -> Result<(), BufferError> {
        if bytes_read > self.len() {
            Err(BufferError::NoData)
        } else {
            self.read_position += bytes_read;
            self.wake_writers();
            Ok(())
        }
    }

    pub(crate) fn poll_read_lock(&mut self, cx: &mut Context<'_>) -> Poll<(*const u8, usize)> {
        if self.read_loked {
            self.add_read_waker(cx);
            Poll::Pending
        } else {
            Poll::Ready(self.read_lock())
        }
    }

    pub(crate) fn read_lock(&mut self) -> (*const u8, usize) {
        let readable = self.readable_data()
            .expect("can only lock for reading if not already locked");
        let result = (readable.as_ptr(), readable.len());
        self.read_loked = true;
        result
    }

    pub(crate) fn poll_write_lock(&mut self, cx: &mut Context<'_>) -> Poll<(*mut u8, usize)> {
        if self.write_locked {
            self.add_write_waker(cx);
            return Poll::Pending;
        } 
        
        // only shift if there is dead capacity
        if self.read_position > 0 {
            match self.try_shift() {
                Ok(()) => {}, // Shift successful
                Err(BufferError::Locked) => {
                    self.add_write_waker(cx);
                    return Poll::Pending;
                },
                Err(err) => panic!("unexpected error returned from try_shift: {}", err)
            }
        }

        Poll::Ready(self.write_lock())
    }

    pub(crate) fn write_lock(&mut self) -> (*mut u8, usize) {
        let readable = self.writeable_data()
            .expect("can only lock for writing if not already locked");
        let result = (readable.as_mut_ptr(), readable.len());
        self.write_locked = true;
        result
    }

    // pub(crate) fn is_read_locked(&self) -> bool {
    //     self.read_loked
    // }

    pub(crate) fn is_write_locked(&self) -> bool {
        self.write_locked
    }

    pub(crate) unsafe fn read_unlock(&mut self) {
        assert!(self.read_loked, "read_unlock makes no sense when not locked");
        self.read_loked = false;
        self.wake_readers();
        self.wake_writers();
    }

    pub(crate) unsafe fn write_unlock(&mut self) {
        assert!(self.write_locked, "read_unlock makes no sense when not locked");
        self.write_locked = false;
        // self.wake_readers();
        self.wake_writers();
    }

    pub(crate) unsafe fn read_write_unlock(&mut self) {
        assert!(self.write_locked, "read_write_unlock makes no sense when write not locked");
        assert!(self.read_loked, "read_write_unlock makes no sense when read not locked");
        self.write_locked = false;
        self.read_loked = false;
        self.wake_readers();
        self.wake_writers();
    }
}


pub struct ReadWriteLockFuture<'a, const C: usize, T: BufferSource> {
    buffer: &'a AsyncBuffer<C, T>,
    has_read_lock: bool,
    has_write_lock: bool
}

impl <'a, const C: usize, T: BufferSource> ReadWriteLockFuture<'a, C, T> {
    pub(super) fn new(buffer: &'a AsyncBuffer<C, T>) -> Self {
        Self {
            buffer: buffer,
            has_read_lock: false,
            has_write_lock: false
        }
    }
}

impl <'a, const C: usize, T: BufferSource> Unpin for ReadWriteLockFuture<'a, C, T> {}

impl <'a, const C: usize, T: BufferSource> Future for ReadWriteLockFuture<'a, C, T> {
    type Output = ReadWriteLock<'a, C, T>;

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.buffer.inner.lock_mut(|inner| {
            if ! self.has_read_lock {
                if inner.read_loked {
                    inner.add_read_waker(cx);
                } else {
                    inner.read_loked = true;
                    self.has_read_lock = true;
                }
            }

            if ! self.has_write_lock {
                if inner.write_locked {
                    inner.add_write_waker(cx);
                } else {
                    inner.write_locked = true;
                    self.has_write_lock = true;
                }
            }

            if self.has_read_lock && self.has_write_lock {
                Poll::Ready(ReadWriteLock {
                    buffer: self.buffer
                })
            } else {
                Poll::Pending
            }

        })
    }
}

pub trait RWLock {
    fn reset(&self);
    fn writeable_data<'b>(&'b self) -> impl WLock + 'b;
}

pub struct ReadWriteLock <'a, const C: usize, T: BufferSource> {
    buffer: &'a AsyncBuffer<C, T>
}

impl <'a, const C: usize, T: BufferSource> RWLock for ReadWriteLock<'a, C, T> {
    fn writeable_data<'b>(&'b self) -> impl WLock + 'b {
        let (data, len) = self.buffer.inner.lock_mut(|inner| unsafe {
            let writeable = inner.writeable_data_unchecked();
            (writeable.as_mut_ptr(), writeable.len())
        });

        RWWriteLock::new(self, data, len)
    }

    fn reset(&self) {
        self.buffer.inner.lock_mut(|inner| {
            unsafe { inner.reset_unchecked() };
        });
    }
}

impl <'a, const C: usize, T: BufferSource> Drop for ReadWriteLock<'a, C, T> {
    fn drop(&mut self) {
        self.buffer.inner.lock_mut(|inner| {
            unsafe { inner.read_write_unlock() };
        })
    }
}

pub struct RWWriteLock<'a, 'b, const C: usize, T: BufferSource> {
    writer: &'b ReadWriteLock<'a, C, T>,
    data: *mut u8,
    len: usize,
}

impl <'a, 'b, const C: usize, T: BufferSource> RWWriteLock<'a, 'b, C, T> {
    fn new(writer: &'b ReadWriteLock<'a, C, T>, data: *mut u8, len: usize,) -> Self {
        Self {
            data: data,
            writer: writer,
            len: len,
        }
    }
}

impl <'a, 'b, const C: usize, T: BufferSource> WLock for RWWriteLock<'a, 'b, C, T> {
    fn commit(self, bytes_written: usize) -> Result<(), BufferError> {
        self.writer.buffer.inner.lock_mut(|inner| {
            inner.write_commit(bytes_written)
        })
    }
}

impl <'a, 'b, const C: usize, T: BufferSource> Deref for RWWriteLock<'a, 'b, C, T> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        unsafe {
            from_raw_parts_mut(self.data, self.len)
        }
    }
}

impl <'a, 'b, const C: usize, T: BufferSource> DerefMut for RWWriteLock<'a, 'b, C, T> {

    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            from_raw_parts_mut(self.data, self.len)
        }
    }
}

unsafe impl <'a, 'b, const C: usize, T: BufferSource> Send for RWWriteLock<'a, 'b, C, T> {}

#[cfg(all(test, feature = "std"))]
mod tests {
    use crate::BufferInner;

    #[test]
    fn test_inner_shift() {
        let mut source = [1, 2, 3, 4, 5, 0, 0, 0];
        let mut inner = BufferInner::<1, _>::new(&mut source);
        inner.write_position = 5;
        inner.read_position = 0;

        unsafe { inner.shift() };
        assert_eq!(inner.write_position, 5);
        assert_eq!(inner.read_position, 0);

        inner.read_position = 2;
        unsafe { inner.shift() };
        assert_eq!(inner.write_position, 3);
        assert_eq!(inner.read_position, 0);
        assert_eq!(&source[..3], &[3, 4, 5]);
    }
}
