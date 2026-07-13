//! keel-probe — a dev harness that drives the keel MTP kernel two ways:
//!
//!   * DIRECT (default): calls `keel-vfs` in-process — no FFI, no JSON, fast
//!     iteration.
//!   * `--via-ffi`: `dlopen`s the built `libkeel.dylib` and drives the frozen
//!     C ABI (Initialize/Walk/UploadFiles/…), collecting every callback payload
//!     and printing it (or dumping numbered `%04d.json` files).
//!
//! Subcommands:
//!   info | storages | walk <path> [--recursive] | up <local> <remote> |
//!   down <remote> <local> | rm <path> | mv <path> <newname> | mkdir <path> |
//!   exists <path...> | golden | soak <local-tree>
//!
//! Argument parsing is hand-rolled (no clap, zero deps). Logging is
//! `env_logger` driven by `RUST_LOG`.

use std::path::PathBuf;
use std::process::exit;

mod direct;
mod ffi;
mod util;

/// Global options shared by both modes; parsed off the flat flag stream.
pub struct Options {
    /// `--via-ffi`: drive the C ABI instead of keel-vfs directly.
    pub via_ffi: bool,
    /// `--dump-dir <dir>` (ffi mode): write each callback payload to
    /// `<dir>/%04d.json`.
    pub dump_dir: Option<PathBuf>,
    /// `--lib <path>` (ffi mode): explicit `libkeel.dylib` path.
    pub lib: Option<PathBuf>,
    /// `--release` (ffi mode): default lib dir = `target/release` (else debug).
    pub release: bool,
    /// `--storage <sid>`: use this storage id instead of the first discovered.
    pub storage: Option<u32>,
    /// `--debug`: pass `Init.debug_mode = true` to keel-vfs (direct mode).
    pub debug_mode: bool,
    /// `--recursive` / `-r`: walk recursively (consumed by `walk`).
    pub recursive: bool,
    /// `--iterations` / `-n`: soak loop count (default 10).
    pub iterations: u32,
}

/// The parsed subcommand + its positional operands.
pub enum Command {
    Info,
    Storages,
    Walk { path: String },
    Up { local: String, remote: String },
    Down { remote: String, local: String },
    Rm { path: String },
    Mv { path: String, new_name: String },
    Mkdir { path: String },
    Thumb { path: String },
    Exists { paths: Vec<String> },
    Golden,
    Soak { tree: String },
}

fn usage() -> &'static str {
    "\
keel-probe — drive the keel MTP kernel (direct or via the C ABI)

USAGE:
    keel-probe [GLOBAL FLAGS] <subcommand> [ARGS]

GLOBAL FLAGS:
    --via-ffi              dlopen libkeel.dylib and drive the C ABI
    --dump-dir <dir>       (ffi) write each callback payload to <dir>/%04d.json
    --lib <path>           (ffi) explicit path to libkeel.dylib
    --release              (ffi) default lib dir = target/release (else debug)
    --storage <sid>        use storage id <sid> (default: first discovered)
    --debug                keel-vfs Init.debug_mode = true (direct mode)
    -r, --recursive        recurse (walk)
    -n, --iterations <N>   soak loop count (default 10)
    -h, --help             this help

SUBCOMMANDS:
    info                       device + USB info
    storages                   list storages
    walk <path> [--recursive]  list a directory tree
    up <local> <remote>        upload a local file/dir to the device
    down <remote> <local>      download a device file/dir to local disk
    rm <path>                  delete a device object (device-side recursive)
    mv <path> <newname>        rename a device object
    mkdir <path>               mkdir -p on the device
    exists <path...>           existence check for one or more device paths
    golden                     scripted session mirroring Swift --golden
    soak <local-tree>          upload+download loop w/ random cancel injection
"
}

fn fail_usage(msg: &str) -> ! {
    eprintln!("error: {msg}\n\n{}", usage());
    exit(2);
}

