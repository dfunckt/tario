use std::io;
use std::num::NonZeroUsize;

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::Archive;
use crate::shared::block::BLOCK_SIZE;
use crate::shared::test::*;

const FILES: [(&str, usize); 4] = [("512", 512), ("1024", 1024), ("500", 500), ("1000", 1000)];

async fn read_archive<R: AsyncRead + Unpin>(mut archive: Archive<R>) -> io::Result<()> {
    let buf = &mut [0u8; BLOCK_SIZE];
    while let Some(mut entry) = archive.next_entry().await? {
        loop {
            let n = entry.read(buf).await?;
            if n == 0 {
                break;
            }
        }
    }
    assert!(archive.next_entry().await.unwrap().is_none());
    Ok(())
}

#[tokio::test]
async fn basic() {
    let data = make_archive_data(&FILES);

    for cap in [1, 10] {
        let io = io::Cursor::new(data.as_slice());
        let mut archive = Archive::with_capacity(io, NonZeroUsize::new(cap).unwrap());

        let buf = &mut [0u8; BLOCK_SIZE];
        let mut pos = 0usize;
        let mut i = 0;

        while let Some(mut entry) = archive.next_entry().await.unwrap() {
            let (path, size) = FILES[i];
            i += 1;

            assert_eq!(entry.path_lossy(), path.to_owned());
            pos += BLOCK_SIZE; // header bytes

            loop {
                let n = entry.read(buf).await.unwrap();
                if n == 0 {
                    break;
                }
                assert_eq!(&buf[..n], &data[pos..pos + n]);
                pos += n;
            }

            pos += size.next_multiple_of(BLOCK_SIZE) - size; // alignment bytes
        }

        assert!(archive.next_entry().await.unwrap().is_none());
    }
}

#[cfg(feature = "streams")]
#[tokio::test]
async fn stream() {
    use futures_util::StreamExt;

    let data = make_archive_data(&FILES);

    for cap in [1, 10] {
        let io = io::Cursor::new(data.as_slice());
        let mut archive = Archive::with_capacity(io, NonZeroUsize::new(cap).unwrap());
        let mut entries = archive.entries();
        let mut i = 0;

        while let Some(res) = entries.next().await {
            let (path, size) = FILES[i];
            let mut entry = res.unwrap();
            assert_eq!(entry.path_lossy(), path.to_owned());
            assert_eq!(entry.len(), size as u64);
            entry.skip().await.unwrap();
            i += 1;
        }

        assert!(archive.next_entry().await.unwrap().is_none());
    }
}

#[tokio::test]
async fn ignore_entry_data() {
    let data = make_archive_data(&FILES);

    for cap in [1, 10] {
        let io = io::Cursor::new(data.as_slice());
        let mut archive = Archive::with_capacity(io, NonZeroUsize::new(cap).unwrap());

        for (path, size) in FILES.iter() {
            let mut entry = archive.next_entry().await.unwrap().unwrap();
            assert_eq!(entry.path_lossy(), path.to_owned());
            assert_eq!(entry.len(), *size as u64);
            entry.skip().await.unwrap();
        }

        assert!(archive.next_entry().await.unwrap().is_none());
    }
}

async fn expect_eof(data: &[u8], cap: usize, offset: usize) {
    eprintln!("cap = {cap}, offset = {offset}");

    let data = &data[..offset];
    let io = io::Cursor::new(data);
    let archive = Archive::with_capacity(io, NonZeroUsize::new(cap).unwrap());
    let res = read_archive(archive).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
}

#[tokio::test]
async fn unexpected_eof_at_random_position() {
    use rand::Rng;

    let data = make_archive_data(&FILES);

    for _ in 0..1_000 {
        let offset = rand::rng().random_range(0..data.len());
        for cap in [1, 10] {
            expect_eof(data.as_slice(), cap, offset).await;
        }
    }
}

#[tokio::test]
async fn unexpected_eof_cases() {
    let data = make_archive_data(&FILES);
    let cases = [(1, 5098)];

    for (cap, offset) in cases {
        expect_eof(data.as_slice(), cap, offset).await;
    }
}

