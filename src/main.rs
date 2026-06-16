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
use std::{
    fs,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::Command,
};

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
    #[command(trailing_var_arg = true, allow_hyphen_values = true)]
    Build {
        /// Arguments given directly to `cargo build`
        cargo_args: Vec<String>,
    },
    /// Create a new project. Alias for `cargo new foo && cd foo && cargo wiiu init`.
    New { path: PathBuf },
    /// Initializes an existing project
    Init { path: PathBuf },
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
    env_logger::Builder::default()
        .format_timestamp(None)
        .format_module_path(false)
        .format_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Build { cargo_args } => {
            build(&cargo_args)?;
        }
        Commands::New { path } => {
            new(&path)?;
            init(&path)?;
        }
        Commands::Init { path } => init(&path)?,
        Commands::Upload { binary, ip } => {
            log::info!("Read input file");
            let data =
                fs::read(&binary).context(format!("Failed to read file: {}", binary.display()))?;
            upload::upload_binary(data, ip)?;
        }
        Commands::Rpx { elf, rpx } => {
            let rpx = rpx.unwrap_or_else(|| elf.with_extension("rpx"));

            log::info!("Read input file");
            let input =
                fs::read(&elf).context(format!("Failed to read file: {}", elf.display()))?;

            let output = rpl::from_elf(input, false);

            log::info!("Write output file");
            fs::write(&rpx, output).context(format!("Failed to write file: {}", rpx.display()))?;
        }
        Commands::Rpl { elf, rpl } => {
            let rpl = rpl.unwrap_or_else(|| elf.with_extension("rpl"));

            log::info!("Read input file");
            let input =
                fs::read(&elf).context(format!("Failed to read file: {}", elf.display()))?;

            let output = rpl::from_elf(input, true);

            log::info!("Write output file");
            fs::write(&rpl, output).context(format!("Failed to write file: {}", rpl.display()))?;
        }
        Commands::Wuhb {
            rpx,
            wuhb,
            mut config,
        } => {
            let wuhb = wuhb.unwrap_or_else(|| rpx.with_extension("wuhb"));

            log::info!("Read input file");
            let input =
                fs::read(&rpx).context(format!("Failed to read file: {}", rpx.display()))?;

            log::info!("Read manifest file");
            config.read_manifest_metadata()?;

            let output = wuhb::from_rpx(input, config)?;

            log::info!("Write output file");
            fs::write(&wuhb, output)
                .context(format!("Failed to write file: {}", wuhb.display()))?;
        }
    }

    Ok(())
}

fn new(path: impl AsRef<Path>) -> anyhow::Result<()> {
    Command::new("cargo")
        .arg("new")
        .args(path.as_ref())
        .status()
        .context("Failed to execute cargo new")?;

    Ok(())
}

fn init(path: impl AsRef<Path>) -> anyhow::Result<()> {
    let path = path.as_ref();

    let cafe = path.join(".cafe");
    if !cafe.is_dir() {
        log::info!("Add `.cafe` submodule");
        Command::new("git")
            .current_dir(path)
            .args([
                "submodule",
                "add",
                "https://github.com/rust-wiiu/cafe-target-spec",
                ".cafe",
            ])
            .status()
            .context("Failed to initialize `.cafe` submodule. Make sure you are in a git repository or clone the files manually.")?;

        Command::new("git")
            .current_dir(path)
            .args(["submodule", "update", "--init", "--recursive", ".cafe"])
            .status()
            .context("Failed to init `.cafe` submodule")?;
    } else {
        log::warn!("{} folder already exists. Do nothing.", cafe.display());
    }

    let toolchain = path.join("rust-toolchain.toml");
    if !toolchain.is_file() {
        log::info!("Create `rust-toolchain.toml`");
        fs::write(&toolchain, include_str!("templates/rust-toolchain.toml"))
            .context("Failed to create `rust-toolchain.toml`")?;
    } else {
        log::warn!("{} already exists. Do nothing.", toolchain.display());
    }

    let cargo = path.join(".cargo");
    let config = cargo.join("config.toml");
    if !config.is_file() {
        log::info!("Create `.cargo/config.toml`");

        if !cargo.is_dir() {
            fs::create_dir(&cargo).context("Failed to create `.cargo` directory")?;
        }

        fs::write(&config, include_str!("templates/cargo-config.toml"))
            .context("Failed to create `.cargo/config.toml`")?;
    } else {
        log::warn!("{} already exists. Do nothing.", config.display());
    }

    Ok(())
}

fn build(args: &Vec<String>) -> anyhow::Result<()> {
    Command::new("cargo")
        .arg("build")
        .args(args)
        .status()
        .context("Failed to build")?;

    let target_dir = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .unwrap()
        .target_directory
        .to_string();

    let profile = if let Some(pos) = args.iter().position(|x| x == "--profile") {
        args.get(pos + 1).map(|s| s.as_str()).unwrap_or("debug")
    } else if args.iter().any(|x| x == "--release") {
        "release"
    } else {
        "debug"
    };

    let name = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .unwrap()
        .root_package()
        .unwrap()
        .name
        .to_string();

    let binary = PathBuf::from(target_dir)
        .join("powerpc-cafe-nintendo")
        .join(profile)
        .join(name);

    // println!("{}", elf.display());

    {
        let input = fs::read(binary.with_extension("elf")).context("Failed to read elf file")?;

        let output = rpl::from_elf(input, false);

        fs::write(&binary.with_extension("rpx"), output).context("Failed to write rpx file")?;
    }

    // check if [package.metadata.wuhb] is present in Cargo.toml
    if cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .context("Failed to read manifest")?
        .root_package()
        .and_then(|pkg| pkg.metadata.get("wuhb"))
        .is_some()
    {
        let mut config = WuhbConfig::default();
        config.read_manifest_metadata()?;

        let input = fs::read(&binary.with_extension("rpx")).context("Failed to read rpx file")?;

        let output = wuhb::from_rpx(input, config)?;

        fs::write(&binary.with_extension("rpx"), output).context("Failed to write rpx file")?;
    }

    Ok(())
}
