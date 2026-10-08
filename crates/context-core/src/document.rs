#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    ffi::OsStr,
    fmt::Write as _,
    fs::{self, File, Metadata},
    io::Read,
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};

use crate::{
    ContextError, Result,
    frontmatter::{ParsedFrontmatter, split_markdown_frontmatter},
    ontology::OntologyMetadata,
};

const MAX_MARKDOWN_DOCUMENTS: usize = 10_000;
const MAX_MARKDOWN_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_MARKDOWN_TOTAL_BYTES: usize = 100 * 1024 * 1024;
const MAX_MARKDOWN_DIRECTORY_DEPTH: usize = 128;

#[derive(Default)]
struct DocumentReadBudget {
    documents: usize,
    total_bytes: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MarkdownSource {
    document: Document,
    frontmatter: Option<ParsedFrontmatter>,
    content_digest: String,
}

impl MarkdownSource {
    pub(crate) fn document(&self) -> &Document {
        &self.document
    }

    pub(crate) fn frontmatter(&self) -> Option<&ParsedFrontmatter> {
        self.frontmatter.as_ref()
    }

    pub(crate) fn content_digest(&self) -> &str {
        &self.content_digest
    }

    fn into_document(self) -> Document {
        self.document
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Document {
    source_path: String,
    scope: Scope,
    language: Option<String>,
    aliases: Vec<String>,
    exportable: bool,
    ontology: OntologyMetadata,
    title: String,
    body: String,
}

impl Document {
    #[must_use]
    pub fn new(
        source_path: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self::with_metadata(source_path, None, Vec::new(), title, body)
    }

    #[must_use]
    pub fn with_metadata(
        source_path: impl Into<String>,
        language: Option<String>,
        aliases: Vec<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        let source_path = source_path.into();
        let scope = scope_from_relative_path(Path::new(&source_path));
        Self {
            source_path,
            scope,
            language,
            aliases,
            exportable: true,
            ontology: OntologyMetadata::default(),
            title: title.into(),
            body: body.into(),
        }
    }

    #[must_use]
    pub fn source_path(&self) -> &str {
        &self.source_path
    }

    #[must_use]
    pub fn scope(&self) -> Scope {
        self.scope
    }

    #[must_use]
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    #[must_use]
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    #[must_use]
    pub fn exportable(&self) -> bool {
        self.exportable
    }

    #[must_use]
    pub fn ontology(&self) -> &OntologyMetadata {
        &self.ontology
    }

    /// Storage-neutral ontology text for redacted search projection.
    #[must_use]
    pub fn ontology_search_text(&self) -> String {
        self.ontology.search_text()
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Scope {
    Other,
    Personal,
    Profile,
    Work,
}

impl Scope {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Other => "other",
            Self::Personal => "personal",
            Self::Profile => "profile",
            Self::Work => "work",
        }
    }
}

pub fn read_markdown_documents(root: impl AsRef<Path>) -> Result<Vec<Document>> {
    let root = root.as_ref();
    ensure_readable_vault_root(root)?;
    let mut documents = Vec::new();
    let mut budget = DocumentReadBudget::default();
    visit_directory(root, root, &mut documents, &mut budget, 0)?;
    documents.sort_by(|left, right| left.source_path.cmp(&right.source_path));
    Ok(documents)
}

fn ensure_readable_vault_root(root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|source| ContextError::io("read context root metadata", root, source))?;
    if metadata.file_type().is_symlink() {
        return Err(ContextError::invalid_input(
            "context root must not be a symbolic link",
        ));
    }
    if !metadata.is_dir() {
        return Err(ContextError::invalid_input(
            "context root must be a directory",
        ));
    }
    Ok(())
}

fn visit_directory(
    root: &Path,
    current: &Path,
    documents: &mut Vec<Document>,
    budget: &mut DocumentReadBudget,
    depth: usize,
) -> Result<()> {
    if depth > MAX_MARKDOWN_DIRECTORY_DEPTH {
        return Err(ContextError::invalid_input(
            "vault directory nesting exceeds the read limit",
        ));
    }
    for path in sorted_directory_entries(current)? {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|source| ContextError::io("read file metadata", &path, source))?;
        if metadata.file_type().is_symlink() {
            continue;
        }

        if metadata.is_dir() {
            if should_skip_directory(root, &path) {
                continue;
            }
            visit_directory(root, &path, documents, budget, depth + 1)?;
        } else if metadata.is_file() && path.extension() == Some(OsStr::new("md")) {
            if budget.documents >= MAX_MARKDOWN_DOCUMENTS {
                return Err(ContextError::invalid_input(
                    "vault contains too many Markdown documents",
                ));
            }
            let (document, bytes) = read_markdown_document_with_size(root, &path)?;
            budget.documents += 1;
            budget.total_bytes = budget
                .total_bytes
                .checked_add(bytes)
                .ok_or_else(|| ContextError::invalid_input("Markdown input size overflow"))?;
            if budget.total_bytes > MAX_MARKDOWN_TOTAL_BYTES {
                return Err(ContextError::invalid_input(
                    "vault Markdown content exceeds the total read limit",
                ));
            }
            documents.push(document);
        }
    }
    Ok(())
}

fn sorted_directory_entries(path: &Path) -> Result<Vec<PathBuf>> {
    #[cfg(test)]
    read_observation::directory_enumeration();
    let mut entries = Vec::new();
    let read_dir =
        fs::read_dir(path).map_err(|source| ContextError::io("read directory", path, source))?;
    for entry in read_dir {
        let entry =
            entry.map_err(|source| ContextError::io("read directory entry", path, source))?;
        entries.push(entry.path());
    }
    entries.sort();
    Ok(entries)
}

fn read_markdown_document_with_size(root: &Path, path: &Path) -> Result<(Document, usize)> {
    read_markdown_source_with_size(root, path)
        .map(|(source, bytes)| (source.into_document(), bytes))
}

pub(crate) fn read_markdown_source(root: &Path, path: &Path) -> Result<MarkdownSource> {
    read_markdown_source_with_size(root, path).map(|(source, _)| source)
}

pub(crate) fn validate_context_markdown(markdown: &str) -> Result<()> {
    let frontmatter = ParsedFrontmatter::from_markdown(markdown)?;
    let Some(frontmatter) = frontmatter.as_ref() else {
        return Ok(());
    };
    frontmatter.optional_string("title")?;
    frontmatter.optional_string("scope")?;
    frontmatter.optional_string("language")?;
    frontmatter.string_list("aliases")?;
    frontmatter.optional_bool("export")?;
    OntologyMetadata::parse(Some(frontmatter))?;
    Ok(())
}

fn read_markdown_source_with_size(root: &Path, path: &Path) -> Result<(MarkdownSource, usize)> {
    let content = read_verified_markdown(root, path)?;
    parse_markdown_source(root, path, &content)
}

/// A parsed exact original; construction performs no file or database I/O.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ParsedMarkdownDocument {
    document: Document,
    declared_scope: Option<String>,
    original_content_digest: String,
}
impl ParsedMarkdownDocument {
    #[must_use]
    pub fn declared_scope(&self) -> Option<&str> {
        self.declared_scope.as_deref()
    }
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }
    #[must_use]
    pub fn original_content_digest(&self) -> &str {
        &self.original_content_digest
    }
}

