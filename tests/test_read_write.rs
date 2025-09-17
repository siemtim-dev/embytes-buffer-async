use std::{str::from_utf8, sync::Arc, time::Duration};

use embytes_buffer_async::{AsyncBuffer, BufferRead, BufferWrite, RLock, ReadSliceAsyncResult, WLock, WriteSliceAsyncResult};

use ntest::timeout;

#[cfg(feature = "embedded")]
#[tokio::test]
#[timeout(10000)]
async fn test_read_write() {
    use embedded_io_async::{Read, Write};
    use embytes_buffer_async::AsyncBuffer;


    let mut source = [0; 64];
    let buffer = AsyncBuffer::<2, _>::new(&mut source);
    
    let sender_future = async {
        let mut writer = buffer.create_writer();

        for i in 0..=255 {
            let n = writer.write(&[i]).await.unwrap();
            assert_eq!(n, 1);
        }
    };

    let receiver_future = async {
        let mut reader = buffer.create_reader();

        let mut buf = [0; 1];
        
        for i in 0..=255 {
            let n = reader.read(&mut buf).await.unwrap();
            assert_eq!(n, 1);
            assert_eq!(buf[0], i);
        }
    };

    tokio::join!(sender_future, receiver_future);
}

#[test]
#[timeout(10000)]
fn test_read_slice() {
    let mut source = [0; 1024];
    let buffer = AsyncBuffer::<2, _>::new(&mut source);

    let writer = buffer.create_writer();

    writer.push("sdhfksjdhf".as_bytes()).unwrap();

    let reader = buffer.create_reader();
    reader.read_slice(|data|{
        assert_eq!(&data[..], "sdhfksjdhf".as_bytes());
        (1, ())
    }).unwrap();

    reader.read_slice(|data|{
        assert_eq!(&data[..], "dhfksjdhf".as_bytes());
        (1, ())
    }).unwrap();
}

#[cfg(all(feature = "std", feature = "embedded"))]
#[tokio::test]
#[timeout(10000)]
async fn test_write_read_slice() {
    use embedded_io_async::Write;
    use embytes_buffer_async::ReadSliceAsyncResult;

    const DATA: &[u8] = "123456789-123456789-123456789-123456789-;".as_bytes();

    let mut source = [0; 1024];
    let buffer = AsyncBuffer::<2, _>::new(&mut source);
    
    let sender_future = async {
        
        let mut writer = buffer.create_writer();

        for byte in DATA {
            let n = writer.write(&[ *byte ]).await.unwrap();
            assert_eq!(n, 1);
        }
    };

    let receiver_future = async {
        let reader = buffer.create_reader();

        let expected_chunks = DATA.chunks(8);
        for expected_chunk in expected_chunks {
            let chunk = reader.read_slice_async(|readable|{
                if readable.len() >= expected_chunk.len() {
                    let mut result = [0; 8];
                    result[..expected_chunk.len()].copy_from_slice(&readable[..expected_chunk.len()]);
                    ReadSliceAsyncResult::Ready(expected_chunk.len(), result)
                } else {
                    ReadSliceAsyncResult::Wait
                }
            }).await.unwrap();
            assert_eq!(expected_chunk, &chunk[..expected_chunk.len()]);
        }
    };

    tokio::join!(sender_future, receiver_future);
}


#[tokio::test]
#[timeout(10000)]
async fn test_embedded_read_write_with_waiting() {
    use embedded_io_async::Write;
    use embedded_io_async::Read;

    let buffer_1 = Arc::new(AsyncBuffer::<1, _>::new([0; 64]));
    let buffer_2 = buffer_1.clone();

    let write_join = tokio::spawn(async move {
        let mut writer = buffer_1.create_writer();
        for byte in 0..254 {
            writer.write(&[byte]).await.unwrap();
        }
    });

    let read_join = tokio::spawn(async move {
        let mut reader = buffer_2.create_reader();

        let mut expected_byte = 0;
        let mut target = [0; 16];

        while expected_byte < 254 {
            let bytes_read = reader.read(&mut target).await.unwrap();

            for byte in &target[..bytes_read] {
                assert_eq!(*byte, expected_byte);
                expected_byte += 1;
            }
        }
    });

    tokio::try_join!(read_join, write_join).unwrap();
}


#[tokio::test]
#[timeout(10000)]
async fn test_lock_read_write_with_waiting() {
    use embedded_io_async::Write;

    let buffer_1 = Arc::new(AsyncBuffer::<1, _>::new([0; 16]));
    let buffer_2 = buffer_1.clone();

    let write_join = tokio::spawn(async move {
        let mut writer = buffer_1.create_writer();
        for i in 0..16 {
            tokio::time::sleep(Duration::from_millis(20)).await;

            for byte in "<<<>>>".as_bytes() {
                tokio::time::sleep(Duration::from_millis(10)).await;
                let n = writer.write(&[*byte]).await.unwrap();
                assert_eq!(n, 1);

                println!("write: iteration = {}, byte = {}", i, *byte as char);
            }
        }
    });

    let read_join = tokio::spawn(async move {
        let reader = buffer_2.create_reader();

        for _ in 0..16 {
            reader.read_slice_async(|readable| {
                if readable.len() >= 6 {
                    let str = str::from_utf8(&readable[..6]).unwrap();
                    assert_eq!(str, "<<<>>>");
                    ReadSliceAsyncResult::Ready(6, ())
                } else {
                    ReadSliceAsyncResult::Wait
                }
            }).await.unwrap();
        }
    });

    tokio::try_join!(read_join, write_join).unwrap();
}

