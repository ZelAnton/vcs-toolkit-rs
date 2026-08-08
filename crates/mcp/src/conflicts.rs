//! The plumbing behind the two conflict tools —
//! [`repo_conflict_regions`](crate::VcsMcpServer::repo_conflict_regions) and
//! [`repo_resolve_conflict`](crate::VcsMcpServer::repo_resolve_conflict): the
//! repo-containment path guard, the JSON wire shapes, and the git/jj dispatch
//! over `vcs_git::conflict` / `vcs_jj::conflict`. Crate-internal
//! (`pub(crate)`); the public surface is the two `#[tool]` methods in
//! `repo_tools`.
//!
//! # Why the working copy, not `repo_show_file`
//!
//! Conflict markers are **materialized in the working copy**, and on git they
//! exist *only* there: a conflicted path has no stage-0 index entry, so
//! `git show :<path>` fails outright and `git show HEAD:<path>` returns `HEAD`'s
//! clean version with no markers at all (verified against git 2.55). Routing
//! these tools through the `show_file` facade would therefore report "no
//! conflicts" for a file `repo_conflicts` lists as conflicted — a silent, badly
//! wrong answer. On jj `jj file show -r @` *does* return the materialized markers
//! (verified against jj 0.38, byte-identical to the file on disk), but reading
//! the working copy on both backends keeps one semantics, and — decisively —
//! makes the read tool report exactly the bytes the resolve tool will overwrite.
//!
//! Because neither tool's read spawns a backend command, the read tool records no
//! jj operation and is honestly `readOnlyHint` (see `repo_info`'s precedent);
//! `repo_resolve_conflict` still spawns (its conflicted-path check and, on git,
//! the finalizing `git add`) and is write-gated and `destructiveHint`.
//!
//! # Why the budget is re-applied here
//!
//! Spawning nothing also means inheriting nothing: the [`OutputBudget`] a caller
//! configures on the git/jj client bounds only what a *subprocess* writes to its
//! pipe, so it cannot bound a read that never runs one. These tools therefore
//! carry the server's own `content_budget` and enforce it at the filesystem —
//! same knob (`--max-output-bytes`), same unit, same fail-loud contract as
//! `repo_show_file` (refuse naming the ceiling; never hand back a truncated
//! file). Without it `repo_conflict_regions` — ungated, callable in the server's
//! most restricted mode — would buffer a file of any size into the server and
//! then into a larger JSON rendering of it.

use std::path::{Component, Path, PathBuf};

use rmcp::ErrorData;
use rmcp::model::CallToolResult;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vcs_core::OutputBudget;
use vcs_core::processkit::{Error as VcsError, ErrorReason};
use vcs_core::{BackendKind, vcs_git, vcs_jj};

use crate::output::ok_json;
use crate::params::ConflictSideArg;

/// An agent-supplied path, validated to stay inside the repository.
pub(crate) struct RepoPath {
    /// Repo-relative — the shape `Repo::conflicted_files`/`mark_resolved` use.
    pub(crate) rel: PathBuf,
    /// Canonical repository root used as the anchor for direct filesystem I/O.
    #[cfg(any(unix, windows))]
    root: PathBuf,
}