/// Raw YAML bytes selected by the exact reader's shared splitter, without parsing
/// or copying it. Callers can preflight their own metadata resource budget.
#[must_use]
pub fn markdown_frontmatter_bytes(content: &str) -> usize {
    split_markdown_frontmatter(content).0.map_or(0, str::len)
}

/// Borrow the same trimmed body stored by the exact parser, without allocating
/// a Markdown tree or parsing YAML.
#[must_use]
pub fn markdown_body(content: &str) -> &str {
    split_markdown_frontmatter(content).1.trim()
}

/// Validated title and declared scope, without retaining another owned body.
/// This runs the exact reader's complete path, UTF-8 and metadata validation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ParsedMarkdownTitle {
    title: String,
    declared_scope: Option<String>,
}
impl ParsedMarkdownTitle {
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
    #[must_use]
    pub fn declared_scope(&self) -> Option<&str> {
        self.declared_scope.as_deref()
    }
}

pub fn parse_markdown_title_bytes(
    logical_path: &Path,
    bytes: &[u8],
) -> Result<ParsedMarkdownTitle> {
    let parsed = parse_markdown_exact(logical_path, bytes, false)?;
    Ok(ParsedMarkdownTitle {
        title: parsed.document.title,
        declared_scope: parsed.declared_scope,
    })
}

pub fn parse_markdown_bytes(logical_path: &Path, bytes: &[u8]) -> Result<ParsedMarkdownDocument> {
    parse_markdown_exact(logical_path, bytes, true)
}

fn parse_markdown_exact(
    logical_path: &Path,
    bytes: &[u8],
    retain_body: bool,
) -> Result<ParsedMarkdownDocument> {
    let path = logical_path
        .to_str()
        .ok_or_else(|| ContextError::invalid_input("Markdown path must be UTF-8"))?;
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', '\0'])
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || !logical_path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
    {
        return Err(ContextError::invalid_input(
            "Markdown path must be normalized and nonescaping",
        ));
    }
    if bytes.len() as u64 > MAX_MARKDOWN_FILE_BYTES {
        return Err(ContextError::invalid_input(
            "Markdown bytes exceed the document limit",
        ));
    }
    let content = std::str::from_utf8(bytes)
        .map_err(|_| ContextError::invalid_input("Markdown bytes must be UTF-8"))?;
    validate_closed_markdown_metadata(content)?;
    let source = parse_markdown_content_with_body(logical_path, content, retain_body)?.0;
    let declared_scope = source
        .frontmatter
        .as_ref()
        .map(|fields| fields.optional_string("scope"))
        .transpose()?
        .flatten();
    Ok(ParsedMarkdownDocument {
        declared_scope,
        document: source.document,
        original_content_digest: source.content_digest,
    })
}

fn validate_closed_markdown_metadata(content: &str) -> Result<()> {
    let (_, body) = split_markdown_frontmatter(content);
    if body == content
        && content
            .lines()
            .next()
            .is_some_and(|line| line.trim_start_matches('\u{feff}').trim() == "---")
    {
        return Err(ContextError::invalid_input(
            "invalid or unclosed Markdown metadata in exact read",
        ));
    }
    Ok(())
}

fn parse_markdown_source(
    root: &Path,
    path: &Path,
    content: &str,
) -> Result<(MarkdownSource, usize)> {
    let relative_path = path.strip_prefix(root).unwrap_or(path);
    parse_markdown_content(relative_path, content)
}

fn parse_markdown_content(path: &Path, content: &str) -> Result<(MarkdownSource, usize)> {
    parse_markdown_content_with_body(path, content, true)
}

// An omitted body is private to the title-only result; no public Document with
// an incomplete body escapes this helper.
fn parse_markdown_content_with_body(
    path: &Path,
    content: &str,
    retain_body: bool,
) -> Result<(MarkdownSource, usize)> {
    let content_bytes = content.len();
    // The title-only API returns no digest. Its native caller has already
    // checked the full original SHA before parsing; avoid a discarded second
    // hash. Complete Document parsing continues to compute its source digest.
    let content_digest = if retain_body {
        sha256_hex(content.as_bytes())
    } else {
        String::new()
    };
    let (raw_frontmatter, body) = split_markdown_frontmatter(content);
    let frontmatter = ParsedFrontmatter::parse(raw_frontmatter)?;
    let relative_path = path;
    let title = frontmatter
        .as_ref()
        .map(|fields| fields.optional_string("title"))
        .transpose()?
        .flatten()
        .or_else(|| first_heading(body))
        .unwrap_or_else(|| title_from_path(path));
    let language = frontmatter
        .as_ref()
        .map(|fields| fields.optional_string("language"))
        .transpose()?
        .flatten();
    let aliases = frontmatter
        .as_ref()
        .map_or_else(|| Ok(Vec::new()), |fields| fields.string_list("aliases"))?;
    let exportable = frontmatter
        .as_ref()
        .map(|fields| fields.optional_bool("export"))
        .transpose()?
        .flatten()
        .unwrap_or_else(|| !is_original_import_path(relative_path));
    let ontology = OntologyMetadata::parse(frontmatter.as_ref())?;
    let source_path = relative_path.to_string_lossy().into_owned();

    let mut document = Document::with_metadata(
        source_path,
        language,
        aliases,
        title,
        if retain_body {
            markdown_body(content).to_owned()
        } else {
            String::new()
        },
    );
    document.exportable = exportable;
    document.ontology = ontology;
    Ok((
        MarkdownSource {
            document,
            frontmatter,
            content_digest,
        },
        content_bytes,
    ))
}

fn sha256_hex(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn read_verified_markdown(root: &Path, path: &Path) -> Result<String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|source| ContextError::io("resolve context root", root, source))?;
    let mut file =
        File::open(path).map_err(|source| ContextError::io("open Markdown file", path, source))?;
    let before = file
        .metadata()
        .map_err(|source| ContextError::io("inspect opened Markdown file", path, source))?;
    if !before.is_file() {
        return Err(ContextError::invalid_input(
            "Markdown input must be a regular file",
        ));
    }
    let canonical_path = path
        .canonicalize()
        .map_err(|source| ContextError::io("resolve Markdown file", path, source))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(ContextError::invalid_input(
            "Markdown input must remain inside the context root",
        ));
    }
    let canonical_file = File::open(&canonical_path).map_err(|source| {
        ContextError::io("open resolved Markdown file", &canonical_path, source)
    })?;
    let canonical_metadata = canonical_file.metadata().map_err(|source| {
        ContextError::io("inspect resolved Markdown file", &canonical_path, source)
    })?;
    if !same_open_file(&before, &canonical_metadata) {
        return Err(ContextError::invalid_input(
            "Markdown path changed while it was being opened",
        ));
    }
    read_verified_file(&mut file, path, &before, MAX_MARKDOWN_FILE_BYTES)
}

