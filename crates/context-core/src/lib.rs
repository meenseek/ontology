#![allow(clippy::missing_errors_doc)]
#![allow(clippy::module_name_repetitions)]

mod atomic_file;
pub mod document;
mod frontmatter;
pub mod harness;
pub mod ontology;
pub mod redaction;
pub mod search_text;
pub mod vault;
pub mod verification;

use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    fs, io,
    path::{Path, PathBuf},
};

pub use document::{Document, Scope, read_markdown_documents};
pub use vault::{
    ContextMetadata, SearchScope, delete_context, export_context, read_context, save_context,
    save_context_with_metadata,
};

#[derive(Debug)]
pub enum ContextVaultError {
    InvalidInput(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl ContextVaultError {
    #[must_use]
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    #[must_use]
    pub fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}

impl Display for ContextVaultError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(formatter, "{message}"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} `{}`: {source}",
                path.display()
            ),
        }
    }
}

impl Error for ContextVaultError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidInput(_) => None,
            Self::Io { source, .. } => Some(source),
        }
    }
}

pub type Result<T> = std::result::Result<T, ContextVaultError>;

pub fn init_vault(root: impl AsRef<Path>) -> Result<()> {
    let root = root.as_ref();
    for directory in [
        "profile/preferences",
        "profile/rules",
        "profile/working-style",
        "personal/business",
        "personal/projects",
        "personal/decisions",
        "personal/knowledge",
        "personal/learning",
        "personal/ontology",
        "personal/writing",
        "work/common/preferences",
        "work/common/router",
        "work/common/rules",
        "index",
    ] {
        let path = root.join(directory);
        fs::create_dir_all(&path)
            .map_err(|source| ContextVaultError::io("create directory", path, source))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_expected_directories() {
        let temp = test_support::TempDirectory::new("init");

        init_vault(temp.path()).expect("vault directories should be created");

        for directory in [
            "profile/preferences",
            "profile/rules",
            "profile/working-style",
            "personal/business",
            "personal/projects",
            "personal/decisions",
            "personal/knowledge",
            "personal/learning",
            "personal/ontology",
            "personal/writing",
            "work/common/preferences",
            "work/common/router",
            "work/common/rules",
            "index",
        ] {
            assert!(
                temp.path().join(directory).is_dir(),
                "`{directory}` should be created"
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{
        fs,
        path::{Path, PathBuf},
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    pub(crate) struct TempDirectory {
        path: PathBuf,
    }

    impl TempDirectory {
        pub(crate) fn new(name: &str) -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "llm-context-vault-{name}-{}-{timestamp}",
                process::id()
            ));
            fs::create_dir_all(&path).expect("temporary directory should be created");
            Self { path }
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
