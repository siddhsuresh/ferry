//! Direct mode — drive `keel-vfs` in-process, no FFI. This is the
//! fast-iteration path: the same calls go-mtpx's `main.go` exports make, exposed
//! as the probe subcommands. Every op opens its own session and disposes it,
//! except `golden` (one scripted session) and `soak` (a transfer loop).
//!
//! The scripted `golden` session is a line-for-line port of Swift
//! `Probe.runGoldenSession` (Sources/FerryProbe/Probe.swift:131-227): same steps,
//! same order, same device paths, printing `✓`/`✗` per step.

use std::error::Error;
use std::fs::Metadata;

use keel_vfs::device::{self, Device, Init};
use keel_vfs::dirops::{delete_file, file_exists, make_directory, rename_file};
use keel_vfs::download::download_files;
use keel_vfs::path::FileProp;
use keel_vfs::upload::upload_files;
use keel_vfs::walk::walk;
use keel_vfs::{FileInfo, ProgressInfo, TransferStatus, VfsError};

use crate::util::{self, CancelInjector, Rng};
use crate::{Command, Options};

/// The device staging dir the golden session creates/removes (Probe.swift:160).
const GOLDEN_BASE: &str = "/Download/keel-golden-test";
/// The device staging dir the soak loop uses.
const SOAK_BASE: &str = "/Download/keel-soak";

pub fn run(cmd: Command, opts: &Options) -> Result<(), Box<dyn Error>> {
    match cmd {
        Command::Info => cmd_info(opts),
        Command::Storages => cmd_storages(opts),
        Command::Walk { path } => cmd_walk(&path, opts),
        Command::Up { local, remote } => cmd_up(&local, &remote, opts),
        Command::Down { remote, local } => cmd_down(&remote, &local, opts),
        Command::Rm { path } => cmd_rm(&path, opts),
        Command::Mv { path, new_name } => cmd_mv(&path, &new_name, opts),
        Command::Mkdir { path } => cmd_mkdir(&path, opts),
        Command::Thumb { path } => cmd_thumb(&path, opts),
        Command::Exists { paths } => cmd_exists(&paths, opts),
        Command::Golden => cmd_golden(opts),
        Command::Soak { tree } => cmd_soak(&tree, opts),
    }
}

// ---------------------------------------------------------------------------
// Session helpers
// ---------------------------------------------------------------------------

/// go-mtpx `Initialize` (device.rs:70) — discover + session ladder.
fn open(opts: &Options) -> Result<Device, VfsError> {
    device::initialize(Init {
        debug_mode: opts.debug_mode,
    })
}

/// The storage id to operate on: the `--storage` override, else the first
/// storage go-mtpx's `FetchStorages` returns (Probe.swift:156 picks `first`).
fn resolve_sid(dev: &mut Device, opts: &Options) -> Result<u32, VfsError> {
    if let Some(s) = opts.storage {
        return Ok(s);
    }
    let storages = dev.fetch_storages()?;
    storages
        .first()
        .map(|s| s.sid)
        .ok_or(VfsError::NoStorage)
}

fn print_progress(pi: &ProgressInfo) {
    let a = &pi.active_file_size;
    let b = &pi.bulk_file_size;
    println!(
        "  [{}/{}] {}  {}/{} ({:.1}%)  bulk {:.1}%  {:.2} MB/s  {}",
        pi.files_sent,
        pi.total_files,
        pi.file_info.name,
        util::human_bytes(a.sent),
        util::human_bytes(a.total),
        a.progress,
        b.progress,
        pi.speed,
        pi.status.as_str()
    );
}

// ---------------------------------------------------------------------------
// Simple subcommands
// ---------------------------------------------------------------------------

fn cmd_info(opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let di = dev.fetch_device_info()?;
    {
        // usb_info() is set at configure (device.rs:88); borrow after the mutable
        // fetch_device_info returned its owned value.
        let usb = dev.session().usb_info();
        println!("device connected");
        println!("  manufacturer: {}", di.manufacturer);
        println!("  model:        {}", di.model);
        println!("  version:      {}", di.device_version);
        println!("  serial:       {}", di.serial_number);
        println!("  mtp ext:      {}", di.mtp_extension);
        println!(
            "  usb:          vid={:#06x} pid={:#06x} bcd={:#06x}",
            usb.vendor_id, usb.product_id, usb.bcd_device
        );
        println!("  usb product:  {}", usb.product);
        println!("  usb serial:   {}", usb.serial);
    }
    dev.dispose();
    Ok(())
}

fn cmd_storages(opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let storages = dev.fetch_storages()?;
    println!("storages ({})", storages.len());
    for s in &storages {
        println!(
            "  [{}] {}  free {} / {}",
            s.sid,
            s.info.storage_description,
            util::human_bytes(s.info.free_space_in_bytes as i64),
            util::human_bytes(s.info.max_capability as i64)
        );
    }
    dev.dispose();
    Ok(())
}

