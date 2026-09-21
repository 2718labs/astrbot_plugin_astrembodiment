//! Narrow platform filesystem primitives whose OS APIs require FFI.

use std::fs::File;
use std::io;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity(u64, u64);

/// An exact, handle-anchored package of real regular files.
///
/// Unix opens the directory with `O_DIRECTORY | O_NOFOLLOW`, opens an
/// independent `.` descriptor relative to that anchor for each `fdopendir`
/// enumeration, and opens members with `openat` and `O_NOFOLLOW`. Windows
/// holds a non-reparse directory handle with write and
/// delete sharing denied, performs bounded exact enumeration, holds both file
/// handles with the same sharing fence, and verifies directory/member file
/// identities before and after use. No package member is subsequently reopened
/// by an unchecked path.
pub struct ExactRegularFilePackage {
    directory: File,
    directory_identity: FileIdentity,
    directory_path: std::path::PathBuf,
    expected_names: Vec<String>,
    members: Vec<(String, File, FileIdentity)>,
}

impl ExactRegularFilePackage {
    /// Open exactly `expected_names` from `directory`. Unknown, missing,
    /// duplicate, non-UTF-8, linked/reparse, directory, device, and other
    /// non-regular members are rejected.
    pub fn open(directory: &Path, expected_names: &[&str]) -> io::Result<Self> {
        validate_expected_member_names(expected_names)?;
        let directory_handle = open_directory_no_follow(directory)?;
        let directory_identity = file_identity(&directory_handle)?;
        let mut package = Self {
            directory: directory_handle,
            directory_identity,
            directory_path: directory.to_path_buf(),
            expected_names: expected_names
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            members: Vec::with_capacity(expected_names.len()),
        };
        package.require_exact_members()?;
        for name in &package.expected_names {
            let file =
                open_regular_member_no_follow(&package.directory, &package.directory_path, name)?;
            let identity = file_identity(&file)?;
            package.members.push((name.clone(), file, identity));
        }
        package.revalidate()?;
        Ok(package)
    }

    /// Borrow a member already opened relative to the held directory anchor.
    pub fn member_mut(&mut self, name: &str) -> io::Result<&mut File> {
        self.members
            .iter_mut()
            .find_map(|(member_name, file, _)| (member_name == name).then_some(file))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "package member is not open"))
    }

    /// Recheck exact membership and the identities of the held directory and
    /// files. On Windows this is also the final identity-stability fence around
    /// path-based enumeration; Unix performs every lookup relative to the fd.
    pub fn revalidate(&mut self) -> io::Result<()> {
        validate_open_directory(&self.directory)?;
        if file_identity(&self.directory)? != self.directory_identity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "anchored directory identity changed",
            ));
        }
        verify_directory_path_identity(&self.directory_path, self.directory_identity)?;
        self.require_exact_members()?;
        for (name, held, identity) in &self.members {
            validate_open_regular_file(held)?;
            if file_identity(held)? != *identity {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "held package member identity changed",
                ));
            }
            let reopened =
                open_regular_member_no_follow(&self.directory, &self.directory_path, name)?;
            if file_identity(&reopened)? != *identity {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "package member was replaced",
                ));
            }
        }
        Ok(())
    }

    /// Make the anchored directory entry durable where the platform exposes a
    /// directory flush. Windows publication uses `MOVEFILE_WRITE_THROUGH`;
    /// directory `FlushFileBuffers` may return ACCESS_DENIED, in which case the
    /// held-handle identity/readback fence remains mandatory and is repeated.
    pub fn sync_directory(&mut self) -> io::Result<()> {
        self.revalidate()?;
        match self.directory.sync_all() {
            Ok(()) => {}
            #[cfg(windows)]
            Err(error) if error.raw_os_error() == Some(5) => {}
            Err(error) => return Err(error),
        }
        self.revalidate()
    }

    fn require_exact_members(&self) -> io::Result<()> {
        let observed = enumerate_directory_members(
            &self.directory,
            &self.directory_path,
            self.expected_names.len().saturating_add(1),
        )?;
        let mut expected = self.expected_names.clone();
        expected.sort();
        let mut observed = observed;
        observed.sort();
        if observed != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory does not contain the exact package member set",
            ));
        }
        Ok(())
    }
}

