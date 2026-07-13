//! Path normalization and path → object-handle resolution.
//!
//! Provides the path helpers (`fix_slash`, `get_full_path`, and their lexical
//! `path_clean` dependency) and the resolvers
//! [`get_object_from_parent_id_and_filename`], [`get_object_from_path`], and
//! [`get_object_from_object_id_or_path`]. The FileInfo-from-handle constructor
//! lives in `object.rs` as [`from_object_id`].
//!
//! Resolution walks the path segment by segment from the storage root
//! (`0xFFFFFFFF`): for each segment it lists the parent's children
//! (`GetObjectHandles(parent, format=0)` — 0 means *all* formats), reads each
//! child's `OPC_ObjectFileName` and Unicode-case-insensitively compares it to the
//! segment, then re-verifies the match against the object's `ObjectInfo.Filename`.
//! The caller's casing is echoed back into the returned `full_path`.
//!
//! Generic over [`Transport`] — only `device.rs` names the concrete USB type.
//! [`FileProp`] is defined here (the sibling `dirops` module imports it as
//! `crate::path::FileProp`).

use keel_mtp::{MtpSession, Transport};
use keel_proto::{ObjectPropCode, PropValue};

use crate::error::VfsError;
use crate::object::{FileInfo, from_object_id};

/// The path separator. Ferry targets macOS, so this is `"/"`.
pub const PATH_SEP: &str = "/";

/// The synthetic parent handle of a storage root (`0xFFFFFFFF`).
pub const PARENT_OBJECT_ID: u32 = 0xFFFF_FFFF;

/// The ObjectFormatCode filter for `GetObjectHandles`. `0` means "all formats",
/// not "associations" — despite what the constant name in some MTP stacks implies.
pub const FORMAT_ALL: u16 = 0x0000;

/// An object addressed by handle and/or path. `object_id == 0` means "resolve by
/// `full_path` instead". Defined here; `dirops.rs` imports it for the metadata ops.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileProp {
    pub object_id: u32,
    pub full_path: String,
}

/// Ensure a leading `/`, then lexically clean the path.
pub fn fix_slash(abs_filepath: &str) -> String {
    let s = if abs_filepath.starts_with(PATH_SEP) {
        abs_filepath.to_string()
    } else {
        format!("{PATH_SEP}{abs_filepath}")
    };
    path_clean(&s)
}

/// Join parent + `/` + filename, then clean.
pub fn get_full_path(parent_path: &str, filename: &str) -> String {
    fix_slash(&format!("{parent_path}{PATH_SEP}{filename}"))
}

/// Lexically clean a path, which `fix_slash` calls. Purely lexical: collapse `//`,
/// drop `.`, resolve `..` against preceding elements (and swallow `..` at the
/// root). Operates on bytes — only the ASCII `/` and `.` bytes are ever inspected,
/// and UTF-8 continuation bytes are all ≥ 0x80, so byte-wise processing preserves
/// any multibyte filename unchanged.
fn path_clean(path: &str) -> String {
    let path = path.as_bytes();
    if path.is_empty() {
        return ".".to_string();
    }

    let rooted = path[0] == b'/';
    let n = path.len();

    // A write buffer with a length index `w`; the `..` backtrack peeks at `buf[w]`
    // — the byte *just past* the logical end (still in the backing array) — to find
    // the previous separator without erasing it. A `Vec` that actually pops can't
    // peek past its end, so use a fixed buffer plus an explicit `w`. The cleaned
    // output is never longer than the input, so a buffer of length `n` always
    // suffices.
    let mut buf = vec![0u8; n];
    let mut w = 0usize;
    let mut r = 0usize;
    let mut dotdot = 0usize;

    if rooted {
        buf[w] = b'/';
        w += 1;
        r = 1;
        dotdot = 1;
    }

    while r < n {
        if path[r] == b'/' {
            // empty path element — skip.
            r += 1;
        } else if path[r] == b'.' && (r + 1 == n || path[r + 1] == b'/') {
            // `.` element — skip.
            r += 1;
        } else if path[r] == b'.'
            && r + 1 < n
            && path[r + 1] == b'.'
            && (r + 2 == n || path[r + 2] == b'/')
        {
            // `..` element — back up over the previous element.
            r += 2;
            if w > dotdot {
                w -= 1;
                while w > dotdot && buf[w] != b'/' {
                    w -= 1;
                }
            } else if !rooted {
                // Cannot backtrack and not rooted: keep the `..`.
                if w > 0 {
                    buf[w] = b'/';
                    w += 1;
                }
                buf[w] = b'.';
                w += 1;
                buf[w] = b'.';
                w += 1;
                dotdot = w;
            }
            // rooted && w <= dotdot: swallow the `..` (root's parent is root).
        } else {
            // A real path element: add a separator if one is needed, then copy.
            if (rooted && w != 1) || (!rooted && w != 0) {
                buf[w] = b'/';
                w += 1;
            }
            while r < n && path[r] != b'/' {
                buf[w] = path[r];
                w += 1;
                r += 1;
            }
        }
    }

    if w == 0 {
        return ".".to_string();
    }

    // buf[..w] is valid UTF-8: every byte was copied whole from the valid-UTF-8
    // input, and only ASCII `/` `.` were synthesized.
    String::from_utf8(buf[..w].to_vec()).unwrap_or_else(|_| ".".to_string())
}