fn cmd_walk(path: &str, opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let sid = resolve_sid(&mut dev, opts)?;
    let mut n = 0u64;
    // Swift `list` defaults: skipDisallowedFiles=false, skipHiddenFiles=true
    // (KeelEngine.swift:62-73).
    let (_oid, tf, td) = walk(
        dev.session_mut(),
        sid,
        path,
        opts.recursive,
        false,
        true,
        &mut |_oid: u32, fi: &FileInfo| -> Result<(), VfsError> {
            let marker = if fi.is_dir { "d" } else { "-" };
            let size = if fi.is_dir {
                String::new()
            } else {
                format!("  {}", util::human_bytes(fi.size))
            };
            println!("  {} {}{}", marker, fi.full_path, size);
            n += 1;
            Ok(())
        },
    )?;
    println!("\n{n} entries (files {tf}, dirs {td})");
    dev.dispose();
    Ok(())
}

fn cmd_up(local: &str, remote: &str, opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let sid = resolve_sid(&mut dev, opts)?;
    let sources = vec![local.to_string()];
    let mut files = 0u64;
    let mut ticks = 0u64;
    let mut pre = |_m: &Metadata, p: &str| -> Result<(), VfsError> {
        files += 1;
        log::debug!("preprocess: {p}");
        Ok(())
    };
    let mut prog = |pi: &ProgressInfo| -> Result<(), VfsError> {
        ticks += 1;
        if pi.status == TransferStatus::Completed || ticks % 16 == 0 {
            print_progress(pi);
        }
        Ok(())
    };
    let (_obj, fsent, bytes) = upload_files(
        dev.session_mut(),
        sid,
        &sources,
        remote,
        true,
        &mut pre,
        &mut prog,
        &|| false,
    )?;
    println!(
        "\nuploaded {fsent} files, {} ({files} preprocessed)",
        util::human_bytes(bytes)
    );
    dev.dispose();
    Ok(())
}

fn cmd_down(remote: &str, local: &str, opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let sid = resolve_sid(&mut dev, opts)?;
    // os.Create silently overwrites, so ensure the dest dir exists first.
    if let Some(parent) = std::path::Path::new(local).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let sources = vec![remote.to_string()];
    let mut files = 0u64;
    let mut ticks = 0u64;
    let mut pre = |fi: &FileInfo| -> Result<(), VfsError> {
        files += 1;
        log::debug!("preprocess: {}", fi.full_path);
        Ok(())
    };
    let mut prog = |pi: &ProgressInfo| -> Result<(), VfsError> {
        ticks += 1;
        if pi.status == TransferStatus::Completed || ticks % 16 == 0 {
            print_progress(pi);
        }
        Ok(())
    };
    let (fsent, bytes) = download_files(
        dev.session_mut(),
        sid,
        &sources,
        local,
        true,
        &mut pre,
        &mut prog,
        &|| false,
    )?;
    println!(
        "\ndownloaded {fsent} files, {} ({files} preprocessed)",
        util::human_bytes(bytes)
    );
    dev.dispose();
    Ok(())
}

fn cmd_rm(path: &str, opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let sid = resolve_sid(&mut dev, opts)?;
    delete_file(
        dev.session_mut(),
        sid,
        &[FileProp {
            object_id: 0,
            full_path: path.to_string(),
        }],
    )?;
    println!("deleted {path}");
    dev.dispose();
    Ok(())
}

fn cmd_mv(path: &str, new_name: &str, opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let sid = resolve_sid(&mut dev, opts)?;
    let fp = FileProp {
        object_id: 0,
        full_path: path.to_string(),
    };
    let oid = rename_file(dev.session_mut(), sid, &fp, new_name)?;
    println!("renamed {path} -> {new_name} (object {oid})");
    dev.dispose();
    Ok(())
}

fn cmd_mkdir(path: &str, opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let sid = resolve_sid(&mut dev, opts)?;
    let oid = make_directory(dev.session_mut(), sid, path)?;
    println!("created {path} (object {oid})");
    dev.dispose();
    Ok(())
}

fn cmd_exists(paths: &[String], opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let sid = resolve_sid(&mut dev, opts)?;
    let props: Vec<FileProp> = paths
        .iter()
        .map(|p| FileProp {
            object_id: 0,
            full_path: p.clone(),
        })
        .collect();
    let results = file_exists(dev.session_mut(), sid, &props)?;
    for (p, r) in paths.iter().zip(results.iter()) {
        println!("  {} {}", if r.exists { "yes" } else { "no " }, p);
    }
    // FileExists aborts the batch to an empty vec on an unexpected error type
    // (dirops.rs:153) — surface that rather than silently truncating.
    if results.len() != paths.len() {
        println!(
            "  (batch returned {} of {} results)",
            results.len(),
            paths.len()
        );
    }
    dev.dispose();
    Ok(())
}