/// Resolve an agent-supplied repo-relative `path` against the repository `root`,
/// refusing anything that could address a file outside it.
///
/// Every path component must be [`Component::Normal`]: that rejects an absolute
/// path, a Windows drive prefix (`C:\…`), a bare root (`/…`), `.`, and — the one
/// that matters — any `..` traversal. Both `/` and `\` are accepted as separators
/// on Windows (`Path::components` splits on both there), while on Unix a
/// backslash stays an ordinary filename character, as it must. On Windows a
/// component naming a legacy DOS **device** is refused too — see
/// [`is_reserved_device_name`].
///
/// This is the guard the *facade* would otherwise provide: every other `repo_*`
/// tool reaches the filesystem through a `git`/`jj` subprocess run inside the
/// repo, which confines the path for us. These two touch the filesystem directly,
/// so the confinement has to be explicit. Existing symlink/reparse-point
/// components are refused after their resolved target is checked, and the actual
/// opens repeat the no-follow policy so the validation cannot become a TOCTOU
/// escape.
pub(crate) fn repo_path(root: &Path, path: &str) -> Result<RepoPath, ErrorData> {
    let refuse = |why: &str| {
        Err(ErrorData::invalid_params(
            format!("path {path:?} {why}; pass a repo-relative path, as repo_conflicts reports"),
            None,
        ))
    };
    if path.is_empty() {
        return refuse("is empty");
    }
    let mut rel = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => {
                // `cfg!` rather than `#[cfg]`: the predicate compiles (and is
                // unit-tested) on every platform, but only Windows resolves these
                // names as devices, and only there would refusing them cost
                // nothing — Win32 forbids creating such a file in the first place.
                if cfg!(windows) && is_reserved_device_name(&part.to_string_lossy()) {
                    return refuse(
                        "names a reserved Windows device (CON, NUL, COM1, …), which cannot be \
                         a repository file — opening it would read a device, not a file",
                    );
                }
                rel.push(part);
            }
            Component::ParentDir => return refuse("contains a `..` component"),
            Component::CurDir => return refuse("contains a `.` component"),
            Component::RootDir | Component::Prefix(_) => return refuse("is absolute"),
        }
    }
    if rel.as_os_str().is_empty() {
        return refuse("names no file");
    }
    // Work from a stable, physical repository root. The repository handles
    // normally already provide this spelling, but canonicalising here also
    // keeps a caller-provided root symlink out of the direct-open path.
    let root = root.canonicalize().map_err(|e| {
        ErrorData::invalid_params(format!("cannot resolve repository root: {e}"), None)
    })?;
    let mut current = root.clone();
    for component in rel.components() {
        let Component::Normal(part) = component else {
            unreachable!("repo_path only builds normal components");
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let outside = current
                    .canonicalize()
                    .ok()
                    .is_some_and(|resolved| !resolved.starts_with(&root));
                return refuse(if outside {
                    "resolves outside the repository through a symbolic link"
                } else {
                    "contains a symbolic link; direct conflict-file access refuses links"
                });
            }
            Ok(_) => {}
            // A missing path is left for the existing read/conflict-state
            // checks to report; this guard must not change their semantics.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => break,
        }
    }
    // Canonicalisation catches Windows junctions and any other reparse-point
    // form that `Metadata::is_symlink` does not classify as a symlink. It is a
    // containment check only; the descriptor-based opens below remain the race
    // resistant enforcement point.
    if let Ok(resolved) = root.join(&rel).canonicalize()
        && !resolved.starts_with(&root)
    {
        return refuse("resolves outside the repository");
    }
    Ok(RepoPath {
        #[cfg(any(unix, windows))]
        root: root.clone(),
        rel,
    })
}

/// Whether `component` is one of Win32's legacy DOS device names.
///
/// Windows resolves these in **every** directory — `<repo>\CON` is the console,
/// `<repo>\COM1` a serial port — regardless of an extension (`CON.txt` is still
/// `CON`) and ignoring trailing spaces/dots, which Win32 strips. Opening one
/// yields a *device*, and a read from it can block indefinitely; since these two
/// tools are the only ones that open a path themselves (everywhere else the
/// backend subprocess does it), the component guard is where that has to be
/// caught. Nothing is lost by refusing them: Win32 will not let such a file be
/// created, so no repository can legitimately contain one on Windows.
///
/// Applied on Windows only — on Unix these are ordinary, perfectly valid
/// filenames with no device meaning at all.
pub(crate) fn is_reserved_device_name(component: &str) -> bool {
    // Win32 strips trailing spaces and dots, then matches the stem before the
    // first `.` case-insensitively.
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches([' ', '.']);
    const RESERVED: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    let upper = stem.to_ascii_uppercase();
    if RESERVED.contains(&upper.as_str()) {
        return true;
    }
    // `COM1`–`COM9` / `LPT1`–`LPT9`, plus the superscript-digit spellings
    // (`COM¹`) Win32 maps to the same devices — hence `is_numeric`, which covers
    // them where `is_ascii_digit` would not.
    let numbered = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"));
    match numbered {
        Some(digit) => {
            let mut chars = digit.chars();
            matches!((chars.next(), chars.next()), (Some(c), None) if c.is_numeric() && c != '0')
        }
        None => false,
    }
}

