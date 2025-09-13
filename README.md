This libray contains a buffer implementation that is made for `no_std` environments.

The buffer is backed ba any bytes source that satisfies AsMut<[u8]> + AsRef<[u8]>.

The buffer implemnts `embedded_io_async::Read` and `embedded_io_async::Write`. 

In contrast to the `embytes_buffer` crate this crate allows for concurrent read and write access.

Example

// Create a new buffern with an array as byte source on the stack
let mut buffer = new_stack_buffer::<1024>();

// Write some bytes to buffer
buffer.write_all("hello world".as_bytes()).unwrap();

// read the bytes again
let mut buf = [0; 128];
let bytes_read = buffer.read(&mut buf).unwrap();
See examples directory for more examples