fn main() {
    // env_logger driven by RUST_LOG.
    env_logger::Builder::from_default_env()
        .format_timestamp_millis()
        .init();

    let raw: Vec<String> = std::env::args().skip(1).collect();

    let mut opts = Options {
        via_ffi: false,
        dump_dir: None,
        lib: None,
        release: false,
        storage: None,
        debug_mode: false,
        recursive: false,
        iterations: 10,
    };
    let mut positional: Vec<String> = Vec::new();

    // Single flat pass: flags may appear anywhere; everything else is positional.
    let mut it = raw.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--via-ffi" => opts.via_ffi = true,
            "--release" => opts.release = true,
            "--debug" => opts.debug_mode = true,
            "-r" | "--recursive" => opts.recursive = true,
            "-h" | "--help" => {
                print!("{}", usage());
                exit(0);
            }
            "--dump-dir" => {
                let v = it.next().unwrap_or_else(|| fail_usage("--dump-dir needs a value"));
                opts.dump_dir = Some(PathBuf::from(v));
            }
            "--lib" => {
                let v = it.next().unwrap_or_else(|| fail_usage("--lib needs a value"));
                opts.lib = Some(PathBuf::from(v));
            }
            "--storage" | "--sid" => {
                let v = it.next().unwrap_or_else(|| fail_usage("--storage needs a value"));
                opts.storage = Some(
                    parse_u32(&v).unwrap_or_else(|| fail_usage("--storage must be a number")),
                );
            }
            "-n" | "--iterations" => {
                let v = it.next().unwrap_or_else(|| fail_usage("--iterations needs a value"));
                opts.iterations = v
                    .parse()
                    .unwrap_or_else(|_| fail_usage("--iterations must be a number"));
            }
            s if s.starts_with("--") => fail_usage(&format!("unknown flag {s}")),
            s => positional.push(s.to_string()),
        }
    }

    let cmd = parse_command(&positional);

    // --dump-dir is meaningless without --via-ffi; warn but proceed.
    if opts.dump_dir.is_some() && !opts.via_ffi {
        log::warn!("--dump-dir is only used with --via-ffi; ignoring for direct mode");
    }

    let result = if opts.via_ffi {
        ffi::run(cmd, &opts)
    } else {
        direct::run(cmd, &opts)
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        exit(1);
    }
}

/// Accept both decimal and `0x`-prefixed hex storage ids.
fn parse_u32(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

fn parse_command(pos: &[String]) -> Command {
    let name = pos.first().map(String::as_str).unwrap_or("");
    let rest = &pos[pos.len().min(1)..];

    match name {
        "" => fail_usage("no subcommand given"),
        "info" => Command::Info,
        "storages" => Command::Storages,
        "walk" => {
            let path = rest
                .first()
                .cloned()
                .unwrap_or_else(|| fail_usage("walk needs a <path>"));
            Command::Walk { path }
        }
        "up" => {
            if rest.len() < 2 {
                fail_usage("up needs <local> <remote>");
            }
            Command::Up {
                local: rest[0].clone(),
                remote: rest[1].clone(),
            }
        }
        "down" => {
            if rest.len() < 2 {
                fail_usage("down needs <remote> <local>");
            }
            Command::Down {
                remote: rest[0].clone(),
                local: rest[1].clone(),
            }
        }
        "rm" => {
            let path = rest
                .first()
                .cloned()
                .unwrap_or_else(|| fail_usage("rm needs a <path>"));
            Command::Rm { path }
        }
        "mv" => {
            if rest.len() < 2 {
                fail_usage("mv needs <path> <newname>");
            }
            Command::Mv {
                path: rest[0].clone(),
                new_name: rest[1].clone(),
            }
        }
        "mkdir" => {
            let path = rest
                .first()
                .cloned()
                .unwrap_or_else(|| fail_usage("mkdir needs a <path>"));
            Command::Mkdir { path }
        }
        "thumb" => {
            let path = rest
                .first()
                .cloned()
                .unwrap_or_else(|| fail_usage("thumb needs a <path>"));
            Command::Thumb { path }
        }
        "exists" => {
            if rest.is_empty() {
                fail_usage("exists needs at least one <path>");
            }
            Command::Exists {
                paths: rest.to_vec(),
            }
        }
        "golden" => Command::Golden,
        "soak" => {
            let tree = rest
                .first()
                .cloned()
                .unwrap_or_else(|| fail_usage("soak needs a <local-tree>"));
            Command::Soak { tree }
        }
        other => fail_usage(&format!("unknown subcommand {other}")),
    }
}
