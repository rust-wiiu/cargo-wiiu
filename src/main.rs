mod elf;
mod elf2rpl;
mod wuhb;

use clap::{Parser, Subcommand};
use std::{fs, path::PathBuf};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build
    #[command(trailing_var_arg = true)]
    Build {
        /// Arguments given directly to `cargo build`
        cargo_args: Vec<String>,
    },
    /// Run application on emulator
    Run {
        /// Path to Cemu.
        cemu: Option<PathBuf>,
    },
    /// Upload
    Upload { wiiload: Option<PathBuf> },
    /// Convert ELF to RPX (executable)
    Rpx {
        /// Path to the elf binary
        elf: PathBuf,
        /// Path to the resulting rpx binary. Defaults to elf path with ".rpx" extension.
        rpx: Option<PathBuf>,
    },
    /// Convert ELF to RPL (library)
    Rpl {
        /// Path to the elf binary
        elf: PathBuf,
        /// Path to the resulting rpl binary. Defaults to elf path with ".rpl" extension.
        rpl: Option<PathBuf>,
    },
    Wuhb {
        /// Path to the binary (elf / rpx)
        binary: PathBuf,
        /// Path to the resulting WUHB archive. Defaults to binary path with ".wuhb" extension.
        wuhb: Option<PathBuf>,
    },
}

fn main() {
    let args = Args::parse();

    match &args.command {
        Commands::Build { cargo_args: args } => {
            println!("cargo wiiu build {args:?}");
        }
        Commands::Run { cemu } => {
            println!("cargo wiiu run {cemu:?}");
        }
        Commands::Upload { wiiload } => {
            println!("cargo wiiu upload {wiiload:?}");
        }
        Commands::Rpx { elf, rpx } => {
            let rpx = rpx.clone().unwrap_or_else(|| elf.with_extension("rpx"));
            elf2rpl::convert(elf, rpx, false);
        }
        Commands::Rpl { elf, rpl } => {
            let rpl = rpl.clone().unwrap_or_else(|| elf.with_extension("rpl"));
            elf2rpl::convert(elf, rpl, true);
        }
        Commands::Wuhb { binary, wuhb } => {
            let wuhb = wuhb
                .clone()
                .unwrap_or_else(|| binary.with_extension("wuhb"));

            let content = wuhb::from_rpx(binary, &wuhb);
            fs::write(wuhb, content).unwrap();
        }
    }
}
