use crate::{
    domain::{Error, Scope, SourceKind},
    importer::GitReader,
    store::Store,
    vault_importer::{VaultReader, VaultScope},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::RwLock;

pub const MAX_CONFIG_BYTES: usize = 32_768;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncConfig {
    pub interval_seconds: u64,
    pub sources: Vec<SyncSource>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum SyncSource {
    Git {
        root: PathBuf,
        scope: Scope,
        #[serde(rename = "ref")]
        reference: String,
        paths: Vec<String>,
    },
    Vault {
        root: PathBuf,
        scope: Scope,
        vault_scope: VaultScope,
        binary: PathBuf,
        paths: Vec<String>,
    },
}
fn absolute(path: &Path) -> bool {
    path.to_str().is_some_and(|p| {
        p.len() <= 4096
            && p.starts_with('/')
            && p[1..]
                .split('/')
                .all(|part| !matches!(part, "" | "." | ".."))
            && !p.chars().any(char::is_control)
    })
}
impl SyncSource {
    fn parts(&self) -> (&Path, Scope, SourceKind, &[String]) {
        match self {
            Self::Git {
                root, scope, paths, ..
            } => (root, *scope, SourceKind::Git, paths),
            Self::Vault {
                root, scope, paths, ..
            } => (root, *scope, SourceKind::Vault, paths),
        }
    }
    fn identities(&self) -> Result<Vec<String>, Error> {
        let (root, scope, kind, paths) = self.parts();
        paths
            .iter()
            .map(|path| match self {
                Self::Vault { vault_scope, .. } => crate::vault_importer::identity(
                    root,
                    &format!("{}/{path}", vault_scope.as_str()),
                    scope,
                )
                .map(|v| v.0),
                _ if kind == SourceKind::Git => {
                    crate::importer::identity(root, path, scope).map(|v| v.0)
                }
                _ => Err(Error::Invalid),
            })
            .collect()
    }
}
impl SyncConfig {
    pub fn load(path: &Path) -> Result<Self, Error> {
        if !absolute(path) {
            return Err(Error::Invalid);
        }
        let file = std::fs::File::open(path).map_err(|_| Error::Invalid)?;
        if !file
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= MAX_CONFIG_BYTES as u64)
        {
            return Err(Error::Limit);
        }
        let mut bytes = Vec::new();
        file.take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(Error::Limit);
        }
        let config: Self = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<(), Error> {
        if !(15..=3600).contains(&self.interval_seconds)
            || self.sources.is_empty()
            || self.sources.len() > 8
        {
            return Err(Error::Invalid);
        }
        let mut identities = HashSet::new();
        let mut count = 0usize;
        for source in &self.sources {
            let (root, _, kind, paths) = source.parts();
            if !absolute(root) || paths.is_empty() || paths.len() > 100 {
                return Err(Error::Invalid);
            }
            count = count.checked_add(paths.len()).ok_or(Error::Limit)?;
            if count > 100 {
                return Err(Error::Limit);
            }
            for path in paths {
                if path.len() > 512
                    || path
                        .chars()
                        .any(|c| c.is_control() || "\\:*?[]{}".contains(c))
                    || path
                        .split('/')
                        .any(|p| matches!(p, "" | "." | ".." | ".git"))
                    || !matches!(
                        Path::new(path).extension().and_then(|e| e.to_str()),
                        Some("md" | "txt" | "rst")
                    )
                    || (kind == SourceKind::Vault && !path.ends_with(".md"))
                {
                    return Err(Error::Invalid);
                }
            }
            match source {
                SyncSource::Git { reference, .. } => {
                    if reference.is_empty()
                        || reference.len() > 160
                        || reference.starts_with('-')
                        || reference.contains("..")
                        || !reference
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"/_-.".contains(&c))
                    {
                        return Err(Error::Invalid);
                    }
                }
                SyncSource::Vault { binary, .. } => {
                    if !absolute(binary) {
                        return Err(Error::Invalid);
                    }
                }
            }
            for id in source.identities()? {
                if !identities.insert(id) {
                    return Err(Error::Invalid);
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct SourceResult {
    pub index: usize,
    pub kind: SourceKind,
    pub documents: usize,
    pub provider_calls: u64,
    pub response_bytes: u64,
    pub ok: bool,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SyncReport {
    pub interval_seconds: u64,
    pub sources: Vec<SourceResult>,
    pub ok: bool,
}
// Both sync-once and the app loop call this function. Read config completely before any source.
pub async fn refresh(store: &Store, path: &Path) -> Result<SyncReport, Error> {
    let config = SyncConfig::load(path)?;
    let guard = store.lock_import().await?;
    let mut results = Vec::new();
    for (index, source) in config.sources.iter().enumerate() {
        let (_, _, kind, paths) = source.parts();
        let (result, provider_calls, response_bytes) = refresh_source(store, source).await;
        if result.is_err() {
            store.mark_failed(&source.identities()?, kind).await?;
        }
        results.push(SourceResult {
            index,
            kind,
            documents: paths.len(),
            provider_calls,
            response_bytes,
            ok: result.is_ok(),
            error: result.err().map(|e| e.to_string()),
        });
    }
    store.finish_import(guard).await?;
    Ok(SyncReport {
        interval_seconds: config.interval_seconds,
        ok: results.iter().all(|r| r.ok),
        sources: results,
    })
}
async fn refresh_source(store: &Store, source: &SyncSource) -> (Result<usize, Error>, u64, u64) {
    let (root, scope, _, paths) = source.parts();
    let (records, calls, bytes) = match source {
        SyncSource::Git { reference, .. } => {
            let reader = match GitReader::new(vec![root.to_path_buf()]) {
                Ok(reader) => reader,
                Err(e) => return (Err(e), 0, 0),
            };
            let records = match reader.resolve_ref(root, reference).await {
                Ok(commit) => reader.read(root, &commit, paths, scope).await,
                Err(e) => Err(e),
            };
            (records, reader.calls(), reader.response_bytes())
        }
        SyncSource::Vault {
            binary,
            vault_scope,
            ..
        } => {
            let reader = match VaultReader::new(binary.clone(), vec![root.to_path_buf()]) {
                Ok(reader) => reader,
                Err(e) => return (Err(e), 0, 0),
            };
            let records = reader.read(root, *vault_scope, paths, scope).await;
            (records, reader.calls(), reader.response_bytes())
        }
    };
    let result = match records {
        Ok(records) => store.apply_import(&records).await.map(|()| records.len()),
        Err(e) => Err(e),
    };
    (result, calls, bytes)
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct SyncStatus {
    pub enabled: bool,
    pub running: bool,
    pub last_completed_at: Option<u64>,
    pub report: Option<SyncReport>,
    pub error: Option<String>,
}
pub type SharedStatus = Arc<RwLock<SyncStatus>>;
pub async fn run_loop(store: Store, path: PathBuf, status: SharedStatus) {
    loop {
        {
            let mut state = status.write().await;
            state.enabled = true;
            state.running = true;
        }
        let result = refresh(&store, &path).await;
        let interval = result.as_ref().map(|v| v.interval_seconds).unwrap_or(60);
        {
            let mut state = status.write().await;
            state.running = false;
            state.last_completed_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|v| v.as_secs());
            match result {
                Ok(report) => {
                    state.report = Some(report);
                    state.error = None;
                }
                Err(error) => {
                    state.report = None;
                    state.error = Some(error.to_string());
                }
            }
        }
        // No catch-up burst: re-read current config/source state after wake or restart.
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}
