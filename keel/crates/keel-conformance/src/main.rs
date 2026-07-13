//! keel-conformance — the differential oracle.
//!
//! `dlopen`s two frozen-ABI dylibs (`--go` and `--rust`), runs the IDENTICAL
//! scripted "golden" session against the attached phone *sequentially* (the go
//! side first, `Dispose` to release the USB device, then the rust side — the two
//! kernels cannot share the device), captures every callback payload per side,
//! normalizes the volatile fields, then does a structural, exact-key-casing,
//! JSON-pointer diff. Exit code 0 only on full parity (documented intentional
//! divergences do not count against it).
//!
//! Dependency-light on purpose: `libc` (dlopen/dlsym/free) + `serde_json` only.
//! No `libloading`, no clap, no async runtime.
//!
//! ── The callback ABI (the load-bearing quirk) ────────────────────────────────
//! The ABI header declares each callback slot as `on_cb_result_t*`
//! (pointer-to-function-pointer), but the C shim casts that pointer value
//! straight to a function pointer and calls it — the pointer *bits* are the fn,
//! never dereferenced. So from the caller's side each export takes a plain
//! `extern "C" fn(*mut c_char)` value in those slots.
//!
//! Payloads are `malloc`'d NUL-terminated UTF-8 C strings; the *caller* owns
//! them — we copy, then `libc::free`. Both dylibs use the same system allocator,
//! so `free` is correct for both.

use libc::{c_char, c_void};
use serde_json::Value;
use std::ffi::{CStr, CString};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ─────────────────────────────────────────────────────────────────────────────
// Volatile-field policy
// ─────────────────────────────────────────────────────────────────────────────
//
// Fields that legitimately vary between two independent runs against the same
// device and MUST be normalized away before diffing: elapsedTime, speed,
// dateAdded, and any *_time. We additionally normalize `objectId` and
// `parentId`: MTP object *handles* are assigned by the device per session, so
// any object freshly created inside the session (the uploaded tree, step "walk
// after upload") gets a different handle on the go side than on the rust side.
// Without this, every post-write walk entry would falsely diverge on its
// handle — the same category as a per-run volatile field.
//
// Not normalized (deliberately): name / path / parentPath / extension / size /
// isFolder / Sid / the error envelope strings — those are the parity signal.
const VOLATILE_EXACT_KEYS: &[&str] = &[
    "elapsedTime", // transfer wall-clock, ms
    "speed",       // instantaneous MB/s
    "dateAdded",   // modification time; fresh uploads get a new mtime each run
    "objectId",    // device-assigned MTP handle — differs per session
    "parentId",    // device-assigned MTP handle — differs per session
    // Live-device storage counters: the oracle runs the whole go session, then
    // the whole rust session; Android writes to /data constantly, so these drift
    // between the two runs (measured: ~296 KB across one session; ~13 MB over a
    // few minutes). Not a decode difference — two back-to-back reads are byte-
    // identical. Same category as speed/timestamps.
    "FreeSpaceInBytes",
    "FreeSpaceInImages",
];

/// True for any key we treat as volatile: the explicit set above, or any key
/// whose name ends in "time" (covers elapsedTime and any future `*_time`/`*Time`).
fn is_volatile_key(k: &str) -> bool {
    VOLATILE_EXACT_KEYS.contains(&k) || k.to_ascii_lowercase().ends_with("time")
}

/// Relative tolerance for non-integer numeric comparison. The two kernels' JSON
/// encoders render floats differently (~6 significant digits vs full precision),
/// so we compare non-integers with tolerance and only integers exactly. This
/// absorbs the formatting deviation without hiding a real ~0.1% computation bug.
const FLOAT_REL_EPS: f64 = 1e-4;
const FLOAT_ABS_EPS: f64 = 1e-6;

// ─────────────────────────────────────────────────────────────────────────────
// Callback capture (global, because the FFI callbacks are plain C fn pointers
// that cannot carry state, and the 500 ms sampler thread inside Upload/Download
// invokes preprocess/progress from a *different* thread than the caller).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CbKind {
    Done,
    Preprocess,
    Progress,
}

static CAPTURE: Mutex<Vec<(CbKind, String)>> = Mutex::new(Vec::new());

/// Callback body: copy the malloc'd payload, free it, stash the text. Must never
/// panic — it runs across the C FFI boundary where unwinding would be UB. Every
/// operation here is panic-free (null check, lossy UTF-8, poison-recovering
/// lock).
fn capture(kind: CbKind, s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: the shim guarantees `s` is a NUL-terminated C string valid for the
    // duration of this call. Copy the bytes out before freeing.
    let bytes = unsafe { CStr::from_ptr(s) }.to_bytes().to_vec();
    // SAFETY: payload was malloc'd by the dylib; caller owns it (see module doc).
    unsafe { libc::free(s as *mut c_void) };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut g = CAPTURE.lock().unwrap_or_else(|e| e.into_inner());
    g.push((kind, text));
}

extern "C" fn cb_done(s: *mut c_char) {
    capture(CbKind::Done, s);
}
extern "C" fn cb_preprocess(s: *mut c_char) {
    capture(CbKind::Preprocess, s);
}
extern "C" fn cb_progress(s: *mut c_char) {
    capture(CbKind::Progress, s);
}

