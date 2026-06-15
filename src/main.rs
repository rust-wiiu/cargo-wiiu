mod elf;
mod rpl;
mod wuhb;

use clap::{Parser, Subcommand};
use std::{fs, path::PathBuf};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

fn extension<const N: usize>(
    exts: [&'static str; N],
) -> impl Fn(&str) -> Result<PathBuf, String> + Clone + Send + Sync + 'static {
    move |s| {
        let path = PathBuf::from(s);
        match path.extension() {
            Some(e) if exts.iter().any(|&ext| e == ext) => Ok(path),
            _ => {
                let listed = exts.join(", ");
                Err(format!(
                    "'{s}' does not have a valid extension (expected: {listed})"
                ))
            }
        }
    }
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
        #[arg(value_parser = extension(["elf"]))]
        elf: PathBuf,
        /// Path to the resulting rpx binary. Defaults to elf path with ".rpx" extension.
        #[arg(value_parser = extension(["rpx"]))]
        rpx: Option<PathBuf>,
    },
    /// Convert ELF to RPL (library)
    Rpl {
        /// Path to the elf binary
        #[arg(value_parser = extension(["elf"]))]
        elf: PathBuf,
        /// Path to the resulting rpl binary. Defaults to elf path with ".rpl" extension.
        #[arg(value_parser = extension(["rpl"]))]
        rpl: Option<PathBuf>,
    },
    Wuhb {
        /// Path to the binary (elf / rpx)
        #[arg(value_parser = extension(["elf", "rpx"]))]
        binary: PathBuf,
        /// Path to the resulting WUHB archive. Defaults to binary path with ".wuhb" extension.
        #[arg(value_parser = extension(["wuhb"]))]
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
            rpl::from_elf(elf, rpx, false);
        }
        Commands::Rpl { elf, rpl } => {
            let rpl = rpl.clone().unwrap_or_else(|| elf.with_extension("rpl"));
            rpl::from_elf(elf, rpl, true);
        }
        Commands::Wuhb { binary, wuhb } => {
            let wuhb = wuhb
                .clone()
                .unwrap_or_else(|| binary.with_extension("wuhb"));

            let rpx = match binary.extension().unwrap().to_str().unwrap() {
                "rpx" => fs::read(binary).unwrap(),
                "elf" => todo!("Indirect conversion: elf -> rpx -> wuhb"),
                e => panic!("Unsupported main executable: {e}"),
            };

            let content = wuhb::from_rpx(rpx, "Test App");

            fs::write(wuhb, content).unwrap();
        }
    }
}
