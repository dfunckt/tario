# tario

A library to asynchronously read and write TAR archives in Rust.

Tario supports the *[ustar]* TAR format. It focuses on efficiently abstracting
TAR I/O rather than being a general purpose utility to make all kinds of TAR
archives. If that's not what you need, there's [tar-rs] that you should check
out.

[ustar]: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/pax.html#tag_20_92_13_06
[tar-rs]: https://docs.rs/tar/latest/tar/

Tario is:

- **Minimal**: Tario only does a few things, but well.
- **Lightweight**: Tario has minimal dependencies, is conservative on resources,
  makes no uneccessary copies and performs no allocations at runtime.
- **Fast**: Tario adds minimal overhead while working with archives.

[API Documentation](https://docs.rs/tario/latest/tario)

[![Crates.io][crates-badge]][crates-url]
[![MIT licensed][mit-badge]][mit-url]
[![Build Status][actions-badge]][actions-url]

[crates-badge]: https://img.shields.io/crates/v/tario.svg
[crates-url]: https://crates.io/crates/tario
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/dfunckt/tario/blob/master/LICENSE
[actions-badge]: https://github.com/dfunckt/tario/workflows/ci/badge.svg
[actions-badge]: https://github.com/dfunckt/tario/actions/workflows/ci.yml/badge.svg?branch=main
[actions-url]: https://github.com/dfunckt/tario/actions?query=workflow%3Aci+branch%3Amain


## Installation

Using Cargo:

```sh
$ cargo add tario
```

Or manually by editing your project's Cargo.toml:

```
[dependencies]
tario = 0.1
```

### Crate features

Tario currently has the following feature switches:

- `streams`: support for [Streams]. Enabled by default.

[Streams]: https://docs.rs/futures/latest/futures/stream/index.html


## Usage

Tario builds on Tokio's [async read and write traits][asynctraits] but is
otherwise not using anything at all from it.

[asynctraits]: https://docs.rs/tokio/latest/tokio/io/index.html

### Writing

```rust
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tario::{Archive, Header};

let mut io: Vec<u8> = Vec::new();
let mut archive = Archive::new(&mut io);
let mut files = [
    // Assume a single entry for this example.
    ("hello.txt", "hello world!"),
];

for (path, contents) in files {
    let mut header = Header::new_ustar();
    header.set_path(path)?;
    header.set_size(contents.len() as u64);
    header.set_cksum();

    let mut entry = archive.add_entry(header).await?;
    entry.write(contents.as_bytes()).await?;
}

archive.finish().await?;
```

### Reading

```rust
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt};
use tario::Archive;

let io = io::Cursor::new(&[]); // Get a reader from somewhere
let mut archive = Archive::new(io);
let buf = &mut [0u8; 100];

while let Some(mut entry) = archive.next_entry().await? {
    loop {
        let bytes_written = entry.read(buf).await?;
        if bytes_written == 0 {
            // Reached entry EOF
            break;
        }
        // do_something_with_buffer(&buf[..bytes_written]);
    }
}
```


## Roadmap

- Support for `wasm-unknown-unknown` ([#1])
- Support `no_std` ([#2])
- Support `no_alloc` ([#3])

[#1]: https://github.com/dfunckt/tario/issues/1
[#2]: https://github.com/dfunckt/tario/issues/2
[#3]: https://github.com/dfunckt/tario/issues/3


## License

This project is licensed under the [MIT license].

[MIT license]: https://github.com/dfunckt/tario/blob/master/LICENSE
