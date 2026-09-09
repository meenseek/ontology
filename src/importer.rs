use crate::{
    domain::{Error, ImportedRecord, MAX_DOCUMENT_BYTES, Scope, SourceKind},
    source_process::run_bounded,
    store::{Store, digest},
};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::process::Command;

#[derive(Clone)]
pub struct GitReader {
    allowed: Vec<PathBuf>,
    calls: Arc<AtomicU64>,
    response_bytes: Arc<AtomicU64>,
    deadline: Duration,
    output_limit: usize,
}
impl GitReader {
    pub fn new(allowed: Vec<PathBuf>) -> Result<Self, Error> {
        if allowed.is_empty()
            || allowed.iter().any(|path| {
                !path.is_absolute()
                    || path
                        .components()
                        .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
            })
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            allowed,
            calls: Arc::new(AtomicU64::new(0)),
            response_bytes: Arc::new(AtomicU64::new(0)),
            deadline: Duration::from_secs(3),
            output_limit: MAX_DOCUMENT_BYTES,
        })
    }
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
    pub fn response_bytes(&self) -> u64 {
        self.response_bytes.load(Ordering::Relaxed)
    }
    fn validate(&self, repo: &Path, commit: &str, paths: &[String]) -> Result<(), Error> {
        if !self.allowed.iter().any(|root| root == repo)
            || !matches!(commit.len(), 40 | 64)
            || !commit
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || paths.is_empty()
            || paths.len() > 100
        {
            return Err(Error::Invalid);
        }
        let mut unique = HashSet::new();
        for path in paths {
            let p = Path::new(path);
            if path.is_empty()
                || path.len() > 512
                || path.contains(['\0', '\n', '\r', '\\', ':'])
                || !unique.insert(path)
                || p.components().any(|c| !matches!(c, Component::Normal(_)))
                || p.components().any(|c| c.as_os_str() == ".git")
                || !matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("md" | "txt" | "rst")
                )
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    async fn git(&self, repo: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let mut command = Command::new("/usr/bin/git");
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "protocol.allow")
            .env("GIT_CONFIG_VALUE_0", "never")
            .env("GIT_LITERAL_PATHSPECS", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .arg("--no-optional-locks")
            .arg("-C")
            .arg(repo)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let bytes = run_bounded(command, self.deadline, self.output_limit).await?;
        self.response_bytes
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        Ok(bytes)
    }
    pub async fn resolve_ref(&self, repo: &Path, reference: &str) -> Result<String, Error> {
        if !self.allowed.iter().any(|root| root == repo)
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
        if repo.canonicalize().ok().as_deref() != Some(repo) {
            return Err(Error::Import);
        }
        let revision = format!("{reference}^{{commit}}");
        let bytes = self
            .git(
                repo,
                &["rev-parse", "--verify", "--end-of-options", &revision],
            )
            .await?;
        let resolved = String::from_utf8(bytes).map_err(|_| Error::Import)?;
        let resolved = resolved.trim_end_matches('\n');
        if !matches!(resolved.len(), 40 | 64)
            || !resolved
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(Error::Import);
        }
        Ok(resolved.to_owned())
    }
    pub async fn read(
        &self,
        repo: &Path,
        commit: &str,
        paths: &[String],
        scope: Scope,
    ) -> Result<Vec<ImportedRecord>, Error> {
        self.validate(repo, commit, paths)?;
        if repo.canonicalize().ok().as_deref() != Some(repo) {
            return Err(Error::Import);
        }
        for path in paths {
            let p = Path::new(path);
            let mut part = repo.to_path_buf();
            for component in p.components() {
                part.push(component);
                if std::fs::symlink_metadata(&part).is_ok_and(|meta| meta.file_type().is_symlink())
                {
                    return Err(Error::Invalid);
                }
            }
        }
        let top = self.git(repo, &["rev-parse", "--show-toplevel"]).await?;
        if String::from_utf8(top)
            .map_err(|_| Error::Import)?
            .trim_end()
            != repo.to_str().ok_or(Error::Invalid)?
        {
            return Err(Error::Invalid);
        }
        let format = self
            .git(repo, &["rev-parse", "--show-object-format"])
            .await?;
        let expected = match format.as_slice() {
            b"sha1\n" => 40,
            b"sha256\n" => 64,
            _ => return Err(Error::Import),
        };
        if commit.len() != expected {
            return Err(Error::Invalid);
        }
        let revision = format!("{commit}^{{commit}}");
        let resolved = self
            .git(repo, &["rev-parse", "--verify", &revision])
            .await?;
        if resolved != format!("{commit}\n").as_bytes() {
            return Err(Error::Import);
        }
        let mut records = Vec::with_capacity(paths.len());
        for path in paths {
            let tree = self
                .git(repo, &["ls-tree", "-z", "-l", commit, "--", path])
                .await?;
            let content = if tree.is_empty() {
                None
            } else {
                let entry = String::from_utf8(tree).map_err(|_| Error::Import)?;
                let (meta, name) = entry
                    .strip_suffix('\0')
                    .and_then(|s| s.split_once('\t'))
                    .ok_or(Error::Import)?;
                let fields = meta.split_whitespace().collect::<Vec<_>>();
                if fields.len() != 4
                    || !matches!(fields.first(), Some(&"100644" | &"100755"))
                    || fields.get(1) != Some(&"blob")
                    || name != path
                {
                    return Err(Error::Import);
                }
                let size = fields
                    .get(3)
                    .ok_or(Error::Import)?
                    .parse::<usize>()
                    .map_err(|_| Error::Import)?;
                let object = fields.get(2).ok_or(Error::Import)?;
                if size > MAX_DOCUMENT_BYTES
                    || object.len() != expected
                    || !object.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(Error::Limit);
                }
                let bytes = self.git(repo, &["cat-file", "blob", object]).await?;
                if bytes.len() != size || bytes.contains(&0) {
                    return Err(Error::Import);
                }
                Some(String::from_utf8(bytes).map_err(|_| Error::Import)?)
            };
            let (source_id, entity_id) = identity(repo, path, scope)?;
            records.push(ImportedRecord {
                source_id,
                entity_id,
                scope,
                repository: repo.to_str().ok_or(Error::Invalid)?.into(),
                path: path.clone(),
                kind: SourceKind::Git,
                source_revision: commit.into(),
                digest: content.as_ref().map(|text| digest(text.as_bytes())),
                content,
            });
        }
        Ok(records)
    }
    pub async fn import(
        &self,
        store: &Store,
        repo: &Path,
        commit: &str,
        paths: &[String],
        scope: Scope,
    ) -> Result<usize, Error> {
        self.validate(repo, commit, paths)?;
        let mut guard = store.lock_import().await?;
        let result = self
            .import_locked(store, repo, commit, paths, scope, &mut guard)
            .await;
        store.finish_import(guard).await?;
        result
    }
    pub(crate) async fn import_locked(
        &self,
        store: &Store,
        repo: &Path,
        commit: &str,
        paths: &[String],
        scope: Scope,
        _guard: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<usize, Error> {
        self.validate(repo, commit, paths)?;
        match self.read(repo, commit, paths, scope).await {
            Ok(records) => {
                store.apply_import(&records).await?;
                Ok(records.len())
            }
            Err(error) => {
                let ids = paths
                    .iter()
                    .map(|path| identity(repo, path, scope).map(|(id, _)| id))
                    .collect::<Result<Vec<_>, _>>()?;
                store.mark_failed(&ids, SourceKind::Git).await?;
                Err(error)
            }
        }
    }
}
pub fn identity(repo: &Path, path: &str, scope: Scope) -> Result<(String, String), Error> {
    let hash = digest(
        format!(
            "{}\0{}\0{}",
            scope.as_str(),
            repo.to_str().ok_or(Error::Invalid)?,
            path
        )
        .as_bytes(),
    );
    Ok((format!("s_{hash}"), format!("e_{hash}")))
}
