///
/// This module provides an abstract interface for a blocking mutex and provides implementations for [`std`] ans [`embassy_sync`]
/// 
/// 

/// A generictrait for a blocking mutex
pub trait Mutex<T> {

    /// Creates a new mutex with the provided initial value
    fn new(value: T) -> Self;
    
    /// locks the mutex for reading and performs the provided operation
    /// defaults to [`Self::lock_mut`]
    fn lock<U, F: FnOnce(&T) -> U>(&self, f: F) -> U {
        self.lock_mut(|inner| f(inner))
    }


    /// locks the mutex for reading and mutating the contained data
    fn lock_mut<U, F: FnOnce(&mut T) -> U>(&self, f: F) -> U;

}

#[cfg(feature = "std")]
pub type MutexImpl<T> = std_mutex::StdMutex<T>;

#[cfg(all(feature = "embedded", not(feature = "std")))]
pub type MutexImpl<T> = embassy_mutex::EmbeddedMutex<T>;

#[cfg(not(any(feature = "embedded", feature = "std")))]
compile_error!("cannot create mutex, no feature for mutex selected. select 'embedded' or 'std");

/// Module containing the mutex implementation for [`embassy_sync::blocking_mutex::Mutex`] with [`embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex`]
#[cfg(feature = "embedded")]
pub mod embassy_mutex {
    use core::cell::RefCell;

    use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};

    #[cfg_attr(feature = "std", allow(dead_code))]
    pub struct EmbeddedMutex<T>(Mutex<CriticalSectionRawMutex, RefCell<T>>);

    impl <T> super::Mutex<T> for EmbeddedMutex<T> {
        fn new(value: T) -> Self {
            Self(Mutex::new(RefCell::new(value)))
        }

        fn lock<U, F: FnOnce(&T) -> U>(&self, f: F) -> U {
            self.0.lock(|inner| {
                let inner = inner.borrow();
                f(&inner)
            })
        }
    
        fn lock_mut<U, F: FnOnce(&mut T) -> U>(&self, f: F) -> U {
            self.0.lock(|inner| {
                let mut inner = inner.borrow_mut();
                f(&mut inner)
            })
        }
    }

}

/// Module containing the mutex implementation for [`std::sync::Mutex`]
#[cfg(feature = "std")]
pub mod std_mutex {
    use std::sync::Mutex;

    pub struct StdMutex<T>(Mutex<T>);

    impl <T> super::Mutex<T> for StdMutex<T> {
        fn new(value: T) -> Self {
            Self(Mutex::new(value))
        }

        fn lock<U, F: FnOnce(&T) -> U>(&self, f: F) -> U {
            let l = self.0.lock()
                .expect("std lock must not be poisened");

            f(&l)
        }
    
        fn lock_mut<U, F: FnOnce(&mut T) -> U>(&self, f: F) -> U {
            let mut l = self.0.lock()
                .expect("std lock must not be poisened");

            f(&mut l)
        }
    }

}