// The callback fn-pointer value we drop into the `on_cb_result_t*` slots.
type OnCb = Option<extern "C" fn(*mut c_char)>;

// Export ABIs, by argument shape. The callback slots take the fn-pointer-as-value
// (see module doc); a nullable fn pointer (`Option<fn>`) is ABI-identical to the
// `void*`-sized slot the C side expects.
type FnCb = unsafe extern "C" fn(OnCb); // Initialize / FetchDeviceInfo / FetchStorages / Dispose
type FnJson1 = unsafe extern "C" fn(*const c_char, OnCb); // MakeDirectory / FileExists / DeleteFile / RenameFile / Walk
type FnJson3 = unsafe extern "C" fn(*const c_char, OnCb, OnCb, OnCb); // UploadFiles / DownloadFiles

// ─────────────────────────────────────────────────────────────────────────────
// Dylib loading
// ─────────────────────────────────────────────────────────────────────────────

struct Lib {
    label: &'static str,
    handle: *mut c_void,
    initialize: FnCb,
    fetch_device_info: FnCb,
    fetch_storages: FnCb,
    dispose: FnCb,
    make_directory: FnJson1,
    file_exists: FnJson1,
    delete_file: FnJson1,
    rename_file: FnJson1,
    walk: FnJson1,
    upload_files: FnJson3,
    download_files: FnJson3,
}

