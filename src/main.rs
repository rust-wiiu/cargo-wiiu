mod elf;
mod elf2rpl;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    /// Convert ELF to RPX
    Rpx {
        /// Path to the elf binary
        elf: PathBuf,
        /// Path to the resulting rpx binary. Defaults to elf path with ".rpx" extension.
        rpx: Option<PathBuf>,
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
            elf2rpl::convert(elf, rpx);
        }
    }
}