fn validate_expected_member_names(names: &[&str]) -> io::Result<()> {
    if names.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an exact package must name at least one member",
        ));
    }
    let mut sorted = names.to_vec();
    sorted.sort_unstable();
    for (index, name) in sorted.iter().enumerate() {
        if name.is_empty()
            || *name == "."
            || *name == ".."
            || !name.is_ascii()
            || name.contains('/')
            || name.contains('\\')
            || name.as_bytes().contains(&0)
            || (index > 0 && sorted[index - 1] == *name)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid or duplicate exact package member name",
            ));
        }
    }
    Ok(())
}

/// Sync a real directory through the same no-follow handle that was checked.
pub fn sync_directory_no_follow(path: &Path) -> io::Result<()> {
    let directory = open_directory_no_follow(path)?;
    validate_open_directory(&directory)?;
    match directory.sync_all() {
        Ok(()) => Ok(()),
        #[cfg(windows)]
        Err(error) if error.raw_os_error() == Some(5) => validate_open_directory(&directory),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn open_directory_no_follow(path: &Path) -> io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory path contains NUL"))?;
    // SAFETY: path is a live NUL-terminated C string. The returned descriptor
    // is uniquely transferred into File on success.
    let descriptor = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: descriptor is valid and uniquely owned after successful open.
    let file = unsafe { File::from_raw_fd(descriptor) };
    validate_open_directory(&file)?;
    Ok(file)
}

#[cfg(windows)]
fn open_directory_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
    };

    let directory = std::fs::OpenOptions::new()
        .read(true)
        // Deny new write/delete handles while validation is in flight. Existing
        // hostile handles cannot be revoked, so identity is checked again after
        // all reads and any visible replacement fails closed.
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    validate_open_directory(&directory)?;
    Ok(directory)
}

#[cfg(not(any(unix, windows)))]
fn open_directory_no_follow(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "handle-anchored exact packages are unsupported on this platform",
    ))
}

#[cfg(unix)]
fn open_regular_member_no_follow(
    directory: &File,
    _directory_path: &Path,
    name: &str,
) -> io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    let name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "member name contains NUL"))?;
    // O_NONBLOCK prevents a raced FIFO/device from hanging before fstat; it has
    // no effect on ordinary regular-file reads.
    // SAFETY: directory fd and name are live; successful fd ownership is moved
    // into File exactly once.
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: descriptor is valid and uniquely owned after successful openat.
    let file = unsafe { File::from_raw_fd(descriptor) };
    validate_open_regular_file(&file)?;
    Ok(file)
}

#[cfg(windows)]
fn open_regular_member_no_follow(
    _directory: &File,
    directory_path: &Path,
    name: &str,
) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};

    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(directory_path.join(name))?;
    validate_open_regular_file(&file)?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_regular_member_no_follow(
    _directory: &File,
    _directory_path: &Path,
    _name: &str,
) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "handle-relative member opens are unsupported on this platform",
    ))
}