fn dlerror_text() -> String {
    // SAFETY: dlerror returns a static/thread-local C string or null.
    let p = unsafe { libc::dlerror() };
    if p.is_null() {
        "unknown dl error".to_string()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

fn load_symbol(handle: *mut c_void, name: &str) -> Result<*mut c_void, String> {
    let cname = CString::new(name).map_err(|e| e.to_string())?;
    // Clear any stale error, then resolve. dlsym can legitimately return null
    // for a null-valued symbol, so we check dlerror rather than the pointer.
    unsafe { libc::dlerror() };
    let sym = unsafe { libc::dlsym(handle, cname.as_ptr()) };
    if sym.is_null() {
        return Err(format!("symbol `{name}` not found: {}", dlerror_text()));
    }
    Ok(sym)
}

impl Lib {
    fn open(label: &'static str, path: &str) -> Result<Lib, String> {
        let cpath = CString::new(path).map_err(|e| e.to_string())?;
        // SAFETY: standard dlopen. RTLD_LOCAL keeps the two kernels' symbols in
        // separate namespaces so `Initialize` from the go dylib can't collide
        // with the rust one's.
        let handle = unsafe { libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(format!("dlopen({path}) failed: {}", dlerror_text()));
        }
        // SAFETY: each symbol is transmuted from its dlsym'd address to the
        // matching export ABI. Sizes are checked at compile time
        // (pointer == fn-pointer on the supported targets).
        unsafe {
            Ok(Lib {
                label,
                handle,
                initialize: std::mem::transmute::<*mut c_void, FnCb>(load_symbol(handle, "Initialize")?),
                fetch_device_info: std::mem::transmute::<*mut c_void, FnCb>(load_symbol(handle, "FetchDeviceInfo")?),
                fetch_storages: std::mem::transmute::<*mut c_void, FnCb>(load_symbol(handle, "FetchStorages")?),
                dispose: std::mem::transmute::<*mut c_void, FnCb>(load_symbol(handle, "Dispose")?),
                make_directory: std::mem::transmute::<*mut c_void, FnJson1>(load_symbol(handle, "MakeDirectory")?),
                file_exists: std::mem::transmute::<*mut c_void, FnJson1>(load_symbol(handle, "FileExists")?),
                delete_file: std::mem::transmute::<*mut c_void, FnJson1>(load_symbol(handle, "DeleteFile")?),
                rename_file: std::mem::transmute::<*mut c_void, FnJson1>(load_symbol(handle, "RenameFile")?),
                walk: std::mem::transmute::<*mut c_void, FnJson1>(load_symbol(handle, "Walk")?),
                upload_files: std::mem::transmute::<*mut c_void, FnJson3>(load_symbol(handle, "UploadFiles")?),
                download_files: std::mem::transmute::<*mut c_void, FnJson3>(load_symbol(handle, "DownloadFiles")?),
            })
        }
    }
}

impl Drop for Lib {
    fn drop(&mut self) {
        // Best-effort: the loaded runtime may not truly support unload, but the
        // device was already released by Dispose, so this is only tidiness.
        if !self.handle.is_null() {
            unsafe { libc::dlclose(self.handle) };
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The golden session script
// ─────────────────────────────────────────────────────────────────────────────
//
// The scripted session that both dylibs run. Fixtures under
// keel/fixtures/golden/*.json are numbered per *payload*; this script is per
// *step* — the UploadFiles step alone emits payloads 0009-0014. Placeholders
// {SID} / {SRC} / {DL} are filled at run time: {SID} from the live FetchStorages
// result (so the oracle works on any device, not just Sid 65537), {SRC}/{DL}
// from the temp scratch tree this binary creates.

/// Which export a step drives, plus its (templated) JSON input.
enum Op {
    Initialize,
    FetchDeviceInfo,
    FetchStorages,
    Dispose,
    MakeDirectory(&'static str),
    FileExists(&'static str),
    DeleteFile(&'static str),
    RenameFile(&'static str),
    Walk(&'static str),
    UploadFiles(&'static str),
    DownloadFiles(&'static str),
}

struct Step {
    name: &'static str,
    /// Golden payload numbers this step reproduces (for cross-reference).
    fixtures: &'static str,
    op: Op,
    /// A step where the two sides are *expected* to differ because the rust side
    /// fixes a bug present in the go side. A divergence here is reported but does
    /// NOT fail the run; agreement is warned about (it would mean the fix
    /// regressed or the oracle over-normalized).
    expect_divergence: bool,
}

fn golden_script() -> Vec<Step> {
    vec![
        Step { name: "Initialize", fixtures: "0001", op: Op::Initialize, expect_divergence: false },
        Step { name: "FetchDeviceInfo", fixtures: "0002", op: Op::FetchDeviceInfo, expect_divergence: false },
        Step { name: "FetchStorages", fixtures: "0003", op: Op::FetchStorages, expect_divergence: false },
        Step {
            name: "Walk / (non-recursive)",
            fixtures: "0004",
            op: Op::Walk(r#"{"storageId":{SID},"fullPath":"/","recursive":false,"skipDisallowedFiles":true,"skipHiddenFiles":true}"#),
            expect_divergence: false,
        },
        Step {
            name: "Walk /Download (non-recursive)",
            fixtures: "0005",
            op: Op::Walk(r#"{"storageId":{SID},"fullPath":"/Download","recursive":false,"skipDisallowedFiles":true,"skipHiddenFiles":true}"#),
            expect_divergence: false,
        },
        Step {
            name: "MakeDirectory /Download/keel-golden-test",
            fixtures: "0006",
            op: Op::MakeDirectory(r#"{"storageId":{SID},"fullPath":"/Download/keel-golden-test"}"#),
            expect_divergence: false,
        },
        Step {
            name: "MakeDirectory /Download/keel-golden-test (idempotent)",
            fixtures: "0007",
            op: Op::MakeDirectory(r#"{"storageId":{SID},"fullPath":"/Download/keel-golden-test"}"#),
            expect_divergence: false,
        },
        Step {
            name: "FileExists (present + missing)",
            fixtures: "0008",
            op: Op::FileExists(r#"{"storageId":{SID},"Files":["/Download/keel-golden-test","/Download/keel-golden-test/definitely-missing.bin"]}"#),
            expect_divergence: false,
        },
        Step {
            name: "UploadFiles keel-golden-src -> /Download/keel-golden-test",
            fixtures: "0009-0014",
            op: Op::UploadFiles(r#"{"storageId":{SID},"sources":["{SRC}"],"destination":"/Download/keel-golden-test","preprocessFiles":true}"#),
            expect_divergence: false,
        },
        Step {
            // The 🛳️ (U+1F6F3 + U+FE0F) filename round-trips through the device.
            // The go side's UCS-2 codec drops the surrogate pair, walking it back
            // as "note-️.txt"; the rust side's real UTF-16 codec keeps it. So the
            // two sides WILL differ on this entry's name/path — expected.
            name: "Walk /Download/keel-golden-test (recursive, emoji)",
            fixtures: "0015",
            op: Op::Walk(r#"{"storageId":{SID},"fullPath":"/Download/keel-golden-test","recursive":true,"skipDisallowedFiles":true,"skipHiddenFiles":false}"#),
            expect_divergence: true,
        },
        Step {
            name: "RenameFile blob-1.5mb.bin -> blob-renamed.bin",
            fixtures: "0016",
            op: Op::RenameFile(r#"{"storageId":{SID},"fullPath":"/Download/keel-golden-test/keel-golden-src/blob-1.5mb.bin","newFileName":"blob-renamed.bin"}"#),
            expect_divergence: false,
        },
        Step {
            name: "RenameFile blob-renamed.bin -> blob-1.5mb.bin (restore)",
            fixtures: "0017",
            op: Op::RenameFile(r#"{"storageId":{SID},"fullPath":"/Download/keel-golden-test/keel-golden-src/blob-renamed.bin","newFileName":"blob-1.5mb.bin"}"#),
            expect_divergence: false,
        },
        Step {
            name: "DownloadFiles no-such-dir (ErrorInvalidPath)",
            fixtures: "0018",
            op: Op::DownloadFiles(r#"{"storageId":{SID},"sources":["/Download/keel-golden-test/no-such-dir"],"destination":"{DL}","preprocessFiles":true}"#),
            expect_divergence: false,
        },
        Step {
            name: "DownloadFiles no-such-file.bin (ErrorInvalidPath)",
            fixtures: "0019",
            op: Op::DownloadFiles(r#"{"storageId":{SID},"sources":["/Download/keel-golden-test/no-such-file.bin"],"destination":"{DL}","preprocessFiles":true}"#),
            expect_divergence: false,
        },
        Step {
            name: "DeleteFile keel-golden-src (cleanup)",
            fixtures: "0020",
            op: Op::DeleteFile(r#"{"storageId":{SID},"Files":["/Download/keel-golden-test/keel-golden-src"]}"#),
            expect_divergence: false,
        },
        Step {
            name: "DeleteFile keel-golden-test (cleanup)",
            fixtures: "0021",
            op: Op::DeleteFile(r#"{"storageId":{SID},"Files":["/Download/keel-golden-test"]}"#),
            expect_divergence: false,
        },
        Step { name: "Dispose", fixtures: "0022", op: Op::Dispose, expect_divergence: false },
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Session runner
// ─────────────────────────────────────────────────────────────────────────────

/// One captured, parsed-and-normalized callback event.
struct Event {
    kind: CbKind,
    /// serde_json parse of the payload, with volatile keys normalized. None if
    /// the payload was not valid JSON (then `raw` is diffed textually).
    value: Option<Value>,
    raw: String,
}

/// All events a single step produced on one side.
struct StepCapture {
    events: Vec<Event>,
}

/// Runtime substitutions for the templated inputs.
struct Env {
    src_dir: String,
    dl_dir: String,
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn fill(template: &str, sid: u32, env: &Env) -> String {
    template
        .replace("{SID}", &sid.to_string())
        .replace("{SRC}", &json_escape(&env.src_dir))
        .replace("{DL}", &json_escape(&env.dl_dir))
}

fn drain_events() -> Vec<Event> {
    let raw = std::mem::take(&mut *CAPTURE.lock().unwrap_or_else(|e| e.into_inner()));
    raw.into_iter()
        .map(|(kind, text)| {
            let value = serde_json::from_str::<Value>(&text).ok().map(|mut v| {
                normalize(&mut v);
                v
            });
            Event { kind, value, raw: text }
        })
        .collect()
}

/// Run the whole script against one dylib, returning per-step captures. Never
/// aborts on a device error: a failing export just emits an error envelope,
/// which is itself diffable.
fn run_session(lib: &Lib, script: &[Step], env: &Env) -> Vec<StepCapture> {
    // Default storage id (Nothing A059's Sid, 0x10001) as a fallback; overwritten
    // by the live FetchStorages result so we adapt to whatever device is present.
    let mut sid: u32 = 65537;
    let mut out = Vec::with_capacity(script.len());

    for step in script {
        // Fresh buffer for this step.
        CAPTURE.lock().unwrap_or_else(|e| e.into_inner()).clear();

        match &step.op {
            Op::Initialize => unsafe { (lib.initialize)(Some(cb_done)) },
            Op::FetchDeviceInfo => unsafe { (lib.fetch_device_info)(Some(cb_done)) },
            Op::FetchStorages => unsafe { (lib.fetch_storages)(Some(cb_done)) },
            Op::Dispose => unsafe { (lib.dispose)(Some(cb_done)) },
            Op::MakeDirectory(t) => call_json1(lib.make_directory, &fill(t, sid, env)),
            Op::FileExists(t) => call_json1(lib.file_exists, &fill(t, sid, env)),
            Op::DeleteFile(t) => call_json1(lib.delete_file, &fill(t, sid, env)),
            Op::RenameFile(t) => call_json1(lib.rename_file, &fill(t, sid, env)),
            Op::Walk(t) => call_json1(lib.walk, &fill(t, sid, env)),
            Op::UploadFiles(t) => call_json3(lib.upload_files, &fill(t, sid, env)),
            Op::DownloadFiles(t) => call_json3(lib.download_files, &fill(t, sid, env)),
        }

        let events = drain_events();

        // Learn the real storage id from the live result so later steps address
        // the right storage on whatever device is attached.
        if matches!(step.op, Op::FetchStorages) {
            if let Some(found) = extract_sid(&events) {
                sid = found;
            }
        }

        out.push(StepCapture { events });
    }
    out
}

fn call_json1(f: FnJson1, input: &str) {
    // NUL-terminated; the export copies it, so it only needs to outlive the call.
    let Ok(c) = CString::new(input) else { return };
    unsafe { f(c.as_ptr(), Some(cb_done)) }
}

fn call_json3(f: FnJson3, input: &str) {
    let Ok(c) = CString::new(input) else { return };
    unsafe { f(c.as_ptr(), Some(cb_preprocess), Some(cb_progress), Some(cb_done)) }
}

/// Pull `data[0].Sid` out of a FetchStorages capture (raw PascalCase key).
fn extract_sid(events: &[Event]) -> Option<u32> {
    let v = events.first()?.value.as_ref()?;
    let arr = v.get("data")?.as_array()?;
    arr.first()?.get("Sid")?.as_u64().map(|n| n as u32)
}

// ─────────────────────────────────────────────────────────────────────────────
// Normalization
// ─────────────────────────────────────────────────────────────────────────────

/// Replace every volatile-keyed value (anywhere in the tree) with null so the
/// diff ignores it. Non-volatile scalars and structure are preserved.
fn normalize(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if is_volatile_key(k) {
                    *val = Value::Null;
                } else {
                    normalize(val);
                }
            }
        }
        Value::Array(arr) => {
            for val in arr.iter_mut() {
                normalize(val);
            }
        }
        _ => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Diff engine
// ─────────────────────────────────────────────────────────────────────────────

struct FieldDiff {
    path: String,
    go: String,
    rust: String,
    note: &'static str,
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 200 {
        format!("{}…", &s[..200])
    } else {
        s
    }
}

fn numbers_equal(a: &serde_json::Number, b: &serde_json::Number) -> bool {
    if let (Some(x), Some(y)) = (a.as_i64(), b.as_i64()) {
        return x == y;
    }
    if let (Some(x), Some(y)) = (a.as_u64(), b.as_u64()) {
        return x == y;
    }
    // At least one is a float: compare with tolerance.
    let (x, y) = match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };
    (x - y).abs() <= FLOAT_ABS_EPS + FLOAT_REL_EPS * x.abs().max(y.abs())
}

/// Structural, exact-key-casing diff. Emits JSON-pointer-style paths.
fn diff_value(path: &str, go: &Value, rust: &Value, out: &mut Vec<FieldDiff>) {
    match (go, rust) {
        (Value::Object(a), Value::Object(b)) => {
            // Union of keys, exact casing, sorted for a stable report.
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let p = format!("{path}/{k}");
                match (a.get(k), b.get(k)) {
                    (Some(av), Some(bv)) => diff_value(&p, av, bv, out),
                    (Some(av), None) => out.push(FieldDiff {
                        path: p,
                        go: short(av),
                        rust: "<absent>".into(),
                        note: "key present in go, absent in rust",
                    }),
                    (None, Some(bv)) => out.push(FieldDiff {
                        path: p,
                        go: "<absent>".into(),
                        rust: short(bv),
                        note: "key present in rust, absent in go",
                    }),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                out.push(FieldDiff {
                    path: path.to_string(),
                    go: format!("<array len {}>", a.len()),
                    rust: format!("<array len {}>", b.len()),
                    note: "array length mismatch",
                });
            }
            for i in 0..a.len().min(b.len()) {
                diff_value(&format!("{path}/{i}"), &a[i], &b[i], out);
            }
        }
        (Value::Number(a), Value::Number(b)) => {
            if !numbers_equal(a, b) {
                out.push(FieldDiff { path: path.to_string(), go: short(go), rust: short(rust), note: "value mismatch" });
            }
        }
        _ => {
            if go != rust {
                out.push(FieldDiff {
                    path: path.to_string(),
                    go: short(go),
                    rust: short(rust),
                    note: if std::mem::discriminant(go) != std::mem::discriminant(rust) {
                        "type mismatch"
                    } else {
                        "value mismatch"
                    },
                });
            }
        }
    }
}

/// Collect the *set* of key-paths (structure, ignoring array length/order and
/// scalar values) — used for schema comparison of sampler streams whose exact
/// values and counts are timing-dependent.
fn key_skeleton(v: &Value, prefix: &str, set: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, val) in m {
                let p = format!("{prefix}/{k}");
                set.push(p.clone());
                key_skeleton(val, &p, set);
            }
        }
        Value::Array(a) => {
            if let Some(first) = a.first() {
                key_skeleton(first, &format!("{prefix}/[]"), set);
            }
        }
        _ => {}
    }
}

fn pointer<'a>(v: &'a Value, ptr: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in ptr.split('/') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// A callback stream whose payloads are single and deterministic (the `done`
/// slot, and every non-transfer step): require equal count and diff every
/// element exactly.
fn diff_exact_stream(label: &str, go: &[&Value], rust: &[&Value], out: &mut Vec<FieldDiff>) {
    if go.len() != rust.len() {
        out.push(FieldDiff {
            path: format!("[{label}]"),
            go: format!("{} event(s)", go.len()),
            rust: format!("{} event(s)", rust.len()),
            note: "callback count mismatch",
        });
    }
    for i in 0..go.len().min(rust.len()) {
        diff_value(&format!("[{label}#{i}]"), go[i], rust[i], out);
    }
}

/// A sampler-driven stream (Upload/Download preprocess & progress). The 500 ms
/// latest-value sampler makes both the *count* and *which* snapshots land
/// timing-dependent — even the terminal one — so we do NOT diff values. We
/// assert: presence parity (both emitted, or neither), the
/// terminal payload's key *schema* matches, and a handful of run-invariant
/// `stable_fields` (e.g. totalFiles) agree. Everything else is reported as an
/// informational note. Content correctness of a transfer is verified separately
/// by the walk-after-upload step.
fn diff_sampled_stream(
    label: &str,
    go: &[&Value],
    rust: &[&Value],
    stable_fields: &[&str],
    out: &mut Vec<FieldDiff>,
    infos: &mut Vec<String>,
) {
    if go.is_empty() != rust.is_empty() {
        out.push(FieldDiff {
            path: format!("[{label}]"),
            go: format!("{} event(s)", go.len()),
            rust: format!("{} event(s)", rust.len()),
            note: "one side emitted this callback stream, the other did not",
        });
        return;
    }
    if go.is_empty() {
        return;
    }
    infos.push(format!(
        "{label}: go emitted {}, rust emitted {} (sampled — count & intermediate values not compared)",
        go.len(),
        rust.len()
    ));

    let gt = go.last().unwrap();
    let rt = rust.last().unwrap();

    // Schema (key-path set) of the terminal payload.
    let (mut gs, mut rs) = (Vec::new(), Vec::new());
    key_skeleton(gt, &format!("[{label}]"), &mut gs);
    key_skeleton(rt, &format!("[{label}]"), &mut rs);
    gs.sort();
    gs.dedup();
    rs.sort();
    rs.dedup();
    for p in gs.iter().filter(|p| !rs.contains(p)) {
        out.push(FieldDiff { path: p.clone(), go: "<present>".into(), rust: "<absent>".into(), note: "schema: key only in go" });
    }
    for p in rs.iter().filter(|p| !gs.contains(p)) {
        out.push(FieldDiff { path: p.clone(), go: "<absent>".into(), rust: "<present>".into(), note: "schema: key only in rust" });
    }

    // Run-invariant fields on the terminal payload.
    for f in stable_fields {
        match (pointer(gt, f), pointer(rt, f)) {
            (Some(a), Some(b)) => {
                let mut sub = Vec::new();
                diff_value(&format!("[{label}]/{f}"), a, b, &mut sub);
                out.extend(sub);
            }
            (ga, rb) => {
                if ga.is_some() != rb.is_some() {
                    out.push(FieldDiff {
                        path: format!("[{label}]/{f}"),
                        go: if ga.is_some() { "<present>".into() } else { "<absent>".into() },
                        rust: if rb.is_some() { "<present>".into() } else { "<absent>".into() },
                        note: "stable field present on only one side",
                    });
                }
            }
        }
    }
}

fn partition(events: &[Event], kind: CbKind) -> Vec<&Value> {
    events
        .iter()
        .filter(|e| e.kind == kind)
        .filter_map(|e| e.value.as_ref())
        .collect()
}

/// Compare one step's go capture to its rust capture.
fn diff_step(go: &StepCapture, rust: &StepCapture) -> (Vec<FieldDiff>, Vec<String>) {
    let mut diffs = Vec::new();
    let mut infos = Vec::new();

    // Any payload that failed to parse is diffed textually (rare — malformed
    // JSON from either kernel is itself a finding).
    for (i, (g, r)) in go.events.iter().zip(rust.events.iter()).enumerate() {
        if (g.value.is_none() || r.value.is_none()) && g.raw != r.raw {
            diffs.push(FieldDiff {
                path: format!("[raw#{i}]"),
                go: g.raw.chars().take(200).collect(),
                rust: r.raw.chars().take(200).collect(),
                note: "unparseable payload differs textually",
            });
        }
    }

    // `done` and all single-payload steps: exact.
    diff_exact_stream("done", &partition(&go.events, CbKind::Done), &partition(&rust.events, CbKind::Done), &mut diffs);

    // Sampler streams: presence + terminal schema + invariants only.
    diff_sampled_stream(
        "preprocess",
        &partition(&go.events, CbKind::Preprocess),
        &partition(&rust.events, CbKind::Preprocess),
        &[],
        &mut diffs,
        &mut infos,
    );
    diff_sampled_stream(
        "progress",
        &partition(&go.events, CbKind::Progress),
        &partition(&rust.events, CbKind::Progress),
        // Deterministic regardless of which snapshot the sampler caught.
        &["totalFiles", "totalDirectories", "bulkFileSize/total"],
        &mut diffs,
        &mut infos,
    );

    (diffs, infos)
}

// ─────────────────────────────────────────────────────────────────────────────
// Scratch tree (upload source + download dest)
// ─────────────────────────────────────────────────────────────────────────────

/// The emoji filename the golden session uploads: "note-" + U+1F6F3 (PASSENGER
/// SHIP, an astral-plane char → UTF-16 surrogate pair) + U+FE0F (variation
/// selector) + ".txt". This is the input that exposes the go side's UCS-2 codec
/// bug on the round-trip walk.
const EMOJI_NOTE_NAME: &str = "note-\u{1F6F3}\u{FE0F}.txt";

/// Build `<root>/keel-golden-src/{blob-1.5mb.bin, note-🛳️.txt, sub/nested.bin}`
/// with the fixed byte sizes the golden fixtures expect (1_500_000 / 51 /
/// 300_000), plus an empty `<root>/keel-golden-dl` download target. Returns
/// (src, dl).
fn setup_scratch(root: &Path) -> std::io::Result<(PathBuf, PathBuf)> {
    let src = root.join("keel-golden-src");
    let sub = src.join("sub");
    fs::create_dir_all(&sub)?;
    let dl = root.join("keel-golden-dl");
    fs::create_dir_all(&dl)?;

    write_sized(&src.join("blob-1.5mb.bin"), 1_500_000)?;
    write_sized(&src.join(EMOJI_NOTE_NAME), 51)?;
    write_sized(&sub.join("nested.bin"), 300_000)?;
    Ok((src, dl))
}

fn write_sized(path: &Path, len: usize) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(fs::File::create(path)?);
    let chunk = [b'k'; 8192];
    let mut remaining = len;
    while remaining > 0 {
        let n = remaining.min(chunk.len());
        f.write_all(&chunk[..n])?;
        remaining -= n;
    }
    f.flush()
}

// ─────────────────────────────────────────────────────────────────────────────
// CLI + main
// ─────────────────────────────────────────────────────────────────────────────

struct Args {
    go: String,
    rust: String,
    dump: Option<String>,
    keep: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut go = None;
    let mut rust = None;
    let mut dump = None;
    let mut keep = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--go" => go = Some(it.next().ok_or("--go needs a path")?),
            "--rust" => rust = Some(it.next().ok_or("--rust needs a path")?),
            "--dump" => dump = Some(it.next().ok_or("--dump needs a dir")?),
            "--keep" => keep = true,
            "-h" | "--help" => return Err("help".into()),
            other => return Err(format!("unknown arg: {other}")),
        }
    }
    Ok(Args {
        go: go.ok_or("missing --go <path to the legacy kernel dylib>")?,
        rust: rust.ok_or("missing --rust <path to libkeel.dylib>")?,
        dump,
        keep,
    })
}

const USAGE: &str = "keel-conformance — differential oracle (legacy vs keel)\n\
\n\
USAGE:\n\
    keel-conformance --go <legacy-dylib> --rust <libkeel.dylib> [--dump <dir>] [--keep]\n\
\n\
Runs the golden session against the attached phone through each dylib in turn\n\
(Go first, then Rust), diffs every callback payload, and exits 0 only on full\n\
parity. Documented intentional fixes (plan §3.5, e.g. the emoji filename) are\n\
reported but do not fail the run.\n\
\n\
    --dump <dir>   also write every captured payload to <dir>/{go,rust}-NN-*.json\n\
    --keep         do not delete the temp scratch tree on exit\n";

/// Optionally persist raw payloads for offline inspection.
fn dump_side(dir: &Path, side: &str, captures: &[StepCapture]) {
    let _ = fs::create_dir_all(dir);
    let mut n = 0u32;
    for cap in captures {
        for ev in &cap.events {
            n += 1;
            let kind = match ev.kind {
                CbKind::Done => "done",
                CbKind::Preprocess => "preprocess",
                CbKind::Progress => "progress",
            };
            let _ = fs::write(dir.join(format!("{side}-{n:04}-{kind}.json")), &ev.raw);
        }
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if e == "help" {
                print!("{USAGE}");
                std::process::exit(0);
            }
            eprintln!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    // Scratch tree for the upload source / download dest.
    let root = std::env::temp_dir().join(format!("keel-conformance-{}", std::process::id()));
    let (src, dl) = match setup_scratch(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not create scratch tree at {}: {e}", root.display());
            std::process::exit(2);
        }
    };
    let env = Env {
        src_dir: src.to_string_lossy().into_owned(),
        dl_dir: dl.to_string_lossy().into_owned(),
    };

    let script = golden_script();

    // Load both kernels. RTLD_LOCAL keeps their identical symbol names apart.
    let go_lib = match Lib::open("go", &args.go) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            cleanup(&root, args.keep);
            std::process::exit(2);
        }
    };
    let rust_lib = match Lib::open("rust", &args.rust) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            cleanup(&root, args.keep);
            std::process::exit(2);
        }
    };

    println!("keel-conformance");
    println!("  go   : {} ({})", args.go, go_lib.label);
    println!("  rust : {} ({})", args.rust, rust_lib.label);
    println!("  src  : {}", env.src_dir);
    println!("  steps: {}\n", script.len());

    // The go side first (fully, ending in Dispose which releases the device),
    // then rust.
    println!("running Go session…");
    let go_caps = run_session(&go_lib, &script, &env);
    println!("running Rust session…\n");
    let rust_caps = run_session(&rust_lib, &script, &env);

    if let Some(d) = &args.dump {
        let d = Path::new(d);
        dump_side(d, "go", &go_caps);
        dump_side(d, "rust", &rust_caps);
        println!("payloads dumped to {}\n", d.display());
    }

    // Diff step by step.
    let total = script.len();
    let mut hard_failures = 0usize;

    for (i, step) in script.iter().enumerate() {
        let go = &go_caps[i];
        let rust = &rust_caps[i];
        let (diffs, infos) = diff_step(go, rust);

        let (verdict, is_failure) = if diffs.is_empty() {
            if step.expect_divergence {
                ("PASS (expected divergence NOT observed — verify fix)", false)
            } else {
                ("PASS", false)
            }
        } else if step.expect_divergence {
            ("EXPECTED DIVERGENCE (keel correct, plan §3.5)", false)
        } else {
            ("DIVERGENCE", true)
        };
        if is_failure {
            hard_failures += 1;
        }

        println!("[{:>2}/{total}] {:<52} {verdict}", i + 1, step.name);
        println!("          fixtures {}", step.fixtures);
        for info in &infos {
            println!("          · {info}");
        }
        for d in &diffs {
            println!("          {}", d.path);
            println!("              go   = {}", d.go);
            println!("              rust = {}", d.rust);
            println!("              ({})", d.note);
        }
    }

    println!();
    if hard_failures == 0 {
        println!("RESULT: PARITY ({total} steps; intentional divergences excluded)");
    } else {
        println!("RESULT: {hard_failures} DIVERGENCE(S) of {total} steps");
    }

    cleanup(&root, args.keep);
    std::process::exit(if hard_failures == 0 { 0 } else { 1 });
}

fn cleanup(root: &Path, keep: bool) {
    if keep {
        eprintln!("(kept scratch tree at {})", root.display());
    } else {
        let _ = fs::remove_dir_all(root);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests — pure logic (no device, no dylib). Prove normalize/diff behavior.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn diff(a: Value, b: Value) -> Vec<FieldDiff> {
        let mut out = Vec::new();
        diff_value("", &a, &b, &mut out);
        out
    }

    #[test]
    fn identical_objects_no_diff() {
        assert!(diff(json!({"a":1,"b":"x"}), json!({"a":1,"b":"x"})).is_empty());
    }

    #[test]
    fn volatile_keys_normalized_away() {
        let mut a = json!({"speed": 9.63, "elapsedTime": 1502, "dateAdded": "2026-07-12T23:26:21.000Z", "name": "x"});
        let mut b = json!({"speed": 0.0, "elapsedTime": 9999, "dateAdded": "2001-01-01T00:00:00.000Z", "name": "x"});
        normalize(&mut a);
        normalize(&mut b);
        assert!(diff(a, b).is_empty(), "volatile fields must not diverge");
    }

    #[test]
    fn object_id_normalized_but_name_is_not() {
        // Device-assigned handles differ per session → normalized. Name is signal.
        let mut go = json!({"objectId": 474, "parentId": 0, "name": "note-️.txt"});
        let mut rs = json!({"objectId": 991, "parentId": 7, "name": "note-🛳️.txt"});
        normalize(&mut go);
        normalize(&mut rs);
        let d = diff(go, rs);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path, "/name");
    }

    #[test]
    fn float_formatting_tolerated_but_real_gap_caught() {
        // The two kernels' encoders render the same float32 differently → tolerated.
        assert!(diff(json!({"p": 83.330971}), json!({"p": 83.33097})).is_empty());
        // A ~1% real difference is still caught.
        assert_eq!(diff(json!({"p": 83.3}), json!({"p": 84.5})).len(), 1);
    }

    #[test]
    fn integers_compared_exactly() {
        assert_eq!(diff(json!({"size": 300000}), json!({"size": 300001})).len(), 1);
    }

    #[test]
    fn exact_key_casing() {
        // PascalCase Sid vs camelCase sid must be flagged (missing on each side).
        let d = diff(json!({"Sid": 65537}), json!({"sid": 65537}));
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn array_length_and_element_diff() {
        let d = diff(json!([1, 2, 3]), json!([1, 9]));
        // one length-mismatch + one element (index 1) mismatch
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn missing_key_reported() {
        let d = diff(json!({"a": 1, "b": 2}), json!({"a": 1}));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path, "/b");
    }

    #[test]
    fn type_mismatch_reported() {
        let d = diff(json!({"data": null}), json!({"data": []}));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].note, "type mismatch");
    }

    #[test]
    fn key_skeleton_ignores_values_and_counts() {
        let a = json!({"activeFileSize": {"total": 51, "sent": 51}, "status": "InProgress"});
        let b = json!({"activeFileSize": {"total": 1500000, "sent": 700000}, "status": "Completed"});
        let (mut sa, mut sb) = (Vec::new(), Vec::new());
        key_skeleton(&a, "", &mut sa);
        key_skeleton(&b, "", &mut sb);
        sa.sort();
        sb.sort();
        assert_eq!(sa, sb, "same schema regardless of values");
    }

    #[test]
    fn pointer_navigates_nested() {
        let v = json!({"bulkFileSize": {"total": 1800051}});
        assert_eq!(pointer(&v, "bulkFileSize/total").and_then(|x| x.as_u64()), Some(1800051));
        assert!(pointer(&v, "bulkFileSize/missing").is_none());
    }

    #[test]
    fn sampled_stream_ignores_count_and_volatile_but_checks_totals() {
        // Two progress snapshots on the go side, one on rust; totals agree → no divergence.
        let g1 = json!({"totalFiles": 3, "totalDirectories": 2, "bulkFileSize": {"total": 1800051}, "filesSent": 1});
        let g2 = json!({"totalFiles": 3, "totalDirectories": 2, "bulkFileSize": {"total": 1800051}, "filesSent": 2});
        let r1 = json!({"totalFiles": 3, "totalDirectories": 2, "bulkFileSize": {"total": 1800051}, "filesSent": 3});
        let (go, rust): (Vec<&Value>, Vec<&Value>) = (vec![&g1, &g2], vec![&r1]);
        let (mut out, mut infos) = (Vec::new(), Vec::new());
        diff_sampled_stream("progress", &go, &rust, &["totalFiles", "bulkFileSize/total"], &mut out, &mut infos);
        assert!(out.is_empty(), "sampled stream should not diverge on count/volatile");
        assert_eq!(infos.len(), 1);
    }

    #[test]
    fn sampled_stream_flags_wrong_total() {
        let g = json!({"totalFiles": 3});
        let r = json!({"totalFiles": 2});
        let (mut out, mut infos) = (Vec::new(), Vec::new());
        diff_sampled_stream("progress", &[&g], &[&r], &["totalFiles"], &mut out, &mut infos);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn sampled_stream_presence_parity() {
        let g = json!({"x": 1});
        let (mut out, mut infos) = (Vec::new(), Vec::new());
        // The go side emitted a preprocess event, rust emitted none → divergence.
        diff_sampled_stream("preprocess", &[&g], &[], &[], &mut out, &mut infos);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn extract_sid_reads_pascalcase() {
        let ev = Event {
            kind: CbKind::Done,
            raw: String::new(),
            value: Some(json!({"data": [{"Sid": 65537, "Info": {}}]})),
        };
        assert_eq!(extract_sid(&[ev]), Some(65537));
    }

    #[test]
    fn fill_substitutes_placeholders() {
        let env = Env { src_dir: "/tmp/a b".into(), dl_dir: "/tmp/dl".into() };
        let s = fill(r#"{"storageId":{SID},"sources":["{SRC}"]}"#, 65537, &env);
        assert_eq!(s, r#"{"storageId":65537,"sources":["/tmp/a b"]}"#);
    }
}
