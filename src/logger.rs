use std::{
    io::{ErrorKind, Read, Write},
    time::Duration,
};

use anyhow::Context;
use serialport::{DataBits, FlowControl, Parity, SerialPortType, StopBits};

pub fn run(port: &str, baud: u32) -> anyhow::Result<()> {
    let mut serial = serialport::new(port, baud)
        .data_bits(DataBits::Eight)
        .parity(Parity::None)
        .stop_bits(StopBits::One)
        .flow_control(FlowControl::None)
        .timeout(Duration::from_millis(100))
        .open()
        .context(format!("Failed to open serial port: {port}"))?;

    log::info!("Listening on {port} ({baud} baud). Press Ctrl+C to exit.");

    let mut stdout = std::io::stdout().lock();
    let mut buf = [0u8; 1024];
    let mut out = Vec::with_capacity(buf.len());
    let mut last_cr = false;

    loop {
        let n = match serial.read(&mut buf) {
            Ok(0) => continue,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::Interrupted => {
                continue;
            }
            Err(e) => return Err(e).context(format!("Failed to read from {port}")),
        };

        out.clear();
        for &b in &buf[..n] {
            match b {
                b'\r' => out.push(b'\n'),
                b'\n' if last_cr => {}
                _ => out.push(b),
            }
            last_cr = b == b'\r';
        }

        stdout.write_all(&out)?;
        stdout.flush()?;
    }
}

pub fn list_ports(all: bool) -> anyhow::Result<()> {
    let ports = serialport::available_ports().context("Failed to list serial ports")?;

    let (mut usb, mut other): (Vec<_>, Vec<_>) = ports
        .into_iter()
        .partition(|p| matches!(p.port_type, SerialPortType::UsbPort(_)));
    usb.sort_by(|a, b| a.port_name.cmp(&b.port_name));
    other.sort_by_key(|p| natural_key(&p.port_name));

    if usb.is_empty() && (!all || other.is_empty()) {
        println!("No USB serial ports found");
    } else {
        println!("Available serial ports:");
    }

    for port in &usb {
        if let SerialPortType::UsbPort(info) = &port.port_type {
            let product = info.product.as_deref().unwrap_or("USB serial");
            let manufacturer = info
                .manufacturer
                .as_ref()
                .map(|m| format!(" by {m}"))
                .unwrap_or_default();
            println!(
                "  {}  ({product}{manufacturer}, {:04x}:{:04x})",
                port.port_name, info.vid, info.pid
            );
        }
    }

    if all {
        for port in &other {
            println!("  {}", port.port_name);
        }
    } else if !other.is_empty() {
        println!("  ({} non-USB ports hidden, use --all to show)", other.len());
    }

    println!("\nUsage: cargo wiiu logger <PORT>");

    Ok(())
}

fn natural_key(name: &str) -> (String, u64) {
    let split = name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    (name[..split].to_string(), name[split..].parse().unwrap_or(0))
}
