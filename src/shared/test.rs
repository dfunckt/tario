use super::block::{BLOCK_SIZE, Header};

pub fn make_archive_data(entries: &[(&str, usize)]) -> Vec<u8> {
    entries
        .iter()
        .flat_map(|(path, size)| {
            [
                make_entry_header(path, *size).as_bytes().to_vec(),
                make_entry_data(*size),
            ]
            .concat()
        })
        .chain(make_eof_data())
        .collect()
}

pub fn make_entry_header(path: &str, size: usize) -> Header {
    let mut header = Header::new_ustar();
    header.set_path(path).unwrap();
    header.set_size(size as u64);
    header.set_cksum();
    header
}

pub fn make_entry_data(size: usize) -> Vec<u8> {
    let total_size = size.next_multiple_of(BLOCK_SIZE);
    let mut buf = vec![0u8; total_size];

    buf.iter_mut().enumerate().for_each(|(i, b)| {
        if i < size {
            *b = (i.next_multiple_of(u8::MAX as usize) - i) as u8;
        } else {
            *b = 0;
        }
    });

    buf
}

pub fn make_eof_data() -> Vec<u8> {
    vec![0u8; 1024]
}
