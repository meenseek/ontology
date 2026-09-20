#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    ContextVaultError, Document, Result, Scope,
    atomic_file::copy_permission_metadata,
    document::{
        contains_raw_conversation_path, is_skipped_directory_name, read_exact_markdown_source,
        read_markdown_source,
    },
    frontmatter::ParsedFrontmatter,
    read_markdown_documents,
    redaction::redact_secrets,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SearchScope {
    All,
    Personal,
    Profile,
    Work,
}

impl SearchScope {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Personal => "personal",
            Self::Profile => "profile",
            Self::Work => "work",
        }
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct ContextMetadata {
    pub language: Option<String>,
    pub aliases: Vec<String>,
    pub exportable: Option<bool>,
}

pub const MAX_EXACT_READ_PATHS: usize = 100;
const MAX_EXACT_READ_PATH_BYTES: usize = 512;
const MAX_EXACT_READ_FILE_BYTES: u64 = 64 * 1024;
const MAX_EXACT_READ_JSON_BYTES: usize = 1024 * 1024;

#[derive(serde::Serialize)]
struct ReadDocument {
    scope: &'static str,
    path: String,
    title: String,
    body: String,
    source_digest: String,
}

#[derive(serde::Serialize)]
struct ReadResponse {
    documents: Vec<ReadDocument>,
}

/// Read only the selected exact paths, returning one bounded, redacted JSON response.
/// No directory traversal, index, database, or source writes are performed.
pub fn read_context(root: impl AsRef<Path>, scope: Scope, paths: &[PathBuf]) -> Result<String> {
    if paths.is_empty() || paths.len() > MAX_EXACT_READ_PATHS {
        return Err(ContextVaultError::invalid_input(
            "exact read requires between 1 and 100 paths",
        ));
    }
    let paths = normalize_exact_read_paths(scope, paths)?;
    let root = std::path::absolute(root.as_ref()).map_err(|source| {
        ContextVaultError::io("resolve exact read root", root.as_ref(), source)
    })?;
    let documents = paths
        .iter()
        .map(|path| read_context_document(&root, scope, path))
        .collect::<Result<Vec<_>>>()?;
    let mut output = BoundedReadJson(Vec::new());
    serde_json::to_writer(&mut output, &ReadResponse { documents }).map_err(|_| {
        ContextVaultError::invalid_input("exact read JSON exceeds the 1 MiB response limit")
    })?;
    String::from_utf8(output.0)
        .map_err(|_| ContextVaultError::invalid_input("failed to encode exact read JSON"))
}

pub fn normalize_exact_read_paths(scope: Scope, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut seen = BTreeSet::new();
    paths
        .iter()
        .map(|path| {
            let text = path.to_str().ok_or_else(|| {
                ContextVaultError::invalid_input("exact read path must be valid UTF-8")
            })?;
            if text.len() > MAX_EXACT_READ_PATH_BYTES
                || text.ends_with('/')
                || text.ends_with("/.")
                || text.contains(['*', '?', '[', ']', '{', '}', '\\', '\0'])
                || text.chars().any(char::is_control)
            {
                return Err(ContextVaultError::invalid_input(
                    "exact read requires literal file paths of at most 512 bytes",
                ));
            }
            let relative = scoped_relative_path(scope, path)?;
            if relative.as_os_str().len() > MAX_EXACT_READ_PATH_BYTES {
                return Err(ContextVaultError::invalid_input(
                    "exact read path including scope must not exceed 512 bytes",
                ));
            }
            let lower = relative.to_string_lossy().to_ascii_lowercase();
            if contains_raw_conversation_path(Path::new(&lower))
                || Path::new(&lower).components().any(|component| {
                    let Component::Normal(name) = component else {
                        return true;
                    };
                    let Some(name) = name.to_str() else {
                        return true;
                    };
                    let stem = name.strip_suffix(".md").unwrap_or(name);
                    name.starts_with('.')
                        || is_skipped_directory_name(name)
                        || matches!(
                            stem,
                            "journal" | "secret" | "secrets" | "credential" | "credentials"
                        )
                })
            {
                return Err(ContextVaultError::invalid_input(
                    "exact read excludes generated, secret, journal, and raw conversation paths",
                ));
            }
            if !seen.insert(relative.clone()) {
                return Err(ContextVaultError::invalid_input(
                    "exact read paths must be unique after normalization",
                ));
            }
            Ok(relative)
        })
        .collect()
}

fn read_context_document(root: &Path, scope: Scope, path: &Path) -> Result<ReadDocument> {
    let source = read_exact_markdown_source(root, path, MAX_EXACT_READ_FILE_BYTES)?;
    let declared_scope = source
        .frontmatter()
        .map(|fields| fields.optional_string("scope"))
        .transpose()
        .map_err(|_| ContextVaultError::invalid_input("invalid scope metadata in exact read"))?
        .flatten();
    if declared_scope.is_some_and(|declared| declared != scope.as_str()) {
        return Err(ContextVaultError::invalid_input(
            "document scope metadata must match the selected scope",
        ));
    }
    let document = source.document();
    Ok(ReadDocument {
        scope: scope.as_str(),
        path: document.source_path().to_owned(),
        title: redact_secrets(document.title()),
        body: redact_secrets(document.body()),
        source_digest: source.content_digest().to_owned(),
    })
}

struct BoundedReadJson(Vec<u8>);

impl Write for BoundedReadJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_EXACT_READ_JSON_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("exact read response limit exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn save_context(
    root: impl AsRef<Path>,
    scope: Scope,
    scope_relative_path: impl AsRef<Path>,
    title: &str,
    body: &str,
) -> Result<PathBuf> {
    save_context_with_metadata(
        root,
        scope,
        scope_relative_path,
        title,
        body,
        ContextMetadata::default(),
    )
}

pub fn save_context_with_metadata(
    root: impl AsRef<Path>,
    scope: Scope,
    scope_relative_path: impl AsRef<Path>,
    title: &str,
    body: &str,
    metadata: ContextMetadata,
) -> Result<PathBuf> {
    let title = title.trim();
    if title.is_empty() {
        return Err(ContextVaultError::invalid_input("title must not be empty"));
    }
    if contains_line_break(title) {
        return Err(ContextVaultError::invalid_input(
            "title must be a single line",
        ));
    }

    let body = body.trim();
    if body.is_empty() {
        return Err(ContextVaultError::invalid_input("body must not be empty"));
    }

    let root = root.as_ref();
    let relative_path = scoped_relative_path(scope, scope_relative_path)?;
    ensure_vault_root_directory(root)?;
    create_parent_directories_without_symlinks(root, &relative_path)?;

    let path = root.join(&relative_path);
    let _save_lock = ContextSaveLock::acquire(&path)?;
    reject_existing_symlink(&path)?;
    let existing = existing_metadata(root, &path)?;
    let metadata = normalize_metadata(metadata, existing.as_ref())?;
    let rendered = markdown_document(scope, title, body, &metadata)?;
    write_context_atomically(
        root,
        &path,
        rendered.as_bytes(),
        existing
            .as_ref()
            .map(|metadata| metadata.content_digest.as_str()),
    )?;

    Ok(relative_path)
}

pub fn delete_context(
    root: impl AsRef<Path>,
    scope: Scope,
    scope_relative_path: impl AsRef<Path>,
) -> Result<PathBuf> {
    let root = root.as_ref();
    let relative_path = scoped_relative_path(scope, scope_relative_path)?;
    ensure_existing_path_without_symlinks(root, &relative_path)?;

    let path = root.join(&relative_path);
    let _save_lock = ContextSaveLock::acquire(&path)?;
    ensure_existing_path_without_symlinks(root, &relative_path)?;
    fs::remove_file(&path)
        .map_err(|source| ContextVaultError::io("delete context file", &path, source))?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| ContextVaultError::io("sync context directory", parent, source))?;
    }
    Ok(relative_path)
}

pub fn export_context(root: impl AsRef<Path>, scope: SearchScope) -> Result<String> {
    let documents = read_markdown_documents(root)?;
    let documents = documents
        .iter()
        .filter(|document| document.exportable())
        .filter(|document| scope_includes_document(scope, document.scope()))
        .collect::<Vec<_>>();

    let mut output = String::from("# LLM Context Vault Export\n\n");
    output.push_str("- scope: `");
    output.push_str(scope.as_str());
    output.push_str("`\n");
    output.push_str("- source: `llm-context-vault`\n\n");
    output.push_str(
        "Use the user's requested language when presenting this context. Preserve code identifiers, file paths, commands, and quoted source terms as written.\n\n",
    );

    if documents.is_empty() {
        output.push_str("No matching context.\n");
        return Ok(output);
    }

    for document in documents {
        append_document(&mut output, document);
    }

    Ok(output)
}

