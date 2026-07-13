//! `FileInfo` — the resolved object model — plus the `ObjectInfo` → `FileInfo`
//! construction and the size / extension rules.
//!
//! Holds [`FileInfo`], the [`extension`] rule, and the object helpers
//! [`get_file_size`], [`is_object_a_dir`], and [`from_object_id`]. The
//! path-normalization helpers those depend on (`fix_slash`, `get_full_path`) live
//! in `path.rs`.
//!
//! Generic over the [`Transport`] so the same code drives the real
//! `UsbTransport` and the test `FakeDevice` — matching the rest of keel-vfs
//! (`path.rs` / `walk.rs` / `dirops.rs`); only `device.rs` names the concrete USB
//! type. The sibling `walk` module calls [`from_object_id`] directly.

use std::time::SystemTime;

use keel_mtp::{MtpError, MtpSession, Transport};
use keel_proto::{ObjectInfo, ObjectPropCode, PropValue, ProtoError};

use crate::error::VfsError;

/// Fetch the device-generated thumbnail (MTP `GetThumb`) for the object at
/// `full_path`. Returns `Ok(None)` when the object has no thumbnail available —
/// folders, documents, or any device that declines — so the caller falls back
/// to a type glyph. Only genuine transport/sync failures are `Err`.
pub fn thumbnail<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    full_path: &str,
) -> Result<Option<Vec<u8>>, VfsError> {
    let fi = crate::path::get_object_from_path(session, storage_id, full_path)?;
    if fi.is_dir {
        return Ok(None);
    }
    session.get_thumb(fi.object_id).map_err(VfsError::Mtp)
}
use crate::path::{PARENT_OBJECT_ID, fix_slash, get_full_path};

/// The MTP `OFC_Association` format code. An object with this format code is a
/// directory (association); everything else is a file.
pub const OFC_ASSOCIATION: u16 = 0x3001;

/// The single registered two-part extension is `tar`, which produces the
/// `archive.tar.gz` → `"tar.gz"` result.
const ALLOWED_SECOND_EXTENSIONS: &[&str] = &["tar"];

fn is_allowed_second_extension(s: &str) -> bool {
    ALLOWED_SECOND_EXTENSIONS.contains(&s)
}

/// An object's resolved metadata.
///
/// Two field mappings worth naming:
///   * `mod_time` is `Option<SystemTime>` because it is sourced from
///     `ObjectInfo.modification_date`, which keel-proto models as
///     `Option<SystemTime>`; `None` is the zero/absent timestamp.
///   * `info` owns an `ObjectInfo`; the root shortcut uses
///     `ObjectInfo::default()`.
#[derive(Clone, Debug, Default)]
pub struct FileInfo {
    pub size: i64,
    pub is_dir: bool,
    pub mod_time: Option<SystemTime>,
    pub name: String,
    pub full_path: String,
    pub parent_path: String,
    pub extension: String,
    pub parent_id: u32,
    pub object_id: u32,

    pub info: ObjectInfo,
}

/// A directory is an object whose format code is `OFC_Association`.
pub fn is_object_a_dir(obj: &ObjectInfo) -> bool {
    obj.object_format == OFC_ASSOCIATION
}

/// Compute a file's extension.
///
/// Directories have no extension. Otherwise take the base name, split on `.`,
/// and:
///   * if there are ≥3 segments and the second-to-last is a registered second
///     extension (`tar`), return the last two joined (`"tar.gz"`);
///   * else if there are ≥2 segments, return the last one;
///   * else the empty string.
pub fn extension(filename: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }

    // Take the base name after the last separator. Filenames rarely contain a
    // slash, but honor the split anyway.
    let base = match filename.rsplit_once('/') {
        Some((_, f)) => f,
        None => filename,
    };

    let f: Vec<&str> = base.split('.').collect();
    let length = f.len();

    // Unreachable since `split` yields at least one element, but guarded anyway.
    if length == 0 {
        return String::new();
    }

    // Two-part special case: the `foo.tar.gz` → "tar.gz" rule.
    if length > 2 && is_allowed_second_extension(f[length - 2]) {
        return format!("{}.{}", f[length - 2], f[length - 1]);
    }

    if length > 1 {
        return f[length - 1].to_string();
    }

    String::new()
}