/// Whether `index` is in bounds for `arr` (`arr.len() > index`).
fn index_exists(arr: &[&str], index: usize) -> bool {
    arr.len() > index
}

/// Unicode case-insensitive string comparison.
///
/// keel's dependency budget excludes any Unicode-table crate, so this approximates
/// case-folding via the full-Unicode lowercase mapping in `str`. That agrees with
/// true case-folding for every practical filename and is exact for ASCII. The rare
/// divergent cases (e.g. the Kelvin sign, ſ, final sigma) do not occur in MTP
/// filenames.
fn eq_fold(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Resolve a single filename within a parent directory.
///
/// List the parent's children (all formats), and for each read
/// `OPC_ObjectFileName`; skip non-matches cheaply (avoids a full GetObjectInfo
/// per child), and on a case-insensitive hit build the full [`FileInfo`] and
/// re-verify its `name` before returning. No match → `FileNotFound`.
pub fn get_object_from_parent_id_and_filename<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    parent_id: u32,
    filename: &str,
) -> Result<FileInfo, VfsError> {
    let handles = session
        .object_handles(storage_id, FORMAT_ALL, parent_id)
        .map_err(VfsError::FileObject)?;

    for object_id in handles {
        let val = session
            .object_prop_value(object_id, ObjectPropCode::OBJECT_FILE_NAME)
            .map_err(VfsError::FileObject)?;
        let prop_name = match val {
            PropValue::Str(s) => s,
            // OPC_ObjectFileName's fixed type is STR, so object_prop_value always
            // yields Str here (or a Proto error caught above). Skip defensively.
            _ => continue,
        };

        // Cheap pre-filter on the property value.
        if !eq_fold(&prop_name, filename) {
            continue;
        }

        // `from_object_id` already returns VfsError::FileObject, so propagate it
        // directly rather than re-wrapping.
        let fi = from_object_id(session, object_id, "")?;

        // Re-verify against the object's real filename.
        if eq_fold(&fi.name, filename) {
            return Ok(fi);
        }
    }

    Err(VfsError::FileNotFound(format!(
        "file not found: {filename}"
    )))
}

/// Resolve an absolute path to its object.
///
/// Empty input → `InvalidPath`. Root (`/`) → the synthetic root object. Otherwise
/// split the cleaned path and resolve each segment against the running parent
/// handle; a missing segment becomes `InvalidPath` ("path not found"), and a
/// non-directory segment with more path after it is also `InvalidPath`. The
/// returned object's `full_path` is overwritten with the cleaned input path so
/// the caller's casing is echoed back.
pub fn get_object_from_path<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    full_path: &str,
) -> Result<FileInfo, VfsError> {
    if full_path.is_empty() {
        // The capital-E "Exists" is intentional: this exact message is part of the
        // frozen wire contract.
        return Err(VfsError::InvalidPath(format!(
            "path does not Exists. path: {full_path}"
        )));
    }

    let file_path = fix_slash(full_path);

    if file_path == PATH_SEP {
        return from_object_id(session, PARENT_OBJECT_ID, "");
    }

    // fix_slash guarantees a leading '/', so split yields ["", seg1, seg2, …];
    // skip the leading empty element.
    let splitted: Vec<&str> = file_path.split(PATH_SEP).collect();
    const SKIP_INDEX: usize = 1;

    let mut object_id = PARENT_OBJECT_ID;
    let mut result_count = 0usize;
    let mut fi: Option<FileInfo> = None;

    for (i, &f_name) in splitted[SKIP_INDEX..].iter().enumerate() {
        let cur =
            match get_object_from_parent_id_and_filename(session, storage_id, object_id, f_name) {
                Ok(v) => v,
                // FileNotFound → "path not found" InvalidPath.
                Err(VfsError::FileNotFound(reason)) => {
                    return Err(VfsError::InvalidPath(format!(
                        "path not found: {full_path}\nreason: {reason}"
                    )));
                }
                // Any other error propagates unchanged.
                Err(e) => return Err(e),
            };

        // A non-directory segment followed by more path is invalid.
        // `i + 1 + SKIP_INDEX` indexes the *next* segment in the full split.
        if !cur.is_dir && index_exists(&splitted, i + 1 + SKIP_INDEX) {
            return Err(VfsError::InvalidPath(format!(
                "path not found: {full_path}"
            )));
        }

        object_id = cur.object_id;
        fi = Some(cur);
        result_count += 1;
    }

    // Require at least one resolved segment.
    let mut fi = match fi {
        Some(f) if result_count >= 1 => f,
        _ => {
            return Err(VfsError::InvalidPath(format!(
                "file not found: {full_path}"
            )));
        }
    };

    // Echo the caller's cleaned path back.
    fi.full_path = file_path;
    Ok(fi)
}

