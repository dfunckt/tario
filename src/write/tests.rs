use std::io;
use std::num::NonZeroUsize;

use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::Archive;
use crate::shared::test::*;

const FILES: [(&str, usize); 4] = [("512", 512), ("1024", 1024), ("500", 500), ("1000", 1000)];

async fn write_archive<W: AsyncWrite + Unpin>(mut archive: Archive<W>) -> io::Result<()> {
    for (path, size) in FILES.iter() {
        let header = make_entry_header(path, *size);
        let data = make_entry_data(*size);

        let mut entry = archive.add_entry(header.clone()).await?;
        let n = entry.write(&data[..*size]).await?;
        assert_eq!(n, *size);
    }

    archive.finish().await?;

    Ok(())
}

#[tokio::test]
async fn basic() {
    let data = make_archive_data(&FILES);

    for cap in [1, 10] {
        eprintln!("cap = {cap}");

        let mut io: Vec<u8> = Vec::new();
        let archive = Archive::with_capacity(&mut io, NonZeroUsize::new(cap).unwrap());
        let res = write_archive(archive).await;
        assert!(res.is_ok());
        assert_eq!(io.len(), data.len());
        assert_eq!(io, data);
    }
}

#[tokio::test]
async fn overlapping_entries() {
    for cap in [1, 10] {
        eprintln!("cap = {cap}");

        let mut io: Vec<u8> = Vec::new();
        let mut archive = Archive::with_capacity(&mut io, NonZeroUsize::new(cap).unwrap());

        let (path, size) = &FILES[0];
        let header = make_entry_header(path, *size);
        let data = make_entry_data(*size);
        let mut entry = archive.add_entry(header.clone()).await.unwrap();
        let n = entry.write(&data[..100]).await.unwrap();
        assert_eq!(n, 100);

        let (path, size) = &FILES[1];
        let header = make_entry_header(path, *size);
        let res = archive.add_entry(header.clone()).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind(), io::ErrorKind::Unsupported);
    }
}
