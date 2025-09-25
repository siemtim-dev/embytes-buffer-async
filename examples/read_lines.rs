use std::str::from_utf8;

use embedded_io_async::Write;

use embytes_buffer_async::{AsyncBuffer, Buffer, BufferRead, ReadSliceAsyncResult};

const DATA: &str = r#"
    Dear user of this crate,

    please do not create more than one reader and writer.

    Yours sincerely

    Contributor of this crate
"#;


#[tokio::main]
pub async fn main() {
    let buffer = AsyncBuffer::<2, [_; 64]>::new_stack();

    let read_future = async {
        let reader = buffer.create_reader();
        
        loop {
            let line = reader.read_slice_async(|readable| {
                let str = from_utf8(readable).unwrap();
                match str.find('\n'){
                    Some(i) => {
                        let result = &str[..i];
                        let n = result.as_bytes().len();
                        ReadSliceAsyncResult::Ready(n + 1, String::from(result))
                    },
                    None => ReadSliceAsyncResult::Wait,
                }
            }).await.unwrap();

            println!("Found line: {}", line);
            if line.contains("Contributor of this crate") {
                break;
            }
        }

    };

    let write_future = async {
        let mut writer = buffer.create_writer();
        for byte in DATA.as_bytes() {
            writer.write_all(&[ *byte ]).await.unwrap();
        }
    };

    tokio::join!(read_future, write_future);


}