#[cfg(unix)]
fn enumerate_directory_members(
    directory: &File,
    _directory_path: &Path,
    maximum: usize,
) -> io::Result<Vec<String>> {
    use std::ffi::CStr;
    use std::os::fd::AsRawFd;

    // fdopendir owns and closes its descriptor. A dup/fcntl duplicate would
    // share this directory's open-file-description and therefore its readdir
    // offset. Open `.` relative to the held anchor instead so every validation
    // starts from an independent open-file-description at offset zero.
    // SAFETY: directory is live and DOT is a NUL-terminated relative path.
    const DOT: &[u8] = b".\0";
    let enumeration = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            DOT.as_ptr().cast(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if enumeration < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: enumeration is a valid, independently opened directory
    // descriptor and transfers to DIR on success.
    let stream = unsafe { libc::fdopendir(enumeration) };
    if stream.is_null() {
        // SAFETY: fdopendir did not take ownership on failure.
        unsafe { libc::close(enumeration) };
        return Err(io::Error::last_os_error());
    }
    struct DirectoryStream(*mut libc::DIR);
    impl Drop for DirectoryStream {
        fn drop(&mut self) {
            // SAFETY: this wrapper uniquely owns the DIR pointer.
            unsafe { libc::closedir(self.0) };
        }
    }
    let stream = DirectoryStream(stream);
    let mut names = Vec::new();
    loop {
        errno::set_errno(errno::Errno(0));
        // SAFETY: stream remains valid for this call and returned dirent is read
        // before the next readdir invocation.
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let error = errno::errno().0;
            if error == 0 {
                break;
            }
            return Err(io::Error::from_raw_os_error(error));
        }
        // SAFETY: POSIX d_name is NUL-terminated within the returned dirent.
        let raw = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        let name = std::str::from_utf8(raw).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "package member name is not UTF-8",
            )
        })?;
        names.push(name.to_owned());
        if names.len() > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "package contains too many members",
            ));
        }
    }
    Ok(names)
}

#[cfg(windows)]
fn enumerate_directory_members(
    directory: &File,
    directory_path: &Path,
    maximum: usize,
) -> io::Result<Vec<String>> {
    let identity = file_identity(directory)?;
    verify_directory_path_identity(directory_path, identity)?;
    let mut names = Vec::new();
    for entry in std::fs::read_dir(directory_path)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "package member name is not UTF-8",
            )
        })?;
        names.push(name);
        if names.len() > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "package contains too many members",
            ));
        }
    }
    verify_directory_path_identity(directory_path, identity)?;
    Ok(names)
}

#[cfg(not(any(unix, windows)))]
fn enumerate_directory_members(
    _directory: &File,
    _directory_path: &Path,
    _maximum: usize,
) -> io::Result<Vec<String>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "handle-anchored enumeration is unsupported on this platform",
    ))
}

fn validate_open_directory(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "opened handle is not a directory",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "opened directory is a reparse point",
            ));
        }
    }
    Ok(())
}

fn validate_open_regular_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "opened package member is not a regular file",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "opened package member is a reparse point",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok(FileIdentity(metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    // SAFETY: zero is a valid initial representation for the output-only C
    // structure, and the raw handle remains owned by `file` for the call.
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FileIdentity(
        u64::from(information.dwVolumeSerialNumber),
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    ))
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_file: &File) -> io::Result<FileIdentity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stable file identity is unsupported on this platform",
    ))
}

#[cfg(unix)]
fn verify_directory_path_identity(_path: &Path, _expected: FileIdentity) -> io::Result<()> {
    // All Unix enumeration and child opens are relative to the held descriptor;
    // path replacement cannot redirect package authority.
    Ok(())
}

#[cfg(windows)]
fn verify_directory_path_identity(path: &Path, expected: FileIdentity) -> io::Result<()> {
    let reopened = open_directory_no_follow(path)?;
    if file_identity(&reopened)? != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "directory path no longer names the anchored directory",
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn verify_directory_path_identity(_path: &Path, _expected: FileIdentity) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "directory identity verification is unsupported on this platform",
    ))
}

/// Open one real regular file without following a final-component link or
/// reparse point. Callers can then hash/read the returned handle without a
/// second path traversal reopening authority outside the validated package.
pub fn open_regular_file_no_follow(path: &Path) -> io::Result<std::fs::File> {
    let path_metadata = std::fs::symlink_metadata(path)?;
    if !path_metadata.file_type().is_file() || path_metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is not a real regular file",
        ));
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        if path_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path is a reparse point",
            ));
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let opened_metadata = file.metadata()?;
        if !opened_metadata.file_type().is_file()
            || opened_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "opened handle is not a real regular file",
            ));
        }
        Ok(file)
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?;
        if !file.metadata()?.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "opened handle is not a regular file",
            ));
        }
        return Ok(file);
    }

    #[cfg(not(any(windows, unix)))]
    {
        let file = std::fs::File::open(path)?;
        if !file.metadata()?.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "opened handle is not a regular file",
            ));
        }
        Ok(file)
    }
}

