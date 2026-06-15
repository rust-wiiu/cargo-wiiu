use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    path::Path,
};

use flate2::{Compression, write::ZlibEncoder};

pub fn upload_binary(data: Vec<u8>, path: impl AsRef<Path>, ip: Ipv4Addr) {
    let (payload, uncompressed_size) = if !data.starts_with(b"PK\x03\x04") {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
        encoder.write_all(&data).unwrap();
        let compressed = encoder.finish().unwrap();

        let uncompressed_size = data.len() as u32;

        if compressed.len() < data.len() {
            (compressed, uncompressed_size)
        } else {
            (data, uncompressed_size)
        }
    } else {
        (data, 0)
    };

    let mut stream = TcpStream::connect(SocketAddrV4::new(ip, 4299)).unwrap();

    stream.write_all(b"HAXX").unwrap();

    const WIILOAD_VERSION: (u8, u8) = (0, 5);

    stream
        .write_all(&[
            WIILOAD_VERSION.0,
            WIILOAD_VERSION.1,
            0, // args len >> 8
            0, // args_len & 0xFF
        ])
        .unwrap();

    stream
        .write_all(&(payload.len() as u32).to_be_bytes())
        .unwrap();

    stream.write_all(&uncompressed_size.to_be_bytes()).unwrap();

    stream.write_all(&payload).unwrap();
}
