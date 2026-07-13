//! Directory / file metadata operations, go-mtpx parity: `make_directory`,
//! `file_exists`, `delete_file`, `rename_file`.
//!
//! Faithful port of go-mtpx `main.go` (`MakeDirectory` 85-126, `FileExists`
//! 176-207, `DeleteFile` 214-231, `RenameFile` 240-264) and the `handleMakeDirectory`
//! helper (`helpers.go` 209-226). Generic over [`Transport`], matching the rest of
//! keel-vfs (path.rs/object.rs). Load-bearing quirks preserved verbatim, each cited
//! to its Go origin:
//!   * MakeDirectory is `mkdir -p`: per segment, resolve-or-create; a segment that
//!     exists but is a *file* is `InvalidPath`; idempotent on re-run (main.go:96-123).
//!     Directory creation is a `SendObjectInfo` (format 0x3001) with NO data phase.
//!   * FileExists is case-insensitive (the fold lives in path resolution) and never
//!     returns an error — every branch yields `Ok` (main.go:195 returns `[], nil`
//!     even on the "impossible" default). RC 0x2009 InvalidObjectHandle ⇒ not-exists
//!     (main.go:186-192).
//!   * DeleteFile batch quirk (PRESERVED per plan §3.5, fixed only in the v2 API):
//!     a missing file mid-batch `return`s from the whole loop, silently skipping
//!     every remaining entry (main.go:221-223 uses `return nil`, not `continue`).
//!     Delete itself is device-side recursive — one `DeleteObject` on a directory
//!     removes its whole subtree (the device does the recursion).
//!   * RenameFile tolerates RC 0x2002 GeneralError as success — Android devices
//!     return it on a same-name rename (main.go:253-257).

use std::time::SystemTime;

use keel_mtp::{MtpError, MtpSession, Transport};
use keel_proto::{ObjectFormat, ObjectInfo, ObjectPropCode, PropValue, RespCode};

use crate::error::VfsError;
use crate::object::FileInfo;
use crate::path::{
    FileProp, PARENT_OBJECT_ID, PATH_SEP, fix_slash, get_object_from_object_id_or_path,
    get_object_from_parent_id_and_filename,
};

/// go-mtpx `structs.go::FileExistsContainer` (110-113). `file_info` is `Some`
/// exactly when `exists` is `true` (main.go:198-201).
#[derive(Clone, Debug, Default)]
pub struct FileExistsContainer {
    pub exists: bool,
    pub file_info: Option<FileInfo>,
}

/// Create `full_path` and any missing ancestors (`mkdir -p`). Returns the object
/// id of the leaf directory. Port of `main.go::MakeDirectory` (85-126).
pub fn make_directory<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    full_path: &str,
) -> Result<u32, VfsError> {
    // main.go:86-90 — normalize; the storage root needs no creation.
    let full = fix_slash(full_path);
    if full == PATH_SEP {
        return Ok(PARENT_OBJECT_ID);
    }

    // main.go:91-96 — split and skip the empty leading segment (Go's `[1:]`).
    // `fix_slash` guarantees a leading '/', so the first split piece is always "".
    let mut object_id = PARENT_OBJECT_ID;

    for f_name in full.split(PATH_SEP).skip(1) {
        match get_object_from_parent_id_and_filename(session, storage_id, object_id, f_name) {
            // main.go:117-122 — segment exists: it MUST be a directory to walk on.
            Ok(fi) => {
                if !fi.is_dir {
                    return Err(VfsError::InvalidPath(format!(
                        "invalid path: {}. The object is not a directory",
                        f_name
                    )));
                }
                object_id = fi.object_id;
            }
            // main.go:102-111 — segment missing: create it and continue downward.
            Err(VfsError::FileNotFound(_)) => {
                object_id = handle_make_directory(session, storage_id, object_id, f_name)?;
            }
            // main.go:112-113 — any other error is fatal.
            Err(e) => return Err(e),
        }
    }

    Ok(object_id)
}

/// Create a single directory object under `parent_id`. Port of
/// `helpers.go::handleMakeDirectory` (209-226): a `SendObjectInfo` for an
/// association with NO following `SendObject` data phase (an association carries
/// no bytes). go-mtpx discards the returned storage/parent ids and keeps only the
/// new handle (helpers.go:220).
fn handle_make_directory<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    parent_id: u32,
    filename: &str,
) -> Result<u32, VfsError> {
    // helpers.go:210-217. All unset fields are Go zero-values (Default), incl.
    // capture_date == None; modification_date is `time.Now()`.
    let info = ObjectInfo {
        storage_id,
        object_format: ObjectFormat::ASSOCIATION.0, // OFC_Association = 0x3001
        parent_object: parent_id,
        filename: filename.to_string(),
        compressed_size: 0,
        modification_date: Some(SystemTime::now()),
        ..Default::default()
    };

    session
        .send_object_info(storage_id, parent_id, &info)
        .map_err(VfsError::SendObject)
}