/// Direct conflict-file I/O must not resolve a path through a symlink after
/// `repo_path` has validated it. Unix has the required descriptor-relative
/// primitives; Windows uses `FILE_FLAG_OPEN_REPARSE_POINT` for the final
/// component and the canonical/component checks above for parent components.
#[cfg(unix)]
mod secure_fs {
    use super::RepoPath;
    use std::ffi::CString;
    use std::fs::File;
    use std::io;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path};

    fn c_string(path: &std::ffi::OsStr) -> io::Result<CString> {
        CString::new(path.as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "repository path contains an embedded NUL byte",
            )
        })
    }

    fn open_relative(root: &Path, rel: &Path, write: bool) -> io::Result<File> {
        let root = c_string(root.as_os_str())?;
        let directory_flags =
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: `root` is a NUL-terminated path made by `CString`; the flags
        // request a directory without following a symlink. The returned fd is
        // owned by this function and closed on every error path below.
        let mut dir_fd = unsafe { libc::open(root.as_ptr(), directory_flags) };
        if dir_fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut components = rel.components();
        let Some(last) = components.next_back() else {
            // `repo_path` rejects an empty relative path; keep this helper
            // defensive because it is the security boundary.
            unsafe { libc::close(dir_fd) };
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty repository-relative path",
            ));
        };

        for component in components {
            let Component::Normal(part) = component else {
                unsafe { libc::close(dir_fd) };
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "non-normal repository path component",
                ));
            };
            let part = match c_string(part) {
                Ok(part) => part,
                Err(error) => {
                    unsafe { libc::close(dir_fd) };
                    return Err(error);
                }
            };
            // O_NOFOLLOW is applied to every parent, not just the leaf: an
            // attacker cannot swap a checked directory for a link to outside
            // content between component opens.
            let next = unsafe { libc::openat(dir_fd, part.as_ptr(), directory_flags, 0) };
            if next < 0 {
                let error = io::Error::last_os_error();
                unsafe { libc::close(dir_fd) };
                return Err(error);
            }
            unsafe { libc::close(dir_fd) };
            dir_fd = next;
        }

        let last = match c_string(last.as_os_str()) {
            Ok(last) => last,
            Err(error) => {
                unsafe { libc::close(dir_fd) };
                return Err(error);
            }
        };
        let file_flags = if write {
            libc::O_WRONLY | libc::O_TRUNC | libc::O_NOFOLLOW | libc::O_CLOEXEC
        } else {
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC
        };
        // SAFETY: `dir_fd` is an open directory descriptor and `last` is a
        // NUL-terminated single path component. O_NOFOLLOW makes this final
        // open atomic with respect to a leaf symlink swap; parent descriptors
        // above provide the same property for every directory component.
        let file_fd = unsafe { libc::openat(dir_fd, last.as_ptr(), file_flags, 0) };
        let open_error = (file_fd < 0).then(io::Error::last_os_error);
        let close_error = unsafe { libc::close(dir_fd) };
        if let Some(error) = open_error {
            return Err(error);
        }
        if close_error < 0 {
            let error = io::Error::last_os_error();
            unsafe { libc::close(file_fd) };
            return Err(error);
        }
        // SAFETY: `file_fd` is a newly opened descriptor transferred into the
        // standard library file, which takes ownership and closes it on drop.
        Ok(unsafe { File::from_raw_fd(file_fd) })
    }

    pub(super) fn open_read(path: &RepoPath) -> io::Result<File> {
        open_relative(&path.root, &path.rel, false)
    }

    pub(super) fn open_write(path: &RepoPath) -> io::Result<File> {
        open_relative(&path.root, &path.rel, true)
    }
}

