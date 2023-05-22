//! Build CosmosSDK/Tendermint/IBC proto files. This build script clones the CosmosSDK version
//! specified in the COSMOS_SDK_REV constant and then uses that to build the required
//! proto files for further compilation. This is based on the proto-compiler code
//! in github.com/informalsystems/ibc-rs

use regex::Regex;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{self, create_dir_all, remove_dir_all},
    io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{self, AtomicBool},
};
use walkdir::WalkDir;

/// Suppress log messages
// TODO(tarcieri): use a logger for this
static QUIET: AtomicBool = AtomicBool::new(false);

/// The Cosmos SDK commit or tag to be cloned and used to build the proto files
const COSMOS_SDK_REV: &str = "v0.46.12";

/// The Cosmos ibc-go commit or tag to be cloned and used to build the proto files
const IBC_REV: &str = "v5.2.0";

/// The wasmd commit or tag to be cloned and used to build the proto files
const WASMD_REV: &str = "v0.29.2";

const JUNO_REV: &str = "nguyen/replace-validate-claim-logic";

const OSMOSIS_REV: &str = "v10.0.0";

// All paths must end with a / and either be absolute or include a ./ to reference the current
// working directory.

/// The directory generated cosmos-sdk proto files go into in this repo
const COSMOS_SDK_PROTO_DIR: &str = "../juno-rust-proto/src/prost/";
/// Directory where the cosmos-sdk submodule is located
const COSMOS_SDK_DIR: &str = "../dependencies/cosmos-sdk";
/// Directory where the cosmos/ibc-go submodule is located
const IBC_DIR: &str = "../dependencies/ibc-go";
/// Directory where the submodule is located
const WASMD_DIR: &str = "../dependencies/wasmd";
/// Directory where the submodule is located
const JUNO_DIR: &str = "../dependencies/quicksilver";
/// Directory where the submodule is located
const OSMOSIS_DIR: &str = "../dependencies/osmosis";
/// A temporary directory for proto building
const TMP_BUILD_DIR: &str = "/tmp/tmp-protobuf/";

// Patch strings used by `copy_and_patch`

/// Protos belonging to these Protobuf packages will be excluded
/// (i.e. because they are sourced from `tendermint-proto`)
const EXCLUDED_PROTO_PACKAGES: &[&str] = &["gogoproto", "google", "tendermint"];

/// Log info to the console (if `QUIET` is disabled)
// TODO(tarcieri): use a logger for this
macro_rules! info {
    ($msg:expr) => {
        if !is_quiet() {
            println!("[info] {}", $msg)
        }
    };
    ($fmt:expr, $($arg:tt)+) => {
        info!(&format!($fmt, $($arg)+))
    };
}

fn main() {
    if is_github() {
        set_quiet();
    }

    let tmp_build_dir: PathBuf = TMP_BUILD_DIR.parse().unwrap();
    let proto_dir: PathBuf = COSMOS_SDK_PROTO_DIR.parse().unwrap();

    if tmp_build_dir.exists() {
        fs::remove_dir_all(tmp_build_dir.clone()).unwrap();
    }

    let temp_sdk_dir = tmp_build_dir.join("cosmos-sdk");
    let temp_ibc_dir = tmp_build_dir.join("ibc-go");
    let temp_wasmd_dir = tmp_build_dir.join("wasmd");
    let temp_juno_dir = tmp_build_dir.join("quicksilver");
    let temp_osmosis_dir = tmp_build_dir.join("osmosis");


    fs::create_dir_all(&temp_sdk_dir).unwrap();
    fs::create_dir_all(&temp_ibc_dir).unwrap();
    fs::create_dir_all(&temp_wasmd_dir).unwrap();
    fs::create_dir_all(&temp_juno_dir).unwrap();
    fs::create_dir_all(&temp_osmosis_dir).unwrap();


    // cannot update.
    // update_submodules();
    output_sdk_version(&temp_sdk_dir);
    output_ibc_version(&temp_ibc_dir);
    // output_wasmd_version(&temp_wasmd_dir);
    output_juno_version(&temp_juno_dir);
    output_osmosis_version(&temp_osmosis_dir);
    compile_sdk_protos_and_services(&temp_sdk_dir);
    compile_ibc_protos_and_services(&temp_ibc_dir);
    // compile_wasmd_proto_and_services(&temp_wasmd_dir);
    compile_juno_protos_and_services(&temp_juno_dir);
    compile_osmosis_protos_and_services(&temp_osmosis_dir);

    copy_generated_files(&temp_sdk_dir, &proto_dir.join("cosmos-sdk"));
    copy_generated_files(&temp_ibc_dir, &proto_dir.join("ibc-go"));
    // copy_generated_files(&temp_wasmd_dir, &proto_dir.join("wasmd"));
    copy_generated_files(&temp_juno_dir, &proto_dir.join("quicksilver"));
    copy_generated_files(&temp_osmosis_dir, &proto_dir.join("osmosis"));

    apply_patches(&proto_dir);

    info!("Running rustfmt on prost/tonic-generated code");
    run_rustfmt(&proto_dir);

    if is_github() {
        println!(
            "Rebuild protos with proto-build (cosmos-sdk rev: {} ibc-go rev: {} wasmd rev: {}))",
            COSMOS_SDK_REV, IBC_REV, WASMD_REV
        );
    }
}

