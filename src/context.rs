//! Bounded original-byte transport. Nothing here accepts assertions as memories or policy.
use crate::{
    domain::Error,
    store::{Store, digest},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder, Row};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    str::FromStr,
};

pub const MAX_FILES: usize = 10_000;
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_READ_BYTES: usize = 1024 * 1024;
pub const MAX_CONTEXT_SCOPES: usize = 64;
pub const MAX_COMMAND_BYTES: usize = 32 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 1024;
const MAX_DIRECTORY_ENTRIES: usize = 20_000;
const BATCH_SIZE: usize = 100;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(try_from = "String", into = "String")]
pub struct ContextScope(String);
impl ContextScope {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl FromStr for ContextScope {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        let company = value.strip_prefix("work/");
        if matches!(value, "profile" | "personal")
            || company.is_some_and(|name| {
                !name.is_empty()
                    && name.len() <= 64
                    && name
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphanumeric)
                    && name.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'
                    })
            })
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(Error::Invalid)
        }
    }
}
impl TryFrom<String> for ContextScope {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        value.parse()
    }
}
impl From<ContextScope> for String {
    fn from(value: ContextScope) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InventoryEntry {
    pub scope: ContextScope,
    pub path: String,
    pub content_digest: String,
    pub byte_len: u64,
    pub restricted: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub source_root: String,
    pub scopes: Vec<ContextScope>,
    pub entries: Vec<InventoryEntry>,
    pub total_bytes: u64,
    pub inventory_digest: String,
}
struct Scan {
    inventory: Inventory,
    contents: Vec<Vec<u8>>,
}
fn default_limit() -> usize {
    20
}
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ContextCommand {
    ReadDocuments {
        scope: ContextScope,
        paths: Vec<String>,
    },
    Identity,
    ProjectionStatus {
        scopes: Vec<ContextScope>,
    },
    Project {
        scopes: Vec<ContextScope>,
        manifest_digest: String,
    },
    SemanticSearch {
        scopes: Vec<ContextScope>,
        query: String,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    OntologyEdges {
        scopes: Vec<ContextScope>,
        query: String,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    Inventory {
        root: PathBuf,
        scopes: Vec<ContextScope>,
    },
    Import {
        root: PathBuf,
        scopes: Vec<ContextScope>,
        inventory_digest: String,
    },
    Verify {
        root: PathBuf,
        scopes: Vec<ContextScope>,
    },
    Search {
        scope: ContextScope,
        #[serde(default)]
        query: String,
        #[serde(default)]
        after: Option<String>,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    Read {
        scope: ContextScope,
        path: String,
        #[serde(default)]
        archive: bool,
    },
    Export {
        scope: ContextScope,
        paths: Vec<String>,
        destination: PathBuf,
        #[serde(default)]
        archive: bool,
    },
}
pub enum ContextOutput {
    Json(Value),
    Text(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ContextOrigin {
    ImportedFile,
    Native,
}
impl FromStr for ContextOrigin {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        match value {
            "imported-file" => Ok(Self::ImportedFile),
            "native" => Ok(Self::Native),
            _ => Err(Error::Storage),
        }
    }
}

/// Stored provenance without a server filesystem location or a material body.
#[derive(Debug, Serialize)]
pub struct ContextMetadata {
    pub material_id: String,
    pub revision: i64,
    pub origin_kind: ContextOrigin,
    pub scope: ContextScope,
    pub path: String,
    pub source_path: String,
    pub source_digest: Option<String>,
    pub content_digest: String,
    pub byte_len: usize,
}
#[derive(Debug)]
pub struct ContextMaterial {
    pub metadata: ContextMetadata,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Serialize)]
pub struct ContextText {
    pub metadata: ContextMetadata,
    pub content: String,
}

fn validate_path(path: &str) -> Result<(), Error> {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.split('/').count() > 32
        || path.split('/').any(|part| !valid_component(part))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 255
        && !value.chars().any(|c| c.is_control() || c == '\\')
}
fn validate_root(root: &Path) -> Result<String, Error> {
    let value = root.to_str().ok_or(Error::Invalid)?;
    if !root.is_absolute() || value.len() > MAX_PATH_BYTES {
        return Err(Error::Invalid);
    }
    if value != "/" {
        validate_path(value.strip_prefix('/').ok_or(Error::Invalid)?)?;
    }
    Ok(value.to_owned())
}
pub(crate) fn restricted(path: &str) -> bool {
    path.split('/').any(|part| {
        part.starts_with('.')
            || part.eq_ignore_ascii_case("journal")
            || part.eq_ignore_ascii_case("raw")
    })
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn selected_scopes(scopes: &[ContextScope]) -> Result<Vec<ContextScope>, Error> {
    if scopes.is_empty() || scopes.len() > 64 {
        return Err(Error::Invalid);
    }
    let sorted: BTreeSet<_> = scopes.iter().cloned().collect();
    if sorted.len() != scopes.len() {
        return Err(Error::Invalid);
    }
    Ok(sorted.into_iter().collect())
}
pub fn inventory(root: &Path, scopes: &[ContextScope]) -> Result<Inventory, Error> {
    Ok(scan(root, scopes, false)?.inventory)
}
fn scan(root: &Path, scopes: &[ContextScope], retain: bool) -> Result<Scan, Error> {
    let source_root = validate_root(root)?;
    let scopes = selected_scopes(scopes)?;
    let root_fd = filesystem::open_root(root)?;
    let mut files = Vec::new();
    let mut visited = 0;
    let mut total_bytes = 0;
    for scope in &scopes {
        let directory = filesystem::open_directory(&root_fd, scope.as_str())?;
        filesystem::walk(
            &directory,
            scope,
            "",
            &mut files,
            &mut visited,
            &mut total_bytes,
            retain,
        )?;
    }
    files.sort_by(|a, b| (&a.0.scope, &a.0.path).cmp(&(&b.0.scope, &b.0.path)));
    let (entries, contents): (Vec<_>, Vec<_>) = files.into_iter().unzip();
    let bytes =
        serde_json::to_vec(&(&source_root, &scopes, &entries)).map_err(|_| Error::Invalid)?;
    Ok(Scan {
        inventory: Inventory {
            source_root,
            scopes,
            entries,
            total_bytes,
            inventory_digest: digest(&bytes),
        },
        contents,
    })
}

impl Store {
    pub async fn context(&self, command: ContextCommand) -> Result<ContextOutput, Error> {
        let value = match command {
            ContextCommand::ReadDocuments { scope, paths } => {
                self.read_context_documents(&scope, &paths).await?
            }
            ContextCommand::Identity => self.context_identity().await?,
            ContextCommand::ProjectionStatus { scopes } => self.projection_status(&scopes).await?,
            ContextCommand::Project {
                scopes,
                manifest_digest,
            } => self.project_context(&scopes, &manifest_digest).await?,
            ContextCommand::SemanticSearch {
                scopes,
                query,
                limit,
            } => self.semantic_context(&scopes, &query, limit, false).await?,
            ContextCommand::OntologyEdges {
                scopes,
                query,
                limit,
            } => self.semantic_context(&scopes, &query, limit, true).await?,
            ContextCommand::Inventory { root, scopes } => {
                serde_json::to_value(inventory(&root, &scopes)?).map_err(|_| Error::Storage)?
            }
            ContextCommand::Import {
                root,
                scopes,
                inventory_digest,
            } => {
                self.import_context(&root, &scopes, &inventory_digest)
                    .await?
            }
            ContextCommand::Verify { root, scopes } => self.verify_context(&root, &scopes).await?,
            ContextCommand::Search {
                scope,
                query,
                after,
                limit,
            } => {
                self.search_context(&scope, &query, after.as_deref(), limit)
                    .await?
            }
            ContextCommand::Read {
                scope,
                path,
                archive,
            } => {
                return Ok(ContextOutput::Text(
                    self.read_context(&scope, &path, archive).await?,
                ));
            }
            ContextCommand::Export {
                scope,
                paths,
                destination,
                archive,
            } => {
                self.export_context(&scope, &paths, &destination, archive)
                    .await?
            }
        };
        Ok(ContextOutput::Json(value))
    }

    pub async fn import_context(
        &self,
        root: &Path,
        scopes: &[ContextScope],
        expected: &str,
    ) -> Result<Value, Error> {
        if !valid_digest(expected) {
            return Err(Error::Invalid);
        }
        validate_root(root)?;
        selected_scopes(scopes)?;
        let mut tx = self.lock_context(true).await?;
        let result = async {
            let initial = scan(root, scopes, true)?;
            if initial.inventory.inventory_digest != expected {
                return Err(Error::Conflict);
            }
            let inserted = self.insert_context(&mut tx, &initial, root, scopes).await?;
            Ok(json!({"inserted":inserted,"verified":initial.inventory.entries.len(),"total_bytes":initial.inventory.total_bytes,"inventory_digest":expected}))
        }.await;
        self.finish_context(tx, result).await
    }

    async fn insert_context(
        &self,
        tx: &mut sqlx::Transaction<'_, Postgres>,
        initial: &Scan,
        root: &Path,
        scopes: &[ContextScope],
    ) -> Result<u64, Error> {
        let mut inserted = 0u64;
        for (entries, contents) in initial
            .inventory
            .entries
            .chunks(BATCH_SIZE)
            .zip(initial.contents.chunks(BATCH_SIZE))
        {
            let mut query = QueryBuilder::<Postgres>::new(
                "INSERT INTO context_materials(scope,path,source_root,source_path,source_digest,content_digest,content,byte_len,restricted,search_text) ",
            );
            query.push_values(entries.iter().zip(contents), |mut row, (entry, bytes)| {
                let text = if entry.restricted || bytes.contains(&0) {
                    None
                } else {
                    std::str::from_utf8(bytes).ok()
                };
                row.push_bind(entry.scope.as_str())
                    .push_bind(&entry.path)
                    .push_bind(&initial.inventory.source_root)
                    .push_bind(format!("{}/{}", entry.scope.as_str(), entry.path))
                    .push_bind(&entry.content_digest)
                    .push_bind(&entry.content_digest)
                    .push_bind(bytes)
                    .push_bind(entry.byte_len as i64)
                    .push_bind(entry.restricted)
                    .push_bind(text);
            });
            query.push(" ON CONFLICT(scope,path) DO NOTHING RETURNING material_id::text,revision,scope,path");
            self.count(1);
            let rows = query
                .build()
                .fetch_all(&mut **tx)
                .await
                .map_err(|_| Error::Storage)?;
            inserted = inserted
                .checked_add(rows.len() as u64)
                .ok_or(Error::Limit)?;
            let mut projections = Vec::with_capacity(rows.len());
            for row in rows {
                let scope: String = row.get("scope");
                let path: String = row.get("path");
                let (entry, bytes) = entries
                    .iter()
                    .zip(contents)
                    .find(|(e, _)| e.scope.as_str() == scope && e.path == path)
                    .ok_or(Error::Storage)?;
                let payload = crate::context_projection::projection(
                    &entry.scope,
                    &entry.path,
                    &entry.content_digest,
                    entry.restricted,
                    false,
                    Some(bytes),
                )?;
                projections.push(json!({"material_id":row.get::<String,_>("material_id"),"revision":row.get::<i64,_>("revision"),"payload":payload}));
            }
            crate::context_projection::append_projections(self, tx, &projections).await?;
        }
        // A new scan catches additions, deletions and source drift after the reviewed inventory.
        let fresh = inventory(root, scopes)?;
        if fresh != initial.inventory {
            return Err(Error::Conflict);
        }
        self.count(1);
        let rows = stored_inventory(&mut **tx, &fresh).await?;
        verify_rows(&fresh, &rows)?;
        Ok(inserted)
    }

    pub async fn verify_context(
        &self,
        root: &Path,
        scopes: &[ContextScope],
    ) -> Result<Value, Error> {
        validate_root(root)?;
        selected_scopes(scopes)?;
        let mut tx = self.lock_context(false).await?;
        let result = async {
            let fresh = inventory(root, scopes)?;
            self.count(1);
            let rows = stored_inventory(&mut *tx, &fresh).await?;
            verify_rows(&fresh, &rows)?;
            Ok(json!({"verified":fresh.entries.len(),"total_bytes":fresh.total_bytes,"inventory_digest":fresh.inventory_digest}))
        }.await;
        self.finish_context(tx, result).await
    }

    pub async fn search_context(
        &self,
        scope: &ContextScope,
        query: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Value, Error> {
        if query.len() > 240 || query.chars().any(char::is_control) || limit == 0 || limit > 100 {
            return Err(Error::Invalid);
        }
        if let Some(after) = after {
            validate_path(after)?;
        }
        let mut tx = self.lock_context(false).await?;
        let result = async {
        self.count(1);
        let rows = sqlx::query("SELECT material_id::text,revision,origin_kind,path,content_digest,byte_len,source_root,source_path,source_digest FROM context_materials WHERE scope=$1 AND NOT deleted AND NOT restricted AND ($2='' OR strpos(lower(path),lower($2))>0 OR strpos(lower(search_text),lower($2))>0) AND ($3::text IS NULL OR path COLLATE \"C\">$3 COLLATE \"C\") ORDER BY path COLLATE \"C\" LIMIT $4")
            .bind(scope.as_str()).bind(query).bind(after).bind((limit + 1) as i64).fetch_all(&mut *tx).await.map_err(|_| Error::Storage)?;
        let more = rows.len() > limit;
        let items: Vec<_> = rows.iter().take(limit).map(|row| json!({"scope":scope,"path":row.get::<String,_>("path"),"content_digest":row.get::<String,_>("content_digest"),"byte_len":row.get::<i64,_>("byte_len"),"material_id":row.get::<String,_>("material_id"),"revision":row.get::<i64,_>("revision"),"origin_kind":row.get::<String,_>("origin_kind"),"source_digest":row.get::<Option<String>,_>("source_digest"),"source_root":row.get::<Option<String>,_>("source_root"),"source_path":row.get::<String,_>("source_path")})).collect();
        let next_after = if more {
            items.last().map(|v| v["path"].clone())
        } else {
            None
        };
        Ok(json!({"items":items,"next_after":next_after,"limit":limit}))
        }.await;
        self.finish_context(tx, result).await
    }

    /// Discovery never selects bodies or includes scopes containing only restricted material.
    pub async fn context_scopes(&self) -> Result<Vec<ContextScope>, Error> {
        let mut tx = self.lock_context(false).await?;
        let result = async {
        self.count(1);
        let scopes: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT scope COLLATE \"C\" AS scope FROM context_materials WHERE NOT deleted AND NOT restricted ORDER BY scope LIMIT $1",
        )
        .bind((MAX_CONTEXT_SCOPES + 1) as i64)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| Error::Storage)?;
        if scopes.len() > MAX_CONTEXT_SCOPES {
            return Err(Error::Limit);
        }
        scopes.into_iter().map(|scope| scope.parse()).collect()
        }.await;
        self.finish_context(tx, result).await
    }

    pub async fn read_context(
        &self,
        scope: &ContextScope,
        path: &str,
        archive: bool,
    ) -> Result<String, Error> {
        let material = self
            .context_material(scope, path, archive, MAX_READ_BYTES)
            .await?;
        String::from_utf8(material.bytes).map_err(|_| Error::Invalid)
    }

    pub async fn read_context_material(
        &self,
        scope: &ContextScope,
        path: &str,
    ) -> Result<ContextText, Error> {
        let material = self
            .context_material(scope, path, false, MAX_READ_BYTES)
            .await?;
        Ok(ContextText {
            metadata: material.metadata,
            content: String::from_utf8(material.bytes).map_err(|_| Error::Invalid)?,
        })
    }

    pub async fn download_context(
        &self,
        scope: &ContextScope,
        path: &str,
    ) -> Result<ContextMaterial, Error> {
        self.context_material(scope, path, false, MAX_FILE_BYTES)
            .await
    }

    async fn context_material(
        &self,
        scope: &ContextScope,
        path: &str,
        archive: bool,
        byte_limit: usize,
    ) -> Result<ContextMaterial, Error> {
        validate_path(path)?;
        let mut tx = self.lock_context(false).await?;
        let result = async {
        if !archive && restricted(path) {
            return Err(Error::NotFound);
        }
        self.count(1);
        // Both stored length and actual length bound transfer before allocating a Rust body.
        let row = sqlx::query("SELECT material_id::text,revision,origin_kind,byte_len,content_digest,source_digest,source_path,CASE WHEN byte_len BETWEEN 0 AND $4 AND octet_length(content) <= $4 THEN content END AS content FROM context_materials WHERE scope=$1 AND path=$2 AND NOT deleted AND (NOT restricted OR $3)")
            .bind(scope.as_str()).bind(path).bind(archive).bind(byte_limit as i64).fetch_optional(&mut *tx).await.map_err(|_| Error::Storage)?.ok_or(Error::NotFound)?;
        let bytes = row
            .get::<Option<Vec<u8>>, _>("content")
            .ok_or(Error::Limit)?;
        let content_digest: String = row.get("content_digest");
        let source_digest: Option<String> = row.get("source_digest");
        let origin_kind: ContextOrigin = row.get::<String, _>("origin_kind").parse()?;
        let source_path: String = row.get("source_path");
        if bytes.len() > byte_limit {
            return Err(Error::Limit);
        }
        if row.get::<i64, _>("byte_len") != bytes.len() as i64
            || digest(&bytes) != content_digest
            || match origin_kind {
                ContextOrigin::ImportedFile => !source_digest.as_deref().is_some_and(valid_digest),
                ContextOrigin::Native => source_digest.is_some(),
            }
            || source_path != format!("{}/{}", scope.as_str(), path)
        {
            return Err(Error::Storage);
        }
        Ok(ContextMaterial {
            metadata: ContextMetadata {
                material_id: row.get("material_id"),
                revision: row.get("revision"),
                origin_kind,
                scope: scope.clone(),
                path: path.to_owned(),
                source_path,
                source_digest,
                content_digest,
                byte_len: bytes.len(),
            },
            bytes,
        })
        }.await;
        self.finish_context(tx, result).await
    }

    pub async fn export_context(
        &self,
        scope: &ContextScope,
        paths: &[String],
        destination: &Path,
        archive: bool,
    ) -> Result<Value, Error> {
        validate_root(destination)?;
        if paths.is_empty() || paths.len() > 100 {
            return Err(Error::Invalid);
        }
        let mut selected = BTreeSet::new();
        for path in paths {
            validate_path(path)?;
            if !selected.insert(path.as_str()) {
                return Err(Error::Invalid);
            }
        }
        let mut tx = self.lock_context(false).await?;
        let result = async {
        self.count(1);
        // The window sum bounds the complete result before PostgreSQL sends any bodies.
        let rows = sqlx::query("WITH selected AS MATERIALIZED (SELECT path,byte_len,content_digest,content,sum(byte_len) OVER () AS total FROM context_materials WHERE scope=$1 AND path=ANY($2) AND NOT deleted AND (NOT restricted OR $3)) SELECT path,byte_len,content_digest,CASE WHEN total <= $4 THEN content END AS content FROM selected ORDER BY path COLLATE \"C\"")
            .bind(scope.as_str()).bind(paths).bind(archive).bind(MAX_TOTAL_BYTES as i64).fetch_all(&mut *tx).await.map_err(|_| Error::Storage)?;
        if rows.len() != paths.len() {
            return Err(Error::NotFound);
        }
        let mut files = Vec::new();
        let mut total_bytes = 0u64;
        for row in rows {
            let path: String = row.get("path");
            validate_path(&path)?;
            let bytes = row
                .get::<Option<Vec<u8>>, _>("content")
                .ok_or(Error::Limit)?;
            if digest(&bytes) != row.get::<String, _>("content_digest") {
                return Err(Error::Storage);
            }
            total_bytes = total_bytes
                .checked_add(bytes.len() as u64)
                .ok_or(Error::Limit)?;
            if total_bytes > MAX_TOTAL_BYTES {
                return Err(Error::Limit);
            }
            files.push((path, bytes));
        }
        filesystem::export(destination, &files)?;
        Ok(json!({"exported":files.len(),"total_bytes":total_bytes}))
        }.await;
        self.finish_context(tx, result).await
    }
}

async fn stored_inventory<'e, E: sqlx::Executor<'e, Database = Postgres>>(
    executor: E,
    inventory: &Inventory,
) -> Result<Vec<sqlx::postgres::PgRow>, Error> {
    let scopes: Vec<_> = inventory.scopes.iter().map(ContextScope::as_str).collect();
    sqlx::query("SELECT scope,path,source_path,source_digest,content_digest,byte_len,restricted,encode(sha256(content),'hex') AS actual_digest FROM context_materials WHERE source_root=$1 AND scope=ANY($2) AND NOT deleted ORDER BY scope COLLATE \"C\",path COLLATE \"C\" LIMIT $3")
        .bind(&inventory.source_root).bind(scopes).bind((MAX_FILES + 1) as i64).fetch_all(executor).await.map_err(|_| Error::Storage)
}
fn verify_rows(inventory: &Inventory, rows: &[sqlx::postgres::PgRow]) -> Result<(), Error> {
    if rows.len() != inventory.entries.len() {
        return Err(Error::Conflict);
    }
    for (entry, row) in inventory.entries.iter().zip(rows) {
        if row.get::<String, _>("scope") != entry.scope.as_str()
            || row.get::<String, _>("path") != entry.path
            || row.get::<String, _>("source_path")
                != format!("{}/{}", entry.scope.as_str(), entry.path)
            || row.get::<String, _>("source_digest") != entry.content_digest
            || row.get::<String, _>("content_digest") != entry.content_digest
            || row.get::<String, _>("actual_digest") != entry.content_digest
            || row.get::<i64, _>("byte_len") != entry.byte_len as i64
            || row.get::<bool, _>("restricted") != entry.restricted
        {
            return Err(Error::Conflict);
        }
    }
    Ok(())
}

// All traversal is relative to already-open directory descriptors. No symlink component is
// followed, and files are checked before and after reading, including link count and timestamps.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "android"
))]
mod filesystem {
    use super::*;
    use std::{
        ffi::{CStr, CString},
        fs::File,
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd, RawFd},
            unix::fs::MetadataExt,
        },
    };

    fn open_at(parent: RawFd, name: &str, flags: i32, mode: libc::c_int) -> Result<File, Error> {
        let name = CString::new(name).map_err(|_| Error::Invalid)?;
        // SAFETY: name is NUL terminated; the returned owned descriptor has exactly one File owner.
        let fd = unsafe {
            libc::openat(
                parent,
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                mode,
            )
        };
        if fd < 0 {
            return Err(Error::Invalid);
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub(super) fn open_root(path: &Path) -> Result<File, Error> {
        let root = validate_root(path)?;
        let mut file = open_at(libc::AT_FDCWD, "/", libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        if root != "/" {
            for part in root[1..].split('/') {
                file = open_at(
                    file.as_raw_fd(),
                    part,
                    libc::O_RDONLY | libc::O_DIRECTORY,
                    0,
                )?;
            }
        }
        Ok(file)
    }
    pub(super) fn open_directory(parent: &File, path: &str) -> Result<File, Error> {
        let mut directory = parent.try_clone().map_err(|_| Error::Invalid)?;
        for part in path.split('/') {
            directory = open_at(
                directory.as_raw_fd(),
                part,
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            )?;
        }
        Ok(directory)
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    fn names(directory: &File, visited: &mut usize) -> Result<Vec<String>, Error> {
        let fd = open_at(
            directory.as_raw_fd(),
            ".",
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        use std::os::fd::IntoRawFd;
        let raw = fd.into_raw_fd();
        let pointer = unsafe { libc::fdopendir(raw) };
        if pointer.is_null() {
            unsafe {
                libc::close(raw);
            }
            return Err(Error::Invalid);
        }
        let directory = Directory(pointer);
        let mut result = Vec::new();
        loop {
            #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
            unsafe {
                *libc::__error() = 0;
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            unsafe {
                *libc::__errno_location() = 0;
            }
            let entry = unsafe { libc::readdir(directory.0) };
            if entry.is_null() {
                if std::io::Error::last_os_error().raw_os_error() != Some(0) {
                    return Err(Error::Invalid);
                }
                break;
            }
            // SAFETY: readdir owns a NUL-terminated name until its next call, copied immediately.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| Error::Invalid)?;
            if name == "." || name == ".." {
                continue;
            }
            if !valid_component(name) {
                return Err(Error::Invalid);
            }
            *visited = visited.checked_add(1).ok_or(Error::Limit)?;
            if *visited > MAX_DIRECTORY_ENTRIES {
                return Err(Error::Limit);
            }
            result.push(name.to_owned());
        }
        result.sort();
        Ok(result)
    }
    fn stable(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.len() == after.len()
            && before.nlink() == after.nlink()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    }
    pub(super) fn walk(
        directory: &File,
        scope: &ContextScope,
        prefix: &str,
        files: &mut Vec<(InventoryEntry, Vec<u8>)>,
        visited: &mut usize,
        total: &mut u64,
        retain: bool,
    ) -> Result<(), Error> {
        let before_directory = directory.metadata().map_err(|_| Error::Invalid)?;
        for name in names(directory, visited)? {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            validate_path(&path)?;
            let c_name = CString::new(name.as_str()).map_err(|_| Error::Invalid)?;
            let mut observed = std::mem::MaybeUninit::<libc::stat>::uninit();
            // Inspect without opening devices or following aliases, then verify the opened inode.
            if unsafe {
                libc::fstatat(
                    directory.as_raw_fd(),
                    c_name.as_ptr(),
                    observed.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(Error::Invalid);
            }
            let observed = unsafe { observed.assume_init() };
            let kind = observed.st_mode & libc::S_IFMT;
            if kind != libc::S_IFREG && kind != libc::S_IFDIR {
                return Err(Error::Invalid);
            }
            if kind == libc::S_IFREG && observed.st_nlink != 1 {
                return Err(Error::Invalid);
            }
            let mut file = open_at(directory.as_raw_fd(), &name, libc::O_RDONLY, 0)?;
            let before = file.metadata().map_err(|_| Error::Invalid)?;
            if before.dev() != observed.st_dev as u64
                || u128::from(before.ino()) != u128::from(observed.st_ino)
            {
                return Err(Error::Conflict);
            }
            if before.is_dir() {
                walk(&file, scope, &path, files, visited, total, retain)?;
                continue;
            }
            if !before.is_file() || before.nlink() != 1 {
                return Err(Error::Invalid);
            }
            if before.len() > MAX_FILE_BYTES as u64 || files.len() >= MAX_FILES {
                return Err(Error::Limit);
            }
            *total = total.checked_add(before.len()).ok_or(Error::Limit)?;
            if *total > MAX_TOTAL_BYTES {
                return Err(Error::Limit);
            }
            let mut bytes = Vec::new();
            (&mut file)
                .take((MAX_FILE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Invalid)?;
            let after = file.metadata().map_err(|_| Error::Invalid)?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err(Error::Limit);
            }
            if !stable(&before, &after) || bytes.len() as u64 != before.len() {
                return Err(Error::Conflict);
            }
            files.push((
                InventoryEntry {
                    scope: scope.clone(),
                    restricted: restricted(&path),
                    path,
                    content_digest: digest(&bytes),
                    byte_len: bytes.len() as u64,
                },
                if retain { bytes } else { Vec::new() },
            ));
        }
        if !stable(
            &before_directory,
            &directory.metadata().map_err(|_| Error::Invalid)?,
        ) {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    fn mkdir(parent: &File, name: &str) -> Result<(), Error> {
        let name = CString::new(name).map_err(|_| Error::Invalid)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    pub(super) fn export(destination: &Path, files: &[(String, Vec<u8>)]) -> Result<(), Error> {
        let parent = destination.parent().ok_or(Error::Invalid)?;
        let name = destination
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or(Error::Invalid)?;
        let parent = open_root(parent)?;
        // An entirely new destination prevents overwriting any existing path, including aliases.
        mkdir(&parent, name)?;
        let root = open_directory(&parent, name)?;
        let mut created = BTreeSet::new();
        for (path, bytes) in files {
            let mut directory = root.try_clone().map_err(|_| Error::Storage)?;
            let mut parts = path.split('/').peekable();
            let mut prefix = String::new();
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    let mut file = open_at(
                        directory.as_raw_fd(),
                        part,
                        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                        0o600,
                    )?;
                    file.write_all(bytes).map_err(|_| Error::Storage)?;
                    file.sync_all().map_err(|_| Error::Storage)?;
                } else {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(part);
                    if created.insert(prefix.clone()) {
                        mkdir(&directory, part)?;
                    }
                    directory = open_directory(&directory, part)?;
                }
            }
        }
        root.sync_all().map_err(|_| Error::Storage)
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "android"
)))]
mod filesystem {
    use super::*;
    // Fail closed on platforms without descriptor-relative no-follow traversal.
    pub(super) fn open_root(_: &Path) -> Result<(), Error> {
        Err(Error::Invalid)
    }
    pub(super) fn open_directory(_: &(), _: &str) -> Result<(), Error> {
        Err(Error::Invalid)
    }
    pub(super) fn walk(
        _: &(),
        _: &ContextScope,
        _: &str,
        _: &mut Vec<(InventoryEntry, Vec<u8>)>,
        _: &mut usize,
        _: &mut u64,
        _: bool,
    ) -> Result<(), Error> {
        Err(Error::Invalid)
    }
    pub(super) fn export(_: &Path, _: &[(String, Vec<u8>)]) -> Result<(), Error> {
        Err(Error::Invalid)
    }
}
