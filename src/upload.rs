use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
};

use anyhow::Context;
use flate2::{Compression, write::ZlibEncoder};

pub fn upload_binary(data: Vec<u8>, ip: Ipv4Addr) -> anyhow::Result<()> {
    let original_size = data.len();
    log::info!(
        "Starting binary upload to {} (size: {} bytes)",
        ip,
        original_size
    );

    let (payload, uncompressed_size) = if !data.starts_with(b"PK\x03\x04") {
        log::debug!("Data is not zipped; attempting zlib compression");

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
        encoder.write_all(&data).unwrap();
        let compressed = encoder.finish().unwrap();

        let uncompressed_size = original_size as u32;

        if compressed.len() < original_size {
            log::debug!(
                "Compression effective: {} -> {} bytes",
                original_size,
                compressed.len()
            );
            (compressed, uncompressed_size)
        } else {
            log::debug!("Compression ineffective; sending raw data");
            (data, uncompressed_size)
        }
    } else {
        log::debug!("Data is already zipped; skipping compression");
        (data, 0)
    };

    let server_addr = SocketAddrV4::new(ip, 4299);
    log::debug!("Connecting to {}...", server_addr);
    let mut stream = TcpStream::connect(&server_addr)
        .context(format!("Cannot open socket to: {}", &server_addr))?;
    log::info!("Connected to target device");

    log::debug!("Sending Wiiload headers");
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

    let payload_len = payload.len() as u32;
    stream.write_all(&payload_len.to_be_bytes()).unwrap();
    stream.write_all(&uncompressed_size.to_be_bytes()).unwrap();

    log::info!("Sending payload ({} bytes)...", payload_len);
    stream.write_all(&payload).unwrap();
    log::info!("Upload completed successfully");

    Ok(())
}