fn read_verified_file(
    file: &mut File,
    path: &Path,
    before: &Metadata,
    max_bytes: u64,
) -> Result<String> {
    if before.len() > max_bytes {
        return Err(ContextError::invalid_input(
            "Markdown file exceeds the per-file read limit",
        ));
    }
    let mut content = String::new();
    #[cfg(test)]
    read_observation::content_read();
    #[cfg(test)]
    read_observation::before_content_read(path);
    file.by_ref()
        .take(max_bytes + 1)
        .read_to_string(&mut content)
        .map_err(|source| ContextError::io("read Markdown file", path, source))?;
    if content.len() as u64 > max_bytes {
        return Err(ContextError::invalid_input(
            "Markdown file exceeds the per-file read limit",
        ));
    }
    let after = file
        .metadata()
        .map_err(|source| ContextError::io("reinspect Markdown file", path, source))?;
    if !same_open_file_version(before, &after) {
        return Err(ContextError::invalid_input(
            "Markdown file changed while it was being read",
        ));
    }
    Ok(content)
}

/// Use descriptor-relative, no-follow opens for the exact-read boundary only.
/// Traversal and save retain their existing path and size semantics.
#[cfg(unix)]
pub(crate) fn read_exact_markdown_source(
    root: &Path,
    relative_path: &Path,
    max_bytes: u64,
) -> Result<MarkdownSource> {
    let path = root.join(relative_path);
    let mut opened = ExactMarkdownPath::open(&path)?;
    let content = read_verified_file(&mut opened.file, &path, &opened.metadata, max_bytes)?;
    opened.verify_current()?;
    validate_closed_markdown_metadata(&content)?;
    // Parser diagnostics can contain source text. Do not expose it on this boundary.
    parse_markdown_source(root, &path, &content)
        .map(|(source, _)| source)
        .map_err(|_| ContextError::invalid_input("invalid Markdown metadata in exact read"))
}

#[cfg(not(unix))]
pub(crate) fn read_exact_markdown_source(
    _root: &Path,
    _relative_path: &Path,
    _max_bytes: u64,
) -> Result<MarkdownSource> {
    Err(ContextError::invalid_input(
        "exact context reads require Unix no-follow file access",
    ))
}

#[cfg(unix)]
struct ExactMarkdownPath {
    // Each directory remains open until the final path and file are rechecked.
    directories: Vec<(File, std::ffi::OsString)>,
    file: File,
    metadata: Metadata,
    path: PathBuf,
}

#[cfg(unix)]
impl ExactMarkdownPath {
    fn open(path: &Path) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt as _;

