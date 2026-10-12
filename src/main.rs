//! cargo wiiu
//!
//! Note
//!
//! unwrap() must be used in places that can not be fixed by the user (e.g. Cursor::write on a Vec). Result::context() by anyhow should be used in places where the user can fix the error (e.g. reading missing file).

mod elf;
mod logger;
mod rpl;
mod upload;
mod wuhb;

use anyhow::Context;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, parser::ValueSource};
use std::{
    fs,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Parser)]
#[command(name = "cargo", bin_name = "cargo")]
enum Cargo {
    #[command(name = "wiiu")]
    Input(Input),
}

#[derive(Args)]
#[command(author, version, about)]
struct Input {
    #[command(subcommand)]
    pub cmd: Commands,
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
    /// Name of the app in the Home Menu. At most 63 bytes.
    #[arg(long, default_value = DEFAULT_NAME)]
    name: String,
    /// Icon of the app in the Home Menu (128x128)
    #[arg(long, value_parser = extension(IMAGE_EXTENSIONS))]
    icon: Option<PathBuf>,
    /// Splash screen on TV (1280x720)
    #[arg(long, value_parser = extension(IMAGE_EXTENSIONS))]
    tv_image: Option<PathBuf>,
    /// Splash screen on DRC (854x480)
    #[arg(long, value_parser = extension(IMAGE_EXTENSIONS))]
    drc_image: Option<PathBuf>,
    /// Path to the content directory
    #[arg(long)]
    content: Option<PathBuf>,
    /// Taken from `package.authors` of the local manifest
    #[arg(skip = String::from(DEFAULT_AUTHOR))]
    author: String,
}

const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "webp", "tga"];
const DEFAULT_AUTHOR: &str = "rust-wiiu";
const DEFAULT_NAME: &str = "RustApp";

impl Default for WuhbConfig {
    fn default() -> Self {
        Self {
            name: String::from(DEFAULT_NAME),
            icon: None,
            tv_image: None,
            drc_image: None,
            content: None,
            author: String::from(DEFAULT_AUTHOR),
        }
    }
}