fn append_document(output: &mut String, document: &Document) {
    output.push_str("## ");
    output.push_str(&redact_secrets(document.title()));
    output.push('\n');
    output.push_str("- source: `");
    output.push_str(document.source_path());
    output.push_str("`\n");
    output.push_str("- scope: `");
    output.push_str(document.scope().as_str());
    output.push_str("`\n");
    if let Some(language) = document.language() {
        output.push_str("- language: `");
        output.push_str(language);
        output.push_str("`\n");
    }
    if !document.aliases().is_empty() {
        output.push_str("- aliases:\n");
        for alias in document.aliases() {
            output.push_str("  - `");
            output.push_str(&redact_secrets(alias));
            output.push_str("`\n");
        }
    }
    output.push('\n');
    output.push_str(&redact_secrets(document.body()));
    output.push_str("\n\n");
}

fn markdown_document(
    scope: Scope,
    title: &str,
    body: &str,
    metadata: &ResolvedContextMetadata,
) -> Result<String> {
    let mut output = format!(
        "---\ntitle: {}\nscope: {}\n",
        quoted_frontmatter_value(title),
        scope.as_str(),
    );
    if let Some(language) = &metadata.language {
        output.push_str("language: ");
        output.push_str(&quoted_frontmatter_value(language));
        output.push('\n');
    }
    if !metadata.aliases.is_empty() {
        output.push_str("aliases:\n");
        for alias in &metadata.aliases {
            output.push_str("  - ");
            output.push_str(&quoted_frontmatter_value(alias));
            output.push('\n');
        }
    }
    if !metadata.exportable {
        output.push_str("export: false\n");
    }
    if !metadata.preserved_frontmatter.is_empty() {
        output.push_str(&metadata.preserved_frontmatter);
        if !metadata.preserved_frontmatter.ends_with('\n') {
            output.push('\n');
        }
    }
    output.push_str("---\n\n# ");
    output.push_str(title);
    output.push_str("\n\n");
    output.push_str(body);
    output.push('\n');
    ParsedFrontmatter::from_markdown(&output)?;
    Ok(output)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ResolvedContextMetadata {
    language: Option<String>,
    aliases: Vec<String>,
    exportable: bool,
    preserved_frontmatter: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ExistingContextMetadata {
    language: Option<String>,
    aliases: Vec<String>,
    exportable: bool,
    preserved_frontmatter: String,
    content_digest: String,
}

const MANAGED_FRONTMATTER_KEYS: [&str; 5] = ["title", "scope", "language", "aliases", "export"];

fn quoted_frontmatter_value(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn normalize_metadata(
    metadata: ContextMetadata,
    existing: Option<&ExistingContextMetadata>,
) -> Result<ResolvedContextMetadata> {
    let language = normalize_optional_metadata_value(metadata.language, "language")?;
    let aliases = normalize_aliases(metadata.aliases)?;
    let existing_language = existing.and_then(|metadata| metadata.language.clone());
    let existing_aliases = existing
        .map(|metadata| metadata.aliases.clone())
        .unwrap_or_default();
    let existing_exportable = existing.map(|metadata| metadata.exportable);
    let preserved_frontmatter = existing
        .map(|metadata| metadata.preserved_frontmatter.clone())
        .unwrap_or_default();

    Ok(ResolvedContextMetadata {
        language: language.or(existing_language),
        aliases: if aliases.is_empty() {
            existing_aliases
        } else {
            aliases
        },
        exportable: metadata.exportable.or(existing_exportable).unwrap_or(true),
        preserved_frontmatter,
    })
}

fn existing_metadata(root: &Path, path: &Path) -> Result<Option<ExistingContextMetadata>> {
    if !path.exists() {
        return Ok(None);
    }
    let source = read_markdown_source(root, path)?;
    let document = source.document();
    Ok(Some(ExistingContextMetadata {
        language: document.language().map(ToOwned::to_owned),
        aliases: document.aliases().to_vec(),
        exportable: document.exportable(),
        preserved_frontmatter: preserve_unmanaged_frontmatter(source.frontmatter())?,
        content_digest: source.content_digest().to_owned(),
    }))
}

fn preserve_unmanaged_frontmatter(frontmatter: Option<&ParsedFrontmatter>) -> Result<String> {
    let Some(frontmatter) = frontmatter else {
        return Ok(String::new());
    };
    let raw = frontmatter.raw();
    let mut output = Vec::new();
    let mut skipping_managed_block = false;
    let mut managed_keys = Vec::new();
    let mut top_level_keys = Vec::new();
    let mut pending_trivia = Vec::new();

    let parsed_top_level_keys = frontmatter.keys().collect::<Vec<_>>();
    let parsed_mapping_key_count = frontmatter.mapping_len();
    if parsed_mapping_key_count != parsed_top_level_keys.len() {
        return Err(ContextVaultError::invalid_input(
            "frontmatter keys must be non-empty strings",
        ));
    }

    for line in raw.lines() {
        if is_top_level_frontmatter_entry(line) {
            let key = parsed_top_level_keys
                .get(top_level_keys.len())
                .copied()
                .ok_or_else(|| {
                    ContextVaultError::invalid_input(
                        "frontmatter keys must use one top-level YAML mapping entry per line",
                    )
                })?
                .to_owned();
            top_level_keys.push(key.clone());
            let next_is_managed = MANAGED_FRONTMATTER_KEYS.contains(&key.as_str());
            if skipping_managed_block && !next_is_managed {
                output.append(&mut pending_trivia);
            } else {
                pending_trivia.clear();
            }
            skipping_managed_block = next_is_managed;
            if next_is_managed {
                if managed_keys.contains(&key) {
                    return Err(ContextVaultError::invalid_input(format!(
                        "frontmatter contains duplicate managed key `{key}`"
                    )));
                }
                managed_keys.push(key);
            }
        } else if skipping_managed_block {
            if line.is_empty() || line.starts_with('#') {
                pending_trivia.push(line);
            } else if !line.trim_start().starts_with('#') {
                pending_trivia.clear();
            }
        }

        if !skipping_managed_block {
            output.push(line);
        }
    }
    if skipping_managed_block {
        output.append(&mut pending_trivia);
    }

    let parsed_managed_keys = parsed_top_level_keys
        .iter()
        .copied()
        .filter(|key| MANAGED_FRONTMATTER_KEYS.contains(key))
        .collect::<Vec<_>>();

    if parsed_mapping_key_count != parsed_top_level_keys.len()
        || top_level_keys.len() != parsed_top_level_keys.len()
        || parsed_top_level_keys
            .iter()
            .any(|key| !top_level_keys.iter().any(|detected| detected == key))
    {
        return Err(ContextVaultError::invalid_input(
            "frontmatter keys must use one top-level YAML mapping entry per line",
        ));
    }
    if managed_keys.len() != parsed_managed_keys.len()
        || parsed_managed_keys
            .iter()
            .any(|key| !managed_keys.iter().any(|managed| managed == key))
    {
        return Err(ContextVaultError::invalid_input(
            "managed frontmatter key detection did not match the parsed YAML mapping",
        ));
    }

    while output.last().is_some_and(|line| line.is_empty()) {
        output.pop();
    }
    if output.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{}\n", output.join("\n")))
    }
}

fn is_top_level_frontmatter_entry(line: &str) -> bool {
    if line.is_empty() || line.starts_with(char::is_whitespace) || line.starts_with('#') {
        return false;
    }
    yaml_mapping_separator(line).is_some_and(|separator| {
        let candidate = &line[..separator];
        !candidate.is_empty()
            && !candidate.starts_with(['{', '['])
            && candidate != "?"
            && !candidate.starts_with("? ")
    })
}

fn yaml_mapping_separator(line: &str) -> Option<usize> {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let mut characters = line.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if double_quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                double_quoted = false;
            }
            continue;
        }
        if single_quoted {
            if character == '\'' {
                if characters.peek().is_some_and(|(_, next)| *next == '\'') {
                    characters.next();
                } else {
                    single_quoted = false;
                }
            }
            continue;
        }
        match character {
            '\'' => single_quoted = true,
            '"' => double_quoted = true,
            ':' if line[index + character.len_utf8()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace) =>
            {
                return Some(index);
            }
            _ => {}
        }
    }
    None
}

struct ContextSaveLock {
    // Closing the file releases the kernel lock; the sidecar inode stays reusable.
    _file: fs::File,
}