// ---------------------------------------------------------------------------
// golden — scripted session mirroring Swift `--golden` (Probe.swift:131-227)
// ---------------------------------------------------------------------------

/// Run a step, swallow its error (like Swift's `try?`/step wrapper), print ✓/✗.
fn step(name: &str, f: impl FnOnce() -> Result<(), VfsError>) {
    match f() {
        Ok(()) => println!("  ✓ {name}"),
        Err(e) => println!("  ✗ {name}: {e}"),
    }
}

fn cmd_golden(opts: &Options) -> Result<(), Box<dyn Error>> {
    println!("golden session (direct keel-vfs)");

    // Step 1: Initialize creates the device (special — it produces `dev`).
    let mut dev = match open(opts) {
        Ok(d) => {
            println!("  ✓ Initialize");
            d
        }
        Err(e) => {
            println!("  ✗ Initialize: {e}");
            return Ok(());
        }
    };

    // Step 2: FetchDeviceInfo.
    step("FetchDeviceInfo", || {
        dev.fetch_device_info()?;
        Ok(())
    });

    // Step 3: FetchStorages → sid.
    let mut sid: u32 = 0;
    step("FetchStorages", || {
        let s = dev.fetch_storages()?;
        sid = opts
            .storage
            .or_else(|| s.first().map(|x| x.sid))
            .unwrap_or(0);
        Ok(())
    });
    if sid == 0 {
        // Probe.swift:158 — guard sid != 0 else fail.
        println!("  ✗ no storage — is the phone unlocked?");
        dev.dispose();
        return Ok(());
    }

    let base = GOLDEN_BASE;

    // Step 4: Walk / and /Download (non-recursive).
    step("Walk /", || {
        walk(
            dev.session_mut(),
            sid,
            "/",
            false,
            false,
            true,
            &mut |_o: u32, _f: &FileInfo| -> Result<(), VfsError> { Ok(()) },
        )?;
        Ok(())
    });
    step("Walk /Download", || {
        walk(
            dev.session_mut(),
            sid,
            "/Download",
            false,
            false,
            true,
            &mut |_o: u32, _f: &FileInfo| -> Result<(), VfsError> { Ok(()) },
        )?;
        Ok(())
    });

    // Step 5: MakeDirectory (fresh + idempotent repeat).
    step("MakeDirectory (idempotent x2)", || {
        make_directory(dev.session_mut(), sid, base)?;
        make_directory(dev.session_mut(), sid, base)?;
        Ok(())
    });

    // Step 6: FileExists (hit + miss).
    step("FileExists (hit + miss)", || {
        let props = vec![
            FileProp {
                object_id: 0,
                full_path: base.to_string(),
            },
            FileProp {
                object_id: 0,
                full_path: format!("{base}/definitely-missing.bin"),
            },
        ];
        let r = file_exists(dev.session_mut(), sid, &props)?;
        for c in &r {
            log::info!("  exists={}", c.exists);
        }
        Ok(())
    });

    // Step 7: UploadFiles (small tree with a subfolder).
    let local = std::env::temp_dir().join("keel-golden-src");
    step("UploadFiles (small tree)", || {
        util::create_golden_src_tree(&local).map_err(|e| VfsError::LocalFile(e.to_string()))?;
        let sources = vec![local.to_string_lossy().into_owned()];
        upload_files(
            dev.session_mut(),
            sid,
            &sources,
            base,
            true,
            &mut |_m: &Metadata, _p: &str| -> Result<(), VfsError> { Ok(()) },
            &mut |_pi: &ProgressInfo| -> Result<(), VfsError> { Ok(()) },
            &|| false,
        )?;
        Ok(())
    });

    // Step 8: Walk the uploaded tree (recursive).
    step("Walk uploaded (recursive)", || {
        walk(
            dev.session_mut(),
            sid,
            base,
            true,
            false,
            true,
            &mut |_o: u32, _f: &FileInfo| -> Result<(), VfsError> { Ok(()) },
        )?;
        Ok(())
    });

    // Step 9: RenameFile.
    step("RenameFile", || {
        let fp = FileProp {
            object_id: 0,
            full_path: format!("{base}/keel-golden-src/blob-1.5mb.bin"),
        };
        rename_file(dev.session_mut(), sid, &fp, "blob-renamed.bin")?;
        Ok(())
    });

    // Step 10: DownloadFiles.
    let dst = std::env::temp_dir().join("keel-golden-dst");
    step("DownloadFiles", || {
        std::fs::create_dir_all(&dst).map_err(|e| VfsError::LocalFile(e.to_string()))?;
        let sources = vec![format!("{base}/keel-golden-src")];
        download_files(
            dev.session_mut(),
            sid,
            &sources,
            &dst.to_string_lossy(),
            true,
            &mut |_f: &FileInfo| -> Result<(), VfsError> { Ok(()) },
            &mut |_pi: &ProgressInfo| -> Result<(), VfsError> { Ok(()) },
            &|| false,
        )?;
        Ok(())
    });

    // Step 11: Error-shape fixtures — operations against missing paths. Each
    // SHOULD fail; the step itself passes (Swift wraps them in try?).
    step("Error fixtures (expected failures)", || {
        let _ = walk(
            dev.session_mut(),
            sid,
            &format!("{base}/no-such-dir"),
            false,
            false,
            true,
            &mut |_o: u32, _f: &FileInfo| -> Result<(), VfsError> { Ok(()) },
        );
        let fp = FileProp {
            object_id: 0,
            full_path: format!("{base}/no-such-file.bin"),
        };
        let _ = rename_file(dev.session_mut(), sid, &fp, "x.bin");
        let _ = delete_file(
            dev.session_mut(),
            sid,
            &[FileProp {
                object_id: 0,
                full_path: format!("{base}/no-such-file.bin"),
            }],
        );
        Ok(())
    });

    // Step 12: DeleteFile (cleanup).
    step("DeleteFile (cleanup)", || {
        delete_file(
            dev.session_mut(),
            sid,
            &[FileProp {
                object_id: 0,
                full_path: base.to_string(),
            }],
        )?;
        Ok(())
    });

    // Cleanup local fixtures + Dispose.
    let _ = std::fs::remove_dir_all(&local);
    let _ = std::fs::remove_dir_all(&dst);
    dev.dispose();
    println!("  ✓ Dispose");
    println!("golden session complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// soak — upload+download loop with random cancel injection
// ---------------------------------------------------------------------------

fn report_soak(label: &str, res: Result<(i64, i64), VfsError>) {
    match res {
        Ok((f, b)) => println!("  {label} ok: {f} files, {}", util::human_bytes(b)),
        // The distinct cancelled variant keel-vfs raises on an injected cancel
        // (upload.rs:184 / download.rs:274).
        Err(VfsError::Cancelled) => println!("  {label} CANCELLED (injected)"),
        Err(e) => println!("  {label} error: {e}"),
    }
}

fn cmd_soak(tree: &str, opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let sid = resolve_sid(&mut dev, opts)?;

    let remote = format!("{SOAK_BASE}/{}", util::base_name(tree));
    let dst = std::env::temp_dir().join("keel-soak-dl");
    let mut rng = Rng::new();

    println!("soak: {} iterations, tree={tree}", opts.iterations);
    for i in 0..opts.iterations {
        println!("iteration {}/{}", i + 1, opts.iterations);

        // ---- upload with random cancel injection ----
        let inj = CancelInjector::new(util::maybe_cancel_ticks(&mut rng));
        if inj.armed() {
            println!("  (upload cancel armed)");
        }
        let sources = vec![tree.to_string()];
        let res = upload_files(
            dev.session_mut(),
            sid,
            &sources,
            SOAK_BASE,
            true,
            &mut |_m: &Metadata, _p: &str| -> Result<(), VfsError> { Ok(()) },
            &mut |_pi: &ProgressInfo| -> Result<(), VfsError> { Ok(()) },
            &|| inj.should(),
        );
        report_soak("upload", res.map(|(_, f, b)| (f, b)));

        // ---- download with random cancel injection ----
        let inj = CancelInjector::new(util::maybe_cancel_ticks(&mut rng));
        if inj.armed() {
            println!("  (download cancel armed)");
        }
        let _ = std::fs::create_dir_all(&dst);
        let res = download_files(
            dev.session_mut(),
            sid,
            &[remote.clone()],
            &dst.to_string_lossy(),
            true,
            &mut |_f: &FileInfo| -> Result<(), VfsError> { Ok(()) },
            &mut |_pi: &ProgressInfo| -> Result<(), VfsError> { Ok(()) },
            &|| inj.should(),
        );
        report_soak("download", res);
    }

    let _ = std::fs::remove_dir_all(&dst);
    dev.dispose();
    println!("soak complete");
    Ok(())
}

fn cmd_thumb(path: &str, opts: &Options) -> Result<(), Box<dyn Error>> {
    let mut dev = open(opts)?;
    let sid = resolve_sid(&mut dev, opts)?;
    match keel_vfs::object::thumbnail(dev.session_mut(), sid, path)? {
        Some(bytes) => println!(
            "thumbnail: {} bytes ({}), magic={:02x?}",
            bytes.len(),
            util::human_bytes(bytes.len() as i64),
            &bytes.iter().take(4).copied().collect::<Vec<_>>()
        ),
        None => println!("no thumbnail for {path}"),
    }
    dev.dispose();
    Ok(())
}