/// Resolve by handle, falling back to path resolution when `object_id == 0`. Both
/// empty → `InvalidPath`.
pub fn get_object_from_object_id_or_path<T: Transport>(
    session: &mut MtpSession<T>,
    storage_id: u32,
    file_prop: &FileProp,
) -> Result<FileInfo, VfsError> {
    let object_id = file_prop.object_id;
    let full_path = &file_prop.full_path;

    if object_id == 0 && full_path.is_empty() {
        return Err(VfsError::InvalidPath(format!(
            "invalid path: {full_path}. both objectId and fullPath cannot be empty"
        )));
    }

    if object_id == 0 {
        return get_object_from_path(session, storage_id, full_path);
    }

    from_object_id(session, object_id, full_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_slash_prepends_and_cleans() {
        assert_eq!(fix_slash(""), "/");
        assert_eq!(fix_slash("a/b"), "/a/b");
        assert_eq!(fix_slash("/a/b/"), "/a/b");
        assert_eq!(fix_slash("/a//b/./c"), "/a/b/c");
        assert_eq!(fix_slash("/a/b/../c"), "/a/c");
        assert_eq!(fix_slash("/.."), "/");
        assert_eq!(fix_slash("/a/../../b"), "/b");
    }

    #[test]
    fn fix_slash_preserves_unicode() {
        assert_eq!(fix_slash("/Photos/café/x.jpg"), "/Photos/café/x.jpg");
        assert_eq!(fix_slash("Fotos/日本"), "/Fotos/日本");
    }

    #[test]
    fn get_full_path_joins_and_cleans() {
        assert_eq!(get_full_path("/DCIM", "IMG.jpg"), "/DCIM/IMG.jpg");
        assert_eq!(get_full_path("/", "Download"), "/Download");
        assert_eq!(get_full_path("", "x"), "/x");
    }

    #[test]
    fn eq_fold_is_case_insensitive() {
        // ASCII case-insensitive comparison.
        assert!(eq_fold("DCIM", "dcim"));
        assert!(eq_fold("IMG_0001.JPG", "img_0001.jpg"));
        assert!(!eq_fold("a", "b"));
        assert!(!eq_fold("file", "files"));
    }

    #[test]
    fn eq_fold_is_case_insensitive_unicode() {
        // The Unicode filename compare used per-segment during path resolution.
        // Folding via `str::to_lowercase` (see eq_fold's doc) handles Latin-1
        // accents, Greek, Cyrillic, and full-width forms:
        assert!(eq_fold("Résumé", "résumé"));
        assert!(eq_fold("CAFÉ.txt", "café.TXT"));
        assert!(eq_fold("Ünïcødé", "ünïcødé"));
        assert!(eq_fold("ΕΛΛΗΝΙΚΆ", "ελληνικά")); // Greek
        assert!(eq_fold("Привет", "привет")); // Cyrillic
        assert!(!eq_fold("café", "cafe")); // accent is NOT folded away
        // Non-cased scripts compare as-is (no fold needed).
        assert!(eq_fold("日本語", "日本語"));
        assert!(!eq_fold("日本", "日本語"));
    }

    #[test]
    fn index_exists_matches_go_semantics() {
        let v = ["", "a", "b"];
        assert!(index_exists(&v, 2));
        assert!(!index_exists(&v, 3));
    }
}