#[cfg(windows)]
mod secure_fs {
    use super::RepoPath;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::Component;
    use std::ptr::{null, null_mut};

    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
        FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
    };
    use windows_sys::Win32::Foundation::{HANDLE, RtlNtStatusToDosError, UNICODE_STRING};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, SetFilePointerEx,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    fn nt_error(status: i32) -> io::Error {
        let code = unsafe { RtlNtStatusToDosError(status) };
        io::Error::from_raw_os_error(code as i32)
    }

    fn open_child(
        parent: &File,
        component: &std::ffi::OsStr,
        write: bool,
        directory: bool,
    ) -> io::Result<File> {
        let mut name: Vec<u16> = component.encode_wide().collect();
        if name.is_empty() || name.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "repository path contains an invalid Windows component",
            ));
        }
        let name_bytes = name.len().checked_mul(2).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "repository path component is too long",
            )
        })?;
        let name_bytes = u16::try_from(name_bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "repository path component is too long",
            )
        })?;
        let name = UNICODE_STRING {
            Length: name_bytes,
            MaximumLength: name_bytes,
            Buffer: name.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle() as HANDLE,
            ObjectName: &name,
            Attributes: 0,
            SecurityDescriptor: null(),
            SecurityQualityOfService: null(),
        };
        let mut status_block = IO_STATUS_BLOCK::default();
        let mut handle: HANDLE = null_mut();
        let create_options = if directory {
            FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT
        } else {
            FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT
        };
        // Open first, then write through the pinned handle below. FILE_OVERWRITE
        // asks the Windows object manager to replace the file during open and
        // is rejected for some ordinary files when FILE_OPEN_REPARSE_POINT is
        // also present.
        let disposition = FILE_OPEN;
        // SAFETY: all pointers reference local values kept alive for the native
        // call; `parent` owns a live handle and `name` is a UTF-16 component
        // without an embedded NUL. NtCreateFile returns an owned HANDLE.
        let status = unsafe {
            NtCreateFile(
                &mut handle,
                if write {
                    FILE_GENERIC_READ | FILE_GENERIC_WRITE
                } else {
                    FILE_GENERIC_READ
                },
                &attributes,
                &mut status_block,
                null(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                disposition,
                create_options,
                null(),
                0,
            )
        };
        if status != 0 {
            return Err(nt_error(status));
        }
        if handle.is_null() {
            return Err(io::Error::other("NtCreateFile returned a null handle"));
        }
        // SAFETY: successful NtCreateFile transferred ownership of `handle`.
        let file = unsafe { File::from_raw_handle(handle) };
        if file.metadata()?.file_type().is_symlink() {
            return Err(io::Error::other(
                "direct conflict-file access refuses symbolic links",
            ));
        }
        if write {
            let mut new_position = 0;
            // SAFETY: `file` owns a live synchronous file handle and the
            // output pointer is valid for the duration of this call.
            let repositioned = unsafe {
                SetFilePointerEx(file.as_raw_handle() as HANDLE, 0, &mut new_position, 0)
            };
            if repositioned == 0 {
                return Err(io::Error::last_os_error());
            }
            // Truncation is deliberately deferred until after the resolved bytes
            // are written; on Windows, pre-truncating this NtCreateFile handle
            // can leave the subsequent Tokio write with an empty regular file.
        }
        Ok(file)
    }

    fn open_relative(path: &RepoPath, write: bool) -> io::Result<File> {
        let mut root_options = OpenOptions::new();
        root_options
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        let mut parent = root_options.open(&path.root)?;
        let mut components = path.rel.components();
        let Some(last) = components.next_back() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty repository-relative path",
            ));
        };
        for component in components {
            let Component::Normal(part) = component else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "non-normal repository path component",
                ));
            };
            parent = open_child(&parent, part, false, true)?;
        }
        let Component::Normal(last) = last else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "non-normal repository path component",
            ));
        };
        open_child(&parent, last, write, false)
    }

    pub(super) fn open_read(path: &RepoPath) -> io::Result<File> {
        open_relative(path, false)
    }

    pub(super) fn open_write(path: &RepoPath) -> io::Result<File> {
        open_relative(path, true)
    }
}