fn is_quiet() -> bool {
    QUIET.load(atomic::Ordering::Relaxed)
}

fn set_quiet() {
    QUIET.store(true, atomic::Ordering::Relaxed);
}

/// Parse `--github` flag passed to `proto-build` on the eponymous GitHub Actions job.
/// Disables `info`-level log messages, instead outputting only a commit message.
fn is_github() -> bool {
    env::args().any(|arg| arg == "--github")
}

fn run_cmd(cmd: impl AsRef<OsStr>, args: impl IntoIterator<Item = impl AsRef<OsStr>>) {
    let stdout = if is_quiet() {
        process::Stdio::null()
    } else {
        process::Stdio::inherit()
    };

    let exit_status = process::Command::new(&cmd)
        .args(args)
        .stdout(stdout)
        .status()
        .expect("exit status missing");

    if !exit_status.success() {
        panic!(
            "{:?} exited with error code: {:?}",
            cmd.as_ref(),
            exit_status.code()
        );
    }
}

fn run_git(args: impl IntoIterator<Item = impl AsRef<OsStr>>) {
    run_cmd("git", args)
}

fn run_rustfmt(dir: &Path) {
    let mut args = ["--edition", "2021"]
        .iter()
        .map(Into::into)
        .collect::<Vec<OsString>>();

    args.extend(
        WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file() && e.path().extension() == Some(OsStr::new("rs")))
            .map(|e| e.into_path())
            .map(Into::into),
    );

    run_cmd("rustfmt", args);
}

fn update_submodules() {
    info!("Updating cosmos/cosmos-sdk submodule...");
    run_git(&["submodule", "update", "--init"]);
    run_git(&["-C", COSMOS_SDK_DIR, "fetch"]);
    run_git(&["-C", COSMOS_SDK_DIR, "reset", "--hard", COSMOS_SDK_REV]);

    info!("Updating cosmos/ibc-go submodule...");
    run_git(&["submodule", "update", "--init"]);
    run_git(&["-C", IBC_DIR, "fetch"]);
    run_git(&["-C", IBC_DIR, "reset", "--hard", IBC_REV]);

    info!("Updating wasmd submodule...");
    run_git(&["submodule", "update", "--init"]);
    run_git(&["-C", WASMD_DIR, "fetch"]);
    run_git(&["-C", WASMD_DIR, "reset", "--hard", WASMD_REV]);

    // info!("Updating juno submodule...");
    // run_git(&["submodule", "update", "--init"]);
    // run_git(&["-C", JUNO_DIR, "fetch"]);
    // run_git(&["-C", JUNO_DIR, "reset", "--hard", JUNO_REV]);
}

fn output_sdk_version(out_dir: &Path) {
    let path = out_dir.join("COSMOS_SDK_COMMIT");
    fs::write(path, COSMOS_SDK_REV).unwrap();
}

fn output_ibc_version(out_dir: &Path) {
    let path = out_dir.join("IBC_COMMIT");
    fs::write(path, IBC_REV).unwrap();
}

fn output_wasmd_version(out_dir: &Path) {
    let path = out_dir.join("WASMD_COMMIT");
    fs::write(path, WASMD_REV).unwrap();
}

fn output_juno_version(out_dir: &Path) {
    let path = out_dir.join("QUICKSILVER_COMMIT");
    fs::write(path, JUNO_REV).unwrap();
}

fn output_osmosis_version(out_dir: &Path) {
    let path = out_dir.join("OSMOSIS_COMMIT");
    fs::write(path, OSMOSIS_REV).unwrap();
}

