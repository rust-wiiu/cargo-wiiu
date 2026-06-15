//! cargo wiiu
//!
//! Note
//!
//! unwrap() must be used in places that can not be fixed by the user (e.g. Cursor::write on a Vec). Result::context() by anyhow should be used in places where the user can fix the error (e.g. reading missing file).

mod elf;
mod rpl;
mod upload;
mod wuhb;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use std::{fs, net::Ipv4Addr, path::PathBuf};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
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

#[derive(Args, Debug, Clone)]
struct WuhbConfig {
    /// Display name of the app in the Home Menu
    #[arg(long, default_value = "Rust App")]
    long_name: String,
    /// ???
    #[arg(long, default_value = "Rust App")]
    short_name: String,
    /// Icon of the app in the Home Menu
    #[arg(long, value_parser = extension(["png", "tga"]))]
    icon: Option<PathBuf>,
    /// Splash screen on TV
    #[arg(long, value_parser = extension(["png", "tga"]))]
    tv_image: Option<PathBuf>,
    /// Splash screen on DRC
    #[arg(long, value_parser = extension(["png", "tga"]))]
    drc_image: Option<PathBuf>,
    /// Path to the content directory
    #[arg(long)]
    content: Option<PathBuf>,
}

impl Default for WuhbConfig {
    fn default() -> Self {
        Self {
            long_name: String::from("Rust App"),
            short_name: String::from("Rust App"),
            icon: None,
            tv_image: None,
            drc_image: None,
            content: None,
        }
    }
}

impl WuhbConfig {
    fn read_manifest_metadata(&mut self) -> anyhow::Result<()> {
        let metadata = cargo_metadata::MetadataCommand::new()
            .current_dir(std::env::current_dir().unwrap())
            .no_deps()
            .exec();

        match metadata {
            Ok(metadata) => match metadata.root_package().unwrap().metadata.get("wuhb") {
                Some(wuhb) => {
                    if self.long_name == "Rust App" {
                        if let Some(manifest_val) = wuhb.get("long-name") {
                            self.long_name = manifest_val
                                .as_str()
                                .context("`long-name` manifest entry must be a string")
                                .map(String::from)?;
                        }
                    }

                    if self.short_name == "Rust App" {
                        if let Some(manifest_val) = wuhb.get("short-name") {
                            self.short_name = manifest_val
                                .as_str()
                                .context("`short-name` manifest entry must be a string")
                                .map(String::from)?;
                        }
                    }

                    self.icon = wuhb
                        .get("icon")
                        .map(|v| {
                            v.as_str()
                                .context("`icon` manifest entry must be a string")
                                .map(PathBuf::from) // Removed semicolon here
                        })
                        .transpose()?;

                    self.tv_image = wuhb
                        .get("tv-image")
                        .map(|v| {
                            v.as_str()
                                .context("`tv-image` manifest entry must be a string")
                                .map(PathBuf::from) // Removed semicolon here
                        })
                        .transpose()?;

                    self.drc_image = wuhb
                        .get("drc-image")
                        .map(|v| {
                            v.as_str()
                                .context("`drc-image` manifest entry must be a string")
                                .map(PathBuf::from) // Removed semicolon here
                        })
                        .transpose()?;

                    self.content = wuhb
                        .get("content")
                        .map(|v| {
                            v.as_str()
                                .context("`content` manifest entry must be a string")
                                .map(PathBuf::from) // Removed semicolon here
                        })
                        .transpose()?;
                }
                None => {
                    log::info!("No \"wuhb\" section in manifest found");
                }
            },
            Err(e) => {
                log::warn!("Executed outside of a Rust crate. Using default values.");
                log::info!("Error: {e}");
            }
        }

        Ok(())
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
    Upload {
        /// Binary to upload
        #[arg(value_parser = extension(["rpx", "wuhb"]))]
        binary: PathBuf,
        /// IP address of the console
        #[arg(long)]
        ip: Ipv4Addr,
    },
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
        #[arg(value_parser = extension(["rpx"]))]
        rpx: PathBuf,
        /// Path to the resulting WUHB archive. Defaults to rpx path with ".wuhb" extension.
        #[arg(value_parser = extension(["wuhb"]))]
        wuhb: Option<PathBuf>,
        /// Configuration flags for the WUHB archive
        #[command(flatten)]
        config: WuhbConfig,
    },
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Build { cargo_args: args } => {
            println!("cargo wiiu build {args:?}");
        }
        Commands::Run { cemu } => {
            println!("cargo wiiu run {cemu:?}");
        }
        Commands::Upload { binary, ip } => {
            log::info!("Read input file");
            let data =
                fs::read(&binary).context(format!("Failed to read file: {}", binary.display()))?;
            upload::upload_binary(data, ip)?;
        }
        Commands::Rpx { elf, rpx } => {
            let rpx = rpx.unwrap_or_else(|| elf.with_extension("rpx"));
            rpl::from_elf(elf, rpx, false);
        }
        Commands::Rpl { elf, rpl } => {
            let rpl = rpl.unwrap_or_else(|| elf.with_extension("rpl"));
            rpl::from_elf(elf, rpl, true);
        }
        Commands::Wuhb {
            rpx,
            wuhb,
            mut config,
        } => {
            let wuhb = wuhb.unwrap_or_else(|| rpx.with_extension("wuhb"));

            log::info!("Read input file");
            let rpx = fs::read(&rpx).context(format!("Failed to read file: {}", rpx.display()))?;

            log::info!("Read manifest file");
            config.read_manifest_metadata()?;

            let content = wuhb::from_rpx(rpx, config)?;

            log::info!("Write output file");
            fs::write(&wuhb, content)
                .context(format!("Failed to write file: {}", wuhb.display()))?;
        }
    }

    Ok(())
}
