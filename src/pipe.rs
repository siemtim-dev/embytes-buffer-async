use crate::{BufferRead, BufferWrite};

pub struct ReadPipe<'a, W: BufferWrite, S: embedded_io_async::Read> {
    source: &'a mut S,
    target: W
}

impl <'a, W: BufferWrite, S: embedded_io_async::Read> ReadPipe<'a, W, S> {

    pub fn new(source: &'a mut S, target: W) -> Self {
        Self {
            source: source,
            target: target
        }
    }

    pub async fn run(&mut self) -> Result<(), S::Error> {
        let mut buf = [0; 64];
        loop {
            let n = self.source.read(&mut buf).await?;
            self.target.write_all(&buf[..n]).await.unwrap();
        }
    }
}

pub struct WritePipe<'a, R: BufferRead, S: embedded_io_async::Write> {
    target: &'a mut S,
    source: R
}

impl <'a, R: BufferRead, S: embedded_io_async::Write> WritePipe<'a, R, S> {

    pub fn new(target: &'a mut S, source: R) -> Self {
        Self {
            target: target,
            source: source
        }
    }

    pub async fn run(&mut self) -> Result<(), S::Error> {

        let mut buf = [0; 64];
        loop {
            let n = self.source.read(&mut buf).await.unwrap();
            self.target.write_all(&buf[..n]).await?;
        }
    }
}