fn compile_sdk_protos_and_services(out_dir: &Path) {
    info!(
        "Compiling cosmos-sdk .proto files to Rust into '{}'...",
        out_dir.display()
    );

    let root = env!("CARGO_MANIFEST_DIR");
    let sdk_dir = Path::new(COSMOS_SDK_DIR);

    let proto_includes_paths = [
        format!("{}/../proto", root),
        format!("{}/proto", sdk_dir.display()),
        format!("{}/third_party/proto", sdk_dir.display()),
    ];

    // Paths
    let proto_paths = [
        format!("{}/../proto/definitions/mock", root),
        format!("{}/proto/cosmos/auth", sdk_dir.display()),
        format!("{}/proto/cosmos/authz", sdk_dir.display()),
        format!("{}/proto/cosmos/bank", sdk_dir.display()),
        format!("{}/proto/cosmos/base", sdk_dir.display()),
        format!("{}/proto/cosmos/base/tendermint", sdk_dir.display()),
        format!("{}/proto/cosmos/capability", sdk_dir.display()),
        format!("{}/proto/cosmos/crisis", sdk_dir.display()),
        format!("{}/proto/cosmos/crypto", sdk_dir.display()),
        format!("{}/proto/cosmos/distribution", sdk_dir.display()),
        format!("{}/proto/cosmos/evidence", sdk_dir.display()),
        format!("{}/proto/cosmos/feegrant", sdk_dir.display()),
        format!("{}/proto/cosmos/genutil", sdk_dir.display()),
        format!("{}/proto/cosmos/gov", sdk_dir.display()),
        format!("{}/proto/cosmos/mint", sdk_dir.display()),
        format!("{}/proto/cosmos/params", sdk_dir.display()),
        format!("{}/proto/cosmos/slashing", sdk_dir.display()),
        format!("{}/proto/cosmos/staking", sdk_dir.display()),
        format!("{}/proto/cosmos/tx", sdk_dir.display()),
        format!("{}/proto/cosmos/upgrade", sdk_dir.display()),
        format!("{}/proto/cosmos/vesting", sdk_dir.display()),
    ];

    // List available proto files
    let mut protos: Vec<PathBuf> = vec![];
    collect_protos(&proto_paths, &mut protos);

    // List available paths for dependencies
    let includes: Vec<PathBuf> = proto_includes_paths.iter().map(PathBuf::from).collect();

    // Compile all of the proto files, along with grpc service clients
    info!("Compiling proto definitions and clients for GRPC services!");
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .out_dir(out_dir)
        .extern_path(".tendermint", "::tendermint_proto")
        .compile(&protos, &includes)
        .unwrap();

    info!("=> Done!");
}


fn compile_juno_protos_and_services(out_dir: &Path) {
    info!(
        "Compiling cosmos-sdk .proto files to Rust into '{}'...",
        out_dir.display()
    );

    let root = env!("CARGO_MANIFEST_DIR");
    let sdk_dir = Path::new(JUNO_DIR);

    let proto_includes_paths = [
        format!("{}/../proto", root),
        format!("{}/proto", sdk_dir.display()),
        format!("{}/third_party/proto", sdk_dir.display()),
    ];

    // Paths
    let proto_paths = [
        format!("{}/../proto/definitions/mock", root),
        format!("{}/proto/quicksilver/airdrop", sdk_dir.display()),
        format!("{}/proto/quicksilver/claimsmanager", sdk_dir.display()),
        format!("{}/proto/quicksilver/epochs", sdk_dir.display()),
        format!("{}/proto/quicksilver/interchainquery", sdk_dir.display()),
        format!("{}/proto/quicksilver/interchainstaking", sdk_dir.display()),
        format!("{}/proto/quicksilver/mint", sdk_dir.display()),
        format!("{}/proto/quicksilver/participationrewards", sdk_dir.display()),
        format!("{}/proto/quicksilver/tokenfactory", sdk_dir.display()),
    ];

    // List available proto files
    let mut protos: Vec<PathBuf> = vec![];
    collect_protos(&proto_paths, &mut protos);

    // List available paths for dependencies
    let includes: Vec<PathBuf> = proto_includes_paths.iter().map(PathBuf::from).collect();

    // Compile all of the proto files, along with grpc service clients
    info!("Compiling proto definitions and clients for GRPC services!");
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .out_dir(out_dir)
        .extern_path(".tendermint", "::tendermint_proto")
        .compile(&protos, &includes)
        .unwrap();

    info!("=> Done!");
}