#[tokio::test]
#[timeout(10000)]
async fn test_slice_read_slice_write() {

    const DATA: &[&str] = &[
        "agsdoadsg",
        "123456789012345",
        "sdhfkdsjfhds"
    ];

    let buffer_1 = Arc::new(AsyncBuffer::<1, _>::new([0; 16]));
    let buffer_2 = buffer_1.clone();

    let write_join = tokio::spawn(async move {
        let writer = buffer_1.create_writer();

        for word in DATA {
            let bytes = word.as_bytes();
            writer.write_slice_async(|writeable| {
                if writeable.len() > bytes.len() {
                    println!("write: writing word {} (len = {})", word, bytes.len());
                    writeable[..bytes.len()].copy_from_slice(bytes);
                    writeable[bytes.len()] = ';' as u8;
                    WriteSliceAsyncResult::Ready(bytes.len() + 1, ())
                } else {
                    println!("write: wait; capacity = {}", writeable.len());
                    WriteSliceAsyncResult::Wait
                }
            }).await.unwrap();
        }
    });

    let read_join = tokio::spawn(async move {
        let reader = buffer_2.create_reader();

        for word in DATA {
            reader.read_slice_async(|readable| {
                let index = readable.iter().position(|el| *el == (';' as u8));

                if let Some(index) = index {
                    let result = str::from_utf8(&readable[..index]).unwrap();
                    assert_eq!(result, *word);
                    println!("read: found '{}'", result);
                    ReadSliceAsyncResult::Ready(index + 1, ())
                } else {
                    println!("read: wait for new data; could not find ';' in '{}'", str::from_utf8(readable).unwrap());
                    ReadSliceAsyncResult::Wait
                }

            }).await.unwrap();
        }
    });

    tokio::try_join!(read_join, write_join).unwrap();

}



#[tokio::test]
#[timeout(10000)]
async fn test_push_pull() {

    const DATA: &[&[u8]] = &[
        "agsdoadsg".as_bytes(),
        "123012345".as_bytes(),
        "sdhfkfhds".as_bytes()
    ];

    let buffer_1 = Arc::new(AsyncBuffer::<1, _>::new([0; 16]));
    let buffer_2 = buffer_1.clone();

    let write_join = tokio::spawn(async move {
        let writer = buffer_1.create_writer();

        for word in DATA {
            writer.push_async(word).await.unwrap();
            println!("write: pushed '{}' (len = {})", from_utf8(word).unwrap(), word.len());
        }
    });

    let read_join = tokio::spawn(async move {
        let reader = buffer_2.create_reader();

        for word in DATA {
            let mut b = [0; 9];
            reader.pull(&mut b).await.unwrap();
            assert_eq!(&b[..], *word);
            println!("read: pulled '{}' (len = {})", from_utf8(&b).unwrap(), b.len());
        }
    });

    tokio::try_join!(read_join, write_join).unwrap();

}

#[tokio::test]
#[timeout(10000)]
async fn test_reset() {
    let buffer_1 = Arc::new(AsyncBuffer::<1, _>::new([0; 16]));
    let buffer_2 = buffer_1.clone();

    buffer_1.create_writer().push(&[ 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,13, 14, 15, 16 ]).unwrap();


    let write_join = tokio::spawn(async move {
        let writer = buffer_1.create_writer();

        writer.push_async(&[ 100, 101 ]).await.unwrap();
    });

    let read_join = tokio::spawn(async move {
        let reader = buffer_2.create_reader();

        reader.try_reset().unwrap();

        let mut buf = [0; 2];
        reader.pull(&mut buf).await.unwrap();
        assert_eq!(&buf, &[ 100, 101 ]);
    });

    tokio::try_join!(read_join, write_join).unwrap();
}

#[tokio::test]
#[timeout(10000)]
async fn test_read_lock_write_lock() {

    const DATA: &[&str] = &[
        "agsdoadsg",
        "123012345",
        "sdhfkfhds"
    ];

    let buffer_1 = Arc::new(AsyncBuffer::<1, _>::new([0; 16]));
    let buffer_2 = buffer_1.clone();

    let write_join = tokio::spawn(async move {
        let writer = buffer_1.create_writer();

        let mut i = 0;

        while i < DATA.len() {
            let mut lock = writer.lock().await;
            let word = DATA[i];
            let word_bytes = word.as_bytes();
            if lock.len() >= word.len() {
                lock[..word.len()].copy_from_slice(word_bytes);
                lock.commit(word.len()).unwrap();
                println!("write: wrote {} ({})", word, i);
                i += 1;
            } else {
                tokio::time::sleep(Duration::from_millis(1)).await;
                println!("write: did not write {} ({}): len {} required, {} given", word, i, word_bytes.len(), lock.len());
            }
        }
    });

    let read_join = tokio::spawn(async move {
        let reader = buffer_2.create_reader();
        let mut i = 0;
        while i < DATA.len() {
            let lock = reader.lock().await;
            let expected_word = DATA[i];
            let expected_bytes = expected_word.as_bytes();
            if lock.len() >= expected_bytes.len() {
                let lock_string = from_utf8(&lock[..expected_word.len()]).unwrap();
                assert_eq!(lock_string, expected_word, "expected {} at {}", expected_word, i);
                lock.set_bytes_read(expected_bytes.len()).unwrap();
                i += 1;
            } else {
                lock.wait_for_new_data().await.unwrap();
            }
        }
    });

    tokio::try_join!(read_join, write_join).unwrap();

}