//! `--via-ffi` mode — `dlopen` the built `libkeel.dylib` and drive the frozen
//! C ABI, exercising the real cdylib boundary (not keel-vfs directly).
//!
//! Every export takes an optional JSON input string plus one or more
//! `on_cb_result_t` callbacks. The header declares the callback param as
//! `on_cb_result_t*`, but the shim treats the *value* as the function pointer
//! itself, so the effective ABI passes the callback fn pointer directly — that
//! is what we do here.
//!
//! Callbacks fire synchronously (done) or from the kernel's 500 ms sampler thread
//! (preprocess/progress) before the export returns; each hands us a malloc'd,
//! NUL-terminated payload it transfers ownership of. We copy it out, free it, and
//! record it: printed raw to stdout, or written to `--dump-dir/%04d.json` with a
//! single shared, monotonically-increasing counter across all callbacks.
//!
//! This module is the crate's only `unsafe`: `libc::dlopen`/`dlsym` +
//! `extern "C"` callback trampolines. Every unsafe op is wrapped in an explicit
//! block with a SAFETY note.

use std::error::Error;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use crate::util::{self, Rng};
use crate::{Command, Options};

/// The device staging dirs the scripted sessions use (must match direct.rs so a
/// `--via-ffi golden` reproduces the same device-side layout).
const GOLDEN_BASE: &str = "/Download/keel-golden-test";
const SOAK_BASE: &str = "/Download/keel-soak";

// ---------------------------------------------------------------------------
// C ABI types
// ---------------------------------------------------------------------------

/// `typedef void (*on_cb_result_t)(char*)`.
type OnCbResult = extern "C" fn(*mut c_char);
/// `void Initialize(on_cb_result_t* onDone)` — one callback.
type FnCb = unsafe extern "C" fn(OnCbResult);
/// `void Walk(char* json, on_cb_result_t* onDone)`.
type FnJsonCb = unsafe extern "C" fn(*const c_char, OnCbResult);
/// `void UploadFiles(char* json, on_cb_result_t* onPre, *onProg, *onDone)`.
type FnJson3Cb = unsafe extern "C" fn(*const c_char, OnCbResult, OnCbResult, OnCbResult);
/// `void CancelTransfer()` — the Ferry cancel extension.
type FnVoid = unsafe extern "C" fn();

// ---------------------------------------------------------------------------
// Payload sink
// ---------------------------------------------------------------------------

enum CbKind {
    Done,
    Pre,
    Prog,
}

struct Sink {
    dump_dir: Option<PathBuf>,
    counter: u32,
    last_done: Option<String>,
}

/// Process-global sink. C callbacks can't capture context and the ABI has no
/// user-data pointer, so payloads route through a global. A `Mutex` because
/// preprocess/progress arrive on the kernel's sampler thread while done arrives
/// on the calling thread.
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

fn install_sink(dump_dir: Option<PathBuf>) {
    *SINK.lock().unwrap() = Some(Sink {
        dump_dir,
        counter: 0,
        last_done: None,
    });
}

/// Record a payload: bump the shared counter, remember the last done payload, and
/// either dump it to `%04d.json` or print it raw to stdout.
fn record(kind: CbKind, payload: String) {
    let mut guard = SINK.lock().unwrap();
    let Some(sink) = guard.as_mut() else {
        return;
    };
    sink.counter += 1;
    let n = sink.counter;
    if matches!(kind, CbKind::Done) {
        sink.last_done = Some(payload.clone());
    }
    match &sink.dump_dir {
        Some(dir) => {
            let path = dir.join(format!("{n:04}.json"));
            if let Err(e) = std::fs::write(&path, &payload) {
                log::error!("failed to write {}: {e}", path.display());
            }
        }
        // Print raw, verbatim (one payload per line) so stdout stays greppable.
        None => println!("{payload}"),
    }
}