fn compile_osmosis_protos_and_services(out_dir: &Path) {
    info!(
        "Compiling cosmos-sdk .proto files to Rust into '{}'...",
        out_dir.display()
    );

    let root = env!("CARGO_MANIFEST_DIR");
    let sdk_dir = Path::new(OSMOSIS_DIR);

    let proto_includes_paths = [
        format!("{}/../proto", root),
        format!("{}/proto", sdk_dir.display()),
        format!("{}/third_party/proto", sdk_dir.display()),
    ];

    // Paths
    let proto_paths = [
        format!("{}/../proto/definitions/mock", root),
        format!("{}/proto/osmosis/gamm/pool-models/balancer", sdk_dir.display()),
        format!("{}/proto/osmosis/gamm/pool-models/stableswap", sdk_dir.display()),
        format!("{}/proto/osmosis/gamm/v1beta1", sdk_dir.display()),
        format!("{}/proto/osmosis/lockup", sdk_dir.display()),
    ];

    // List available proto files
    let mut protos: Vec<PathBuf> = vec![];
    collect_protos(&proto_paths, &mut protos);

    // List available paths for dependencies
    let includes: Vec<PathBuf> = proto_includes_paths.iter().map(PathBuf::from).collect();

    // Compile all of the proto files, along with grpc service clients
    info!("Compiling proto definitions and clients for GRPC services!");
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .out_dir(out_dir)
        .extern_path(".tendermint", "::tendermint_proto")
        .compile(&protos, &includes)
        .unwrap();

    info!("=> Done!");
}

fn compile_wasmd_proto_and_services(out_dir: &Path) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let sdk_dir = PathBuf::from(WASMD_DIR);

    let proto_includes_paths = [
        root.join("../proto"),
        sdk_dir.join("proto"),
        sdk_dir.join("third_party/proto"),
    ];

    // List available paths for dependencies
    let includes: Vec<PathBuf> = proto_includes_paths.iter().map(PathBuf::from).collect();

    let proto_paths = [format!("{}/proto/cosmwasm/wasm", sdk_dir.display())];

    // List available proto files
    let mut protos: Vec<PathBuf> = vec![];
    collect_protos(&proto_paths, &mut protos);

    // Compile all proto client for GRPC services
    info!("Compiling wasmd proto clients for GRPC services!");
    tonic_build::configure()
        .build_client(true)
        .build_server(false)
        .out_dir(out_dir)
        .compile(&protos, &includes)
        .unwrap();

    info!("=> Done!");
}

fn compile_ibc_protos_and_services(out_dir: &Path) {
    info!(
        "Compiling .proto files to Rust into '{}'...",
        out_dir.display()
    );

    let root = env!("CARGO_MANIFEST_DIR");
    let ibc_dir = Path::new(IBC_DIR);

    let proto_includes_paths = [
        format!("{}/../proto", root),
        format!("{}/proto", ibc_dir.display()),
        format!("{}/third_party/proto", ibc_dir.display()),
    ];

    let proto_paths = [
        format!("{}/../proto/definitions/mock", root),
        format!(
            "{}/proto/ibc/applications/interchain_accounts",
            ibc_dir.display()
        ),
        format!("{}/proto/ibc/applications/transfer", ibc_dir.display()),
        format!("{}/proto/ibc/core/channel", ibc_dir.display()),
        format!("{}/proto/ibc/core/client", ibc_dir.display()),
        format!("{}/proto/ibc/core/commitment", ibc_dir.display()),
        format!("{}/proto/ibc/core/connection", ibc_dir.display()),
        format!("{}/proto/ibc/core/port", ibc_dir.display()),
        format!("{}/proto/ibc/core/types", ibc_dir.display()),
        format!("{}/proto/ibc/lightclients/localhost", ibc_dir.display()),
        format!("{}/proto/ibc/lightclients/solomachine", ibc_dir.display()),
        format!("{}/proto/ibc/lightclients/tendermint", ibc_dir.display()),
    ];
    // List available proto files
    let mut protos: Vec<PathBuf> = vec![];
    collect_protos(&proto_paths, &mut protos);

    let includes: Vec<PathBuf> = proto_includes_paths.iter().map(PathBuf::from).collect();

    // Compile all of the proto files, along with the grpc service clients
    info!("Compiling proto definitions and clients for GRPC services!");
    tonic_build::configure()
        .build_client(true)
        .build_server(false)
        .out_dir(out_dir)
        .extern_path(".tendermint", "::tendermint_proto")
        .compile(&protos, &includes)
        .unwrap();

    info!("=> Done!");
}