/// Create a directory tree while making each newly created directory entry
/// durable before returning. On Unix, every new child is followed by an fsync
/// of its parent; this orders an entire newly-created ancestor chain. Windows
/// retains the existing `create_dir_all` behavior because backup publication
/// itself uses `MOVEFILE_WRITE_THROUGH` and ordinary directory handles cannot
/// be flushed there.
pub fn create_dir_all_durable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        create_dir_all_with_parent_sync(path, sync_directory)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

#[cfg(any(unix, test))]
fn create_dir_all_with_parent_sync(
    path: &Path,
    mut sync_parent: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let target = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut missing = Vec::new();
    let mut cursor = target.as_path();
    let existing_anchor = loop {
        match std::fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        "durable directory-tree ancestor is not a real directory",
                    ));
                }
                break cursor.to_path_buf();
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(cursor.to_path_buf());
                cursor = cursor.parent().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "durable directory tree has no existing ancestor",
                    )
                })?;
            }
            Err(error) => return Err(error),
        }
    };

    // A previous process may have stopped after mkdir but before syncing the
    // containing directory. Re-sync the deepest existing anchor's entry on
    // every invocation; this makes recovery independent of volatile memory.
    if let Some(parent) = existing_anchor.parent() {
        sync_parent(parent)?;
    }

    for directory in missing.into_iter().rev() {
        let parent = directory.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "durable directory entry has no parent",
            )
        })?;
        match std::fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(&directory)?;
                if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "durable directory entry raced with a non-directory",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
        // Also sync after an AlreadyExists race: another creator may not have
        // made the parent entry durable yet, and returning early would weaken
        // this call's durability contract.
        sync_parent(parent)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

pub fn available_space(path: &Path) -> io::Result<u64> {
    fs2::available_space(path)
}

/// Atomically rename one same-volume directory and request durable publication.
/// The destination must not already exist.
#[cfg(windows)]
pub fn durable_rename_directory(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    if destination.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "durable rename destination already exists",
        ));
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both buffers are owned, NUL-terminated UTF-16 paths and remain
    // alive for the call. No replacement flag is passed, so an existing final
    // package cannot be overwritten.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub fn durable_rename_directory(source: &Path, destination: &Path) -> io::Result<()> {
    rename_directory_no_replace(source, destination)
}

#[cfg(unix)]
fn rename_directory_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "durable rename source contains an interior NUL",
        )
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "durable rename destination contains an interior NUL",
        )
    })?;
    rename_directory_no_replace_c(&source, &destination)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_directory_no_replace_c(
    source: &std::ffi::CStr,
    destination: &std::ffi::CStr,
) -> io::Result<()> {
    // Use the syscall directly so this remains available when the build host's
    // libc predates its renameat2 wrapper. RENAME_NOREPLACE makes destination
    // admission and the directory-tree move one indivisible kernel operation.
    // SAFETY: both C strings are NUL-terminated and live for the syscall.
    let renamed = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if renamed == 0 {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        let code = error.raw_os_error();
        if code == Some(libc::ENOSYS)
            || code == Some(libc::EINVAL)
            || code == Some(libc::EOPNOTSUPP)
        {
            unsupported_no_replace_directory_rename(error)
        } else {
            Err(error)
        }
    }
}

#[cfg(target_vendor = "apple")]
fn rename_directory_no_replace_c(
    source: &std::ffi::CStr,
    destination: &std::ffi::CStr,
) -> io::Result<()> {
    // SAFETY: both C strings are NUL-terminated and live for the call.
    let renamed = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if renamed == 0 {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        let code = error.raw_os_error();
        if code == Some(libc::ENOSYS) || code == Some(libc::EINVAL) || code == Some(libc::ENOTSUP) {
            unsupported_no_replace_directory_rename(error)
        } else {
            Err(error)
        }
    }
}

