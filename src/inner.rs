use core::{mem, task::{Context, Poll, Waker}};

use crate::BufferError;

use heapless::Vec;

/// This struct contains the inner state of the buffer
pub(crate) struct BufferInner <const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> {
    
    /// The index where to start read operations
    read_position: usize,

    /// The index where to start write operations
    write_position: usize,

    /// The underlying memory
    source: T,

    /// wakers registered from waiting readers that wait for new data
    read_wakers: Vec<Waker, C>,

    /// wakers registered from waiting writers that wait for new space to write to
    write_wakers: Vec<Waker, C>,

    read_loked: bool,

    write_locked: bool
    
}

impl <const C: usize, T: AsRef<[u8]> + AsMut<[u8]>> BufferInner<C, T> {
    pub(super) fn new(source: T) -> Self {
        Self {
            read_position: 0,
            write_position: 0,
            source: source,
            
            read_wakers: Vec::new(),
            write_wakers: Vec::new(),

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

    // pub(crate) fn poll_reset(&mut self, cx: &mut Context<'_>) -> Poll<()> {
    //     if self.read_loked || self.write_locked {
    //         self.add_write_waker(cx);
    //         Poll::Pending
    //     } else {
    //         self.reset();
    //         Poll::Ready(())
    //     }
    // }

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
        self.write_wakers.push(cx.waker().clone())
            .expect("expected enaugh space for write waker")
    }

    pub(crate) fn wake_writers(&mut self) {
        mem::replace(&mut self.write_wakers, Vec::new())
            .into_iter()
            .for_each(|waker| waker.wake());
    }

    pub(crate) fn add_read_waker(&mut self, cx: &mut Context<'_>) {
        self.read_wakers.push(cx.waker().clone())
            .expect("expected enaugh space for read waker")
    }

    pub(crate) fn wake_readers(&mut self) {
        mem::replace(&mut self.read_wakers, Vec::new())
            .into_iter()
            .for_each(|waker| waker.wake());
    }

    /// Returns the readable data as a slice
    pub(crate) fn readable_data(&self) -> Option<&[u8]> {
        if self.read_loked {
            None
        } else {
            Some(&self.source.as_ref()[self.read_position..self.write_position])
        }
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
            Poll::Pending
        } else if let Poll::Pending = self.poll_shift(cx) {
            Poll::Pending
        } else {
            Poll::Ready(self.write_lock())
        }
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
}

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