impl WuhbConfig {
    /// `name_from_cli` must be set if `name` was given explicitly, so it is not overwritten by the manifest.
    fn read_manifest_metadata(&mut self, name_from_cli: bool) -> anyhow::Result<()> {
        let metadata = cargo_metadata::MetadataCommand::new()
            .current_dir(std::env::current_dir().unwrap())
            .no_deps()
            .exec();

        if let Ok(metadata) = &metadata
            && let Some(package) = metadata.root_package()
            && !package.authors.is_empty()
        {
            self.author = package.authors.join(", ");
        }

        match metadata {
            Ok(metadata) => match metadata.root_package().unwrap().metadata.get("wuhb") {
                Some(wuhb) => {
                    let get = |key: &str| {
                        wuhb.get(key)
                            .map(|v| {
                                v.as_str()
                                    .map(String::from)
                                    .context(format!("`{key}` manifest entry must be a string"))
                            })
                            .transpose()
                    };

                    // Values given on the command line take precedence over the manifest
                    if let Some(name) = get("name")?
                        && !name_from_cli
                    {
                        self.name = name;
                    }
                    self.icon = self.icon.take().or(get("icon")?.map(PathBuf::from));
                    self.tv_image = self.tv_image.take().or(get("tv-image")?.map(PathBuf::from));
                    self.drc_image = self
                        .drc_image
                        .take()
                        .or(get("drc-image")?.map(PathBuf::from));
                    self.content = self.content.take().or(get("content")?.map(PathBuf::from));
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
    /// Build a project. Same as calling `cargo build` followed by `cargo wiiu rpx`. WUHB file will be created if configured in manifest.
    #[command(trailing_var_arg = true, allow_hyphen_values = true)]
    Build {
        /// Arguments given directly to `cargo build`
        cargo_args: Vec<String>,
    },
    /// Create a new project. Alias for `cargo new foo && cd foo && cargo wiiu init`.
    New { path: PathBuf },
    /// Initializes an existing project
    Init { path: PathBuf },
    /// Build a project and run it in Cemu. When not specified, the `CEMU` environment variable is used.
    #[command(trailing_var_arg = true, allow_hyphen_values = true)]
    Run {
        #[arg(long)]
        cemu: Option<PathBuf>,
        /// Arguments given directly to `cargo build`
        cargo_args: Vec<String>,
    },
    /// Upload a binary to a console via the wiiload-plugin.
    #[command(trailing_var_arg = true, allow_hyphen_values = true)]
    Upload {
        /// IP address of the console
        #[arg(long)]
        ip: Ipv4Addr,
        /// Binary to upload
        #[arg(value_parser = extension(["rpx", "wuhb"]))]
        binary: Option<PathBuf>,
        /// Arguments given directly to `cargo build` if an explicit binary is not provided
        cargo_args: Vec<String>,
    },
    /// Display output from a console's serial port
    Logger {
        /// Serial port (e.g. /dev/ttyUSB0 or COM3). Lists available ports if omitted.
        port: Option<String>,
        /// Baud rate
        #[arg(long, default_value_t = 57600)]
        baud: u32,
        /// List non-USB ports too (only used if no port is given)
        #[arg(long)]
        all: bool,
        /// Only show lines matching this regex. Can be given multiple times; a line is shown if any matches.
        #[arg(short, long, value_parser = regex::bytes::Regex::new)]
        include: Vec<regex::bytes::Regex>,
        /// Hide lines matching this regex. Can be given multiple times.
        #[arg(short, long, value_parser = regex::bytes::Regex::new)]
        exclude: Vec<regex::bytes::Regex>,
        /// Collapse consecutive empty lines into one
        #[arg(long)]
        trim: bool,
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
    /// Convert RPX to WUHB
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
    /// Convert RPX/RPL back to ELF
    Elf {
        /// Path to the rpx / rpl binary
        #[arg(value_parser = extension(["rpx", "rpl"]))]
        input: PathBuf,
        /// Path to the resulting elf binary. Defaults to input path with ".elf" appended.
        #[arg(value_parser = extension(["elf"]))]
        elf: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::default()
        .parse_default_env()
        .format_timestamp(None)
        .format_module_path(false)
        .format_target(false)
        .init();

    let matches = Cargo::command().get_matches();
    let Cargo::Input(input) = Cargo::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());

    match input.cmd {
        Commands::Build { cargo_args } => {
            build(&cargo_args)?;
        }
        Commands::New { path } => {
            new(&path)?;
            init(&path)?;
        }
        Commands::Init { path } => init(&path)?,
        Commands::Run { cemu, cargo_args } => {
            let binary = build(&cargo_args)?;

            let cemu = cemu.unwrap_or_else(|| {
                PathBuf::from(
                    std::env::var("CEMU")
                        .expect("Either `--cemu <PATH>` or env var `CEMU` must be specified"),
                )
            });

            let output = Command::new(cemu)
                .arg("-g")
                .arg(binary.canonicalize()?)
                .output()?;

            if !output.status.success() {
                eprint!("{}", String::from_utf8_lossy(&output.stdout));
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
            }
        }
        Commands::Upload {
            ip,
            binary,
            cargo_args,
        } => {
            log::info!("Read input file");

            let path = match binary {
                Some(path) => path,
                None => build(&cargo_args)?,
            };

            let data =
                fs::read(&path).context(format!("Failed to read file: {}", path.display()))?;

            upload::upload_binary(data, ip)?;
        }
        Commands::Logger {
            port,
            baud,
            all,
            include,
            exclude,
            trim,
        } => match port {
            Some(port) => logger::run(&port, baud, &include, &exclude, trim)?,
            None => logger::list_ports(all)?,
        },
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
        Commands::Elf { input, elf } => {
            let elf = elf.unwrap_or_else(|| {
                let mut path = input.clone().into_os_string();
                path.push(".elf");
                PathBuf::from(path)
            });

            log::info!("Read input file");
            let data =
                fs::read(&input).context(format!("Failed to read file: {}", input.display()))?;

            let output = rpl::to_elf(data)?;

            log::info!("Write output file");
            fs::write(&elf, output).context(format!("Failed to write file: {}", elf.display()))?;
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

            let name_from_cli = matches
                .subcommand_matches("wiiu")
                .and_then(|m| m.subcommand_matches("wuhb"))
                .and_then(|m| m.value_source("name"))
                .is_some_and(|source| source != ValueSource::DefaultValue);

            log::info!("Read manifest file");
            config.read_manifest_metadata(name_from_cli)?;

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

fn build(args: &Vec<String>) -> anyhow::Result<PathBuf> {
    let status = Command::new("cargo")
        .arg("build")
        .args(args)
        .status()
        .context("Failed to build")?;

    anyhow::ensure!(status.success(), "`cargo build` failed with {status}");

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
        .join("powerpc-nintendo-cafe") // powerpc-cafe-nintendo
        .join(profile)
        .join(name);

    let mut final_binary;

    {
        let input = fs::read(binary.with_extension("elf")).context("Failed to read elf file")?;

        let output = rpl::from_elf(input, false);

        final_binary = binary.with_extension("rpx");

        fs::write(&final_binary, output).context("Failed to write rpx file")?;
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
        config.read_manifest_metadata(false)?;

        let input = fs::read(&binary.with_extension("rpx")).context("Failed to read rpx file")?;

        let output = wuhb::from_rpx(input, config)?;

        final_binary = binary.with_extension("wuhb");
        fs::write(&final_binary, output).context("Failed to write rpx file")?;
    }

    Ok(final_binary)
}
