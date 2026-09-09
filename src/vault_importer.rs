use crate::{
    domain::{
        Error, ImportedRecord, MAX_DOCUMENT_BYTES, MAX_RESPONSE_BYTES, MAX_RESULTS, Scope,
        SourceKind,
    },
    source_process::run_bounded,
    store::{Store, digest},
};
use serde::Deserialize;
use std::{
    collections::HashSet,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultScope {
    Profile,
    Personal,
    Work,
}
impl VaultScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Personal => "personal",
            Self::Work => "work",
        }
    }
}
impl FromStr for VaultScope {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        match value {
            "profile" => Ok(Self::Profile),
            "personal" => Ok(Self::Personal),
            "work" => Ok(Self::Work),
            _ => Err(Error::Invalid),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadResponse {
    documents: Vec<Document>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    scope: VaultScope,
    path: String,
    title: String,
    body: String,
    source_digest: String,
}

#[derive(Clone)]
pub struct VaultReader {
    binary: PathBuf,
    allowed: Vec<PathBuf>,
    calls: Arc<AtomicU64>,
    response_bytes: Arc<AtomicU64>,
}
fn absolute_path(path: &Path) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    path.is_absolute()
        && (text == "/"
            || text.strip_prefix('/').is_some_and(|relative| {
                relative
                    .split('/')
                    .all(|part| !matches!(part, "" | "." | ".."))
            }))
}
impl VaultReader {
    pub fn new(binary: PathBuf, allowed: Vec<PathBuf>) -> Result<Self, Error> {
        if !absolute_path(&binary)
            || !std::fs::metadata(&binary)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            || allowed.is_empty()
            || allowed.iter().any(|path| !absolute_path(path))
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            binary,
            allowed,
            calls: Arc::new(AtomicU64::new(0)),
            response_bytes: Arc::new(AtomicU64::new(0)),
        })
    }
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
    pub fn response_bytes(&self) -> u64 {
        self.response_bytes.load(Ordering::Relaxed)
    }
    fn validate(&self, root: &Path, paths: &[String]) -> Result<(), Error> {
        if !self.allowed.iter().any(|allowed| allowed == root) || !absolute_path(root) {
            return Err(Error::Invalid);
        }
        if paths.len() > MAX_RESULTS {
            return Err(Error::Limit);
        }
        let mut unique = HashSet::new();
        for path in paths {
            if path.len() > 512
                || !path.ends_with(".md")
                || path
                    .chars()
                    .any(|c| c.is_control() || "\\:*?[]{}".contains(c))
                || path.split('/').any(|part| matches!(part, "" | "." | ".."))
                || !unique.insert(path)
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    pub async fn read(
        &self,
        root: &Path,
        vault_scope: VaultScope,
        paths: &[String],
        scope: Scope,
    ) -> Result<Vec<ImportedRecord>, Error> {
        self.validate(root, paths)?;
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut command = Command::new(&self.binary);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .arg("read")
            .arg(root)
            .arg("--scope")
            .arg(vault_scope.as_str());
        for path in paths {
            command.arg("--path").arg(path);
        }
        command
            .arg("--json")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        self.calls.fetch_add(1, Ordering::Relaxed);
        let bytes = run_bounded(command, Duration::from_secs(3), MAX_RESPONSE_BYTES).await?;
        self.response_bytes
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        let response: ReadResponse = serde_json::from_slice(&bytes).map_err(|_| Error::Import)?;
        if response.documents.len() != paths.len() {
            return Err(Error::Import);
        }
        let mut expected = paths
            .iter()
            .map(|path| format!("{}/{path}", vault_scope.as_str()))
            .collect::<HashSet<_>>();
        response
            .documents
            .into_iter()
            .map(|document| {
                if document.scope != vault_scope
                    || !expected.remove(&document.path)
                    || document.source_digest.len() != 64
                    || !document
                        .source_digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    || document.title.contains('\0')
                    || document.body.contains('\0')
                {
                    return Err(Error::Import);
                }
                let content = format!("# {}\n\n{}", document.title, document.body);
                if content.len() > MAX_DOCUMENT_BYTES {
                    return Err(Error::Limit);
                }
                let (source_id, entity_id) = identity(root, &document.path, scope)?;
                Ok(ImportedRecord {
                    source_id,
                    entity_id,
                    scope,
                    repository: root.to_str().ok_or(Error::Invalid)?.into(),
                    path: document.path,
                    kind: SourceKind::Vault,
                    source_revision: document.source_digest,
                    digest: Some(digest(content.as_bytes())),
                    content: Some(content),
                })
            })
            .collect()
    }
    pub async fn import(
        &self,
        store: &Store,
        root: &Path,
        vault_scope: VaultScope,
        paths: &[String],
        scope: Scope,
    ) -> Result<usize, Error> {
        self.validate(root, paths)?;
        if paths.is_empty() {
            return Ok(0);
        }
        let mut guard = store.lock_import().await?;
        let result = self
            .import_locked(store, root, vault_scope, paths, scope, &mut guard)
            .await;
        store.finish_import(guard).await?;
        result
    }
    pub(crate) async fn import_locked(
        &self,
        store: &Store,
        root: &Path,
        vault_scope: VaultScope,
        paths: &[String],
        scope: Scope,
        _guard: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<usize, Error> {
        self.validate(root, paths)?;
        match self.read(root, vault_scope, paths, scope).await {
            Ok(records) => {
                store.apply_import(&records).await?;
                Ok(records.len())
            }
            Err(error) => {
                let ids = paths
                    .iter()
                    .map(|path| {
                        identity(root, &format!("{}/{path}", vault_scope.as_str()), scope)
                            .map(|(id, _)| id)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                store.mark_failed(&ids, SourceKind::Vault).await?;
                Err(error)
            }
        }
    }
}

pub fn identity(
    root: &Path,
    root_relative_path: &str,
    scope: Scope,
) -> Result<(String, String), Error> {
    // Git retains its existing IDs; Vault has its own provider identity and full scoped path.
    let hash = digest(
        format!(
            "{}\0vault\0{}\0{}",
            scope.as_str(),
            root.to_str().ok_or(Error::Invalid)?,
            root_relative_path
        )
        .as_bytes(),
    );
    Ok((format!("s_{hash}"), format!("e_{hash}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn executable(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("create synthetic executable");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .expect("make synthetic executable runnable");
    }
    #[tokio::test]
    async fn strict_response_and_request_boundaries() {
        let temp = tempfile::tempdir().expect("isolated fake provider");
        let root = temp.path().canonicalize().expect("canonical fixture root");
        let binary = root.join("provider");
        executable(&binary, "exit 0");
        let reader =
            VaultReader::new(binary.clone(), vec![root.clone()]).expect("explicit test provider");
        let valid = json!({"documents":[{"scope":"personal","path":"personal/x.md","title":"합성", "body":"본문", "source_digest":"a".repeat(64)}]});
        let paths = ["x.md".into()];
        let mut variants = Vec::new();
        let mut changed = valid.clone();
        changed["extra"] = json!(true);
        variants.push(changed);
        let mut changed = valid.clone();
        changed["documents"][0]["extra"] = json!(true);
        variants.push(changed);
        let mut changed = valid.clone();
        changed["documents"][0]["scope"] = json!("work");
        variants.push(changed);
        for path in [
            "x.md",
            "work/x.md",
            "personal/y.md",
            "personal/../x.md",
            "/personal/x.md",
        ] {
            let mut changed = valid.clone();
            changed["documents"][0]["path"] = json!(path);
            variants.push(changed);
        }
        for hash in ["a".repeat(63), "A".repeat(64), "g".repeat(64)] {
            let mut changed = valid.clone();
            changed["documents"][0]["source_digest"] = json!(hash);
            variants.push(changed);
        }
        let mut changed = valid.clone();
        changed["documents"] = json!([]);
        variants.push(changed);
        let mut changed = valid.clone();
        changed["documents"] = json!([valid["documents"][0], valid["documents"][0]]);
        variants.push(changed);
        let mut changed = valid.clone();
        changed["documents"][0]["body"] = Value::Null;
        variants.push(changed);
        let mut changed = valid.clone();
        changed["documents"][0]["body"] = json!("x\0y");
        variants.push(changed);
        let mut changed = valid.clone();
        changed["documents"][0]["body"] = json!("x".repeat(MAX_DOCUMENT_BYTES));
        variants.push(changed);
        for response in variants.iter().map(Value::to_string).chain([
            "not json".into(),
            format!("{valid}{valid}"),
            "{\"documents\":[],\"documents\":[]}".into(),
        ]) {
            let quoted = response.replace('\'', "'\\''");
            executable(&binary, &format!("printf '%s' '{quoted}'"));
            let before = reader.calls();
            assert!(
                reader
                    .read(&root, VaultScope::Personal, &paths, Scope::Meenseek)
                    .await
                    .is_err()
            );
            assert_eq!(reader.calls() - before, 1, "reject responses without retry");
        }
        let quoted = valid.to_string().replace('\'', "'\\''");
        executable(&binary, &format!("printf '%s' '{quoted}'"));
        assert_eq!(
            reader
                .read(&root, VaultScope::Personal, &paths, Scope::Meenseek)
                .await
                .expect("valid response")
                .len(),
            1
        );
        // Duplicate returned paths must also fail when the response count is correct.
        let duplicate = json!({"documents":[valid["documents"][0], valid["documents"][0]]});
        executable(
            &binary,
            &format!(
                "printf '%s' '{}'",
                duplicate.to_string().replace('\'', "'\\''")
            ),
        );
        assert!(
            reader
                .read(
                    &root,
                    VaultScope::Personal,
                    &["x.md".into(), "y.md".into()],
                    Scope::Meenseek
                )
                .await
                .is_err()
        );
        let before = reader.calls();
        for path in [
            "../x.md",
            "/x.md",
            "./x.md",
            "a/../x.md",
            "a//x.md",
            "x.txt",
            "*.md",
            "x\ny.md",
            "a\\x.md",
            "a:x.md",
        ] {
            assert_eq!(
                reader
                    .read(&root, VaultScope::Personal, &[path.into()], Scope::Meenseek)
                    .await
                    .expect_err("invalid path"),
                Error::Invalid
            );
        }
        assert!(
            reader
                .read(
                    &root,
                    VaultScope::Personal,
                    &["x.md".into(), "x.md".into()],
                    Scope::Meenseek
                )
                .await
                .is_err()
        );
        assert!(
            reader
                .read(
                    &root,
                    VaultScope::Personal,
                    &vec!["x.md".into(); 101],
                    Scope::Meenseek
                )
                .await
                .is_err()
        );
        assert!(
            reader
                .read(
                    &root.join("unregistered"),
                    VaultScope::Personal,
                    &paths,
                    Scope::Meenseek
                )
                .await
                .is_err()
        );
        assert_eq!(
            reader.calls(),
            before,
            "invalid requests issue no subprocess"
        );
        assert!(VaultReader::new("relative".into(), vec![root.clone()]).is_err());
        assert!(VaultReader::new(root.clone(), vec![root.clone()]).is_err());
        assert!(VaultReader::new(binary.clone(), vec!["relative".into()]).is_err());
        assert!(!absolute_path(Path::new("/root//vault")));
        assert!(!absolute_path(Path::new("/root/vault/")));
        assert!(!absolute_path(Path::new("/root/./vault")));
        assert!(!absolute_path(Path::new("/root/../vault")));
        assert!("all".parse::<VaultScope>().is_err());
        // A failing executable never yields a partial result even if stdout is valid JSON.
        executable(&binary, &format!("printf '%s' '{quoted}'; exit 1"));
        assert!(
            reader
                .read(&root, VaultScope::Personal, &paths, Scope::Meenseek)
                .await
                .is_err()
        );
        executable(&binary, "exec /usr/bin/yes");
        assert_eq!(
            reader
                .read(&root, VaultScope::Personal, &paths, Scope::Meenseek)
                .await
                .expect_err("bounded stdout"),
            Error::Limit
        );
        executable(&binary, "exec /usr/bin/yes >&2");
        assert_eq!(
            reader
                .read(&root, VaultScope::Personal, &paths, Scope::Meenseek)
                .await
                .expect_err("bounded stderr"),
            Error::Limit
        );
    }
}