/// Batch existence check. Port of `main.go::FileExists` (176-207).
///
/// Never returns `Err` (the signature keeps a `Result` to mirror Go's
/// `(fc, err)` and the public API, but every path yields `Ok`). Case-insensitive
/// matching is done inside path resolution; the caller's casing is not echoed
/// back here (main.go:175).
pub fn file_exists<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    file_props: &[FileProp],
) -> Result<Vec<FileExistsContainer>, VfsError> {
    let mut fc: Vec<FileExistsContainer> = Vec::new();

    for fp in file_props {
        match get_object_from_object_id_or_path(session, storage_id, fp) {
            // main.go:198-201 — found.
            Ok(fi) => fc.push(FileExistsContainer {
                exists: true,
                file_info: Some(fi),
            }),
            Err(e) => match &e {
                // main.go:183-184 — bad/missing path ⇒ not-exists.
                VfsError::InvalidPath(_) => fc.push(FileExistsContainer {
                    exists: false,
                    file_info: None,
                }),
                // main.go:186-192 — a FileObject error yields not-exists. Go's
                // inner `switch` only names RC 0x2009 (InvalidObjectHandle), but it
                // merely re-sets the already-false zero value, so ANY FileObject
                // error appends `{exists:false}` — the 0x2009 branch is vestigial.
                // (0x2009 is what a device returns for GetObjectInfo on a
                // stale/bogus handle, e.g. DeleteFile of a non-existent object id.)
                VfsError::FileObject(_) => fc.push(FileExistsContainer {
                    exists: false,
                    file_info: None,
                }),
                // main.go:194-195 — any OTHER error type aborts the batch,
                // discarding whatever was collected and returning an EMPTY vec with
                // no error. (Unreachable in practice: path resolution only yields
                // InvalidPath / FileObject — but preserved faithfully.)
                _ => return Ok(Vec::new()),
            },
        }
    }

    Ok(fc)
}

/// Delete each file/directory in the batch (device-side recursive). Port of
/// `main.go::DeleteFile` (214-231).
///
/// PRESERVED QUIRK (plan §3.5): the first non-existent entry `return`s from the
/// whole function, so every later entry in `file_props` is silently skipped rather
/// than attempted (main.go:221-223).
pub fn delete_file<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    file_props: &[FileProp],
) -> Result<(), VfsError> {
    for fp in file_props {
        // main.go:216-219 — FileExists never errors; the guard mirrors Go's
        // `if err != nil { return nil }` (dead in practice, kept for fidelity).
        let fc = match file_exists(session, storage_id, std::slice::from_ref(fp)) {
            Ok(fc) => fc,
            Err(_) => return Ok(()),
        };

        // main.go:221-227. `fc[0]` in Go; we treat an (impossible) empty batch
        // result as not-exists rather than indexing out of bounds.
        match fc.first() {
            Some(c) if c.exists => {
                // exists ⇒ file_info is Some (FileExists invariant).
                let handle = c.file_info.as_ref().map(|f| f.object_id).unwrap_or(0);
                session
                    .delete_object(handle)
                    .map_err(VfsError::FileObject)?;
            }
            // Missing ⇒ abort the whole batch (the preserved quirk).
            _ => return Ok(()),
        }
    }

    Ok(())
}

/// Rename a single object to `new_file_name`, returning its (unchanged) object id.
/// Port of `main.go::RenameFile` (240-264).
pub fn rename_file<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    file_prop: &FileProp,
    new_file_name: &str,
) -> Result<u32, VfsError> {
    // main.go:241-244 — FileExists never errors, but keep the `?` for fidelity.
    let fc = file_exists(session, storage_id, std::slice::from_ref(file_prop))?;

    // main.go:246-248 — not found ⇒ InvalidPath (fc[0].Exists in Go; an empty
    // result is treated as not-found here rather than panicking).
    let object_id = match fc.first().filter(|c| c.exists) {
        Some(c) => c.file_info.as_ref().map(|f| f.object_id).unwrap_or(0),
        None => {
            return Err(VfsError::InvalidPath(format!(
                "file not found: {}",
                file_prop.full_path
            )));
        }
    };

    // main.go:252 — SetObjectPropValue(OPC_ObjectFileName, StringValue{new name}).
    let value = PropValue::Str(new_file_name.to_string());
    match session.set_object_prop_value(object_id, ObjectPropCode::OBJECT_FILE_NAME, &value) {
        Ok(()) => Ok(object_id),
        Err(e) => {
            // main.go:253-258 — RC 0x2002 GeneralError is tolerated as success;
            // Android returns it when the new name equals the current one.
            if let MtpError::Rc(rc) = &e {
                if rc.code() == RespCode::GENERAL_ERROR.0 {
                    return Ok(object_id);
                }
            }
            Err(VfsError::FileObject(e))
        }
    }
}
