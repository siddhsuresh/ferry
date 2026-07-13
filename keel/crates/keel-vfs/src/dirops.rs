//! Directory / file metadata operations: `make_directory`, `file_exists`,
//! `delete_file`, `rename_file`.
//!
//! Generic over [`Transport`], matching the rest of keel-vfs (path.rs/object.rs).
//! Load-bearing quirks:
//!   * MakeDirectory is `mkdir -p`: per segment, resolve-or-create; a segment that
//!     exists but is a *file* is `InvalidPath`; idempotent on re-run. Directory
//!     creation is a `SendObjectInfo` (format 0x3001) with NO data phase.
//!   * FileExists is case-insensitive (the fold lives in path resolution) and never
//!     returns an error — every branch yields `Ok`, returning `[], nil` even on the
//!     "impossible" default. RC 0x2009 InvalidObjectHandle ⇒ not-exists.
//!   * DeleteFile batch quirk (preserved for the wire contract, fixed only in the
//!     v2 API): a missing file mid-batch `return`s from the whole loop, silently
//!     skipping every remaining entry instead of continuing. Delete itself is
//!     device-side recursive — one `DeleteObject` on a directory removes its whole
//!     subtree (the device does the recursion).
//!   * RenameFile tolerates RC 0x2002 GeneralError as success — some devices
//!     return it on a same-name rename.

use std::time::SystemTime;

use keel_mtp::{MtpError, MtpSession, Transport};
use keel_proto::{ObjectFormat, ObjectInfo, ObjectPropCode, PropValue, RespCode};

use crate::error::VfsError;
use crate::object::FileInfo;
use crate::path::{
    FileProp, PARENT_OBJECT_ID, PATH_SEP, fix_slash, get_object_from_object_id_or_path,
    get_object_from_parent_id_and_filename,
};

/// Result of an existence check. `file_info` is `Some` exactly when `exists` is
/// `true`.
#[derive(Clone, Debug, Default)]
pub struct FileExistsContainer {
    pub exists: bool,
    pub file_info: Option<FileInfo>,
}

/// Create `full_path` and any missing ancestors (`mkdir -p`). Returns the object
/// id of the leaf directory.
pub fn make_directory<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    full_path: &str,
) -> Result<u32, VfsError> {
    // Normalize; the storage root needs no creation.
    let full = fix_slash(full_path);
    if full == PATH_SEP {
        return Ok(PARENT_OBJECT_ID);
    }

    // Split and skip the empty leading segment. `fix_slash` guarantees a leading
    // '/', so the first split piece is always "".
    let mut object_id = PARENT_OBJECT_ID;

    for f_name in full.split(PATH_SEP).skip(1) {
        match get_object_from_parent_id_and_filename(session, storage_id, object_id, f_name) {
            // Segment exists: it MUST be a directory to walk on.
            Ok(fi) => {
                if !fi.is_dir {
                    return Err(VfsError::InvalidPath(format!(
                        "invalid path: {}. The object is not a directory",
                        f_name
                    )));
                }
                object_id = fi.object_id;
            }
            // Segment missing: create it and continue downward.
            Err(VfsError::FileNotFound(_)) => {
                object_id = handle_make_directory(session, storage_id, object_id, f_name)?;
            }
            // Any other error is fatal.
            Err(e) => return Err(e),
        }
    }

    Ok(object_id)
}

/// Create a single directory object under `parent_id`: a `SendObjectInfo` for an
/// association with NO following `SendObject` data phase (an association carries no
/// bytes). The returned storage/parent ids are discarded; only the new handle is
/// kept.
fn handle_make_directory<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    parent_id: u32,
    filename: &str,
) -> Result<u32, VfsError> {
    // All unset fields take their Default (incl. capture_date == None);
    // modification_date is set to now.
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

/// Batch existence check.
///
/// Never returns `Err` (the signature keeps a `Result` for the public API, but
/// every path yields `Ok`). Case-insensitive matching is done inside path
/// resolution; the caller's casing is not echoed back here.
pub fn file_exists<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    file_props: &[FileProp],
) -> Result<Vec<FileExistsContainer>, VfsError> {
    let mut fc: Vec<FileExistsContainer> = Vec::new();

    for fp in file_props {
        match get_object_from_object_id_or_path(session, storage_id, fp) {
            // Found.
            Ok(fi) => fc.push(FileExistsContainer {
                exists: true,
                file_info: Some(fi),
            }),
            Err(e) => match &e {
                // Bad/missing path ⇒ not-exists.
                VfsError::InvalidPath(_) => fc.push(FileExistsContainer {
                    exists: false,
                    file_info: None,
                }),
                // Any FileObject error yields not-exists. 0x2009
                // (InvalidObjectHandle) is what a device returns for GetObjectInfo
                // on a stale/bogus handle, e.g. a delete of a non-existent object
                // id — but the classification here is FileObject-wide, not keyed on
                // that specific RC.
                VfsError::FileObject(_) => fc.push(FileExistsContainer {
                    exists: false,
                    file_info: None,
                }),
                // Any OTHER error type aborts the batch, discarding whatever was
                // collected and returning an EMPTY vec with no error. Unreachable in
                // practice: path resolution only yields InvalidPath / FileObject.
                _ => return Ok(Vec::new()),
            },
        }
    }

    Ok(fc)
}

/// Delete each file/directory in the batch (device-side recursive).
///
/// PRESERVED QUIRK: the first non-existent entry `return`s from the whole function,
/// so every later entry in `file_props` is silently skipped rather than attempted.
pub fn delete_file<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    file_props: &[FileProp],
) -> Result<(), VfsError> {
    for fp in file_props {
        // file_exists never errors; this guard is dead in practice but kept.
        let fc = match file_exists(session, storage_id, std::slice::from_ref(fp)) {
            Ok(fc) => fc,
            Err(_) => return Ok(()),
        };

        // Treat an (impossible) empty batch result as not-exists rather than
        // indexing out of bounds.
        match fc.first() {
            Some(c) if c.exists => {
                // exists ⇒ file_info is Some (file_exists invariant).
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
pub fn rename_file<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    file_prop: &FileProp,
    new_file_name: &str,
) -> Result<u32, VfsError> {
    // file_exists never errors, but keep the `?` anyway.
    let fc = file_exists(session, storage_id, std::slice::from_ref(file_prop))?;

    // Not found ⇒ InvalidPath; an empty result is treated as not-found rather than
    // panicking.
    let object_id = match fc.first().filter(|c| c.exists) {
        Some(c) => c.file_info.as_ref().map(|f| f.object_id).unwrap_or(0),
        None => {
            return Err(VfsError::InvalidPath(format!(
                "file not found: {}",
                file_prop.full_path
            )));
        }
    };

    // SetObjectPropValue(OPC_ObjectFileName, new name).
    let value = PropValue::Str(new_file_name.to_string());
    match session.set_object_prop_value(object_id, ObjectPropCode::OBJECT_FILE_NAME, &value) {
        Ok(()) => Ok(object_id),
        Err(e) => {
            // RC 0x2002 GeneralError is tolerated as success; some devices return it
            // when the new name equals the current one.
            if let MtpError::Rc(rc) = &e {
                if rc.code() == RespCode::GENERAL_ERROR.0 {
                    return Ok(object_id);
                }
            }
            Err(VfsError::FileObject(e))
        }
    }
}