#[cfg(target_os = "redox")]
fn rename_directory_no_replace_c(
    source: &std::ffi::CStr,
    destination: &std::ffi::CStr,
) -> io::Result<()> {
    // SAFETY: both C strings are NUL-terminated and live for the call.
    let renamed = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if renamed == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "redox"
    ))
))]
fn rename_directory_no_replace_c(
    _source: &std::ffi::CStr,
    _destination: &std::ffi::CStr,
) -> io::Result<()> {
    unsupported_no_replace_directory_rename(io::Error::new(
        io::ErrorKind::Unsupported,
        "this Unix target exposes no atomic no-replace directory rename primitive",
    ))
}

#[cfg(unix)]
fn unsupported_no_replace_directory_rename(source: io::Error) -> io::Result<()> {
    // There is no portable POSIX emulation for a no-replace directory rename:
    // link/unlink cannot operate on directories and exists()+rename has a
    // clobber race. Failing closed preserves both trees instead of silently
    // weakening the public contract.
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("atomic no-replace directory rename is unavailable: {source}"),
    ))
}

#[cfg(not(any(windows, unix)))]
pub fn durable_rename_directory(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace directory rename is unavailable on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchored_package_opens_only_the_exact_regular_members() {
        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-package-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("authority.sqlite"), b"authority").unwrap();
        std::fs::write(root.join("manifest"), b"manifest").unwrap();

        let mut package =
            ExactRegularFilePackage::open(&root, &["authority.sqlite", "manifest"]).unwrap();
        let mut authority = Vec::new();
        std::io::Read::read_to_end(
            package.member_mut("authority.sqlite").unwrap(),
            &mut authority,
        )
        .unwrap();
        assert_eq!(authority, b"authority");
        package.revalidate().unwrap();
        drop(package);

        std::fs::write(root.join("extra"), b"not-attested").unwrap();
        assert!(ExactRegularFilePackage::open(&root, &["authority.sqlite", "manifest"],).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_exact_package_restarts_enumeration_for_every_revalidation() {
        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-unix-repeat-enumeration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("authority.sqlite"), b"authority").unwrap();
        std::fs::write(root.join("manifest"), b"manifest").unwrap();

        let mut package =
            ExactRegularFilePackage::open(&root, &["authority.sqlite", "manifest"]).unwrap();
        for _ in 0..8 {
            package.revalidate().unwrap();
        }

        drop(package);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_enumeration_abstraction_never_inherits_a_prior_offset() {
        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-unix-independent-offset-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("authority.sqlite"), b"authority").unwrap();
        std::fs::write(root.join("manifest"), b"manifest").unwrap();
        let directory = open_directory_no_follow(&root).unwrap();

        for _ in 0..8 {
            let mut names = enumerate_directory_members(&directory, &root, 3).unwrap();
            names.sort();
            assert_eq!(names, ["authority.sqlite", "manifest"]);
        }

        drop(directory);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_package_lookup_stays_on_held_directory_after_path_replacement() {
        use std::io::{Read, Seek, SeekFrom};

        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-unix-anchor-race-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let displaced = root.with_extension("held");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("authority.sqlite"), b"held-authority").unwrap();
        std::fs::write(root.join("manifest"), b"held-manifest").unwrap();
        let mut package =
            ExactRegularFilePackage::open(&root, &["authority.sqlite", "manifest"]).unwrap();

        std::fs::rename(&root, &displaced).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("authority.sqlite"), b"replacement").unwrap();
        std::fs::write(root.join("manifest"), b"replacement").unwrap();

        package.revalidate().unwrap();
        let authority = package.member_mut("authority.sqlite").unwrap();
        authority.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        authority.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"held-authority");
        drop(package);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(displaced).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_package_anchor_denies_or_detects_directory_replacement() {
        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-windows-anchor-race-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let displaced = root.with_extension("held");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("authority.sqlite"), b"held-authority").unwrap();
        std::fs::write(root.join("manifest"), b"held-manifest").unwrap();
        let mut package =
            ExactRegularFilePackage::open(&root, &["authority.sqlite", "manifest"]).unwrap();

        match std::fs::rename(&root, &displaced) {
            Ok(()) => {
                std::fs::create_dir_all(&root).unwrap();
                std::fs::write(root.join("authority.sqlite"), b"replacement").unwrap();
                std::fs::write(root.join("manifest"), b"replacement").unwrap();
                assert!(package.revalidate().is_err());
            }
            Err(error) => {
                assert!(
                    error.kind() == io::ErrorKind::PermissionDenied
                        || matches!(error.raw_os_error(), Some(5 | 32)),
                    "unexpected anchored-directory rename error: {error}"
                );
                package.revalidate().unwrap();
            }
        }
        drop(package);
        if root.exists() {
            std::fs::remove_dir_all(root).unwrap();
        }
        if displaced.exists() {
            std::fs::remove_dir_all(displaced).unwrap();
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn anchored_package_rejects_a_linked_directory_itself() {
        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-directory-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let real = root.join("real");
        let link = root.join("link");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("authority.sqlite"), b"authority").unwrap();
        std::fs::write(real.join("manifest"), b"manifest").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        if let Err(error) = std::os::windows::fs::symlink_dir(&real, &link) {
            if error.kind() == io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(1314)
            {
                std::fs::remove_dir_all(root).unwrap();
                return;
            }
            panic!("directory symlink: {error}");
        }

        assert!(ExactRegularFilePackage::open(&link, &["authority.sqlite", "manifest"]).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_follow_open_accepts_regular_files() {
        let path = std::env::temp_dir().join(format!(
            "ae-platform-fs-regular-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"authority").unwrap();
        let file = open_regular_file_no_follow(&path).unwrap();
        assert!(file.metadata().unwrap().file_type().is_file());
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn no_follow_open_rejects_file_symlinks() {
        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-no-follow-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("target");
        let link = root.join("link");
        std::fs::write(&target, b"authority").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        if let Err(error) = std::os::windows::fs::symlink_file(&target, &link) {
            if error.kind() == io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(1314)
            {
                std::fs::remove_dir_all(root).unwrap();
                return;
            }
            panic!("file symlink: {error}");
        }

        assert!(open_regular_file_no_follow(&link).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn durable_directory_tree_creation_syncs_each_new_entry_parent_in_order() {
        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-create-tree-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("first");
        let leaf = first.join("leaf");
        let mut synced = Vec::new();

        create_dir_all_with_parent_sync(&leaf, |parent| {
            synced.push(parent.to_path_buf());
            Ok(())
        })
        .unwrap();

        assert_eq!(
            synced,
            vec![root.parent().unwrap().to_path_buf(), root.clone(), first]
        );
        assert!(leaf.is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn durable_directory_tree_creation_propagates_parent_sync_failure() {
        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-create-tree-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let leaf = root.join("backup-root");

        let mut sync_calls = 0_u8;
        let error = create_dir_all_with_parent_sync(&leaf, |_parent| {
            sync_calls += 1;
            if sync_calls == 1 {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    "deterministic parent sync failure",
                ))
            }
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(leaf.is_dir());
        let mut recovery_syncs = Vec::new();
        create_dir_all_with_parent_sync(&leaf, |parent| {
            recovery_syncs.push(parent.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert_eq!(recovery_syncs, vec![root.clone()]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn durable_directory_rename_never_replaces_an_existing_final() {
        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("stage")).unwrap();
        durable_rename_directory(&root.join("stage"), &root.join("final")).unwrap();
        assert!(root.join("final").is_dir());
        std::fs::create_dir_all(root.join("stage-2")).unwrap();
        assert!(durable_rename_directory(&root.join("stage-2"), &root.join("final")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_atomic_rename_preserves_a_competing_empty_destination() {
        use std::os::unix::fs::MetadataExt;

        let root = std::env::temp_dir().join(format!(
            "ae-platform-fs-unix-race-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let source = root.join("stage");
        let destination = root.join("final");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(source.join("source-marker"), b"source").unwrap();
        let destination_inode = std::fs::metadata(&destination).unwrap().ino();

        let error = rename_directory_no_replace(&source, &destination).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(source.join("source-marker").is_file());
        assert_eq!(
            std::fs::metadata(&destination).unwrap().ino(),
            destination_inode
        );
        assert!(std::fs::read_dir(&destination).unwrap().next().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