/// Resolve an object's size.
///
/// Directories are size 0. For files, the `ObjectInfo.compressed_size` u32 is the
/// size unless it is the `0xFFFFFFFF` ">4 GiB" sentinel, in which case the true
/// 64-bit size is fetched via `OPC_ObjectSize`.
pub fn get_file_size<T: Transport>(
    session: &mut MtpSession<T>,
    obj: &ObjectInfo,
    object_id: u32,
    is_dir: bool,
) -> Result<i64, VfsError> {
    if is_dir {
        return Ok(0);
    }

    if obj.compressed_size == 0xFFFF_FFFF {
        let val = session
            .object_prop_value(object_id, ObjectPropCode::OBJECT_SIZE)
            .map_err(VfsError::FileObject)?;
        // Wrap the raw MtpError so `rc_code()` can still inspect it; the RC-name
        // substring the FFI keys on is preserved.
        match val {
            PropValue::U64(v) => Ok(v as i64),
            // OPC_ObjectSize is a fixed UINT64 property, so object_prop_value
            // always yields U64 here (or a Proto error caught by `?`). Guard
            // defensively rather than panic — never unwrap on device input.
            _ => Err(VfsError::FileObject(MtpError::Proto(
                ProtoError::Unsupported("OPC_ObjectSize: expected UINT64 value"),
            ))),
        }
    } else {
        Ok(obj.compressed_size as i64)
    }
}

/// Build a [`FileInfo`] from a handle. `parent_path` is threaded through only to
/// compute `full_path`; that `full_path` may be inaccurate when the caller lacks
/// the real parent path.
pub fn from_object_id<T: Transport>(
    session: &mut MtpSession<T>,
    object_id: u32,
    parent_path: &str,
) -> Result<FileInfo, VfsError> {
    // Root shortcut: the root parent has no ObjectInfo.
    if object_id == PARENT_OBJECT_ID {
        return Ok(FileInfo {
            size: 0,
            is_dir: true,
            mod_time: None,
            name: String::new(),
            full_path: "/".to_string(),
            parent_path: String::new(),
            extension: String::new(),
            parent_id: 0,
            object_id: PARENT_OBJECT_ID,
            info: ObjectInfo::default(),
        });
    }

    let obj = session
        .object_info(object_id)
        .map_err(VfsError::FileObject)?;
    let is_dir = is_object_a_dir(&obj);

    // `get_file_size` already returns VfsError::FileObject, so propagate it
    // directly rather than re-wrapping.
    let size = get_file_size(session, &obj, object_id, is_dir)?;

    let parent_path = fix_slash(parent_path);
    let full_path = get_full_path(&parent_path, &obj.filename);
    let ext = extension(&obj.filename, is_dir);
    let mod_time = obj.modification_date;
    let name = obj.filename.clone();
    let parent_id = obj.parent_object;

    Ok(FileInfo {
        size,
        is_dir,
        mod_time,
        name,
        full_path,
        parent_path,
        extension: ext,
        parent_id,
        object_id,
        info: obj,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_simple() {
        assert_eq!(extension("movie.mkv", false), "mkv");
        assert_eq!(extension("IMG_0001.JPG", false), "JPG");
    }

    #[test]
    fn extension_none_for_dir_and_no_dot() {
        assert_eq!(extension("Downloads", true), "");
        assert_eq!(extension("Downloads", false), "");
        assert_eq!(extension("README", false), "");
    }

    #[test]
    fn extension_tar_gz_two_part_special_case() {
        // The only registered second extension is "tar".
        assert_eq!(extension("archive.tar.gz", false), "tar.gz");
        assert_eq!(extension("backup.tar.bz2", false), "tar.bz2");
        // A non-"tar" second segment falls through to the single-extension rule.
        assert_eq!(extension("photo.min.png", false), "png");
    }

    #[test]
    fn extension_dotfile() {
        // ".bashrc".split('.') == ["", "bashrc"], length 2 → last segment.
        assert_eq!(extension(".bashrc", false), "bashrc");
    }

    #[test]
    fn is_dir_only_for_association() {
        let mut obj = ObjectInfo {
            object_format: OFC_ASSOCIATION,
            ..Default::default()
        };
        assert!(is_object_a_dir(&obj));
        obj.object_format = 0x3000; // OFC_Undefined
        assert!(!is_object_a_dir(&obj));
    }
}
