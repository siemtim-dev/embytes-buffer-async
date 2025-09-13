use crate::{AsyncBuffer, BufferReader, BufferWriter};

pub struct ReadPipe<'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>, S: embedded_io_async::Read> {
    source: &'a mut S,
    target: BufferWriter<'b, C, T>
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>, S: embedded_io_async::Read> ReadPipe<'a, 'b, C, T, S> {

    pub fn new(source: &'a mut S, buffer: &'b AsyncBuffer<C, T>) -> Self {
        Self {
            source: source,
            target: buffer.create_writer()
        }
    }

    pub async fn run(&mut self) -> Result<(), S::Error> {
        use embedded_io_async::Write;

        let mut buf = [0; 64];
        loop {
            let n = self.source.read(&mut buf).await?;
            self.target.write_all(&buf[..n]).await.unwrap();
        }
    }
}

pub struct WritePipe<'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>, S: embedded_io_async::Write> {
    target: &'a mut S,
    source: BufferReader<'b, C, T>
}

impl <'a, 'b, const C: usize, T: AsRef<[u8]> + AsMut<[u8]>, S: embedded_io_async::Write> WritePipe<'a, 'b, C, T, S> {

    pub fn new(target: &'a mut S, buffer: &'b AsyncBuffer<C, T>) -> Self {
        Self {
            target: target,
            source: buffer.create_reader()
        }
    }

    pub async fn run(&mut self) -> Result<(), S::Error> {
        use embedded_io_async::Read;

        let mut buf = [0; 64];
        loop {
            let n = self.source.read(&mut buf).await.unwrap();
            self.target.write_all(&buf[..n]).await?;
        }
    }
}