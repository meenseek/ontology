//! Lazy PostgreSQL-backed Core input. A session owns its database and filesystem locks.
use crate::{
    context::{ContextScope, MAX_FILE_BYTES},
    domain::Error,
    store::{Store, digest},
};
use context_core::harness::{
    ContextSource, HarnessError, HarnessResult, SourceMetadata, SourcePathKind,
    SourceStoreIdentity, SourceVersion, StoredSourceState,
};
use sqlx::{Connection, PgConnection, Row};
use std::{
    ffi::{CStr, CString, OsString},
    fs::{File, Metadata},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::runtime::Handle;
const CONTEXT_GATE: i64 = 478310003;
const MAX_CACHED_BODIES: usize = 256;
const MARKER: &str = ".context-store";
pub(crate) fn source_error(error: Error) -> HarnessError {
    HarnessError::InvalidRepository(error.to_string())
}
fn io_error(path: &Path, error: std::io::Error) -> HarnessError {
    HarnessError::FileRead {
        path: path.to_owned(),
        message: error.to_string(),
    }
}
fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 255
        && !value
            .chars()
            .any(|c| c.is_control() || c == '\\' || c == '/')
}
fn parts(path: &Path) -> HarnessResult<Vec<String>> {
    let text = path.to_str().ok_or_else(|| source_error(Error::Invalid))?;
    if text.len() > 4096 || text.ends_with('/') || text.contains("//") {
        return Err(source_error(Error::Invalid));
    }
    path.components()
        .map(|c| match c {
            Component::Normal(s) => s
                .to_str()
                .filter(|s| safe_component(s))
                .map(str::to_owned)
                .ok_or_else(|| source_error(Error::Invalid)),
            _ => Err(source_error(Error::Invalid)),
        })
        .collect()
}
fn owned(meta: &Metadata, directory: bool) -> bool {
    // SAFETY: geteuid has no preconditions.
    meta.uid() == unsafe { libc::geteuid() }
        && if directory {
            meta.is_dir() && meta.mode() & 0o777 == 0o700
        } else {
            meta.is_file() && meta.nlink() == 1 && meta.mode() & 0o777 == 0o600
        }
}
fn open_at(parent: &File, name: &str, flags: i32) -> HarnessResult<File> {
    let name = CString::new(name).map_err(|_| source_error(Error::Invalid))?;
    // SAFETY: parent and NUL-terminated name remain alive; successful fd is uniquely owned.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io_error(
            Path::new(name.to_str().unwrap_or("")),
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: openat returned a fresh descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn make_dir(parent: &File, name: &str) -> HarnessResult<File> {
    let text = CString::new(name).map_err(|_| source_error(Error::Invalid))?;
    // SAFETY: parent is valid and text is NUL terminated.
    let code = unsafe { libc::mkdirat(parent.as_raw_fd(), text.as_ptr(), 0o700) };
    if code < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
        return Err(io_error(Path::new(name), std::io::Error::last_os_error()));
    }
    let dir = open_at(parent, name, libc::O_RDONLY | libc::O_DIRECTORY)?;
    if !owned(
        &dir.metadata().map_err(|e| io_error(Path::new(name), e))?,
        true,
    ) {
        return Err(HarnessError::InvalidRepository(
            "Unsafe context view ownership, permissions, or link".into(),
        ));
    }
    Ok(dir)
}
fn directory_names(directory: &File, budget: &mut usize) -> HarnessResult<Vec<String>> {
    // SAFETY: dup creates an independent descriptor; fdopendir owns it after success.
    let fd = unsafe { libc::dup(directory.as_raw_fd()) };
    if fd < 0 {
        return Err(io_error(
            Path::new("context view"),
            std::io::Error::last_os_error(),
        ));
    }
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        unsafe { libc::close(fd) };
        return Err(io_error(
            Path::new("context view"),
            std::io::Error::last_os_error(),
        ));
    }
    struct DirectoryStream(*mut libc::DIR);
    impl Drop for DirectoryStream {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    let stream = DirectoryStream(stream);
    let mut names = Vec::new();
    loop {
        // SAFETY: the live directory stream owns its descriptor; the name is read before the next call.
        // SAFETY: each platform exposes its thread-local errno pointer.
        #[cfg(target_os = "macos")]
        let errno = unsafe { libc::__error() };
        #[cfg(not(target_os = "macos"))]
        let errno = unsafe { libc::__errno_location() };
        unsafe { *errno = 0 };
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            if unsafe { *errno } != 0 {
                return Err(io_error(
                    Path::new("view directory"),
                    std::io::Error::last_os_error(),
                ));
            }
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
            .to_str()
            .map_err(|_| source_error(Error::Invalid))?;
        if matches!(name, "." | "..") {
            continue;
        }
        if !safe_component(name) || *budget == 0 {
            return Err(source_error(Error::Limit));
        }
        *budget -= 1;
        names.push(name.to_owned());
    }
    names.sort();
    Ok(names)
}
fn remove_projection(
    parent: &File,
    name: &str,
    budget: &mut usize,
    depth: usize,
    preserve_directories: bool,
) -> HarnessResult<()> {
    if depth > 64 {
        return Err(source_error(Error::Limit));
    }
    let file = open_at(parent, name, libc::O_RDONLY)?;
    let metadata = file.metadata().map_err(|e| io_error(Path::new(name), e))?;
    if !owned(&metadata, metadata.is_dir()) {
        return Err(source_error(Error::Forbidden));
    }
    let flags = if metadata.is_dir() {
        for child in directory_names(&file, budget)? {
            remove_projection(&file, &child, budget, depth + 1, preserve_directories)?;
        }
        if preserve_directories {
            // Missing in the store does not make a staging parent disposable: Core
            // may already have frozen this directory's identity for a Create target.
            return Ok(());
        }
        libc::AT_REMOVEDIR
    } else {
        0
    };
    let name = CString::new(name).map_err(|_| source_error(Error::Invalid))?;
    // SAFETY: the descriptor and checked name are valid. The DB proved this subtree absent;
    // the session gate and view flock remain held through this descriptor-relative cleanup.
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) } < 0 {
        return Err(io_error(
            Path::new("projection"),
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}
fn audit_view(directory: &File, budget: &mut usize, depth: usize) -> HarnessResult<()> {
    if depth > 64 {
        return Err(source_error(Error::Limit));
    }
    for name in directory_names(directory, budget)? {
        let child = open_at(directory, &name, libc::O_RDONLY)?;
        let metadata = child
            .metadata()
            .map_err(|e| io_error(Path::new("view entry"), e))?;
        if !owned(&metadata, metadata.is_dir()) {
            return Err(source_error(Error::Forbidden));
        }
        if metadata.is_dir() {
            audit_view(&child, budget, depth + 1)?;
        }
    }
    Ok(())
}

pub(crate) struct View {
    root: PathBuf,
    directory: File,
    _lock: File,
}
impl View {
    fn open(root: &Path, store_id: &str) -> HarnessResult<Self> {
        if !root.is_absolute()
            || root
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(source_error(Error::Invalid));
        }
        let mut directory = File::open("/").map_err(|e| io_error(root, e))?;
        for component in root.components() {
            if let Component::Normal(name) = component {
                directory = open_at(
                    &directory,
                    name.to_str()
                        .filter(|n| safe_component(n))
                        .ok_or_else(|| source_error(Error::Invalid))?,
                    libc::O_RDONLY | libc::O_DIRECTORY,
                )?;
            }
        }
        if root.canonicalize().map_err(|e| io_error(root, e))? != root
            || !owned(&directory.metadata().map_err(|e| io_error(root, e))?, true)
        {
            return Err(HarnessError::InvalidRepository(
                "Unsafe context view ownership, permissions, or link".into(),
            ));
        }
        let lock = open_at(&directory, ".context-lock", libc::O_RDWR | libc::O_CREAT)?;
        if !owned(&lock.metadata().map_err(|e| io_error(root, e))?, false) {
            return Err(HarnessError::InvalidRepository(
                "Unsafe context view ownership, permissions, or link".into(),
            ));
        }
        // SAFETY: flock receives an owned open descriptor; closing it releases the lock.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 {
            return Err(source_error(Error::Conflict));
        }
        match open_at(&directory, MARKER, libc::O_RDONLY) {
            Ok(mut marker) => {
                let meta = marker.metadata().map_err(|e| io_error(root, e))?;
                if !owned(&meta, false) || meta.len() > 64 {
                    return Err(HarnessError::InvalidRepository(
                        "Unsafe context view ownership, permissions, or link".into(),
                    ));
                }
                let mut id = String::new();
                marker
                    .read_to_string(&mut id)
                    .map_err(|e| io_error(root, e))?;
                if id != store_id {
                    return Err(source_error(Error::Conflict));
                }
            }
            Err(_) => {
                for entry in std::fs::read_dir(root).map_err(|e| io_error(root, e))? {
                    if entry.map_err(|e| io_error(root, e))?.file_name() != ".context-lock" {
                        return Err(HarnessError::InvalidRepository(
                            "Unsafe context view ownership, permissions, or link".into(),
                        ));
                    }
                }
                let mut marker = open_at(
                    &directory,
                    MARKER,
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                )?;
                marker
                    .write_all(store_id.as_bytes())
                    .map_err(|e| io_error(root, e))?;
                marker.sync_all().map_err(|e| io_error(root, e))?;
            }
        }
        audit_view(&directory, &mut 20_000, 0)?;
        Ok(Self {
            root: root.to_owned(),
            directory,
            _lock: lock,
        })
    }
    fn parent(&self, path: &[String]) -> HarnessResult<(File, String)> {
        let (last, parents) = path
            .split_last()
            .ok_or_else(|| source_error(Error::Invalid))?;
        let mut dir = self
            .directory
            .try_clone()
            .map_err(|e| io_error(&self.root, e))?;
        for p in parents {
            dir = make_dir(&dir, p)?;
        }
        Ok((dir, last.clone()))
    }
    fn directory(&self, path: &[String]) -> HarnessResult<()> {
        let mut dir = self
            .directory
            .try_clone()
            .map_err(|e| io_error(&self.root, e))?;
        for p in path {
            dir = make_dir(&dir, p)?;
        }
        Ok(())
    }
    fn absent(&self, path: &[String], preserve_directories: bool) -> HarnessResult<()> {
        let (parent, name) = self.parent(path)?;
        match open_at(&parent, &name, libc::O_RDONLY) {
            Ok(_) => remove_projection(&parent, &name, &mut 20_000, 0, preserve_directories),
            Err(HarnessError::FileRead { .. }) => {
                let c = CString::new(name).map_err(|_| source_error(Error::Invalid))?;
                let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
                // SAFETY: valid buffers, and uninitialized stat is never inspected.
                if unsafe {
                    libc::fstatat(
                        parent.as_raw_fd(),
                        c.as_ptr(),
                        stat.as_mut_ptr(),
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                } < 0
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
                {
                    Ok(())
                } else {
                    Err(source_error(Error::Forbidden))
                }
            }
            Err(e) => Err(e),
        }
    }
    fn materialize(&self, path: &[String], bytes: &[u8]) -> HarnessResult<File> {
        let (parent, name) = self.parent(path)?;
        if let Ok(mut old) = open_at(&parent, &name, libc::O_RDONLY) {
            let meta = old.metadata().map_err(|e| io_error(&self.root, e))?;
            if !owned(&meta, meta.is_dir()) {
                return Err(HarnessError::InvalidRepository(
                    "Unsafe context view ownership, permissions, or link".into(),
                ));
            }
            if meta.is_file() && meta.len() == bytes.len() as u64 {
                let mut current = Vec::new();
                std::io::Read::by_ref(&mut old)
                    .take(bytes.len() as u64 + 1)
                    .read_to_end(&mut current)
                    .map_err(|e| io_error(&self.root, e))?;
                if current == bytes {
                    return open_at(&parent, &name, libc::O_RDONLY);
                }
            }
            remove_projection(&parent, &name, &mut 20_000, 0, false)?;
        }
        let mut file = open_at(
            &parent,
            &name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?;
        file.write_all(bytes).map_err(|e| io_error(&self.root, e))?;
        file.sync_all().map_err(|e| io_error(&self.root, e))?;
        open_at(&parent, &name, libc::O_RDONLY)
    }
}
pub struct NativeContextSource {
    store: Store,
    connection: Arc<Mutex<PgConnection>>,
    handle: Handle,
    view: Arc<View>,
    identity: SourceStoreIdentity,
    preserved_targets: std::collections::BTreeSet<String>,
    access: Mutex<SourceAccess>,
    body_queries: AtomicU64,
    metadata_queries: AtomicU64,
}
// Serializes complete reads/view updates with gate handoffs. Recovery sources
// share a mutable exclusive connection and deliberately never cache bodies.
enum SourceGate {
    SharedHeld,
    Suspended,
    Exclusive,
    Failed,
}
struct CachedBody {
    version: SourceVersion,
    bytes: Arc<[u8]>,
}
struct SourceAccess {
    gate: SourceGate,
    bodies: std::collections::BTreeMap<String, CachedBody>,
    bytes: usize,
}
impl SourceAccess {
    fn new(gate: SourceGate) -> Self {
        Self {
            gate,
            bodies: Default::default(),
            bytes: 0,
        }
    }
}
impl NativeContextSource {
    fn metadata_locked(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        let parts = self.relative(path)?;
        if parts.is_empty() {
            return Ok(SourceMetadata {
                kind: SourcePathKind::Directory,
                stored_version: None,
            });
        }
        let full = parts.join("/");
        if parts.first().map(String::as_str) != Some("vault") {
            return Ok(SourceMetadata {
                kind: SourcePathKind::Missing,
                stored_version: Some(SourceVersion {
                    logical_path: full,
                    state: StoredSourceState::Missing,
                }),
            });
        }
        let prefix = parts.iter().skip(1).cloned().collect::<Vec<_>>().join("/");
        let ancestors: Vec<_> = prefix
            .match_indices('/')
            .map(|(end, _)| &prefix[..end])
            .collect();
        let mut conn = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.store.count(1);
        self.metadata_queries.fetch_add(1, Ordering::Relaxed);
        let row=self.handle.block_on(sqlx::query("SELECT (SELECT jsonb_build_object('material_id',material_id::text,'revision',revision,'content_digest',content_digest,'deleted',deleted) FROM context_materials WHERE source_path=$1) AS exact,EXISTS(SELECT 1 FROM context_materials WHERE NOT deleted AND ($1='' OR starts_with(source_path,$1||'/'))) AS children,EXISTS(SELECT 1 FROM context_materials WHERE NOT deleted AND source_path=ANY($2)) AS ancestor").bind(&prefix).bind(&ancestors).fetch_one(&mut *conn)).map_err(|_|source_error(Error::Storage))?;
        drop(conn);
        if row.get::<bool, _>("ancestor") {
            return Err(source_error(Error::Conflict));
        }
        let exact: Option<serde_json::Value> = row.get("exact");
        let children: bool = row.get("children");
        let state = if let Some(exact) = exact {
            let material_id = exact["material_id"]
                .as_str()
                .ok_or_else(|| source_error(Error::Storage))?
                .to_owned();
            let revision = exact["revision"]
                .as_u64()
                .ok_or_else(|| source_error(Error::Storage))?;
            if exact["deleted"] == true {
                StoredSourceState::Deleted {
                    material_id,
                    revision,
                }
            } else {
                if children {
                    return Err(source_error(Error::Conflict));
                }
                StoredSourceState::Live {
                    material_id,
                    revision,
                    content_digest: exact["content_digest"]
                        .as_str()
                        .ok_or_else(|| source_error(Error::Storage))?
                        .to_owned(),
                }
            }
        } else {
            StoredSourceState::Missing
        };
        let kind = if matches!(state, StoredSourceState::Live { .. }) {
            SourcePathKind::RegularFile
        } else if children {
            SourcePathKind::Directory
        } else {
            SourcePathKind::Missing
        };
        if self.preserves_subtree(&full) {
            // Core authenticated these recovery targets. Ancestor cleanup would
            // destroy their retained bytes just as surely as refreshing them directly.
        } else if kind == SourcePathKind::Directory {
            self.view
                .directory(&parts)
                .map_err(|e| HarnessError::InvalidRepository(format!("Directory {full}: {e}")))?;
        } else if kind == SourcePathKind::Missing {
            self.view
                .absent(&parts, true)
                .map_err(|e| HarnessError::InvalidRepository(format!("Absent path {full}: {e}")))?;
        } else {
            let _ = self.view.parent(&parts)?;
        }
        Ok(SourceMetadata {
            kind,
            stored_version: (kind != SourcePathKind::Directory).then_some(SourceVersion {
                logical_path: full,
                state,
            }),
        })
    }
    fn read_access(&self) -> HarnessResult<std::sync::MutexGuard<'_, SourceAccess>> {
        let access = self
            .access
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        if !matches!(access.gate, SourceGate::SharedHeld | SourceGate::Exclusive) {
            return Err(source_error(Error::ContextPending));
        }
        Ok(access)
    }
    pub(crate) fn suspend_reads(&self) -> HarnessResult<()> {
        let mut access = self
            .access
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        match access.gate {
            SourceGate::Suspended => return Ok(()),
            SourceGate::SharedHeld => {}
            _ => return Err(source_error(Error::ContextPending)),
        }
        access.bodies.clear();
        access.bytes = 0;
        // Any uncertain connection/unlock result prevents further source use.
        access.gate = SourceGate::Failed;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.store.count(1);
        let unlocked: bool = self
            .handle
            .block_on(
                sqlx::query_scalar("SELECT pg_advisory_unlock_shared($1)")
                    .bind(CONTEXT_GATE)
                    .fetch_one(&mut *connection),
            )
            .map_err(|_| source_error(Error::Storage))?;
        if !unlocked {
            return Err(source_error(Error::Storage));
        }
        access.gate = SourceGate::Suspended;
        Ok(())
    }
    pub(crate) fn resume_reads(&self) -> HarnessResult<()> {
        let mut access = self
            .access
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        match access.gate {
            SourceGate::SharedHeld => return Ok(()),
            SourceGate::Suspended => {}
            _ => return Err(source_error(Error::ContextPending)),
        }
        access.gate = SourceGate::Failed;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.store.count(1);
        self.handle
            .block_on(
                sqlx::query("SELECT pg_advisory_lock_shared($1)")
                    .bind(CONTEXT_GATE)
                    .execute(&mut *connection),
            )
            .map_err(|_| source_error(Error::Storage))?;
        if let Err(error) = self
            .handle
            .block_on(self.store.check_context_pending(&mut connection))
        {
            // A known acquired gate is released before sibling cancellation/join.
            // The source remains Failed even if cleanup itself is uncertain.
            self.store.count(1);
            let _ = self.handle.block_on(
                sqlx::query("SELECT pg_advisory_unlock_shared($1)")
                    .bind(CONTEXT_GATE)
                    .execute(&mut *connection),
            );
            return Err(source_error(error));
        }
        access.gate = SourceGate::SharedHeld;
        Ok(())
    }
    pub(crate) fn root(&self) -> &Path {
        &self.view.root
    }
    pub fn body_queries(&self) -> u64 {
        self.body_queries.load(Ordering::Relaxed)
    }
    pub fn metadata_queries(&self) -> u64 {
        self.metadata_queries.load(Ordering::Relaxed)
    }
    fn preserves_subtree(&self, path: &str) -> bool {
        self.preserved_targets
            .iter()
            .any(|target| Path::new(target).starts_with(path))
    }
    fn relative(&self, path: &Path) -> HarnessResult<Vec<String>> {
        let p = if path.is_absolute() {
            path.strip_prefix(&self.view.root)
                .map_err(|_| source_error(Error::Invalid))?
        } else {
            path
        };
        parts(p)
    }
    fn logical(parts: &[String]) -> Option<(ContextScope, String)> {
        if parts.first().map(String::as_str) != Some("vault") {
            return None;
        }
        let (scope, start) = match parts.get(1).map(String::as_str) {
            Some("personal") | Some("profile") => (parts[1].clone(), 2),
            Some("work") => (format!("work/{}", parts.get(2)?), 3),
            _ => return None,
        };
        Some((scope.parse().ok()?, parts.get(start..)?.join("/")))
    }
    /// One metadata-only query for one or thousands of selected bindings.
    pub fn stored_versions(&self, paths: &[PathBuf]) -> HarnessResult<Vec<SourceVersion>> {
        let _access = self.read_access()?;
        if paths.len() > 10_000 {
            return Err(source_error(Error::Limit));
        }
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let logical: Vec<_> = paths
            .iter()
            .map(|p| {
                self.relative(p).and_then(|v| {
                    let (s, p) = Self::logical(&v).ok_or_else(|| source_error(Error::Invalid))?;
                    Ok(format!("{}/{p}", s.as_str()))
                })
            })
            .collect::<HarnessResult<_>>()?;
        let mut conn = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.store.count(1);
        self.metadata_queries.fetch_add(1, Ordering::Relaxed);
        let rows=self.handle.block_on(sqlx::query("SELECT source_path,material_id::text,revision,content_digest,byte_len,deleted FROM context_materials WHERE source_path=ANY($1) ORDER BY source_path COLLATE \"C\"").bind(&logical).fetch_all(&mut *conn)).map_err(|_|source_error(Error::Storage))?;
        let states = rows
            .iter()
            .map(|row| Ok((row.get::<String, _>("source_path"), stored_state(row)?)))
            .collect::<HarnessResult<std::collections::BTreeMap<_, _>>>()?;
        logical
            .into_iter()
            .map(|logical| {
                let state = states
                    .get(&logical)
                    .cloned()
                    .unwrap_or(StoredSourceState::Missing);
                Ok(SourceVersion {
                    logical_path: format!("vault/{logical}"),
                    state,
                })
            })
            .collect()
    }
}
pub(crate) fn stored_state(row: &sqlx::postgres::PgRow) -> HarnessResult<StoredSourceState> {
    let revision =
        u64::try_from(row.get::<i64, _>("revision")).map_err(|_| source_error(Error::Storage))?;
    let material_id = row.get("material_id");
    Ok(if row.get("deleted") {
        StoredSourceState::Deleted {
            material_id,
            revision,
        }
    } else {
        StoredSourceState::Live {
            material_id,
            revision,
            content_digest: row.get("content_digest"),
        }
    })
}
impl ContextSource for NativeContextSource {
    fn view_root(&self) -> &Path {
        &self.view.root
    }
    fn store_identity(&self) -> HarnessResult<Option<SourceStoreIdentity>> {
        let _access = self.read_access()?;
        Ok(Some(self.identity.clone()))
    }
    fn source_versions(&self, paths: &[PathBuf]) -> HarnessResult<Vec<SourceVersion>> {
        self.stored_versions(paths)
    }
    fn career_inventory(
        &self,
        configuration: &context_core::harness::PolicyConfiguration,
        comparison: &context_core::career::CareerComparison,
    ) -> HarnessResult<context_core::career::CareerInventory> {
        let _access = self.read_access()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        crate::career::snapshot(
            &self.store,
            &self.handle,
            &mut connection,
            configuration,
            self.identity.store_id.clone(),
            comparison.inventory.request.clone(),
            Some(comparison),
            true,
        )
        .map(|r| r.inventory)
    }
    fn metadata(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        let _access = self.read_access()?;
        self.metadata_locked(path)
    }
    fn prepare_file_target(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        let _access = self.read_access()?;
        let metadata = self.metadata_locked(path)?;
        let parts = self.relative(path)?;
        let full = parts.join("/");
        if metadata.kind == SourcePathKind::Missing
            && parts.first().map(String::as_str) == Some("vault")
            && !self.preserves_subtree(&full)
        {
            // This hook is used before Core binds an authorized exact file target.
            // Only that target may lose its directory inode; its parents stay put.
            self.view.absent(&parts, false).map_err(|e| {
                HarnessError::InvalidRepository(format!("Absent file target {full}: {e}"))
            })?;
        }
        Ok(metadata)
    }
    fn children(&self, path: &Path, limit: usize) -> HarnessResult<Vec<OsString>> {
        let _access = self.read_access()?;
        let parts = self.relative(path)?;
        let limit = limit.min(20_000);
        if parts.is_empty() {
            let metadata = self.metadata_locked(Path::new("vault"))?;
            return if metadata.kind == SourcePathKind::Directory && limit >= 1 {
                Ok(vec![OsString::from("vault")])
            } else if metadata.kind == SourcePathKind::Directory {
                Err(source_error(Error::Limit))
            } else {
                Ok(Vec::new())
            };
        }
        if parts.first().map(String::as_str) != Some("vault") {
            return Ok(Vec::new());
        }
        let prefix = parts.iter().skip(1).cloned().collect::<Vec<_>>().join("/");
        let mut conn = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.store.count(1);
        self.metadata_queries.fetch_add(1, Ordering::Relaxed);
        let rows:Vec<String>=self.handle.block_on(sqlx::query_scalar("SELECT DISTINCT split_part(CASE WHEN $1='' THEN source_path ELSE substring(source_path FROM length($1)+2) END,'/',1) COLLATE \"C\" AS child FROM context_materials WHERE NOT deleted AND ($1='' OR starts_with(source_path,$1||'/')) ORDER BY child LIMIT $2").bind(prefix).bind((limit+1)as i64).fetch_all(&mut *conn)).map_err(|_|source_error(Error::Storage))?;
        if rows.len() > limit || rows.iter().any(|s| !safe_component(s)) {
            return Err(source_error(Error::Limit));
        }
        Ok(rows.into_iter().map(OsString::from).collect())
    }
    fn open_file(&self, path: &Path, max_bytes: u64) -> HarnessResult<File> {
        let mut access = self.read_access()?;
        let parts = self.relative(path)?;
        let (scope, logical) = Self::logical(&parts)
            .filter(|(_, p)| !p.is_empty())
            .ok_or_else(|| source_error(Error::NotFound))?;
        let before = self.metadata_locked(path)?;
        if before.kind != SourcePathKind::RegularFile {
            return Err(source_error(Error::NotFound));
        }
        let limit = max_bytes.min(MAX_FILE_BYTES as u64);
        let key = parts.join("/");
        let version = before
            .stored_version
            .as_ref()
            .ok_or_else(|| source_error(Error::Conflict))?;
        let cached = access
            .bodies
            .get(&key)
            .filter(|body| body.version == *version)
            .map(|body| body.bytes.clone());
        let bytes = if let Some(bytes) = cached {
            if bytes.len() as u64 > limit {
                return Err(source_error(Error::Limit));
            }
            bytes
        } else {
            let mut conn = self
                .connection
                .lock()
                .map_err(|_| source_error(Error::Storage))?;
            self.store.count(1);
            self.body_queries.fetch_add(1, Ordering::Relaxed);
            let row=self.handle.block_on(sqlx::query("SELECT material_id::text,revision,content_digest,byte_len,deleted,CASE WHEN byte_len BETWEEN 0 AND $3 AND octet_length(content)<=$3 THEN content END AS content FROM context_materials WHERE scope=$1 AND path=$2 AND NOT deleted").bind(scope.as_str()).bind(logical).bind(limit as i64).fetch_optional(&mut *conn)).map_err(|_|source_error(Error::Storage))?.ok_or_else(||source_error(Error::NotFound))?;
            drop(conn);
            let bytes: Vec<u8> = row
                .get::<Option<Vec<u8>>, _>("content")
                .ok_or_else(|| source_error(Error::Limit))?;
            if bytes.len() as u64 > limit
                || row.get::<i64, _>("byte_len") != bytes.len() as i64
                || row.get::<String, _>("content_digest") != digest(&bytes)
                || before.stored_version.as_ref().map(|s| &s.state) != Some(&stored_state(&row)?)
            {
                return Err(source_error(Error::Conflict));
            }
            let bytes: Arc<[u8]> = bytes.into();
            if matches!(access.gate, SourceGate::SharedHeld) {
                if let Some(previous) = access.bodies.remove(&key) {
                    access.bytes -= previous.bytes.len();
                }
                if access.bodies.len() < MAX_CACHED_BODIES
                    && bytes.len() <= MAX_FILE_BYTES.saturating_sub(access.bytes)
                {
                    access.bytes += bytes.len();
                    access.bodies.insert(
                        key,
                        CachedBody {
                            version: version.clone(),
                            bytes: bytes.clone(),
                        },
                    );
                }
            }
            bytes
        };
        if self.preserved_targets.contains(&parts.join("/")) {
            let (parent, name) = self.view.parent(&parts)?;
            let mut file = open_at(&parent, &name, libc::O_RDONLY)?;
            let mut actual = Vec::new();
            std::io::Read::by_ref(&mut file)
                .take(limit + 1)
                .read_to_end(&mut actual)
                .map_err(|e| io_error(path, e))?;
            if actual.as_slice() != bytes.as_ref() {
                return Err(source_error(Error::Conflict));
            }
            return open_at(&parent, &name, libc::O_RDONLY);
        }
        if self.preserves_subtree(&parts.join("/")) {
            // A stored file cannot replace a directory containing a recovery target.
            return Err(source_error(Error::Conflict));
        }
        self.view.materialize(&parts, &bytes).map_err(|e| {
            HarnessError::InvalidRepository(format!("Materialization {}: {e}", parts.join("/")))
        })
    }
    fn validate_regular_file(&self, path: &Path) -> HarnessResult<SourceMetadata> {
        let metadata = self.metadata(path)?;
        if metadata.kind != SourcePathKind::RegularFile {
            return Err(source_error(Error::NotFound));
        }
        Ok(metadata)
    }
}
impl Store {
    pub(crate) async fn dedicated_context_connection(
        &self,
        exclusive: bool,
    ) -> Result<PgConnection, Error> {
        let mut conn = self.open_context_connection(exclusive).await?;
        self.check_context_pending(&mut conn).await?;
        Ok(conn)
    }
    async fn open_context_connection(&self, exclusive: bool) -> Result<PgConnection, Error> {
        let mut conn = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            PgConnection::connect_with(&self.database_options),
        )
        .await
        .map_err(|_| Error::Storage)?
        .map_err(|_| Error::Storage)?;
        self.count(1);
        sqlx::raw_sql("SET statement_timeout='5s'; SET lock_timeout='3s'")
            .execute(&mut conn)
            .await
            .map_err(|_| Error::Storage)?;
        self.count(1);
        sqlx::query(if exclusive {
            "SELECT pg_advisory_lock($1)"
        } else {
            "SELECT pg_advisory_lock_shared($1)"
        })
        .bind(CONTEXT_GATE)
        .execute(&mut conn)
        .await
        .map_err(|_| Error::Storage)?;
        Ok(conn)
    }
    pub(crate) async fn check_context_pending(&self, conn: &mut PgConnection) -> Result<(), Error> {
        self.count(1);
        let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM context_apply_batches WHERE state IN ('pending','committed'))").fetch_one(&mut *conn).await.map_err(|_|Error::Storage)?;
        if pending {
            return Err(Error::ContextPending);
        }
        Ok(())
    }
    /// Run synchronous Core away from the async runtime, holding one dedicated store/view session.
    pub async fn with_native_context<T, F>(&self, view: PathBuf, operation: F) -> Result<T, Error>
    where
        T: Send + 'static,
        F: FnOnce(Arc<NativeContextSource>) -> Result<T, Error> + Send + 'static,
    {
        let mut connection = self.dedicated_context_connection(false).await?;
        self.count(1);
        let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
            .fetch_one(&mut connection)
            .await
            .map_err(|_| Error::Storage)?;
        let store = self.clone();
        let handle = Handle::current();
        tokio::task::spawn_blocking(move || {
            let view = View::open(&view, &store_id).map_err(|_| Error::Invalid)?;
            operation(Arc::new(NativeContextSource {
                store,
                connection: Arc::new(Mutex::new(connection)),
                handle,
                view: Arc::new(view),
                preserved_targets: Default::default(),
                identity: SourceStoreIdentity { store_id },
                access: Mutex::new(SourceAccess::new(SourceGate::SharedHeld)),
                body_queries: AtomicU64::new(0),
                metadata_queries: AtomicU64::new(0),
            }))
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

pub use crate::context_commit::{NativeProjectionCall, NativeSqlCall};

/// An exclusive store connection and owned view; no source exists until a gate check succeeds.
pub struct NativeContextSession {
    pub(crate) sql_observations: Arc<Mutex<crate::context_commit::NativeSqlObservations>>,
    pub(crate) store: Store,
    pub(crate) connection: Arc<Mutex<PgConnection>>,
    pub(crate) handle: Handle,
    view: Arc<View>,
    pub(crate) identity: SourceStoreIdentity,
    pub(crate) source_allowed: bool,
    pub(crate) preserved_targets: std::collections::BTreeSet<String>,
}
impl NativeContextSession {
    pub(crate) fn view_root(&self) -> &Path {
        &self.view.root
    }
    pub(crate) fn recovery_source_factory(
        &self,
    ) -> impl FnOnce(
        Store,
        Arc<Mutex<PgConnection>>,
        Handle,
        std::collections::BTreeSet<String>,
    ) -> Arc<NativeContextSource>
    + use<> {
        let view = self.view.clone();
        let identity = self.identity.clone();
        move |store, connection, handle, preserved_targets| {
            Arc::new(NativeContextSource {
                store,
                connection,
                handle,
                view,
                identity,
                preserved_targets,
                access: Mutex::new(SourceAccess::new(SourceGate::Exclusive)),
                body_queries: AtomicU64::new(0),
                metadata_queries: AtomicU64::new(0),
            })
        }
    }

    pub fn fresh_source(&mut self) -> HarnessResult<Arc<NativeContextSource>> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| source_error(Error::Storage))?;
        self.handle
            .block_on(self.store.check_context_pending(&mut connection))
            .map_err(source_error)?;
        drop(connection);
        self.source_allowed = true;
        self.source()
    }
    pub fn source(&self) -> HarnessResult<Arc<NativeContextSource>> {
        if !self.source_allowed {
            return Err(source_error(Error::ContextPending));
        }
        Ok(Arc::new(NativeContextSource {
            store: self.store.clone(),
            connection: self.connection.clone(),
            handle: self.handle.clone(),
            view: self.view.clone(),
            identity: self.identity.clone(),
            preserved_targets: self.preserved_targets.clone(),
            access: Mutex::new(SourceAccess::new(SourceGate::Exclusive)),
            body_queries: AtomicU64::new(0),
            metadata_queries: AtomicU64::new(0),
        }))
    }
}
impl Store {
    pub async fn with_native_commit<T, F>(
        &self,
        view: PathBuf,
        expected_store: String,
        operation: F,
    ) -> Result<T, Error>
    where
        T: Send + 'static,
        F: FnOnce(&mut NativeContextSession) -> Result<T, Error> + Send + 'static,
    {
        let mut connection = self.open_context_connection(true).await?;
        self.count(1);
        let store_id: String = sqlx::query_scalar("SELECT store_id::text FROM context_store")
            .fetch_one(&mut connection)
            .await
            .map_err(|_| Error::Storage)?;
        if store_id != expected_store {
            return Err(Error::Conflict);
        }
        let store = self.clone();
        let handle = Handle::current();
        tokio::task::spawn_blocking(move || {
            let view = Arc::new(View::open(&view, &store_id).map_err(|_| Error::Invalid)?);
            operation(&mut NativeContextSession {
                sql_observations: Default::default(),
                store,
                connection: Arc::new(Mutex::new(connection)),
                handle,
                view,
                identity: SourceStoreIdentity { store_id },
                source_allowed: false,
                preserved_targets: Default::default(),
            })
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

#[cfg(test)]
mod connection_tests {
    use super::*;

    #[tokio::test]
    async fn native_connection_timeout_is_bounded_without_opening_the_pool() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let stall = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
            drop(socket);
        });
        let store = Store::for_native(&format!(
            "postgresql://ontology:synthetic_password_only@127.0.0.1:{port}/ontology_test_timeout"
        ))
        .unwrap();
        assert_eq!(store.calls(), 0);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            store.open_context_connection(false),
        )
        .await
        .expect("dedicated connect must time out");
        stall.abort();
        assert!(matches!(result, Err(Error::Storage)));
        assert_eq!(
            store.calls(),
            0,
            "failed handshake runs no SQL and opens no unused pool"
        );
    }
}