impl ContextSaveLock {
    fn acquire(target: &Path) -> Result<Self> {
        let file_name = target.file_name().and_then(OsStr::to_str).ok_or_else(|| {
            ContextVaultError::invalid_input("context file name must be valid UTF-8")
        })?;
        let path = target.with_file_name(format!(".{file_name}.save.lock"));
        match fs::symlink_metadata(&path) {
            Ok(metadata) => Self::validate_metadata(&path, &metadata)?,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ContextVaultError::io(
                    "inspect context save lock",
                    &path,
                    error,
                ));
            }
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        let file = options
            .open(&path)
            .map_err(|source| ContextVaultError::io("open context save lock", &path, source))?;
        let opened_metadata = file.metadata().map_err(|source| {
            ContextVaultError::io("inspect opened context save lock", &path, source)
        })?;
        Self::validate_metadata(&path, &opened_metadata)?;
        file.try_lock().map_err(|source| {
            ContextVaultError::io("acquire context save lock", &path, source.into())
        })?;
        let locked_metadata = file.metadata().map_err(|source| {
            ContextVaultError::io("reinspect context save lock", &path, source)
        })?;
        Self::validate_metadata(&path, &locked_metadata)?;
        let current_metadata = fs::symlink_metadata(&path).map_err(|source| {
            ContextVaultError::io("verify context save lock path", &path, source)
        })?;
        Self::validate_metadata(&path, &current_metadata)?;
        #[cfg(unix)]
        if locked_metadata.dev() != current_metadata.dev()
            || locked_metadata.ino() != current_metadata.ino()
        {
            return Err(ContextVaultError::invalid_input(format!(
                "context save lock path changed while acquiring `{}`",
                path.display()
            )));
        }
        Ok(Self { _file: file })
    }

    fn validate_metadata(path: &Path, metadata: &fs::Metadata) -> Result<()> {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(ContextVaultError::invalid_input(format!(
                "context save lock `{}` must be a regular file without symbolic links",
                path.display()
            )));
        }
        #[cfg(unix)]
        {
            // SAFETY: `geteuid` has no preconditions and does not retain pointers.
            let effective_user = unsafe { libc::geteuid() };
            // The sidecar contains no context or ownership records, so owned 0644
            // files are safe to reuse without changing their contents or mode.
            if metadata.uid() != effective_user
                || metadata.nlink() != 1
                || metadata.mode() & 0o022 != 0
            {
                return Err(ContextVaultError::invalid_input(format!(
                    "context save lock `{}` must be user-owned with one link and no group or other write permission",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

fn write_context_atomically(
    root: &Path,
    path: &Path,
    content: &[u8],
    expected_content_digest: Option<&str>,
) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        ContextVaultError::invalid_input("context path must have a parent directory")
    })?;
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| ContextVaultError::invalid_input("context file name must be valid UTF-8"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ContextVaultError::invalid_input(error.to_string()))?
        .as_nanos();
    let temporary = parent.join(format!(
        ".{file_name}.save-{}-{nonce}.tmp",
        std::process::id()
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        // Restrict POSIX permissions while writing; existing mode/ACL metadata is
        // restored below. Inherited allow ACLs remain outside this guarantee.
        #[cfg(unix)]
        options.mode(0o600);
        let mut temporary_file = options.open(&temporary).map_err(|source| {
            ContextVaultError::io("create temporary context file", &temporary, source)
        })?;
        temporary_file.write_all(content).map_err(|source| {
            ContextVaultError::io("write temporary context file", &temporary, source)
        })?;
        reject_existing_symlink(path)?;
        let existing_permissions = match (expected_content_digest, fs::symlink_metadata(path)) {
            (Some(_), Err(error)) if error.kind() == ErrorKind::NotFound => {
                return Err(ContextVaultError::invalid_input(
                    "context file changed before the atomic update",
                ));
            }
            (None, Ok(_)) => {
                return Err(ContextVaultError::invalid_input(
                    "context file appeared before the atomic create",
                ));
            }
            (None, Err(error)) if error.kind() == ErrorKind::NotFound => None,
            (_, Err(error)) => {
                return Err(ContextVaultError::io(
                    "inspect context file before atomic write",
                    path,
                    error,
                ));
            }
            (Some(_), Ok(metadata)) => Some(metadata.permissions()),
        };
        if let Some(expected_digest) = expected_content_digest {
            let current = read_markdown_source(root, path)?;
            if current.content_digest() != expected_digest {
                return Err(ContextVaultError::invalid_input(
                    "context file changed before the atomic update",
                ));
            }
        }
        if existing_permissions.is_some() {
            copy_permission_metadata(path, &temporary).map_err(|source| {
                ContextVaultError::io(
                    "preserve context file permission metadata",
                    &temporary,
                    source,
                )
            })?;
        }
        temporary_file.sync_all().map_err(|source| {
            ContextVaultError::io("sync temporary context file", &temporary, source)
        })?;
        fs::rename(&temporary, path)
            .map_err(|source| ContextVaultError::io("replace context file", path, source))?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| ContextVaultError::io("sync context directory", parent, source))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn normalize_optional_metadata_value(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if contains_line_break(value) {
        return Err(ContextVaultError::invalid_input(format!(
            "{field} must be a single line"
        )));
    }
    Ok(Some(value.to_owned()))
}

fn normalize_aliases(aliases: Vec<String>) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    for alias in aliases {
        let alias = alias.trim();
        if alias.is_empty() {
            continue;
        }
        if contains_line_break(alias) {
            return Err(ContextVaultError::invalid_input(
                "aliases must be single-line values",
            ));
        }
        if !normalized.iter().any(|existing| existing == alias) {
            normalized.push(alias.to_owned());
        }
    }
    Ok(normalized)
}

fn contains_line_break(value: &str) -> bool {
    value.contains('\n') || value.contains('\r')
}

fn scoped_relative_path(scope: Scope, path: impl AsRef<Path>) -> Result<PathBuf> {
    if scope == Scope::Other {
        return Err(ContextVaultError::invalid_input(
            "scope must be profile, personal, or work",
        ));
    }

    let path = safe_markdown_path(path.as_ref())?;
    if first_component(&path).is_some_and(is_reserved_scope_name) {
        return Err(ContextVaultError::invalid_input(
            "path must be relative within the selected scope",
        ));
    }

    let mut scoped = PathBuf::from(scope.as_str());
    scoped.push(path);
    Ok(scoped)
}

fn safe_markdown_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(ContextVaultError::invalid_input(
            "path must be a non-empty relative path",
        ));
    }

    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => safe.push(value),
            Component::CurDir
            | Component::ParentDir
            | Component::Prefix(_)
            | Component::RootDir => {
                return Err(ContextVaultError::invalid_input(
                    "path must stay inside the selected scope",
                ));
            }
        }
    }

    if safe.as_os_str().is_empty() {
        return Err(ContextVaultError::invalid_input(
            "path must be a non-empty relative path",
        ));
    }

    if safe.components().any(|component| match component {
        Component::Normal(value) => value.to_str().is_some_and(is_skipped_directory_name),
        _ => false,
    }) {
        return Err(ContextVaultError::invalid_input(
            "context path must not use generated artifact directory names",
        ));
    }

    match safe.extension().and_then(OsStr::to_str) {
        Some("md") => {}
        Some(_) => {
            return Err(ContextVaultError::invalid_input(
                "context path must use the .md extension",
            ));
        }
        None => {
            safe.set_extension("md");
        }
    }

    Ok(safe)
}

fn first_component(path: &Path) -> Option<&str> {
    path.components()
        .next()
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
}

fn is_reserved_scope_name(value: &str) -> bool {
    matches!(value, "personal" | "profile" | "work")
}

fn ensure_vault_root_directory(root: &Path) -> Result<()> {
    match fs::symlink_metadata(root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(ContextVaultError::invalid_input(
                    "vault root must not be a symbolic link",
                ));
            }
            if !metadata.is_dir() {
                return Err(ContextVaultError::invalid_input(
                    "vault root must be a directory",
                ));
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir_all(root)
                .map_err(|source| ContextVaultError::io("create vault root", root, source))?;
            reject_existing_symlink(root)?;
        }
        Err(error) => {
            return Err(ContextVaultError::io(
                "read vault root metadata",
                root,
                error,
            ));
        }
    }

    Ok(())
}

fn create_parent_directories_without_symlinks(root: &Path, relative_path: &Path) -> Result<()> {
    let Some(parent) = relative_path.parent() else {
        return Ok(());
    };

    let mut current = root.to_path_buf();
    for component in parent.components() {
        let Component::Normal(value) = component else {
            return Err(ContextVaultError::invalid_input(
                "path must stay inside the selected scope",
            ));
        };
        current.push(value);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(ContextVaultError::invalid_input(format!(
                        "context path must not contain symbolic links: {}",
                        current.display()
                    )));
                }
                if !metadata.is_dir() {
                    return Err(ContextVaultError::invalid_input(format!(
                        "context path parent must be a directory: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|source| {
                    ContextVaultError::io("create context directory", &current, source)
                })?;
            }
            Err(error) => {
                return Err(ContextVaultError::io("read path metadata", &current, error));
            }
        }
    }

    Ok(())
}