/// Read the working-copy file as text, under the server's content `budget`.
///
/// The conflict parsers work on `&str`, so a file whose bytes are not valid UTF-8
/// is refused with a clear message rather than lossily decoded — the same
/// fail-closed stance [`ok_json`] takes on the way out. A missing file is
/// client-actionable too, so both land as `invalid_params`.
///
/// **The ceiling is applied twice, and never after the fact.** First against the
/// file's *size*, so an oversized file is refused before a byte of it is
/// buffered; then against the read itself ([`AsyncReadExt::take`]), so a file
/// that grows past the ceiling between the two — a race, or a deliberate one —
/// still cannot buffer more than the cap plus the one byte that proves it was
/// exceeded. Over-budget is an error, never a truncated file handed back as if
/// complete; [`OutputBudget::unlimited`] (`--max-output-bytes 0`) removes the
/// ceiling, exactly as it does for the subprocess-backed content tools.
pub(crate) async fn read_working_copy(
    p: &RepoPath,
    budget: OutputBudget,
) -> Result<String, ErrorData> {
    let io_refusal = |e: std::io::Error| {
        let rel = p.rel.display();
        ErrorData::invalid_params(
            match e.kind() {
                std::io::ErrorKind::NotFound => {
                    format!("no such file in the working copy: {rel}")
                }
                std::io::ErrorKind::InvalidData => not_utf8(&p.rel),
                _ => format!("cannot read {rel} from the working copy: {e}"),
            },
            None,
        )
    };
    let limit = budget.max_bytes();
    // Read from the already-open descriptor. Besides eliminating a metadata /
    // open race, the Unix implementation anchors every path component to the
    // repository directory descriptor and applies O_NOFOLLOW throughout.
    let file = secure_fs::open_read(p).map_err(io_refusal)?;
    let size = file.metadata().map_err(io_refusal)?.len();
    // `OutputBudget`'s ceilings fire strictly *past* the cap, so a file sitting
    // exactly on it is still read — matching the subprocess path.
    if let Some(limit) = limit
        && size > limit as u64
    {
        return Err(over_budget(&p.rel, size, limit));
    }
    let file = tokio::fs::File::from_std(file);
    // Right-size the buffer for an ordinary file (the size is already known and,
    // under a ceiling, already known to be within it), but don't reserve
    // gigabytes up front just because an unlimited budget said a file may be
    // huge — past this the `Vec` grows as it reads.
    const MAX_PREALLOC: usize = 8 * 1024 * 1024;
    let mut bytes = Vec::with_capacity(
        usize::try_from(size)
            .unwrap_or(usize::MAX)
            .min(MAX_PREALLOC),
    );
    // One byte past the cap: enough to detect an overrun, bounded either way.
    let ceiling = limit.map_or(u64::MAX, |l| l as u64 + 1);
    file.take(ceiling)
        .read_to_end(&mut bytes)
        .await
        .map_err(io_refusal)?;
    if let Some(limit) = limit
        && bytes.len() > limit
    {
        // The size check passed and the read still overran: the file grew
        // between the two. Refuse the same way — the point of `take` is that
        // this costs one byte over the cap, not the file's real size.
        return Err(ErrorData::internal_error(
            format!(
                "{} grew past this server's content output ceiling of {limit} bytes \
                 (--max-output-bytes) while it was being read; refusing the partial \
                 content rather than parsing a truncated file.",
                p.rel.display()
            ),
            None,
        ));
    }
    String::from_utf8(bytes).map_err(|_| ErrorData::invalid_params(not_utf8(&p.rel), None))
}

/// Write resolved content through a no-follow-safe descriptor.
pub(crate) async fn write_working_copy(p: &RepoPath, content: &[u8]) -> Result<(), std::io::Error> {
    let mut file = tokio::fs::File::from_std(secure_fs::open_write(p)?);
    file.write_all(content).await?;
    // Unix opened the descriptor with O_TRUNC; Windows defers truncation until
    // the write succeeds so a valid regular-file resolution is not reduced to
    // an empty file by the handle-relative open path. Set the exact final size
    // on both platforms to remove any old tail when the resolution is shorter.
    file.set_len(content.len() as u64).await
}

/// The refusal for a file the conflict model can't parse because it isn't text.
fn not_utf8(rel: &Path) -> String {
    format!(
        "{} is not valid UTF-8; the conflict model is text-only, so its markers cannot be \
         parsed (read the raw bytes another way)",
        rel.display()
    )
}

/// The refusal for a working-copy file past the content ceiling.
///
/// Mapped to `internal_error` for the same reason `repo_show_file`'s
/// `OutputTooLarge` is (`output::core_err` classifies only invalid-input and
/// unsupported as client-facing): the request itself was well-formed — it is the
/// operator's ceiling that stopped it — so the two content paths report an
/// exceeded budget identically rather than one of them blaming the caller's
/// params.
fn over_budget(rel: &Path, size: u64, limit: usize) -> ErrorData {
    debug_assert!(size > limit as u64, "only an over-budget size is refused");
    ErrorData::internal_error(
        format!(
            "{} is {size} bytes, which exceeds this server's content output ceiling of \
             {limit} bytes (--max-output-bytes): refusing to buffer it rather than returning \
             a truncated file. Raise the ceiling (or --max-output-bytes 0 to remove it) if a \
             file this large must be read.",
            rel.display()
        ),
        None,
    )
}

