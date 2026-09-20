use crate::{
    context::ContextScope,
    context_importer::{ContextReader, validate_paths, validate_store_id},
    domain::{Error, Scope, SourceKind},
    importer::GitReader,
    store::Store,
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
    Context {
        store_id: String,
        scope: Scope,
        context_scope: ContextScope,
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
    fn paths(&self) -> &[String] {
        match self {
            Self::Git { paths, .. } | Self::Context { paths, .. } => paths,
        }
    }
    fn kind(&self) -> SourceKind {
        match self {
            Self::Git { .. } => SourceKind::Git,
            Self::Context { .. } => SourceKind::Context,
        }
    }
    fn identities(&self) -> Result<Vec<String>, Error> {
        match self {
            Self::Git {
                root, scope, paths, ..
            } => paths
                .iter()
                .map(|p| crate::importer::identity(root, p, *scope).map(|v| v.0))
                .collect(),
            Self::Context {
                store_id,
                context_scope,
                scope,
                paths,
            } => Ok(paths
                .iter()
                .map(|p| {
                    format!(
                        "context:{store_id}:{}:{}:{p}",
                        context_scope.as_str(),
                        scope.as_str()
                    )
                })
                .collect()),
        }
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
            let paths = source.paths();
            if paths.is_empty() || paths.len() > 100 {
                return Err(Error::Invalid);
            }
            count = count.checked_add(paths.len()).ok_or(Error::Limit)?;
            if count > 100 {
                return Err(Error::Limit);
            }
            match source {
                SyncSource::Context {
                    store_id,
                    context_scope,
                    ..
                } => {
                    validate_store_id(store_id)?;
                    validate_paths(context_scope, paths, true)?;
                }
                SyncSource::Git {
                    root, reference, ..
                } => {
                    if !absolute(root)
                        || reference.is_empty()
                        || reference.len() > 160
                        || reference.starts_with('-')
                        || reference.contains("..")
                        || !reference
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"/_-.".contains(&c))
                    {
                        return Err(Error::Invalid);
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
                        {
                            return Err(Error::Invalid);
                        }
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
    let observation = store.dependency_start("sync-refresh", &path)?;
    let result = async {
        let config = SyncConfig::load(path)?;
        let mut tx = store.lock_import().await?;
        if config
            .sources
            .iter()
            .any(|s| matches!(s, SyncSource::Context { .. }))
            && let Err(error) = store.context_gate_in(&mut tx, false).await
        {
            return store.finish_context(tx, Err(error)).await;
        }
        let mut results = Vec::new();
        for (index, source) in config.sources.iter().enumerate() {
            let (result, provider_calls, response_bytes) = match source {
                SyncSource::Context {
                    store_id,
                    context_scope,
                    scope,
                    paths,
                } => {
                    let mut reader = ContextReader::new();
                    let result = reader
                        .refresh_in(store, &mut tx, store_id, context_scope, paths, *scope)
                        .await;
                    (result, reader.body_calls, reader.response_bytes)
                }
                SyncSource::Git {
                    root,
                    scope,
                    reference,
                    paths,
                } => {
                    store
                        .consumer_control(&mut tx, "SAVEPOINT git_consumer_source")
                        .await?;
                    let reader = GitReader::new(vec![root.clone()]);
                    let result = async {
                        let reader = reader.as_ref().map_err(Clone::clone)?;
                        let commit = reader.resolve_ref(root, reference).await?;
                        let records = reader.read(root, &commit, paths, *scope).await?;
                        store.apply_import_in(&mut tx, &records).await?;
                        Ok(records.len())
                    }
                    .await;
                    if result.is_err() {
                        store
                            .consumer_control(&mut tx, "ROLLBACK TO SAVEPOINT git_consumer_source")
                            .await?;
                        store
                            .mark_failed_in(&mut tx, &source.identities()?, SourceKind::Git)
                            .await?;
                    }
                    store
                        .consumer_control(&mut tx, "RELEASE SAVEPOINT git_consumer_source")
                        .await?;
                    (
                        result,
                        reader.as_ref().map_or(0, GitReader::calls),
                        reader.as_ref().map_or(0, GitReader::response_bytes),
                    )
                }
            };
            results.push(SourceResult {
                index,
                kind: source.kind(),
                documents: source.paths().len(),
                provider_calls,
                response_bytes,
                ok: result.is_ok(),
                error: result.err().map(|e| e.to_string()),
            });
        }
        store.finish_import(tx).await?;
        Ok(SyncReport {
            interval_seconds: config.interval_seconds,
            ok: results.iter().all(|r| r.ok),
            sources: results,
        })
    }
    .await;
    store.dependency_finish(observation, &result);
    result
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