fn last_done() -> String {
    SINK.lock()
        .unwrap()
        .as_ref()
        .and_then(|s| s.last_done.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Callback trampolines
// ---------------------------------------------------------------------------

extern "C" fn cb_done(p: *mut c_char) {
    record(CbKind::Done, take_cstr(p));
}
extern "C" fn cb_pre(p: *mut c_char) {
    record(CbKind::Pre, take_cstr(p));
}
extern "C" fn cb_prog(p: *mut c_char) {
    record(CbKind::Prog, take_cstr(p));
}

/// Copy a payload out of its C string and free it.
fn take_cstr(p: *mut c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the ABI hands us a malloc'd, NUL-terminated C string and transfers
    // ownership (keel-ffi's malloc-backed CString). Copy it out, then free with
    // libc::free to match the C allocator.
    unsafe {
        let s = CStr::from_ptr(p).to_string_lossy().into_owned();
        libc::free(p as *mut c_void);
        s
    }
}

// ---------------------------------------------------------------------------
// The loaded kernel
// ---------------------------------------------------------------------------

struct Kernel {
    /// Kept for the process lifetime; we never `dlclose` (the exports may still
    /// have live sampler threads, and the process exits right after).
    #[allow(dead_code)]
    handle: *mut c_void,
    initialize: FnCb,
    fetch_device_info: FnCb,
    fetch_storages: FnCb,
    dispose: FnCb,
    make_directory: FnJsonCb,
    file_exists: FnJsonCb,
    delete_file: FnJsonCb,
    rename_file: FnJsonCb,
    walk: FnJsonCb,
    upload_files: FnJson3Cb,
    download_files: FnJson3Cb,
    /// Optional — absent on kernels built without the cancel extension.
    cancel_transfer: Option<FnVoid>,
}

fn open_lib(path: &Path) -> Result<Kernel, String> {
    let cpath = CString::new(path.as_os_str().to_string_lossy().as_bytes())
        .map_err(|_| "library path contains a NUL byte".to_string())?;

    // SAFETY: standard dlopen — cpath is a valid NUL-terminated path, opened with
    // RTLD_LOCAL + RTLD_NOW.
    let handle = unsafe { libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return Err(format!("dlopen({}) failed: {}", path.display(), dlerror_str()));
    }

    Ok(Kernel {
        handle,
        initialize: sym(handle, "Initialize")?,
        fetch_device_info: sym(handle, "FetchDeviceInfo")?,
        fetch_storages: sym(handle, "FetchStorages")?,
        dispose: sym(handle, "Dispose")?,
        make_directory: sym(handle, "MakeDirectory")?,
        file_exists: sym(handle, "FileExists")?,
        delete_file: sym(handle, "DeleteFile")?,
        rename_file: sym(handle, "RenameFile")?,
        walk: sym(handle, "Walk")?,
        upload_files: sym(handle, "UploadFiles")?,
        download_files: sym(handle, "DownloadFiles")?,
        // CancelTransfer is optional.
        cancel_transfer: sym::<FnVoid>(handle, "CancelTransfer").ok(),
    })
}

fn sym<T>(handle: *mut c_void, name: &str) -> Result<T, String> {
    let cname = CString::new(name).map_err(|_| format!("bad symbol name {name}"))?;
    // SAFETY: `handle` is a live dlopen handle. dlsym returns null (missing) or a
    // valid function pointer. Every `T` we instantiate is an `extern "C" fn`
    // pointer (pointer-sized), so transmute_copy from the pointer-sized dlsym
    // result is size-correct.
    unsafe {
        let raw = libc::dlsym(handle, cname.as_ptr());
        if raw.is_null() {
            return Err(format!("symbol {name} missing from libkeel.dylib"));
        }
        Ok(std::mem::transmute_copy::<*mut c_void, T>(&raw))
    }
}

fn dlerror_str() -> String {
    // SAFETY: dlerror returns a static (or null) C string valid until the next
    // dl* call; we copy it immediately.
    unsafe {
        let e = libc::dlerror();
        if e.is_null() {
            "unknown error".to_string()
        } else {
            CStr::from_ptr(e).to_string_lossy().into_owned()
        }
    }
}

impl Kernel {
    fn call_simple(&self, f: FnCb) -> String {
        // SAFETY: f is a resolved export; cb_done is a valid `extern "C"` fn. The
        // done callback fires synchronously before f returns, setting last_done.
        unsafe { f(cb_done) };
        last_done()
    }

    fn call_json(&self, f: FnJsonCb, json: &str) -> String {
        let c = CString::new(json).unwrap_or_default();
        // SAFETY: c stays alive for the whole (blocking) call; the export copies
        // the input before returning (borrowed-input contract).
        unsafe { f(c.as_ptr(), cb_done) };
        last_done()
    }

    fn call_transfer(&self, f: FnJson3Cb, json: &str) -> String {
        let c = CString::new(json).unwrap_or_default();
        // SAFETY: as call_json; the three callbacks are all valid `extern "C"` fns.
        unsafe { f(c.as_ptr(), cb_pre, cb_prog, cb_done) };
        last_done()
    }

    fn init(&self) -> String {
        self.call_simple(self.initialize)
    }
    fn device_info(&self) -> String {
        self.call_simple(self.fetch_device_info)
    }
    fn storages(&self) -> String {
        self.call_simple(self.fetch_storages)
    }
    fn dispose_dev(&self) -> String {
        self.call_simple(self.dispose)
    }
    fn mkdir(&self, json: &str) -> String {
        self.call_json(self.make_directory, json)
    }
    fn exists(&self, json: &str) -> String {
        self.call_json(self.file_exists, json)
    }
    fn delete(&self, json: &str) -> String {
        self.call_json(self.delete_file, json)
    }
    fn rename(&self, json: &str) -> String {
        self.call_json(self.rename_file, json)
    }
    fn walk_op(&self, json: &str) -> String {
        self.call_json(self.walk, json)
    }
    fn upload(&self, json: &str) -> String {
        self.call_transfer(self.upload_files, json)
    }
    fn download(&self, json: &str) -> String {
        self.call_transfer(self.download_files, json)
    }
}

// ---------------------------------------------------------------------------
// JSON input builders — the exact keys the frozen wire contract expects
// ---------------------------------------------------------------------------

fn walk_json(sid: u32, path: &str, recursive: bool) -> String {
    serde_json::json!({
        "storageId": sid,
        "fullPath": path,
        "recursive": recursive,
        "skipDisallowedFiles": false,
        "skipHiddenFiles": true
    })
    .to_string()
}

fn exists_json(sid: u32, paths: &[String]) -> String {
    // Send lowercase "files"; keel-ffi decodes the key case-insensitively. Kept
    // for wire compatibility with the app.
    serde_json::json!({ "storageId": sid, "files": paths }).to_string()
}

fn delete_json(sid: u32, paths: &[String]) -> String {
    serde_json::json!({ "storageId": sid, "files": paths }).to_string()
}

fn mkdir_json(sid: u32, path: &str) -> String {
    serde_json::json!({ "storageId": sid, "fullPath": path }).to_string()
}

fn rename_json(sid: u32, path: &str, new_name: &str) -> String {
    serde_json::json!({ "storageId": sid, "fullPath": path, "newFileName": new_name }).to_string()
}

fn transfer_json(sid: u32, sources: &[String], dest: &str) -> String {
    serde_json::json!({
        "storageId": sid,
        "sources": sources,
        "destination": dest,
        "preprocessFiles": true
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Payload inspection (envelope: {"errorType":"","error":"","data":...})
// ---------------------------------------------------------------------------

fn payload_ok(payload: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(payload) {
        Ok(v) => v
            .get("errorType")
            .and_then(|x| x.as_str())
            .map(str::is_empty)
            .unwrap_or(false),
        Err(_) => false,
    }
}

fn payload_error(payload: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| payload.to_string())
}

/// Parse `data[0].Sid` out of a FetchStorages payload (shape:
/// `{"data":[{"Sid":65537,...}]}`); `--storage` overrides it.
fn parse_sid(payload: &str, opts: &Options) -> Option<u32> {
    if let Some(s) = opts.storage {
        return Some(s);
    }
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    if !v
        .get("errorType")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .is_empty()
    {
        return None;
    }
    v.get("data")?
        .get(0)?
        .get("Sid")?
        .as_u64()
        .map(|x| x as u32)
}

fn sid_via_ffi(k: &Kernel, opts: &Options) -> Result<u32, Box<dyn Error>> {
    if let Some(s) = opts.storage {
        return Ok(s);
    }
    let payload = k.storages();
    parse_sid(&payload, opts)
        .filter(|&s| s != 0)
        .ok_or_else(|| format!("FetchStorages returned no storage: {payload}").into())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(cmd: Command, opts: &Options) -> Result<(), Box<dyn Error>> {
    let lib_path = resolve_lib_path(opts)?;
    install_sink(opts.dump_dir.clone());
    if let Some(dir) = &opts.dump_dir {
        std::fs::create_dir_all(dir).map_err(|e| format!("--dump-dir {}: {e}", dir.display()))?;
    }
    let k = open_lib(&lib_path)?;
    log::info!("loaded {}", lib_path.display());

    match cmd {
        // Initialize also performs the device-info fetch.
        Command::Info => {
            k.init();
            k.dispose_dev();
        }
        Command::Storages => {
            k.init();
            k.storages();
            k.dispose_dev();
        }
        Command::Walk { path } => {
            k.init();
            let sid = sid_via_ffi(&k, opts)?;
            k.walk_op(&walk_json(sid, &path, opts.recursive));
            k.dispose_dev();
        }
        Command::Up { local, remote } => {
            k.init();
            let sid = sid_via_ffi(&k, opts)?;
            k.upload(&transfer_json(sid, &[local], &remote));
            k.dispose_dev();
        }
        Command::Down { remote, local } => {
            k.init();
            let sid = sid_via_ffi(&k, opts)?;
            k.download(&transfer_json(sid, &[remote], &local));
            k.dispose_dev();
        }
        Command::Rm { path } => {
            k.init();
            let sid = sid_via_ffi(&k, opts)?;
            k.delete(&delete_json(sid, &[path]));
            k.dispose_dev();
        }
        Command::Mv { path, new_name } => {
            k.init();
            let sid = sid_via_ffi(&k, opts)?;
            k.rename(&rename_json(sid, &path, &new_name));
            k.dispose_dev();
        }
        Command::Mkdir { path } => {
            k.init();
            let sid = sid_via_ffi(&k, opts)?;
            k.mkdir(&mkdir_json(sid, &path));
            k.dispose_dev();
        }
        Command::Exists { paths } => {
            k.init();
            let sid = sid_via_ffi(&k, opts)?;
            k.exists(&exists_json(sid, &paths));
            k.dispose_dev();
        }
        Command::Thumb { .. } => {
            eprintln!("thumb is a direct-mode command (drop --via-ffi)");
        }
        Command::Golden => ffi_golden(&k, opts)?,
        Command::Soak { tree } => ffi_soak(&k, opts, &tree)?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// golden via FFI — the scripted session driven over the C ABI
// ---------------------------------------------------------------------------

fn ffi_step(name: &str, payload: String) {
    if payload_ok(&payload) {
        println!("  ✓ {name}");
    } else {
        println!("  ✗ {name}: {}", payload_error(&payload));
    }
}

fn ffi_golden(k: &Kernel, opts: &Options) -> Result<(), Box<dyn Error>> {
    println!("golden session (--via-ffi)");

    ffi_step("Initialize", k.init());
    ffi_step("FetchDeviceInfo", k.device_info());

    let storages_payload = k.storages();
    let sid_opt = parse_sid(&storages_payload, opts);
    ffi_step("FetchStorages", storages_payload);
    let sid = match sid_opt {
        Some(s) if s != 0 => s,
        _ => {
            println!("  ✗ no storage — is the phone unlocked?");
            k.dispose_dev();
            return Ok(());
        }
    };

    let base = GOLDEN_BASE;

    ffi_step("Walk /", k.walk_op(&walk_json(sid, "/", false)));
    ffi_step("Walk /Download", k.walk_op(&walk_json(sid, "/Download", false)));

    // MakeDirectory (fresh + idempotent repeat) — two separate calls so the
    // dump gets two `data:true` payloads.
    ffi_step("MakeDirectory #1", k.mkdir(&mkdir_json(sid, base)));
    ffi_step("MakeDirectory #2", k.mkdir(&mkdir_json(sid, base)));

    ffi_step(
        "FileExists",
        k.exists(&exists_json(
            sid,
            &[
                base.to_string(),
                format!("{base}/definitely-missing.bin"),
            ],
        )),
    );

    let local = std::env::temp_dir().join("keel-golden-src");
    match util::create_golden_src_tree(&local) {
        Ok(()) => ffi_step(
            "UploadFiles",
            k.upload(&transfer_json(
                sid,
                &[local.to_string_lossy().into_owned()],
                base,
            )),
        ),
        Err(e) => println!("  ✗ UploadFiles (local tree): {e}"),
    }

    ffi_step(
        "Walk uploaded (recursive)",
        k.walk_op(&walk_json(sid, base, true)),
    );

    ffi_step(
        "RenameFile",
        k.rename(&rename_json(
            sid,
            &format!("{base}/keel-golden-src/blob-1.5mb.bin"),
            "blob-renamed.bin",
        )),
    );

    let dst = std::env::temp_dir().join("keel-golden-dst");
    let _ = std::fs::create_dir_all(&dst);
    ffi_step(
        "DownloadFiles",
        k.download(&transfer_json(
            sid,
            &[format!("{base}/keel-golden-src")],
            &dst.to_string_lossy(),
        )),
    );

    // Error-shape fixtures — operations against missing paths (payloads dumped;
    // the step itself always "passes" since the errors are swallowed).
    let _ = k.walk_op(&walk_json(sid, &format!("{base}/no-such-dir"), false));
    let _ = k.rename(&rename_json(
        sid,
        &format!("{base}/no-such-file.bin"),
        "x.bin",
    ));
    let _ = k.delete(&delete_json(sid, &[format!("{base}/no-such-file.bin")]));
    println!("  ✓ Error fixtures (expected failures)");

    ffi_step(
        "DeleteFile (cleanup)",
        k.delete(&delete_json(sid, &[base.to_string()])),
    );

    let _ = std::fs::remove_dir_all(&local);
    let _ = std::fs::remove_dir_all(&dst);
    k.dispose_dev();
    println!("  ✓ Dispose");
    println!("golden session complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// soak via FFI — upload+download loop, cancel injected across threads via
// CancelTransfer (an atomic store the kernel polls while the transfer blocks
// the calling thread).
// ---------------------------------------------------------------------------

fn report_ffi(label: &str, payload: &str) {
    if payload_ok(payload) {
        println!("  {label} ok");
    } else {
        println!("  {label} -> {}", payload_error(payload));
    }
}

/// Spawn a thread that waits `fire_ms` then fires `CancelTransfer`, if both an
/// arm delay and the export are present. Returns the join handle to await.
fn arm_canceller(cancel: Option<FnVoid>, fire_ms: Option<u64>) -> Option<thread::JoinHandle<()>> {
    let (Some(f), Some(ms)) = (cancel, fire_ms) else {
        return None;
    };
    println!("  (cancel armed: +{ms}ms)");
    Some(thread::spawn(move || {
        thread::sleep(Duration::from_millis(ms));
        // SAFETY: CancelTransfer takes no args and only does an atomic store;
        // safe to call from any thread while the transfer blocks the main thread.
        // Fn pointers are Send.
        unsafe { f() };
    }))
}

fn join_canceller(guard: Option<thread::JoinHandle<()>>) {
    if let Some(h) = guard {
        let _ = h.join();
    }
}

fn ffi_soak(k: &Kernel, opts: &Options, tree: &str) -> Result<(), Box<dyn Error>> {
    k.init();
    let sid = sid_via_ffi(k, opts)?;
    let remote = format!("{SOAK_BASE}/{}", util::base_name(tree));
    let dst = std::env::temp_dir().join("keel-soak-dl");
    let _ = std::fs::create_dir_all(&dst);
    let mut rng = Rng::new();

    if k.cancel_transfer.is_none() {
        log::warn!("libkeel.dylib has no CancelTransfer export; running without cancel injection");
    }

    println!("soak (--via-ffi): {} iterations, tree={tree}", opts.iterations);
    for i in 0..opts.iterations {
        println!("iteration {}/{}", i + 1, opts.iterations);

        let guard = arm_canceller(k.cancel_transfer, util::maybe_cancel_ms(&mut rng));
        let payload = k.upload(&transfer_json(sid, &[tree.to_string()], SOAK_BASE));
        join_canceller(guard);
        report_ffi("upload", &payload);

        let guard = arm_canceller(k.cancel_transfer, util::maybe_cancel_ms(&mut rng));
        let payload = k.download(&transfer_json(sid, &[remote.clone()], &dst.to_string_lossy()));
        join_canceller(guard);
        report_ffi("download", &payload);
    }

    let _ = std::fs::remove_dir_all(&dst);
    k.dispose_dev();
    println!("soak complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// Library discovery
// ---------------------------------------------------------------------------

fn resolve_lib_path(opts: &Options) -> Result<PathBuf, Box<dyn Error>> {
    if let Some(p) = &opts.lib {
        if p.exists() {
            return Ok(p.clone());
        }
        return Err(format!("--lib {} does not exist", p.display()).into());
    }

    let name = "libkeel.dylib";
    let sub = if opts.release { "release" } else { "debug" };
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Next to the probe binary (target/{debug,release}/) — where cargo places the
    // cdylib built with the same profile as the probe.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(name));
            if let Some(target) = dir.parent() {
                candidates.push(target.join(sub).join(name));
            }
        }
    }
    // CARGO_MANIFEST_DIR = keel/crates/keel-probe → ../../target = keel/target.
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(sub)
            .join(name),
    );
    // Relative to the working dir: run from keel/ (target/…) or keel/crates/
    // (../target/…).
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("target").join(sub).join(name));
        candidates.push(cwd.join("../target").join(sub).join(name));
    }

    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }

    let tried = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n  ");
    Err(format!(
        "could not find {name}; build it with `cargo build -p keel-ffi{}` or pass --lib <path>.\ntried:\n  {tried}",
        if opts.release { " --release" } else { "" }
    )
    .into())
}