/// Map a `vcs-git`/`vcs-jj` conflict-model error into an MCP error. A malformed
/// conflict region (`Parse`) and a refused resolution (an `InvalidInput` spawn
/// error — "this region records no base", "Side(3) does not exist") are both
/// about the *caller's* file/argument, so they are client-facing invalid params;
/// anything else is internal.
fn conflict_err(e: VcsError) -> ErrorData {
    if vcs_cli_support::is_invalid_input(&e) || matches!(e.reason(), ErrorReason::Parse { .. }) {
        ErrorData::invalid_params(e.to_string(), None)
    } else {
        ErrorData::internal_error(e.to_string(), None)
    }
}

/// One conflict region on the wire.
///
/// `number`/`total` are **positional** — computed from the parsed region list, so
/// the "conflict N of M" counter is present identically on both backends (git's
/// grammar has no counter of its own). `region` is the backend's own parsed type,
/// serialized as-is: git 2/3-way marker sides and jj n-way diff/snapshot sections
/// are genuinely different shapes and are not flattened into one lossy union. On
/// jj the nested region additionally carries the file's *own* `number`/`total` as
/// jj wrote them, so both readings are available.
#[derive(serde::Serialize)]
pub(crate) struct WireRegion<R: serde::Serialize> {
    number: usize,
    total: usize,
    region: R,
}

/// [`repo_conflict_regions`](crate::VcsMcpServer::repo_conflict_regions)'s JSON
/// shape. `backend` tags which of the two region shapes `regions` carries.
#[derive(serde::Serialize)]
pub(crate) struct ConflictRegionsOut<'a, R: serde::Serialize> {
    backend: &'static str,
    path: &'a str,
    conflict_count: usize,
    regions: Vec<WireRegion<R>>,
}

impl<'a, R: serde::Serialize> ConflictRegionsOut<'a, R> {
    fn new(backend: &'static str, path: &'a str, regions: Vec<R>) -> Self {
        let total = regions.len();
        Self {
            backend,
            path,
            conflict_count: total,
            regions: regions
                .into_iter()
                .enumerate()
                .map(|(i, region)| WireRegion {
                    number: i + 1,
                    total,
                    region,
                })
                .collect(),
        }
    }
}

/// Parse `content` with the backend's conflict grammar and encode its regions.
///
/// A file with no conflict markers yields an empty `regions` list, **not** an
/// error — symmetric with `repo_conflicts`, which reports `[]` on a clean tree.
pub(crate) fn regions_json(
    kind: BackendKind,
    path: &str,
    content: &str,
) -> Result<CallToolResult, ErrorData> {
    match kind {
        BackendKind::Git => {
            let segments = vcs_git::conflict::parse_conflicts(content).map_err(conflict_err)?;
            let regions = segments
                .into_iter()
                .filter_map(|s| match s {
                    vcs_git::conflict::ConflictSegment::Conflict(region) => Some(*region),
                    vcs_git::conflict::ConflictSegment::Text(_) => None,
                })
                .collect();
            ok_json(&ConflictRegionsOut::new("git", path, regions))
        }
        BackendKind::Jj => {
            let segments = vcs_jj::conflict::parse_conflicts(content).map_err(conflict_err)?;
            let regions = segments
                .into_iter()
                .filter_map(|s| match s {
                    vcs_jj::conflict::JjConflictSegment::Conflict(region) => Some(*region),
                    vcs_jj::conflict::JjConflictSegment::Text(_) => None,
                })
                .collect();
            ok_json(&ConflictRegionsOut::new("jj", path, regions))
        }
        // `BackendKind` is `#[non_exhaustive]`: a backend added later would have
        // its own marker grammar, and guessing one of the existing two could
        // mis-parse a file. Fail loudly instead.
        other => Err(unknown_backend(other)),
    }
}

/// Refuse a backend this crate has no conflict grammar for.
fn unknown_backend(kind: BackendKind) -> ErrorData {
    ErrorData::internal_error(
        format!(
            "no conflict-marker grammar is implemented for the {} backend",
            kind.as_str()
        ),
        None,
    )
}

