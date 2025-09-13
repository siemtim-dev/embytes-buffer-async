This libray contains a buffer implementation that is made for `no_std` environments.

The buffer is backed ba any bytes source that satisfies AsMut<[u8]> + AsRef<[u8]>.

The buffer implemnts `embedded_io_async::Read` and `embedded_io_async::Write`. 

In contrast to the `embytes_buffer` crate this crate allows for concurrent read and write access.

# Example

See examples directory for examples