        let mut components = path.components();
        if components.next() != Some(std::path::Component::RootDir) {
            return Err(ContextError::invalid_input(
                "exact read requires an absolute resolved root",
            ));
        }
        let mut parent = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open("/")
            .map_err(|source| ContextError::io("open filesystem root", "/", source))?;
        let mut directories = Vec::new();
        let mut components = components.peekable();
        while let Some(component) = components.next() {
            let std::path::Component::Normal(name) = component else {
                return Err(ContextError::invalid_input(
                    "exact read root must not contain parent traversal",
                ));
            };
            let is_directory = components.peek().is_some();
            let opened = open_exact_child(&parent, name, path, is_directory)?;
            directories.push((parent, name.to_owned()));
            if is_directory {
                parent = opened;
            } else {
                let metadata = opened.metadata().map_err(|source| {
                    ContextError::io("inspect exact Markdown file", path, source)
                })?;
                validate_exact_file(&metadata)?;
                return Ok(Self {
                    directories,
                    file: opened,
                    metadata,
                    path: path.to_path_buf(),
                });
            }
        }
        Err(ContextError::invalid_input("missing exact Markdown path"))
    }

    fn verify_current(&self) -> Result<()> {
        for (index, (parent, name)) in self.directories.iter().enumerate() {
            let next = self.directories.get(index + 1);
            let current = open_exact_child(parent, name, &self.path, next.is_some())?;
            let current = current.metadata().map_err(|source| {
                ContextError::io("reinspect exact Markdown path", &self.path, source)
            })?;
            let expected = next.map_or(&self.file, |(directory, _)| directory);
            let expected = expected.metadata().map_err(|source| {
                ContextError::io("reinspect pinned Markdown path", &self.path, source)
            })?;
            if !same_open_file(&current, &expected) {
                return Err(ContextError::invalid_input(
                    "Markdown path changed while it was being read",
                ));
            }
            if next.is_none() {
                validate_exact_file(&current)?;
                if !same_open_file_version(&self.metadata, &current) {
                    return Err(ContextError::invalid_input(
                        "Markdown file changed while it was being read",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn open_exact_child(parent: &File, name: &OsStr, path: &Path, directory: bool) -> Result<File> {
    use std::os::{
        fd::{AsRawFd as _, FromRawFd as _},
        unix::ffi::OsStrExt as _,
    };

    let name = std::ffi::CString::new(name.as_bytes())
        .map_err(|_| ContextError::invalid_input("exact read path contains a NUL byte"))?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | if directory {
            libc::O_DIRECTORY
        } else {
            libc::O_NONBLOCK
        };
    // SAFETY: parent owns a live descriptor; name is NUL-terminated and valid
    // throughout openat. No creation flags are used, so no mode argument is needed.
    let descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if descriptor < 0 {
        return Err(ContextError::io(
            "open exact Markdown path without symbolic links",
            path,
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: successful openat returned a new descriptor, owned only here.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn validate_exact_file(metadata: &Metadata) -> Result<()> {
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(ContextError::invalid_input(
            "exact Markdown input must be a regular file with one hard link",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod read_observation {
    use std::{
        cell::{Cell, RefCell},
        path::Path,
    };

    type ReadHook = Box<dyn FnOnce(&Path)>;

    thread_local! {
        static COUNTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
        static BEFORE_READ: RefCell<Option<ReadHook>> = RefCell::new(None);
    }

    pub(super) fn content_read() {
        COUNTS.with(|counts| {
            let (reads, enumerations) = counts.get();
            counts.set((reads + 1, enumerations));
        });
    }

    pub(super) fn directory_enumeration() {
        COUNTS.with(|counts| {
            let (reads, enumerations) = counts.get();
            counts.set((reads, enumerations + 1));
        });
    }

    pub(crate) fn reset() {
        COUNTS.set((0, 0));
    }
    pub(crate) fn counts() -> (usize, usize) {
        COUNTS.get()
    }

    pub(crate) fn set_before_read(hook: impl FnOnce(&Path) + 'static) {
        BEFORE_READ.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    }

    pub(super) fn before_content_read(path: &Path) {
        let hook = BEFORE_READ.with(|slot| slot.borrow_mut().take());
        if let Some(hook) = hook {
            hook(path);
        }
    }
}

#[cfg(unix)]
fn same_open_file(first: &Metadata, second: &Metadata) -> bool {
    first.dev() == second.dev() && first.ino() == second.ino()
}

#[cfg(not(unix))]
fn same_open_file(first: &Metadata, second: &Metadata) -> bool {
    first.len() == second.len() && first.modified().ok() == second.modified().ok()
}

#[cfg(unix)]
fn same_open_file_version(first: &Metadata, second: &Metadata) -> bool {
    same_open_file(first, second)
        && first.len() == second.len()
        && first.ctime() == second.ctime()
        && first.ctime_nsec() == second.ctime_nsec()
}

#[cfg(not(unix))]
fn same_open_file_version(first: &Metadata, second: &Metadata) -> bool {
    same_open_file(first, second)
}

fn is_original_import_path(path: &Path) -> bool {
    let mut previous_was_notion = false;

    for component in path.components().filter_map(|component| match component {
        std::path::Component::Normal(value) => value.to_str(),
        _ => None,
    }) {
        if previous_was_notion && component == "originals" {
            return true;
        }
        previous_was_notion = component == "notion";
    }

    false
}

fn scope_from_relative_path(path: &Path) -> Scope {
    match path
        .components()
        .next()
        .and_then(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        }) {
        Some("personal") => Scope::Personal,
        Some("profile") => Scope::Profile,
        Some("work") => Scope::Work,
        _ => Scope::Other,
    }
}

fn first_heading(body: &str) -> Option<String> {
    // Neither ATX nor Setext H1 syntax can exist without these literal markers.
    if !body.contains(['#', '=']) {
        return None;
    }
    let options =
        pulldown_cmark::Options::ENABLE_STRIKETHROUGH | pulldown_cmark::Options::ENABLE_FOOTNOTES;
    if let Some(prefix) = independent_heading_prefix(body)
        && let Some(parser) = pulldown_cmark::Parser::first_h1_title_events(prefix, options)
        && let Some(title) = first_heading_events(parser)
    {
        return Some(title);
    }
    let parser = pulldown_cmark::Parser::first_h1_title_events(body, options)?;
    if let Some(title) = first_heading_events(parser) {
        return Some(title);
    }
    // Empty first headings can require later inline state and budget consumption.
    first_heading_events(pulldown_cmark::Parser::new_title_events(body, options))
}

// Parse only through the first possible complete H1 block. The existing parser
// decides whether it is actually top-level (not code/HTML/a nested container).
// Any bracket in the prefix could begin a reference/footnote whose definition
// is completed later, so leave all such inputs to the unchanged full parser.
fn independent_heading_prefix(body: &str) -> Option<&str> {
    let mut end = 0;
    for line in body.split_inclusive(['\r', '\n']) {
        end += line.len();
        let unindented = line.trim_start_matches(' ');
        if line.len() - unindented.len() > 3 {
            continue;
        }
        let atx = unindented
            .strip_prefix('#')
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t', '\r', '\n']));
        let underline = unindented.trim_end_matches([' ', '\t', '\r', '\n']);
        let setext = !underline.is_empty() && underline.bytes().all(|byte| byte == b'=');
        if atx || setext {
            let prefix = &body[..end];
            return (!prefix.contains('[')).then_some(prefix);
        }
    }
    None
}

#[cfg(test)]
fn first_heading_full(body: &str) -> Option<String> {
    use pulldown_cmark::{Options, Parser};
    first_heading_events(Parser::new_ext(
        body,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_FOOTNOTES,
    ))
}

fn first_heading_events<'a>(
    events: impl Iterator<Item = pulldown_cmark::Event<'a>>,
) -> Option<String> {
    use pulldown_cmark::{Event, HeadingLevel, Tag, TagEnd};

    let mut title = String::new();
    let mut in_heading = false;
    let mut depth = 0;
    for event in events {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) if depth == 0 => {
                in_heading = true;
                title.clear();
                depth += 1;
            }
            Event::Start(_) => depth += 1,
            Event::Text(text) | Event::Code(text) if in_heading => title.push_str(&text),
            Event::SoftBreak | Event::HardBreak if in_heading => title.push(' '),
            Event::End(TagEnd::Heading(HeadingLevel::H1)) if in_heading && depth == 1 => {
                if !title.trim().is_empty() {
                    return Some(title.trim().to_owned());
                }
                in_heading = false;
                depth -= 1;
            }
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }
    None
}

fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(OsStr::to_str)
        .filter(|title| !title.is_empty())
        .unwrap_or("untitled")
        .to_owned()
}

fn should_skip_directory(root: &Path, path: &Path) -> bool {
    if path.strip_prefix(root).is_ok_and(|relative| {
        is_raw_conversation_path(root, relative) || is_personal_journal_path(root, relative)
    }) {
        return true;
    }

    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    is_skipped_directory_name(name)
}

fn is_raw_conversation_path(root: &Path, path: &Path) -> bool {
    let components = normal_path_components(path);
    contains_raw_conversation_path(path)
        || (root.file_name() == Some(OsStr::new("conversations"))
            && components.first() == Some(&"raw"))
}

pub(crate) fn contains_raw_conversation_path(path: &Path) -> bool {
    normal_path_components(path)
        .windows(2)
        .any(|pair| pair == ["conversations", "raw"])
}

fn is_personal_journal_path(root: &Path, path: &Path) -> bool {
    let components = normal_path_components(path);
    components
        .windows(2)
        .any(|pair| pair == ["personal", "journal"])
        || (root.file_name() == Some(OsStr::new("personal"))
            && components.first() == Some(&"journal"))
}

fn normal_path_components(path: &Path) -> Vec<&str> {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect()
}

pub(crate) fn is_skipped_directory_name(name: &str) -> bool {
    matches!(
        name,
        ".cache"
            | ".git"
            | "cache"
            | "embeddings"
            | "exports"
            | "index"
            | "indexes"
            | "target"
            | "tmp"
            | "vector-store"
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::test_support::TempDirectory;

    #[test]
    fn exact_markdown_title_accepts_the_long_extension() {
        let content = "---\ntitle: 한국어 제목\n---\n# 다른 제목\n본문\n";
        let parsed = parse_markdown_bytes(
            Path::new("personal/notes/example.markdown"),
            content.as_bytes(),
        )
        .expect("valid Markdown original");
        assert_eq!(parsed.document().title(), "한국어 제목");
    }

    #[test]
    fn first_heading_title_uses_visible_markdown_text() {
        for (source, expected) in [
            ("# **회의** `결과`\n\n본문\n", "회의 결과"),
            ("# ~~지원 현황~~\n", "지원 현황"),
            ("소개 문단\n\n# 지원 현황\n", "지원 현황"),
            ("```md\n# 코드 예시\n```\n\n실제 제목\n===\n", "실제 제목"),
            ("> # 인용 제목\n\n# 실제 제목\n", "실제 제목"),
            ("# 지원 현황 [^n]\n\n[^n]: 설명\n", "지원 현황"),
        ] {
            let parsed =
                parse_markdown_bytes(Path::new("personal/notes/formatted.md"), source.as_bytes())
                    .expect("valid Markdown original");
            assert_eq!(parsed.document().title(), expected);
        }
    }

    #[test]
    fn leading_title_matches_full_parser_across_block_and_inline_contexts() {
        let prefixes = [
            "",
            "\n\t \r\n",
            " ",
            "   ",
            "    ",
            "\t",
            "Intro.\n\n",
            "> ",
            "- ",
            "<div>\n",
            "```md\n",
            "[r]: /target \"\n",
            "<!--\n",
        ];
        let headings = [
            "# **회의** `결과`",
            "# ~~상태~~ &amp; &#91;값&#93;",
            "# <b>Visible</b>",
            "# <https://fixture.invalid>",
            "# a\\*b ##",
            "# `a[b]`",
            "# [link][r]",
            "# [shortcut]",
            "# Text [^n]",
            "# ![image](fixture.png)",
            "#",
            "# ###",
            "#\tVisible",
            "## H2",
            "#invalid",
            "Heading\n===",
            "# ``unfinished",
        ];
        let tails = [
            "\nBody",
            "\r\nBody",
            "\rBody",
            "\n\n[r]: /target\n[shortcut]: /target\n[^n]: note",
            "\n\n# Later\n",
            "\n```\n\n# Later\n",
            "\n\"\n\n# Actual\n",
            "\n-->\n\n# Actual\n",
        ];
        for prefix in prefixes {
            for heading in headings {
                for tail in tails {
                    let body = format!("{prefix}{heading}{tail}");
                    assert_eq!(first_heading(&body), first_heading_full(&body), "{body:?}");
                }
            }
        }
    }

    #[test]
    fn heading_prefix_preserves_prior_reference_budget_and_empty_heading_state() {
        let destination = format!("https://example.invalid/{}", "a".repeat(12_000));
        for count in [0, 7, 8, 9, 12] {
            let preceding = "[r]\n\n".repeat(count);
            let body = format!("{preceding}# [Visible][r]\n\n[r]: {destination}\n");
            assert_eq!(first_heading(&body), first_heading_full(&body));
            if count == 0 {
                assert_eq!(first_heading(&body).as_deref(), Some("Visible"));
            }
            if count == 12 {
                assert_eq!(first_heading(&body).as_deref(), Some("[Visible][r]"));
            }
            let empty_first = format!("# <i></i>\n\n{body}");
            assert_eq!(
                first_heading(&empty_first),
                first_heading_full(&empty_first)
            );
        }
    }

    #[test]
    fn heading_prefix_preserves_exact_reference_budget_boundary() {
        let destination = "x".repeat(25_000);
        for (count, expected) in [(3, "Visible"), (4, "[Visible][r]")] {
            let body = format!(
                "{}# [Visible][r]\n\n[r]: {destination}\n",
                "[r]\n\n".repeat(count)
            );
            assert!(body.len() < 100_000);
            assert_eq!(first_heading_full(&body).as_deref(), Some(expected));
            assert_eq!(first_heading(&body), first_heading_full(&body));
        }
    }

    #[test]
    fn heading_prefix_restores_budget_from_the_complete_source_length() {
        let destination = "x".repeat(25_000);
        let prefix = format!("{}# [Visible][r]\n\n", "[r]\n\n".repeat(5));
        let body = format!("{prefix}{}\n\n[r]: {destination}\n", "tail ".repeat(40_000));
        assert!(prefix.len() < 100_000 && body.len() > 125_000);
        assert_eq!(first_heading_full(&body).as_deref(), Some("Visible"));
        assert_eq!(first_heading(&body), first_heading_full(&body));
    }

    #[test]
    fn discarded_prior_links_cannot_replace_heading_link_payloads() {
        use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag};
        let body = "[one][a] ![two][b] [inline](/prior \"prior title\") `code [a]` [^n]\n\n# [A][a] [B][b] ![C][c] [D](/heading \"heading title\") [^n]\n\n[a]: /alpha \"alpha title\"\n[b]: /beta \"beta title\"\n[c]: /gamma \"gamma title\"\n[^n]: Footnote [a]\n";
        let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_FOOTNOTES;
        let expected = Parser::new_ext(body, options)
            .into_offset_iter()
            .collect::<Vec<_>>();
        let heading = expected
            .iter()
            .position(|(event, _)| {
                matches!(
                    event,
                    Event::Start(Tag::Heading {
                        level: HeadingLevel::H1,
                        ..
                    })
                )
            })
            .unwrap();
        let actual = Parser::first_h1_prefix(body, options)
            .unwrap()
            .into_offset_iter()
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            expected[heading..]
                .iter()
                .take_while(|(_, range)| range.start < body.find("\n\n[a]:").unwrap())
                .cloned()
                .collect::<Vec<_>>()
        );
        assert_eq!(first_heading(body), first_heading_full(body));
    }

    #[test]
    fn title_only_exact_parser_matches_full_validation_and_metadata() {
        let path = Path::new("personal/folder/a.md");
        for source in [
            "# **Visible** [link][r]\n\n[r]: /target\n",
            "---\ntitle: Metadata\nscope: personal\n---\n# Heading\n",
            "body only",
            "---\naliases: [a, b]\nexport: false\n---\n# Heading\n",
            "---\ntitle: [invalid]\n---\n",
            "---\nscope: [invalid]\n---\n",
            "---\naliases: 7\n---\n",
            "---\nexport: word\n---\n",
            "---\nontology: 7\n---\n",
            "---\nunclosed\n",
        ] {
            let full = parse_markdown_bytes(path, source.as_bytes());
            let title = parse_markdown_title_bytes(path, source.as_bytes());
            assert_eq!(full.is_ok(), title.is_ok(), "{source:?}");
            if let (Ok(full), Ok(title)) = (full, title) {
                assert_eq!(full.document().title(), title.title());
                assert_eq!(full.declared_scope(), title.declared_scope());
            }
        }
        for path in [
            "../a.md",
            "personal//a.md",
            "personal/a.txt",
            "personal/./a.md",
        ] {
            assert!(parse_markdown_title_bytes(Path::new(path), b"# Title").is_err());
        }
        assert!(parse_markdown_title_bytes(path, &[0xff]).is_err());
    }

    #[test]
    fn heading_prefix_events_and_definitions_match_upstream_corpus() {
        use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
        use std::collections::BTreeMap;
        use syn::visit::Visit;

        #[derive(Default)]
        struct Inputs(Vec<String>);
        impl<'ast> Visit<'ast> for Inputs {
            fn visit_local(&mut self, local: &'ast syn::Local) {
                if matches!(&local.pat, syn::Pat::Ident(name) if name.ident == "original")
                    && let Some(syn::LocalInit { expr, .. }) = &local.init
                    && let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(input),
                        ..
                    }) = &**expr
                {
                    self.0.push(input.value());
                }
                syn::visit::visit_local(self, local);
            }
        }
        let mut inputs = Inputs::default();
        for suite in [
            include_str!("../../../vendor/pulldown-cmark/tests/suite/spec.rs"),
            include_str!("../../../vendor/pulldown-cmark/tests/suite/footnotes.rs"),
            include_str!("../../../vendor/pulldown-cmark/tests/suite/regression.rs"),
            include_str!("../../../vendor/pulldown-cmark/tests/suite/strikethrough.rs"),
        ] {
            inputs.visit_file(&syn::parse_file(suite).unwrap());
        }
        assert!(inputs.0.len() >= 600, "upstream corpus must actually load");
        eprintln!(
            "Upstream Markdown corpus: {} originals, {} differential inputs",
            inputs.0.len(),
            inputs.0.len() * 3
        );
        let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_FOOTNOTES;
        fn title_event(
            mut pair: (Event<'_>, std::ops::Range<usize>),
        ) -> (Event<'_>, std::ops::Range<usize>) {
            match &mut pair.0 {
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    title,
                    id,
                })
                | Event::Start(Tag::Image {
                    link_type,
                    dest_url,
                    title,
                    id,
                }) => {
                    *link_type = pulldown_cmark::LinkType::Inline;
                    *dest_url = "".into();
                    *title = "".into();
                    *id = "".into();
                }
                _ => {}
            }
            pair
        }
        for input in inputs.0 {
            for body in [
                input.clone(),
                format!("# [Visible][foo] [^note]\n\n{input}"),
                format!("{input}\n\n# [Visible][foo]\n\n[foo]: /target\n"),
            ] {
                let original = Parser::new_ext(&body, options);
                let definitions: BTreeMap<_, _> = original
                    .reference_definitions()
                    .iter()
                    .map(|(label, def)| {
                        (
                            label.to_owned(),
                            (
                                def.dest.to_string(),
                                def.title.as_ref().map(ToString::to_string),
                                def.span.clone(),
                            ),
                        )
                    })
                    .collect();
                let mut depth = 0usize;
                let mut in_heading = false;
                let mut expected = Vec::new();
                let mut found = false;
                let mut heading_start = None;
                for pair in original.into_offset_iter() {
                    match &pair.0 {
                        Event::Start(Tag::Heading {
                            level: HeadingLevel::H1,
                            ..
                        }) if depth == 0 => {
                            in_heading = true;
                            heading_start = Some(expected.len());
                            depth += 1;
                        }
                        Event::Start(_) => depth += 1,
                        Event::End(TagEnd::Heading(HeadingLevel::H1))
                            if in_heading && depth == 1 =>
                        {
                            expected.push(pair);
                            found = true;
                            break;
                        }
                        Event::End(_) => depth -= 1,
                        _ => {}
                    }
                    expected.push(pair);
                }
                let prefix = Parser::first_h1_prefix(&body, options);
                assert_eq!(prefix.is_some(), found, "{body:?}");
                if let Some(prefix) = prefix {
                    let actual_definitions: BTreeMap<_, _> = prefix
                        .reference_definitions()
                        .iter()
                        .map(|(label, def)| {
                            (
                                label.to_owned(),
                                (
                                    def.dest.to_string(),
                                    def.title.as_ref().map(ToString::to_string),
                                    def.span.clone(),
                                ),
                            )
                        })
                        .collect();
                    assert_eq!(actual_definitions, definitions, "{body:?}");
                    let actual = prefix.into_offset_iter().collect::<Vec<_>>();
                    assert_eq!(actual, expected[heading_start.unwrap()..], "{body:?}");
                }
                let full_expected = Parser::new_ext(&body, options)
                    .into_offset_iter()
                    .map(title_event)
                    .collect::<Vec<_>>();
                let full_title = Parser::new_title_events(&body, options)
                    .into_offset_iter()
                    .map(title_event)
                    .collect::<Vec<_>>();
                assert_eq!(full_title, full_expected, "{body:?}");
                if found {
                    let actual = Parser::first_h1_title_events(&body, options)
                        .unwrap()
                        .into_offset_iter()
                        .map(title_event)
                        .collect::<Vec<_>>();
                    let title_expected = expected[heading_start.unwrap()..]
                        .iter()
                        .cloned()
                        .map(title_event)
                        .collect::<Vec<_>>();
                    assert_eq!(actual, title_expected, "{body:?}");
                }
                assert_eq!(first_heading(&body), first_heading_full(&body), "{body:?}");
            }
        }
    }

    #[test]
    fn leading_title_keeps_late_reference_and_empty_heading_semantics() {
        for (body, expected) in [
            ("# **Visible** [link][r]\n\n[r]: /target\n", "Visible link"),
            ("# Note [^n]\n\n[^n]: Late definition\n", "Note"),
            ("#\n\n# Actual\n", "Actual"),
            ("    # Code\n\n# Actual\n", "Actual"),
            ("Intro.\n\nActual\n===\n", "Actual"),
        ] {
            assert_eq!(first_heading(body).as_deref(), Some(expected));
        }
        let body = format!("\n# **Visible** `title`\r\n{}", "*a* ".repeat(262_144));
        assert_eq!(first_heading(&body).as_deref(), Some("Visible title"));
        assert_eq!(
            independent_heading_prefix(&body),
            Some("\n# **Visible** `title`\r")
        );
        for prefix in [
            "Introduction.\n\n# **Visible** `title`\n",
            "**Visible** `title`\n===\n",
        ] {
            let body = format!("{prefix}{}", "*a* ".repeat(262_144));
            assert_eq!(first_heading(&body).as_deref(), Some("Visible title"));
            assert_eq!(independent_heading_prefix(&body), Some(prefix));
        }
        for body in [
            "*a* ",
            "<h1>HTML title</h1>",
            "&#35; Title\n&equals;&equals;\n",
            "[^n]: definition\n",
        ] {
            assert_eq!(first_heading(body), first_heading_full(body));
        }
    }

    #[test]
    fn reads_markdown_title_from_frontmatter() {
        let temp = TempDirectory::new("frontmatter");
        let memory = temp.path().join("memory");
        fs::create_dir_all(&memory).expect("memory directory should be created");
        fs::write(
            memory.join("preferences.md"),
            "---\ntitle: Work Preferences\nlanguage: ko\naliases:\n  - rust review\n  - 코드리뷰\n---\n# Ignored Heading\nBody",
        )
        .expect("fixture markdown should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].source_path(), "memory/preferences.md");
        assert_eq!(documents[0].scope(), Scope::Other);
        assert_eq!(documents[0].language(), Some("ko"));
        assert_eq!(documents[0].aliases(), ["rust review", "코드리뷰"]);
        assert_eq!(documents[0].title(), "Work Preferences");
        assert_eq!(documents[0].body(), "# Ignored Heading\nBody");
    }

    #[test]
    fn reads_inline_aliases_from_frontmatter() {
        let temp = TempDirectory::new("inline-aliases");
        let memory = temp.path().join("memory");
        fs::create_dir_all(&memory).expect("memory directory should be created");
        fs::write(
            memory.join("aliases.md"),
            "---\ntitle: Aliases\naliases: [rust review, 코드리뷰]\n---\nBody",
        )
        .expect("fixture markdown should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert_eq!(documents[0].aliases(), ["rust review", "코드리뷰"]);
    }

    #[test]
    fn uses_yaml_semantics_for_quoted_titles_and_aliases() {
        let temp = TempDirectory::new("yaml-semantics");
        let memory = temp.path().join("memory");
        fs::create_dir_all(&memory).expect("memory directory should be created");
        fs::write(
            memory.join("quoted.md"),
            "---\ntitle: 'Don''t Split YAML'\naliases: [\"alpha,beta\", gamma]\n---\nBody",
        )
        .expect("fixture markdown should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert_eq!(documents[0].title(), "Don't Split YAML");
        assert_eq!(documents[0].aliases(), ["alpha,beta", "gamma"]);
    }

    #[test]
    fn reads_block_aliases_with_an_unindented_comment() {
        let temp = TempDirectory::new("commented-aliases");
        let memory = temp.path().join("memory");
        fs::create_dir_all(&memory).expect("memory directory should be created");
        fs::write(
            memory.join("aliases.md"),
            "---\ntitle: Aliases\naliases:\n# explain the first alias\n  - rust review\n  - 코드리뷰\n---\nBody",
        )
        .expect("fixture markdown should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert_eq!(documents[0].aliases(), ["rust review", "코드리뷰"]);
    }

    #[test]
    fn reads_export_flag_from_frontmatter() {
        let temp = TempDirectory::new("export-flag");
        let memory = temp.path().join("memory");
        fs::create_dir_all(&memory).expect("memory directory should be created");
        fs::write(
            memory.join("detail.md"),
            "---\nexport: false\n---\n# Detail\nSearchable but not exported.",
        )
        .expect("fixture markdown should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert!(!documents[0].exportable());
    }

    #[test]
    fn treats_original_imports_as_not_exportable_by_default() {
        let temp = TempDirectory::new("imported-source-fixture");
        let originals = temp.path().join("work/cedar/notion/originals/2025-04-12");
        fs::create_dir_all(&originals).expect("originals directory should be created");
        fs::write(
            originals.join("source.md"),
            "# Source\nOriginal imported Markdown.",
        )
        .expect("source fixture should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert_eq!(documents.len(), 1);
        assert_eq!(
            documents[0].source_path(),
            "work/cedar/notion/originals/2025-04-12/source.md"
        );
        assert!(!documents[0].exportable());
    }

    #[test]
    fn detects_top_level_scope_from_path() {
        let temp = TempDirectory::new("scope");
        let personal = temp.path().join("personal/projects");
        let work = temp.path().join("work/projects");
        let profile = temp.path().join("profile/preferences");
        fs::create_dir_all(&personal).expect("personal directory should be created");
        fs::create_dir_all(&work).expect("work directory should be created");
        fs::create_dir_all(&profile).expect("profile directory should be created");
        fs::write(personal.join("side-project.md"), "# Side Project")
            .expect("personal fixture should be written");
        fs::write(work.join("company-project.md"), "# Company Project")
            .expect("work fixture should be written");
        fs::write(profile.join("style.md"), "# Working Style")
            .expect("profile fixture should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");
        let scopes = documents
            .iter()
            .map(|document| (document.source_path(), document.scope()))
            .collect::<Vec<_>>();

        assert_eq!(
            scopes,
            [
                ("personal/projects/side-project.md", Scope::Personal),
                ("profile/preferences/style.md", Scope::Profile),
                ("work/projects/company-project.md", Scope::Work),
            ]
        );
    }

    #[test]
    fn skips_generated_artifact_directories() {
        let temp = TempDirectory::new("skip-generated");
        let memory = temp.path().join("memory");
        let index = temp.path().join("index");
        fs::create_dir_all(&memory).expect("memory directory should be created");
        fs::create_dir_all(&index).expect("index directory should be created");
        fs::write(memory.join("kept.md"), "# Kept\nSearchable")
            .expect("kept fixture should be written");
        fs::write(index.join("ignored.md"), "# Ignored\nGenerated")
            .expect("ignored fixture should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].source_path(), "memory/kept.md");
    }

    #[test]
    fn skips_raw_conversation_directory_by_default() {
        let temp = TempDirectory::new("skip-raw-conversations");
        let raw = temp.path().join("conversations/raw");
        let summaries = temp.path().join("conversations/summaries");
        fs::create_dir_all(&raw).expect("raw conversation directory should be created");
        fs::create_dir_all(&summaries).expect("summary directory should be created");
        fs::write(
            raw.join("private.md"),
            "# Private raw conversation\nDo not index",
        )
        .expect("raw conversation fixture should be written");
        fs::write(summaries.join("summary.md"), "# Summary\nIndex this")
            .expect("summary fixture should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert_eq!(documents.len(), 1);
        assert_eq!(
            documents[0].source_path(),
            "conversations/summaries/summary.md"
        );
    }

    #[test]
    fn skips_scoped_raw_conversation_directory_by_default() {
        let temp = TempDirectory::new("skip-scoped-raw-conversations");
        let raw = temp.path().join("personal/conversations/raw");
        let summaries = temp.path().join("personal/conversations/summaries");
        fs::create_dir_all(&raw).expect("raw conversation directory should be created");
        fs::create_dir_all(&summaries).expect("summary directory should be created");
        fs::write(raw.join("private.md"), "# Private\nDo not index")
            .expect("raw conversation fixture should be written");
        fs::write(summaries.join("summary.md"), "# Summary\nIndex this")
            .expect("summary fixture should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert_eq!(documents.len(), 1);
        assert_eq!(
            documents[0].source_path(),
            "personal/conversations/summaries/summary.md"
        );
    }

    #[test]
    fn skips_personal_journal_directory_by_default() {
        let temp = TempDirectory::new("skip-personal-journal");
        let journal = temp.path().join("personal/journal");
        let knowledge = temp.path().join("personal/knowledge");
        fs::create_dir_all(&journal).expect("journal directory should be created");
        fs::create_dir_all(&knowledge).expect("knowledge directory should be created");
        fs::write(
            journal.join("private.md"),
            "# Private journal\nDo not index",
        )
        .expect("journal fixture should be written");
        fs::write(knowledge.join("kept.md"), "# Knowledge\nIndex this")
            .expect("knowledge fixture should be written");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].source_path(), "personal/knowledge/kept.md");

        let personal_documents = read_markdown_documents(temp.path().join("personal"))
            .expect("personal documents should be read");
        assert_eq!(personal_documents.len(), 1);
        assert_eq!(personal_documents[0].source_path(), "knowledge/kept.md");
    }

    #[cfg(unix)]
    #[test]
    fn skips_symbolic_linked_directories() {
        use std::os::unix::fs::symlink;

        let temp = TempDirectory::new("skip-symlink");
        let outside = TempDirectory::new("outside-symlink-target");
        let memory = temp.path().join("memory");
        fs::create_dir_all(&memory).expect("memory directory should be created");
        fs::write(memory.join("kept.md"), "# Kept\nSearchable")
            .expect("kept fixture should be written");
        fs::write(outside.path().join("outside.md"), "# Outside\nDo not index")
            .expect("outside fixture should be written");
        symlink(outside.path(), memory.join("linked"))
            .expect("directory symlink should be created");

        let documents = read_markdown_documents(temp.path()).expect("documents should be read");

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].source_path(), "memory/kept.md");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_linked_root() {
        use std::os::unix::fs::symlink;

        let temp = TempDirectory::new("reject-symlink-root");
        let outside = TempDirectory::new("reject-symlink-root-target");
        fs::write(outside.path().join("outside.md"), "# Outside\nDo not index")
            .expect("outside fixture should be written");
        let linked_root = temp.path().join("linked-root");
        symlink(outside.path(), &linked_root).expect("root symlink should be created");

        let error = read_markdown_documents(&linked_root)
            .expect_err("symbolic linked root should be rejected");

        assert!(error.to_string().contains("symbolic link"));
    }

    #[test]
    fn rejects_oversized_markdown_file() {
        let temp = TempDirectory::new("oversized-markdown");
        let oversized_length = usize::try_from(MAX_MARKDOWN_FILE_BYTES)
            .expect("the configured Markdown file limit must fit usize")
            + 1;
        let content = "a".repeat(oversized_length);
        fs::write(temp.path().join("large.md"), content)
            .expect("large Markdown fixture should be written");

        let error =
            read_markdown_documents(temp.path()).expect_err("oversized Markdown must be rejected");

        assert!(error.to_string().contains("per-file read limit"));
    }
}

#[cfg(test)]
mod pure_bytes_tests {
    use super::*;

    #[test]
    fn parses_exact_bytes_without_a_filesystem_and_preserves_source_digest() {
        let bytes=b"---\ntitle: Named original\nexport: false\nlanguage: ko\naliases: [alias]\n---\n# Heading\nExact body\n";
        let parsed = parse_markdown_bytes(Path::new("personal/projects/absent.md"), bytes).unwrap();
        assert_eq!(parsed.document().title(), "Named original");
        assert_eq!(parsed.document().body(), "# Heading\nExact body");
        assert_eq!(parsed.document().scope(), Scope::Personal);
        assert_eq!(parsed.document().language(), Some("ko"));
        assert_eq!(parsed.document().aliases(), &["alias"]);
        assert!(!parsed.document().exportable());
        assert_eq!(parsed.original_content_digest(), sha256_hex(bytes));
    }

    #[test]
    fn pure_markdown_parser_rejects_unclosed_metadata_and_preserves_valid_defaults() {
        let path = Path::new("personal/fallback.md");
        for content in [
            "---",
            "---\ntitle: private-unclosed-marker\nBody without a closing delimiter",
            "---\r\ntitle: private-unclosed-marker\r\nBody without a closing delimiter",
            "\u{feff}---\ntitle: private-unclosed-marker\nBody without a closing delimiter",
            "  ---  \ntitle: private-unclosed-marker\nBody without a closing delimiter",
        ] {
            let error = parse_markdown_bytes(path, content.as_bytes())
                .expect_err("an opening metadata delimiter requires a closing delimiter");
            assert!(!error.to_string().contains("private-unclosed-marker"));
        }
        for (content, title, body) in [
            ("Plain body\n", "fallback", "Plain body"),
            ("# Heading\nBody\n", "Heading", "# Heading\nBody"),
            ("---\n# Metadata comment\n---\nBody\n", "fallback", "Body"),
            ("---\ntitle: Named\n---\nBody\n", "Named", "Body"),
        ] {
            let parsed = parse_markdown_bytes(path, content.as_bytes())
                .expect("plain and closed metadata retain parser defaults");
            assert_eq!(parsed.document().title(), title);
            assert_eq!(parsed.document().body(), body);
            assert_eq!(parsed.declared_scope(), None);
            assert!(parsed.document().exportable());
            assert_eq!(
                parsed.original_content_digest(),
                sha256_hex(content.as_bytes())
            );
        }
    }

    #[test]
    fn pure_markdown_parser_rejects_escaping_nonmarkdown_utf8_yaml_and_ontology_errors() {
        for path in [
            "",
            "/personal/a.md",
            "personal/../a.md",
            "personal//a.md",
            "personal/./a.md",
            "personal/a.txt",
            "personal\\a.md",
        ] {
            assert!(
                parse_markdown_bytes(Path::new(path), b"# Title").is_err(),
                "{path}"
            );
        }
        assert!(parse_markdown_bytes(Path::new("personal/a.md"), &[0xff]).is_err());
        assert!(
            parse_markdown_bytes(
                Path::new("personal/a.md"),
                b"---\ntitle: [broken\n---\nbody"
            )
            .is_err()
        );
        assert!(
            parse_markdown_bytes(
                Path::new("personal/a.md"),
                b"---\nontology: invalid-shape\n---\nbody"
            )
            .is_err()
        );
    }
}