#[tokio::test]
async fn unexpected_eof() {
    let data = make_archive_data(&FILES);

    // eof while scanning for next entry
    let h = [
        // "512"
        500,
        // "1024"
        512 + 512 + 500,
        // "500"
        512 + 512 + 512 + 1024 + 500, // 3.060
        // "1000"
        512 + 512 + 512 + 1024 + 512 + 512 + 500, // 4.084
        // receiving header
        512 + 512 + 512 + 1024 + 512 + 512 + 512 + 1024 + 500, // 5.620
        // receiving eof
        512 + 512 + 512 + 1024 + 512 + 512 + 512 + 1024 + 512 + 500, // 6.132
    ];

    // eof while reading entry data
    let d = [
        // "512"
        512 + 500,
        // "1024"
        512 + 512 + 512 + 500, // 2.036
        // "500"
        512 + 512 + 512 + 1024 + 512 + 250, // 3.322
        // "1000"
        512 + 512 + 512 + 1024 + 512 + 512 + 512 + 500, // 4.596
    ];

    // eof while aligning entry data
    let a = [
        // "512"
        512 + 512,
        // "1024"
        512 + 512 + 512 + 1024, // 2.560
        // "500"
        512 + 512 + 512 + 1024 + 512 + 500,     // 3.572
        512 + 512 + 512 + 1024 + 512 + 500 + 6, // 3.578
        // "1000"
        512 + 512 + 512 + 1024 + 512 + 512 + 512 + 1000, // 5.096
        512 + 512 + 512 + 1024 + 512 + 512 + 512 + 1000 + 12, // 5.108
    ];

    for cap in [1, 10] {
        for offset in h {
            eprintln!("h: cap = {cap}, offset = {offset}");

            let data = &data[..offset];
            let io = io::Cursor::new(data);
            let mut archive = Archive::with_capacity(io, NonZeroUsize::new(cap).unwrap());

            let mut pos = 0usize;

            for (path, size) in FILES.iter() {
                let res = archive.next_entry().await;

                if offset - pos < BLOCK_SIZE {
                    assert_eq!(res.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
                    break;
                }

                let mut entry = res.unwrap().unwrap();
                assert_eq!(entry.path_lossy(), path.to_owned());
                assert_eq!(entry.len(), *size as u64);
                pos += BLOCK_SIZE; // header bytes

                entry.skip().await.unwrap();
                pos += size; // entry bytes
                pos += size.next_multiple_of(BLOCK_SIZE) - size; // alignment bytes
            }
        }

        for offset in d {
            eprintln!("d: cap = {cap}, offset = {offset}");

            let data = &data[..offset];
            let io = io::Cursor::new(data);
            let mut archive = Archive::with_capacity(io, NonZeroUsize::new(cap).unwrap());

            let mut pos = 0usize;

            for (path, size) in FILES.iter() {
                let mut entry = archive.next_entry().await.unwrap().unwrap();
                assert_eq!(entry.path_lossy(), path.to_owned());
                assert_eq!(entry.len(), *size as u64);
                pos += BLOCK_SIZE; // header bytes

                let mut buf = vec![0u8; *size];
                let res = entry.read_exact(buf.as_mut_slice()).await;

                if offset - pos < *size {
                    assert_eq!(res.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
                    break;
                }

                let n = res.unwrap();
                assert_eq!(n, *size);
                assert_eq!(&buf[..n], &data[pos..pos + n]);
                pos += n; // entry bytes
                pos += size.next_multiple_of(BLOCK_SIZE) - size; // alignment bytes
            }
        }

        for offset in a {
            eprintln!("a: cap = {cap}, offset = {offset}");

            let data = &data[..offset];
            let io = io::Cursor::new(data);
            let mut archive = Archive::with_capacity(io, NonZeroUsize::new(cap).unwrap());

            let mut pos = 0usize;

            for (path, size) in FILES.iter() {
                let res = archive.next_entry().await;

                if pos >= offset {
                    assert_eq!(res.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
                    break;
                }

                let mut entry = res.unwrap().unwrap();
                assert_eq!(entry.path_lossy(), path.to_owned());
                assert_eq!(entry.len(), *size as u64);
                pos += BLOCK_SIZE; // header bytes

                let mut buf = vec![0u8; *size];
                let res = entry.read_exact(buf.as_mut_slice()).await;

                let n = res.unwrap();
                assert_eq!(n, *size);
                assert_eq!(&buf[..n], &data[pos..pos + n]);
                pos += n; // entry bytes
                pos += size.next_multiple_of(BLOCK_SIZE) - size; // alignment bytes
            }
        }
    }
}