fn ensure_existing_path_without_symlinks(root: &Path, relative_path: &Path) -> Result<()> {
    ensure_vault_root_directory(root)?;

    let mut current = root.to_path_buf();
    let mut components = relative_path.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(value) = component else {
            return Err(ContextVaultError::invalid_input(
                "path must stay inside the selected scope",
            ));
        };
        current.push(value);
        let is_final = components.peek().is_none();
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(ContextVaultError::invalid_input(format!(
                        "context path must not contain symbolic links: {}",
                        current.display()
                    )));
                }
                if !is_final && !metadata.is_dir() {
                    return Err(ContextVaultError::invalid_input(format!(
                        "context path parent must be a directory: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(ContextVaultError::io("read path metadata", &current, error));
            }
        }
    }

    Ok(())
}

fn reject_existing_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(ContextVaultError::invalid_input(format!(
                "context path must not contain symbolic links: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ContextVaultError::io("read path metadata", path, error)),
    }
}

fn scope_includes_document(search_scope: SearchScope, document_scope: Scope) -> bool {
    match search_scope {
        SearchScope::All => true,
        SearchScope::Personal => matches!(document_scope, Scope::Personal | Scope::Profile),
        SearchScope::Profile => document_scope == Scope::Profile,
        SearchScope::Work => matches!(document_scope, Scope::Work | Scope::Profile),
    }
}

#[cfg(all(test, unix))]
mod exact_read_tests {
    use std::os::unix::fs::symlink;

    use sha2::{Digest as _, Sha256};

    use super::*;
    use crate::{document::read_observation, test_support::TempDirectory};

    fn fixture(name: &str) -> (TempDirectory, PathBuf) {
        let temp = TempDirectory::new(name);
        let root = temp
            .path()
            .canonicalize()
            .expect("fixture root must resolve");
        fs::create_dir_all(root.join("personal/projects")).expect("fixture scope must exist");
        (temp, root)
    }

    fn write_fixture(root: &Path, path: &str, content: impl AsRef<[u8]>) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("fixture path has a parent"))
            .expect("fixture parent must exist");
        fs::write(path, content).expect("fixture must be written");
    }

    fn read_paths(root: &Path, scope: Scope, paths: &[&str]) -> Result<String> {
        read_context(
            root,
            scope,
            &paths.iter().map(PathBuf::from).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn exact_read_redacts_outputs_but_digests_original_bytes_and_keeps_export_false() {
        let (_temp, root) = fixture("exact-redaction");
        let raw = "---\ntitle: 'api_key: synthetic-title-secret'\nscope: personal\nexport: false\n---\n# Fixture\napi_key: synthetic-body-secret\n";
        write_fixture(&root, "personal/projects/a.md", raw);
        write_fixture(&root, "work/projects/a.md", "# Work\nNever include this.");
        write_fixture(
            &root,
            "profile/projects/a.md",
            "# Profile\nNever include this.",
        );
        let before = snapshot(&root);
        read_observation::reset();

        let response = read_paths(&root, Scope::Personal, &["projects/a"])
            .expect("exact read must include an export:false source");
        let json: serde_json::Value = serde_json::from_str(&response).expect("response is JSON");
        let document = &json["documents"][0];
        assert_eq!(json.as_object().expect("response is object").len(), 1);
        assert_eq!(document.as_object().expect("document is object").len(), 5);
        assert_eq!(document["scope"], "personal");
        assert_eq!(document["path"], "personal/projects/a.md");
        assert_eq!(
            document["source_digest"],
            Sha256::digest(raw.as_bytes())
                .iter()
                .fold(String::new(), |mut digest, byte| {
                    use std::fmt::Write as _;
                    write!(digest, "{byte:02x}").expect("writing to a String cannot fail");
                    digest
                })
        );
        assert!(!response.contains("synthetic-title-secret"));
        assert!(!response.contains("synthetic-body-secret"));
        assert!(
            document["title"]
                .as_str()
                .expect("title is string")
                .contains("[REDACTED]")
        );
        assert!(
            document["body"]
                .as_str()
                .expect("body is string")
                .contains("[REDACTED]")
        );
        assert_eq!(read_observation::counts(), (1, 0));
        assert_eq!(
            snapshot(&root),
            before,
            "exact read must preserve files and metadata"
        );
        assert!(
            !export_context(&root, SearchScope::Personal)
                .expect("baseline export must still work")
                .contains("personal/projects/a.md")
        );
    }

    #[test]
    fn exact_read_has_linear_content_reads_and_zero_directory_enumerations() {
        let (_temp, root) = fixture("exact-operation-counts");
        for index in 0..100 {
            write_fixture(
                &root,
                &format!("personal/projects/{index}.md"),
                "# Fixture\nBody",
            );
        }
        // An unrelated malformed Markdown file must never be read.
        write_fixture(&root, "personal/projects/unrequested.md", [0xff]);
        let before = snapshot(&root);
        for count in [0, 1, 100, 101] {
            let paths = (0..count)
                .map(|index| PathBuf::from(format!("projects/{index}.md")))
                .collect::<Vec<_>>();
            read_observation::reset();
            let result = read_context(&root, Scope::Personal, &paths);
            if matches!(count, 1 | 100) {
                let result = result.expect("bounded exact batch must succeed");
                let json: serde_json::Value =
                    serde_json::from_str(&result).expect("response is JSON");
                assert_eq!(
                    json["documents"]
                        .as_array()
                        .expect("documents is array")
                        .len(),
                    count
                );
                assert_eq!(read_observation::counts(), (count, 0));
                assert!(result.len() <= MAX_EXACT_READ_JSON_BYTES);
            } else {
                assert!(result.is_err());
                assert_eq!(read_observation::counts(), (0, 0));
            }
        }
        assert_eq!(snapshot(&root), before);
        assert!(!root.join("index").exists());
    }

    #[test]
    fn exact_read_rejects_unsafe_paths_before_content_reads() {
        let (_temp, root) = fixture("exact-unsafe-paths");
        for path in [
            "",
            ".",
            "./projects/a.md",
            "../work/a.md",
            "/personal/a.md",
            "projects/../../work/a.md",
            "personal/a.md",
            "work/a.md",
            "profile/a.md",
            "projects/",
            "projects/.",
            "projects/*.md",
            "**/a.md",
            "projects/a?.md",
            "projects/[a].md",
            "projects/{a,b}.md",
            "projects/a\\b.md",
            "projects/a.txt",
            "projects/index/a.md",
            "projects/target/a.md",
            "projects/INDEX/a.md",
            ".env.md",
            ".ssh/key.md",
            "secrets/a.md",
            "credentials.md",
            "journal/a.md",
            "projects/journal/a.md",
            "conversations/raw/a.md",
            "projects/conversations/raw/a.md",
            "projects/CONVERSATIONS/RAW/a.md",
            "projects/a\0.md",
            "projects/a\n.md",
        ] {
            read_observation::reset();
            assert!(
                read_paths(&root, Scope::Personal, &[path]).is_err(),
                "must reject {path:?}"
            );
            assert_eq!(read_observation::counts(), (0, 0));
        }
        assert!(read_paths(&root, Scope::Other, &["projects/a.md"]).is_err());
        for paths in [
            ["projects/a", "projects//a.md"],
            ["projects/a.md", "projects/./a.md"],
        ] {
            let error = read_paths(&root, Scope::Personal, &paths)
                .expect_err("normalized duplicates must fail");
            assert!(error.to_string().contains("unique after normalization"));
        }
    }

    #[test]
    fn exact_read_enforces_path_bytes_after_normalization() {
        let scope_prefix_bytes = "personal/".len();
        let path = format!("{}/{}.md", "a".repeat(250), "b".repeat(249));
        assert_eq!(path.len() + scope_prefix_bytes, MAX_EXACT_READ_PATH_BYTES);
        normalize_exact_read_paths(Scope::Personal, &[PathBuf::from(&path)])
            .expect("512-byte scoped path must normalize");
        let too_long = path.replacen('b', "bb", 1);
        assert!(normalize_exact_read_paths(Scope::Personal, &[PathBuf::from(too_long)]).is_err());
        let multibyte = format!("{}.md", "한".repeat(171));
        assert!(normalize_exact_read_paths(Scope::Personal, &[PathBuf::from(multibyte)]).is_err());
    }

    #[test]
    fn exact_read_rejects_scope_mismatch_and_malformed_documents_without_source_text() {
        let (_temp, root) = fixture("exact-malformed");
        for raw in [
            "---\nscope: work\n---\nprivate-body-marker",
            "---\nscope: [private-body-marker]\n---\nBody",
            "---\ntitle: [private-body-marker\n---\nBody",
            "---\ntitle: First\ntitle: private-body-marker\n---\nBody",
            "---\naliases: private-body-marker\n---\nBody",
            "---\nexport: private-body-marker\n---\nBody",
            "---\ntitle: private-body-marker\nBody without a closing delimiter",
        ] {
            write_fixture(&root, "personal/projects/a.md", raw);
            let error = read_paths(&root, Scope::Personal, &["projects/a.md"])
                .expect_err("malformed or mismatched metadata must fail");
            assert!(!error.to_string().contains("private-body-marker"));
        }
        write_fixture(&root, "personal/projects/a.md", [0xff, 0xfe]);
        assert!(read_paths(&root, Scope::Personal, &["projects/a.md"]).is_err());
        for scope in [Scope::Personal, Scope::Profile, Scope::Work] {
            write_fixture(
                &root,
                &format!("{}/projects/a.md", scope.as_str()),
                "# Valid\nBody",
            );
            let output = read_paths(&root, scope, &["projects/a.md"])
                .expect("each explicit scope must work");
            let json: serde_json::Value = serde_json::from_str(&output).expect("response is JSON");
            assert_eq!(json["documents"][0]["scope"], scope.as_str());
        }
    }

    #[test]
    fn exact_read_missing_roots_scopes_and_files_do_not_create_anything() {
        let (_temp, root) = fixture("exact-missing");
        let before = snapshot(&root);
        assert!(read_paths(&root.join("missing"), Scope::Personal, &["projects/a.md"]).is_err());
        assert!(read_paths(&root, Scope::Work, &["projects/a.md"]).is_err());
        assert!(read_paths(&root, Scope::Personal, &["projects/a.md"]).is_err());
        assert_eq!(snapshot(&root), before);
        write_fixture(
            &root,
            "personal/projects/a.md",
            "# First\nNo partial result.",
        );
        read_observation::reset();
        assert!(
            read_paths(
                &root,
                Scope::Personal,
                &["projects/a.md", "projects/missing.md"]
            )
            .is_err()
        );
        assert_eq!(read_observation::counts(), (1, 0));
    }

    #[test]
    fn exact_read_enforces_file_and_encoded_response_limits() {
        let (_temp, root) = fixture("exact-size-limits");
        let limit = usize::try_from(MAX_EXACT_READ_FILE_BYTES).expect("64 KiB fits usize");
        write_fixture(&root, "personal/projects/a.md", "a".repeat(limit));
        read_paths(&root, Scope::Personal, &["projects/a.md"]).expect("64 KiB must be allowed");
        write_fixture(&root, "personal/projects/a.md", "a".repeat(limit + 1));
        read_observation::reset();
        let error = read_paths(&root, Scope::Personal, &["projects/a.md"])
            .expect_err("oversized file must fail");
        assert!(error.to_string().contains("per-file read limit"));
        assert_eq!(read_observation::counts(), (0, 0));
        read_markdown_source(&root, &root.join("personal/projects/a.md"))
            .expect("shared source reader must retain its existing larger limit");

        let paths = (0..9)
            .map(|index| {
                let path = format!("projects/{index}.md");
                write_fixture(&root, &format!("personal/{path}"), "\"".repeat(limit));
                PathBuf::from(path)
            })
            .collect::<Vec<_>>();
        assert!(read_context(&root, Scope::Personal, &paths[..7]).is_ok());
        let error = read_context(&root, Scope::Personal, &paths)
            .expect_err("JSON escaping counts against response limit");
        assert!(error.to_string().contains("1 MiB"));
    }

    #[test]
    fn exact_read_rejects_root_scope_parent_and_file_symlinks() {
        let (_temp, root) = fixture("exact-symlinks");
        write_fixture(&root, "personal/projects/a.md", "# Fixture\nBody");
        symlink(&root, root.join("alias")).expect("root alias must be created");
        symlink(root.join("personal"), root.join("work")).expect("scope alias must be created");
        symlink(root.join("personal/projects"), root.join("personal/linked"))
            .expect("parent alias must be created");
        symlink(
            root.join("personal/projects/a.md"),
            root.join("personal/projects/link.md"),
        )
        .expect("file alias must be created");
        for (selected_root, scope, path) in [
            (root.join("alias"), Scope::Personal, "projects/a.md"),
            (root.join("alias/alias"), Scope::Personal, "projects/a.md"),
            (root.clone(), Scope::Work, "projects/a.md"),
            (root.clone(), Scope::Personal, "linked/a.md"),
            (root.clone(), Scope::Personal, "projects/link.md"),
        ] {
            read_observation::reset();
            assert!(read_paths(&selected_root, scope, &[path]).is_err());
            assert_eq!(read_observation::counts(), (0, 0));
        }
        let outside = TempDirectory::new("exact-outside");
        let outside = outside
            .path()
            .canonicalize()
            .expect("outside root must resolve");
        write_fixture(&outside, "a.md", "# Outside\nBody");
        symlink(&outside, root.join("personal/outside")).expect("outside alias must be created");
        assert!(read_paths(&root, Scope::Personal, &["outside/a.md"]).is_err());
    }

    #[test]
    fn exact_read_rejects_hardlinks_directories_and_fifos() {
        let (_temp, root) = fixture("exact-nonregular");
        write_fixture(&root, "personal/projects/a.md", "# Fixture\nBody");
        fs::hard_link(root.join("personal/projects/a.md"), root.join("linked.md"))
            .expect("hard link must be created");
        fs::create_dir(root.join("personal/projects/directory.md"))
            .expect("directory fixture must exist");
        let fifo = root.join("personal/projects/fifo.md");
        let fifo = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes())
            .expect("fixture path has no NUL");
        // SAFETY: the fixture path is a live NUL-terminated CString and mode is valid.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        for path in ["projects/a.md", "projects/directory.md", "projects/fifo.md"] {
            read_observation::reset();
            assert!(read_paths(&root, Scope::Personal, &[path]).is_err());
            assert_eq!(read_observation::counts(), (0, 0));
        }
    }

    #[test]
    fn exact_read_detects_in_place_changes_replacements_and_parent_replacements() {
        for mutation in 0..4 {
            let (_temp, root) = fixture("exact-changing-path");
            write_fixture(&root, "personal/projects/a.md", "# Original\nBody");
            let changing_root = root.clone();
            read_observation::set_before_read(move |path| match mutation {
                0 => fs::write(path, "# Modified\nBody").expect("content mutation must work"),
                1 => {
                    fs::rename(path, path.with_extension("old")).expect("original must move");
                    fs::write(path, "# Replaced\nBody").expect("replacement must exist");
                }
                2 => {
                    fs::rename(
                        changing_root.join("personal/projects"),
                        changing_root.join("personal/moved"),
                    )
                    .expect("parent must move");
                    write_fixture(&changing_root, "personal/projects/a.md", "# Replaced\nBody");
                }
                _ => fs::hard_link(path, path.with_extension("link"))
                    .expect("link mutation must work"),
            });
            assert!(
                read_paths(&root, Scope::Personal, &["projects/a.md"]).is_err(),
                "mutation {mutation} must fail"
            );
        }
    }

    #[test]
    fn exact_read_dependency_graph_excludes_traversal_index_database_and_source_writes() {
        use syn::visit::Visit as _;

        // Runtime counters measure content reads and enumeration. This syntax-tree
        // gate also closes the exact-read call graph against index/DB entry points,
        // including the pure parser and descriptor-owning methods.
        #[derive(Default)]
        struct Calls {
            functions: BTreeSet<String>,
            forbidden: Vec<String>,
        }
        impl<'ast> syn::visit::Visit<'ast> for Calls {
            fn visit_path(&mut self, path: &'ast syn::Path) {
                for segment in &path.segments {
                    let name = segment.ident.to_string();
                    if matches!(
                        name.as_str(),
                        "read_dir"
                            | "visit_directory"
                            | "sorted_directory_entries"
                            | "read_markdown_documents"
                            | "index_vault"
                            | "SqliteSearchIndex"
                            | "SearchIndex"
                            | "rusqlite"
                            | "Connection"
                            | "export_context"
                            | "init_vault"
                            | "save_context"
                            | "delete_context"
                            | "Command"
                            | "O_CREAT"
                            | "O_TRUNC"
                            | "O_WRONLY"
                            | "O_RDWR"
                    ) {
                        self.forbidden.push(name);
                    }
                }
                if let Some(segment) = path.segments.last() {
                    self.functions.insert(segment.ident.to_string());
                }
                syn::visit::visit_path(self, path);
            }
        }
        let files = [include_str!("document.rs"), include_str!("vault.rs")]
            .into_iter()
            .map(|source| syn::parse_file(source).expect("source must parse"))
            .collect::<Vec<_>>();
        let mut functions = std::collections::BTreeMap::new();
        let mut calls = Calls::default();
        for file in &files {
            for item in &file.items {
                match item {
                    syn::Item::Fn(function) => {
                        // The non-Unix implementation only returns an unsupported error.
                        functions
                            .entry(function.sig.ident.to_string())
                            .or_insert(&function.block);
                    }
                    syn::Item::Impl(implementation) => {
                        if let syn::Type::Path(path) = implementation.self_ty.as_ref()
                            && path.path.segments.last().is_some_and(|segment| {
                                segment.ident == "ExactMarkdownPath"
                                    || segment.ident == "BoundedReadJson"
                            })
                        {
                            calls.visit_item_impl(implementation);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut pending =
            BTreeSet::from(["parse_markdown_bytes".to_owned(), "read_context".to_owned()]);
        pending.append(&mut calls.functions);
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop_first() {
            if visited.insert(name.clone())
                && let Some(block) = functions.get(&name)
            {
                calls.visit_block(block);
                pending.append(&mut calls.functions);
            }
        }
        assert!(visited.contains("read_verified_file"));
        assert!(visited.contains("parse_markdown_source"));
        assert!(visited.contains("open_exact_child"));
        assert!(
            calls.forbidden.is_empty(),
            "forbidden dependency calls: {:?}",
            calls.forbidden
        );
    }

    #[derive(Debug, Eq, PartialEq)]
    struct SnapshotEntry {
        path: PathBuf,
        bytes: Vec<u8>,
        mode: u32,
        inode: u64,
        modified: (i64, i64),
    }

    fn snapshot(root: &Path) -> Vec<SnapshotEntry> {
        fn visit(root: &Path, current: &Path, entries: &mut Vec<SnapshotEntry>) {
            for entry in fs::read_dir(current).expect("fixture directory must be readable") {
                let path = entry.expect("fixture entry must exist").path();
                let metadata =
                    fs::symlink_metadata(&path).expect("fixture metadata must be readable");
                entries.push(SnapshotEntry {
                    path: path
                        .strip_prefix(root)
                        .expect("fixture stays in root")
                        .to_owned(),
                    bytes: if metadata.is_file() {
                        fs::read(&path).expect("fixture must be readable")
                    } else {
                        Vec::new()
                    },
                    mode: metadata.mode(),
                    inode: metadata.ino(),
                    modified: (metadata.mtime(), metadata.mtime_nsec()),
                });
                if metadata.is_dir() {
                    visit(root, &path, entries);
                }
            }
        }
        let mut entries = Vec::new();
        visit(root, root, &mut entries);
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        entries
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::test_support::TempDirectory;

    #[test]
    fn save_context_writes_scoped_markdown() {
        let temp = TempDirectory::new("save");

        let relative_path = save_context(
            temp.path(),
            Scope::Personal,
            "projects/llm-context-vault",
            "LLM Context Vault",
            "Durable context belongs in Markdown.",
        )
        .expect("context file should be saved");

        assert_eq!(
            relative_path,
            PathBuf::from("personal/projects/llm-context-vault.md")
        );
        let content = fs::read_to_string(temp.path().join(relative_path))
            .expect("saved context file should be readable");
        assert!(content.contains("title: \"LLM Context Vault\""));
        assert!(content.contains("scope: personal"));
        assert!(content.contains("Durable context belongs in Markdown."));
    }

    #[test]
    fn save_context_with_metadata_writes_language_and_aliases() {
        let temp = TempDirectory::new("save-metadata");

        let relative_path = save_context_with_metadata(
            temp.path(),
            Scope::Personal,
            "projects/llm-context-vault",
            "한국어 컨텍스트",
            "한국어로 저장하고 영어 별칭으로 검색한다.",
            ContextMetadata {
                language: Some(" ko ".to_owned()),
                aliases: vec![
                    "rust review".to_owned(),
                    "코드리뷰".to_owned(),
                    "rust review".to_owned(),
                ],
                exportable: Some(true),
            },
        )
        .expect("context file should be saved");

        let content = fs::read_to_string(temp.path().join(relative_path))
            .expect("saved context file should be readable");
        assert!(content.contains("language: \"ko\""));
        assert!(content.contains("  - \"rust review\""));
        assert!(content.contains("  - \"코드리뷰\""));
        assert_eq!(content.matches("rust review").count(), 1);
    }

    #[test]
    fn delete_context_removes_scoped_markdown() {
        let temp = TempDirectory::new("delete");
        save_context(
            temp.path(),
            Scope::Work,
            "projects/release.md",
            "Release Context",
            "Delete this fixture.",
        )
        .expect("context file should be saved");

        let relative_path = delete_context(temp.path(), Scope::Work, "projects/release.md")
            .expect("context file should be deleted");

        assert_eq!(relative_path, PathBuf::from("work/projects/release.md"));
        assert!(!temp.path().join(relative_path).exists());
    }

    #[test]
    fn delete_context_respects_the_save_lock() {
        let temp = TempDirectory::new("delete-save-lock");
        let relative = save_context(
            temp.path(),
            Scope::Personal,
            "projects/locked.md",
            "Locked",
            "Keep this fixture while the lock is held.",
        )
        .expect("context file should be saved");
        let path = temp.path().join(&relative);
        let _lock = ContextSaveLock::acquire(&path).expect("save lock should be acquired");

        let error = delete_context(temp.path(), Scope::Personal, "projects/locked.md")
            .expect_err("delete must not race an active save");

        assert!(error.to_string().contains("acquire context save lock"));
        assert!(path.exists());
    }

    #[test]
    fn rejects_paths_that_escape_scope() {
        let temp = TempDirectory::new("path-escape");

        let error = save_context(
            temp.path(),
            Scope::Personal,
            "../work/projects/leak.md",
            "Leak",
            "This should fail.",
        )
        .expect_err("parent directory traversal should be rejected");

        assert!(error.to_string().contains("inside the selected scope"));
    }

    #[test]
    fn rejects_paths_that_include_top_level_scope() {
        let temp = TempDirectory::new("path-scope-prefix");

        let error = save_context(
            temp.path(),
            Scope::Personal,
            "work/projects/leak.md",
            "Leak",
            "This should fail.",
        )
        .expect_err("scope-prefixed paths should be rejected");

        assert!(error.to_string().contains("within the selected scope"));
    }

    #[test]
    fn rejects_generated_artifact_directory_names() {
        let temp = TempDirectory::new("path-generated-artifact");

        let error = save_context(
            temp.path(),
            Scope::Personal,
            "projects/index/hidden.md",
            "Hidden",
            "This should fail.",
        )
        .expect_err("generated artifact directory names should be rejected");

        assert!(error.to_string().contains("generated artifact"));
        assert!(
            !temp
                .path()
                .join("personal/projects/index/hidden.md")
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_rejects_symbolic_linked_parent_directories() {
        use std::os::unix::fs::symlink;

        let temp = TempDirectory::new("save-symlink-parent");
        let outside = TempDirectory::new("save-symlink-parent-outside");
        symlink(outside.path(), temp.path().join("personal"))
            .expect("scope directory symlink should be created");

        let error = save_context(
            temp.path(),
            Scope::Personal,
            "projects/leak.md",
            "Leak",
            "This should fail.",
        )
        .expect_err("symbolic linked parent directories should be rejected");

        assert!(error.to_string().contains("symbolic links"));
        assert!(!outside.path().join("projects/leak.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn save_rejects_symbolic_linked_target_files() {
        use std::os::unix::fs::symlink;

        let temp = TempDirectory::new("save-symlink-target");
        let outside = TempDirectory::new("save-symlink-target-outside");
        let parent = temp.path().join("personal/projects");
        fs::create_dir_all(&parent).expect("context directory should be created");
        fs::write(outside.path().join("target.md"), "outside")
            .expect("outside fixture should be written");
        symlink(outside.path().join("target.md"), parent.join("link.md"))
            .expect("target file symlink should be created");

        let error = save_context(
            temp.path(),
            Scope::Personal,
            "projects/link.md",
            "Leak",
            "This should fail.",
        )
        .expect_err("symbolic linked target files should be rejected");

        assert!(error.to_string().contains("symbolic links"));
        let outside_content = fs::read_to_string(outside.path().join("target.md"))
            .expect("outside fixture should remain readable");
        assert_eq!(outside_content, "outside");
    }

    #[cfg(unix)]
    #[test]
    fn delete_rejects_symbolic_linked_parent_directories() {
        use std::os::unix::fs::symlink;

        let temp = TempDirectory::new("delete-symlink-parent");
        let outside = TempDirectory::new("delete-symlink-parent-outside");
        let outside_parent = outside.path().join("projects");
        fs::create_dir_all(&outside_parent).expect("outside directory should be created");
        fs::write(outside_parent.join("leak.md"), "outside")
            .expect("outside fixture should be written");
        symlink(outside.path(), temp.path().join("work"))
            .expect("scope directory symlink should be created");

        let error = delete_context(temp.path(), Scope::Work, "projects/leak.md")
            .expect_err("symbolic linked parent directories should be rejected");

        assert!(error.to_string().contains("symbolic links"));
        assert!(outside_parent.join("leak.md").exists());
    }

    #[test]
    fn export_context_keeps_personal_and_work_separate() {
        let temp = TempDirectory::new("export");
        save_context(
            temp.path(),
            Scope::Profile,
            "preferences/style.md",
            "Style",
            "Shared context.",
        )
        .expect("profile context should be saved");
        save_context(
            temp.path(),
            Scope::Personal,
            "projects/home.md",
            "Home",
            "Personal context with api_key: synthetic-secret.",
        )
        .expect("personal context should be saved");
        save_context(
            temp.path(),
            Scope::Work,
            "projects/company.md",
            "Company",
            "Work context.",
        )
        .expect("work context should be saved");

        let output =
            export_context(temp.path(), SearchScope::Personal).expect("context should be exported");

        assert!(output.contains("profile/preferences/style.md"));
        assert!(output.contains("personal/projects/home.md"));
        assert!(!output.contains("work/projects/company.md"));
        assert!(!output.contains("synthetic-secret"));
        assert!(output.contains("[REDACTED]"));
        assert!(output.starts_with("# LLM Context Vault Export"));
        assert!(output.contains("Use the user's requested language"));
    }

    #[test]
    fn export_context_includes_language_and_aliases() {
        let temp = TempDirectory::new("export-metadata");
        save_context_with_metadata(
            temp.path(),
            Scope::Personal,
            "projects/korean.md",
            "한국어 컨텍스트",
            "한국어 원문을 유지한다.",
            ContextMetadata {
                language: Some("ko".to_owned()),
                aliases: vec!["korean context".to_owned(), "한국어 맥락".to_owned()],
                exportable: Some(true),
            },
        )
        .expect("personal context should be saved");

        let output =
            export_context(temp.path(), SearchScope::Personal).expect("context should be exported");

        assert!(output.contains("- language: `ko`"));
        assert!(output.contains("- `korean context`"));
        assert!(output.contains("- `한국어 맥락`"));
        assert!(output.contains("한국어 원문을 유지한다."));
    }

    #[test]
    fn export_context_redacts_secret_like_aliases() {
        let temp = TempDirectory::new("export-redacted-alias");
        save_context_with_metadata(
            temp.path(),
            Scope::Personal,
            "projects/secret-alias.md",
            "Secret Alias",
            "Alias values must use the export redaction boundary.",
            ContextMetadata {
                language: None,
                aliases: vec!["sk-synthetic123456789".to_owned()],
                exportable: Some(true),
            },
        )
        .expect("personal context should be saved");

        let output =
            export_context(temp.path(), SearchScope::Personal).expect("context should be exported");

        assert!(!output.contains("sk-synthetic123456789"));
        assert!(output.contains("[REDACTED]"));
    }

    #[test]
    fn save_context_can_mark_source_as_not_exportable() {
        let temp = TempDirectory::new("save-exportable");

        let relative_path = save_context_with_metadata(
            temp.path(),
            Scope::Work,
            "agent-docs/rules/detail.md",
            "Detailed Rule",
            "Searchable detail.",
            ContextMetadata {
                language: None,
                aliases: Vec::new(),
                exportable: Some(false),
            },
        )
        .expect("work context should be saved");

        let content = fs::read_to_string(temp.path().join(relative_path))
            .expect("saved context file should be readable");
        assert!(content.contains("export: false"));
    }

    #[test]
    fn save_context_preserves_existing_metadata_when_not_specified() {
        let temp = TempDirectory::new("save-preserve-metadata");
        let detail = temp.path().join("work/agent-docs/rules/detail.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        fs::write(
            &detail,
            "---\ntitle: Detailed Rule\nlanguage: \"ko\"\naliases:\n  - \"code review\"\n  - \"코드리뷰\"\nexport: false\n---\n# Detailed Rule\nOld detail.",
        )
        .expect("detail fixture should be written");

        let relative_path = save_context(
            temp.path(),
            Scope::Work,
            "agent-docs/rules/detail.md",
            "Detailed Rule",
            "Updated detail.",
        )
        .expect("work context should be saved");

        let content = fs::read_to_string(temp.path().join(relative_path))
            .expect("saved context file should be readable");
        assert!(content.contains("language: \"ko\""));
        assert!(content.contains("  - \"code review\""));
        assert!(content.contains("  - \"코드리뷰\""));
        assert!(content.contains("export: false"));
        assert!(content.contains("Updated detail."));
    }

    #[test]
    fn save_context_preserves_unmanaged_frontmatter_verbatim() {
        let temp = TempDirectory::new("save-preserve-unmanaged-frontmatter");
        let detail = temp.path().join("personal/projects/ontology-metadata.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        let unmanaged = "# ontology projection\ntype: project\ndomain: engineering\nentities:\n  - project:llm-context-vault\nrelations:\n  - from: project:llm-context-vault\n    type: supports\n    to: knowledge:context-retrieval\napplies_to:\n  - writing\ncustom_map:\n  nested: 'quoted value'\n";
        fs::write(
            &detail,
            format!(
                "---\ntitle: Old Title\nscope: personal\nlanguage: ko\naliases:\n  - old alias\nexport: false\n{unmanaged}---\n\n# Old Title\nOld body."
            ),
        )
        .expect("detail fixture should be written");

        save_context(
            temp.path(),
            Scope::Personal,
            "projects/ontology-metadata.md",
            "New Title",
            "New body.",
        )
        .expect("context should be updated");

        let content = fs::read_to_string(&detail).expect("updated context should be readable");
        assert!(content.contains(unmanaged));
        assert!(content.contains("title: \"New Title\""));
        assert!(content.contains("# New Title\n\nNew body."));
        assert!(!content.contains("title: Old Title"));
    }

    #[test]
    fn save_context_handles_a_comment_before_the_first_alias() {
        let temp = TempDirectory::new("save-commented-managed-frontmatter");
        let detail = temp.path().join("personal/projects/commented.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        fs::write(
            &detail,
            "---\ntitle: Old Title\nscope: personal\naliases:\n# explain the first alias\n  - first\ncustom: keep\n---\n\n# Old Title\nOld body.",
        )
        .expect("detail fixture should be written");

        save_context(
            temp.path(),
            Scope::Personal,
            "projects/commented.md",
            "New Title",
            "New body.",
        )
        .expect("commented managed frontmatter should update safely");

        let content = fs::read_to_string(&detail).expect("updated context should be readable");
        assert!(content.contains("aliases:\n  - \"first\""));
        assert!(content.contains("custom: keep"));
        assert!(!content.contains("# explain the first alias"));
        read_markdown_source(temp.path(), &detail)
            .expect("updated frontmatter must remain valid YAML");
    }

    #[test]
    fn save_context_preserves_comment_only_frontmatter() {
        let temp = TempDirectory::new("save-comment-only-frontmatter");
        let detail = temp.path().join("personal/projects/comment-only.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        fs::write(
            &detail,
            "---\n# keep this source note\n---\n\n# Old Title\nOld body.",
        )
        .expect("detail fixture should be written");

        save_context(
            temp.path(),
            Scope::Personal,
            "projects/comment-only.md",
            "New Title",
            "New body.",
        )
        .expect("comment-only frontmatter should update safely");

        let content = fs::read_to_string(&detail).expect("updated context should be readable");
        assert!(content.contains("# keep this source note"));
        read_markdown_source(temp.path(), &detail)
            .expect("updated frontmatter must remain valid YAML");
    }

    #[test]
    fn save_context_rejects_unsupported_managed_flow_mapping() {
        let temp = TempDirectory::new("save-reject-managed-flow-mapping");
        let detail = temp.path().join("personal/projects/flow.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        let original = "---\n{title: Old Title, custom: keep}\n---\n\n# Old Title\nOld body.";
        fs::write(&detail, original).expect("detail fixture should be written");

        let error = save_context(
            temp.path(),
            Scope::Personal,
            "projects/flow.md",
            "New Title",
            "New body.",
        )
        .expect_err("unsupported managed key layout must be rejected");

        assert!(
            error
                .to_string()
                .contains("frontmatter keys must use one top-level YAML mapping entry")
        );
        assert_eq!(
            fs::read_to_string(&detail).expect("original context should remain readable"),
            original
        );
    }

    #[test]
    fn save_context_rejects_unmanaged_flow_mapping_without_modification() {
        let temp = TempDirectory::new("save-reject-unmanaged-flow-mapping");
        let detail = temp.path().join("personal/projects/unmanaged-flow.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        let original = "---\n{custom: keep}\n---\n\n# Old Title\nOld body.";
        fs::write(&detail, original).expect("detail fixture should be written");

        let error = save_context(
            temp.path(),
            Scope::Personal,
            "projects/unmanaged-flow.md",
            "New Title",
            "New body.",
        )
        .expect_err("unmanaged flow mapping must not be combined with block fields");

        assert!(
            error
                .to_string()
                .contains("frontmatter keys must use one top-level YAML mapping entry")
        );
        assert_eq!(
            fs::read_to_string(&detail).expect("original context should remain readable"),
            original
        );
    }

    #[test]
    fn save_context_preserves_a_plain_key_starting_with_a_dash() {
        let temp = TempDirectory::new("save-preserve-dash-key");
        let detail = temp.path().join("personal/projects/dash-key.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        fs::write(
            &detail,
            "---\ntitle: Old\nscope: personal\n-custom: keep\n---\n\n# Old\n\nOld body.\n",
        )
        .expect("detail fixture should be written");

        save_context(
            temp.path(),
            Scope::Personal,
            "projects/dash-key.md",
            "Updated",
            "Updated body.",
        )
        .expect("a valid plain YAML key should be preserved");

        let updated = fs::read_to_string(detail).expect("updated context should be readable");
        assert!(updated.contains("-custom: keep\n"));
        ParsedFrontmatter::from_markdown(&updated)
            .expect("updated frontmatter must remain valid YAML");
    }

    #[test]
    fn save_context_preserves_an_alias_key_using_full_yaml_context() {
        let temp = TempDirectory::new("save-preserve-alias-key");
        let detail = temp.path().join("personal/projects/alias-key.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        fs::write(
            &detail,
            "---\ntitle: Old\nscope: personal\nkey_name: &custom-key custom\n*custom-key: keep\n---\n\n# Old\n\nOld body.\n",
        )
        .expect("detail fixture should be written");

        save_context(
            temp.path(),
            Scope::Personal,
            "projects/alias-key.md",
            "Updated",
            "Updated body.",
        )
        .expect("a YAML alias key should be preserved using its full document context");

        let updated = fs::read_to_string(detail).expect("updated context should be readable");
        assert!(updated.contains("key_name: &custom-key custom\n*custom-key: keep\n"));
        ParsedFrontmatter::from_markdown(&updated)
            .expect("updated frontmatter must remain valid YAML");
    }

    #[test]
    fn save_context_rejects_a_managed_anchor_used_by_unmanaged_frontmatter() {
        let temp = TempDirectory::new("save-reject-cross-field-anchor");
        let detail = temp.path().join("personal/projects/anchor.md");
        fs::create_dir_all(detail.parent().expect("fixture must have a parent"))
            .expect("fixture parent should be created");
        let original = "---\ntitle: &shared-title Original\nscope: personal\ncustom_title: *shared-title\n---\n\n# Original\n\nOriginal body.\n";
        fs::write(&detail, original).expect("fixture should be written");

        let error = save_context(
            temp.path(),
            Scope::Personal,
            "projects/anchor.md",
            "Updated",
            "Updated body.",
        )
        .expect_err("an unresolved preserved alias must be rejected");

        assert!(error.to_string().contains("unknown anchor"));
        assert_eq!(
            fs::read_to_string(detail).expect("source should remain readable"),
            original
        );
    }

    #[test]
    fn save_context_rejects_yaml_control_characters_before_writing() {
        let temp = TempDirectory::new("save-reject-yaml-control-character");

        let error = save_context(
            temp.path(),
            Scope::Personal,
            "projects/control.md",
            "Invalid\u{1} title",
            "Body",
        )
        .expect_err("a YAML control character must be rejected");

        assert!(
            error
                .to_string()
                .contains("control characters are not allowed")
        );
        assert!(!temp.path().join("personal/projects/control.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_update_preserves_existing_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDirectory::new("save-preserve-permissions");
        let detail = temp.path().join("personal/projects/private.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        fs::write(&detail, "# Private\nOriginal body.").expect("private fixture should be written");
        fs::set_permissions(&detail, fs::Permissions::from_mode(0o600))
            .expect("private fixture permissions should be set");

        save_context(
            temp.path(),
            Scope::Personal,
            "projects/private.md",
            "Private",
            "Updated body.",
        )
        .expect("private context should update");

        let mode = fs::metadata(&detail)
            .expect("updated context metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn atomic_update_preserves_an_existing_extended_acl() {
        let temp = TempDirectory::new("save-preserve-extended-acl");
        let detail = temp.path().join("personal/projects/acl.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        fs::write(&detail, "# ACL\nOriginal body.").expect("ACL fixture should be written");
        let status = std::process::Command::new("chmod")
            .arg("+a")
            .arg("everyone deny write")
            .arg(&detail)
            .status()
            .expect("macOS chmod must be available for the ACL fixture");
        assert!(status.success());
        let original_acl = crate::atomic_file::extended_acl_text(&detail)
            .expect("fixture extended ACL should be inspectable")
            .expect("fixture must have an extended ACL");

        save_context(
            temp.path(),
            Scope::Personal,
            "projects/acl.md",
            "ACL",
            "Updated body.",
        )
        .expect("ACL context should update");

        let updated_acl = crate::atomic_file::extended_acl_text(&detail)
            .expect("updated extended ACL should be inspectable")
            .expect("updated file must have an extended ACL");
        assert_eq!(updated_acl, original_acl);
    }

    #[test]
    fn save_context_rejects_duplicate_managed_frontmatter_keys() {
        let temp = TempDirectory::new("save-reject-duplicate-managed-frontmatter");
        let detail = temp.path().join("personal/projects/duplicate.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        let original =
            "---\ntitle: First\ntitle: Second\nscope: personal\n---\n\n# First\nOriginal body.";
        fs::write(&detail, original).expect("detail fixture should be written");

        let error = save_context(
            temp.path(),
            Scope::Personal,
            "projects/duplicate.md",
            "Updated",
            "Updated body.",
        )
        .expect_err("duplicate managed frontmatter must be rejected");

        let message = error.to_string();
        assert!(message.contains("duplicate") && message.contains("title"));
        assert_eq!(
            fs::read_to_string(&detail).expect("original context should remain readable"),
            original
        );
    }

    #[test]
    fn save_context_replaces_quoted_managed_frontmatter_keys() {
        let temp = TempDirectory::new("save-quoted-managed-frontmatter");
        let detail = temp.path().join("personal/projects/quoted.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        fs::write(
            &detail,
            "---\n'title': Old Title\n\"scope\": personal\ncustom: keep\n---\n\n# Old Title\nOld body.",
        )
        .expect("detail fixture should be written");

        save_context(
            temp.path(),
            Scope::Personal,
            "projects/quoted.md",
            "New Title",
            "New body.",
        )
        .expect("quoted managed keys should be replaced");

        let content = fs::read_to_string(&detail).expect("updated context should be readable");
        assert!(content.contains("title: \"New Title\""));
        assert!(content.contains("scope: personal"));
        assert!(content.contains("custom: keep"));
        assert!(!content.contains("'title':"));
        assert!(!content.contains("\"scope\":"));
    }

    #[test]
    fn atomic_update_rejects_a_stale_content_digest() {
        let temp = TempDirectory::new("save-stale-content-digest");
        let detail = temp.path().join("personal/projects/stale.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        fs::write(&detail, "# Original\nFirst version.")
            .expect("original fixture should be written");
        let original = read_markdown_source(temp.path(), &detail)
            .expect("original fixture should be readable");
        fs::write(&detail, "# Concurrent\nSecond version.")
            .expect("concurrent update fixture should be written");

        let error = write_context_atomically(
            temp.path(),
            &detail,
            b"# Stale\nShould not replace the concurrent update.",
            Some(original.content_digest()),
        )
        .expect_err("stale update must be rejected");

        assert!(
            error
                .to_string()
                .contains("changed before the atomic update")
        );
        assert_eq!(
            fs::read_to_string(&detail).expect("concurrent update should remain readable"),
            "# Concurrent\nSecond version."
        );
    }

    #[test]
    fn export_context_excludes_documents_marked_not_exportable() {
        let temp = TempDirectory::new("export-filter");
        save_context(
            temp.path(),
            Scope::Work,
            "preferences/review.md",
            "Review Preference",
            "Export this concise preference.",
        )
        .expect("work context should be saved");
        let detail = temp.path().join("work/agent-docs/rules/code-review.md");
        fs::create_dir_all(detail.parent().expect("detail file should have a parent"))
            .expect("detail directory should be created");
        fs::write(
            &detail,
            "---\ntitle: Code Review Detail\nexport: false\n---\n# Code Review Detail\nSearch this, but do not export it.",
        )
        .expect("detail fixture should be written");

        let output =
            export_context(temp.path(), SearchScope::Work).expect("context should be exported");

        assert!(output.contains("Export this concise preference."));
        assert!(!output.contains("work/agent-docs/rules/code-review.md"));
        assert!(!output.contains("Search this, but do not export it."));
    }
}