/// What a resolution did, for the tool's JSON result.
pub(crate) struct Resolved {
    /// The whole file's content with every region replaced by the chosen side.
    pub(crate) content: String,
    /// How many conflict regions were replaced.
    pub(crate) conflicts_resolved: usize,
}

/// Parse `content`, map `side` onto the backend's resolution domain, and render
/// the resolved file.
///
/// Every refusal here happens **before** the caller writes anything: a side the
/// backend doesn't have (`side` on git), a `theirs` that would be ambiguous
/// (a jj conflict with more than two sides), an `index` supplied without
/// `side = "side"`, a file whose markers don't parse, or a resolution the region
/// can't satisfy (no recorded base). Never a silent no-op, never a panic.
pub(crate) fn resolve_content(
    kind: BackendKind,
    content: &str,
    side: ConflictSideArg,
    index: Option<usize>,
) -> Result<Resolved, ErrorData> {
    let refuse = |why: String| ErrorData::invalid_params(why, None);
    if index.is_some() && side != ConflictSideArg::Side {
        return Err(refuse(format!(
            "`index` applies only to side=\"side\"; drop it (side={side:?} already names \
             the side exactly)"
        )));
    }
    match kind {
        BackendKind::Git => {
            use vcs_git::conflict::{ConflictSegment, ResolutionSide};
            let chosen = match side {
                ConflictSideArg::Ours => ResolutionSide::Ours,
                ConflictSideArg::Base => ResolutionSide::Base,
                ConflictSideArg::Theirs => ResolutionSide::Theirs,
                // git's three sides are named, so an index would be a second,
                // redundant spelling — and 'the 3rd side' has no git meaning.
                ConflictSideArg::Side => {
                    return Err(refuse(
                        "side=\"side\" is jj-only (jj conflicts can have any number of \
                         sides); on git use \"ours\", \"base\", or \"theirs\""
                            .to_string(),
                    ));
                }
            };
            let segments = vcs_git::conflict::parse_conflicts(content).map_err(conflict_err)?;
            let conflicts_resolved = segments
                .iter()
                .filter(|s| matches!(s, ConflictSegment::Conflict(_)))
                .count();
            Ok(Resolved {
                content: vcs_git::conflict::resolve(&segments, chosen).map_err(conflict_err)?,
                conflicts_resolved,
            })
        }
        BackendKind::Jj => {
            use vcs_jj::conflict::{JjConflictSegment, JjResolution};
            let segments = vcs_jj::conflict::parse_conflicts(content).map_err(conflict_err)?;
            let regions: Vec<_> = segments
                .iter()
                .filter_map(|s| match s {
                    JjConflictSegment::Conflict(region) => Some(&**region),
                    JjConflictSegment::Text(_) => None,
                })
                .collect();
            let chosen = match side {
                // jj records sides in file order; the first is the "ours" analogue.
                ConflictSideArg::Ours => JjResolution::Side(0),
                ConflictSideArg::Base => JjResolution::Base,
                ConflictSideArg::Theirs => {
                    // "Theirs" is only unambiguous for a 2-sided conflict. For an
                    // n-way one, `Side(1)` would be the *middle* side — silently
                    // not what "theirs" means — so refuse and make the caller say
                    // which side it wants.
                    if let Some(region) = regions.iter().find(|r| r.sides().len() != 2) {
                        return Err(refuse(format!(
                            "conflict {} of this file has {} sides, so \"theirs\" is \
                             ambiguous on jj; use side=\"side\" with an explicit 0-based \
                             `index` (repo_conflict_regions lists the sides in order)",
                            region.number,
                            region.sides().len()
                        )));
                    }
                    JjResolution::Side(1)
                }
                ConflictSideArg::Side => JjResolution::Side(index.ok_or_else(|| {
                    refuse(
                        "side=\"side\" needs a 0-based `index` naming which side to keep"
                            .to_string(),
                    )
                })?),
            };
            Ok(Resolved {
                content: vcs_jj::conflict::resolve(&segments, chosen).map_err(conflict_err)?,
                conflicts_resolved: regions.len(),
            })
        }
        // See `regions_json`: never guess another backend's grammar.
        other => Err(unknown_backend(other)),
    }
}