/// collect_protos walks every path in `proto_paths` and recursively locates all .proto
/// files in each path's subdirectories, adding the full path of each file to `protos`
///
/// Any errors encountered will cause failure for the path provided to WalkDir::new()
fn collect_protos(proto_paths: &[String], protos: &mut Vec<PathBuf>) {
    for proto_path in proto_paths {
        protos.append(
            &mut WalkDir::new(proto_path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_type().is_file()
                        && e.path().extension().is_some()
                        && e.path().extension().unwrap() == "proto"
                })
                .map(|e| e.into_path())
                .collect(),
        );
    }
}

fn copy_generated_files(from_dir: &Path, to_dir: &Path) {
    info!("Copying generated files into '{}'...", to_dir.display());

    // Remove old compiled files
    remove_dir_all(&to_dir).unwrap_or_default();
    create_dir_all(&to_dir).unwrap();

    let mut filenames = Vec::new();

    // Copy new compiled files (prost does not use folder structures)
    let errors = WalkDir::new(from_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let filename = e.file_name().to_os_string().to_str().unwrap().to_string();
            filenames.push(filename.clone());
            copy_and_patch(e.path(), format!("{}/{}", to_dir.display(), &filename))
        })
        .filter_map(|e| e.err())
        .collect::<Vec<_>>();

    if !errors.is_empty() {
        for e in errors {
            eprintln!("[error] Error while copying compiled file: {}", e);
        }

        panic!("[error] Aborted.");
    }
}

fn copy_and_patch(src: impl AsRef<Path>, dest: impl AsRef<Path>) -> io::Result<()> {
    /// Regex substitutions to apply to the prost-generated output
    const REPLACEMENTS: &[(&str, &str)] = &[
        // Use `tendermint-proto` proto definitions
        ("(super::)+tendermint", "tendermint_proto"),
        // Feature-gate gRPC client modules
        (
            "/// Generated client implementations.",
            "/// Generated client implementations.\n\
             #[cfg(feature = \"grpc\")]\n\
             #[cfg_attr(docsrs, doc(cfg(feature = \"grpc\")))]",
        ),
        // Feature-gate gRPC impls which use `tonic::transport`
        (
            "impl(.+)tonic::transport(.+)",
            "#[cfg(feature = \"grpc-transport\")]\n    \
             #[cfg_attr(docsrs, doc(cfg(feature = \"grpc-transport\")))]\n    \
             impl${1}tonic::transport${2}",
        ),
        // Feature-gate gRPC server modules
        (
            "/// Generated server implementations.",
            "/// Generated server implementations.\n\
             #[cfg(feature = \"grpc\")]\n\
             #[cfg_attr(docsrs, doc(cfg(feature = \"grpc\")))]",
        ),
    ];

    // Skip proto files belonging to `EXCLUDED_PROTO_PACKAGES`
    for package in EXCLUDED_PROTO_PACKAGES {
        if let Some(filename) = src.as_ref().file_name().and_then(OsStr::to_str) {
            if filename.starts_with(&format!("{}.", package)) {
                return Ok(());
            }
        }
    }

    let mut contents = fs::read_to_string(src)?;

    for &(regex, replacement) in REPLACEMENTS {
        contents = Regex::new(regex)
            .unwrap_or_else(|_| panic!("invalid regex: {}", regex))
            .replace_all(&contents, replacement)
            .to_string();
    }

    fs::write(dest, &contents)
}

fn patch_file(path: impl AsRef<Path>, pattern: &Regex, replacement: &str) -> io::Result<()> {
    let mut contents = fs::read_to_string(&path)?;
    contents = pattern.replace_all(&contents, replacement).to_string();
    fs::write(path, &contents)
}

/// Fix clashing type names in prost-generated code. See cosmos/cosmos-rust#154.
fn apply_patches(proto_dir: &Path) {
    for (pattern, replacement) in [
        ("enum Validators", "enum Policy"),
        (
            "stake_authorization::Validators",
            "stake_authorization::Policy",
        ),
    ] {
        patch_file(
            &proto_dir.join("cosmos-sdk/cosmos.staking.v1beta1.rs"),
            &Regex::new(pattern).unwrap(),
            replacement,
        )
        .expect("error patching cosmos.staking.v1beta1.rs");
    }
}
