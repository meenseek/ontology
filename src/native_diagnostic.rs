//! Bounded native-source diagnostics: never format SQLx errors or source values.
use crate::domain::Error;
use serde::Serialize;
use std::io::Write;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    SourceMetadata,
    SourceVersions,
    SourceChildren,
    SourceBody,
    SourceAccess,
    SourceMetadataDecode,
    SourceRevisionDecode,
    CareerInventory,
    CareerSnapshotBegin,
    CareerSnapshotEnd,
    CareerSelectedWave,
    CareerIntegrity,
    CareerRevision,
    ContextConnect,
    ContextConfigure,
    ContextGate,
    ContextPending,
    ContextIdentity,
    ContextJoin,
}

#[derive(Serialize)]
struct Diagnostic {
    kind: &'static str,
    stage: Stage,
    error_kind: &'static str,
    sqlstate: Option<String>,
    os_code: Option<i32>,
}

impl Diagnostic {
    fn sql(stage: Stage, error: &sqlx::Error) -> Self {
        use sqlx::Error::*;
        let error_kind = match error {
            Database(_) => "database",
            Io(_) => "io",
            Configuration(_) | InvalidArgument(_) => "configuration",
            Tls(_) => "tls",
            Protocol(_) => "protocol",
            RowNotFound => "row_not_found",
            TypeNotFound { .. } => "type_not_found",
            ColumnNotFound(_) | ColumnIndexOutOfBounds { .. } => "column_not_found",
            ColumnDecode { .. } | Decode(_) => "decode",
            Encode(_) => "encode",
            PoolTimedOut => "pool_timeout",
            PoolClosed => "pool_closed",
            WorkerCrashed => "worker_crashed",
            _ => "other",
        };
        let sqlstate = error
            .as_database_error()
            .and_then(|e| e.code())
            .filter(|code| {
                code.len() == 5
                    && code
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            })
            .map(|code| code.into_owned());
        let os_code = match error {
            Io(e) => e.raw_os_error(),
            _ => None,
        };
        Self {
            kind: "native-context-error",
            stage,
            error_kind,
            sqlstate,
            os_code,
        }
    }

    fn emit(self) -> Error {
        // A broken diagnostic sink must not replace the original failure.
        if let Ok(json) = serde_json::to_string(&self) {
            let _ = writeln!(std::io::stderr().lock(), "{json}");
        }
        Error::Storage
    }
}

pub(crate) fn sql_error(stage: Stage, error: sqlx::Error) -> Error {
    Diagnostic::sql(stage, &error).emit()
}

pub(crate) fn failure(stage: Stage, kind: &'static str) -> Error {
    Diagnostic {
        kind: "native-context-error",
        stage,
        error_kind: kind,
        sqlstate: None,
        os_code: None,
    }
    .emit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::error::{DatabaseError, ErrorKind};
    use std::{borrow::Cow, error::Error as StdError, fmt};

    #[derive(Debug)]
    struct PrivateDatabaseError(&'static str);
    impl fmt::Display for PrivateDatabaseError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("SELECT private_resume; password=synthetic-secret")
        }
    }
    impl StdError for PrivateDatabaseError {}
    impl DatabaseError for PrivateDatabaseError {
        fn message(&self) -> &str {
            "private source body and credential"
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.0))
        }
        fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    #[test]
    fn database_diagnostic_keeps_only_stage_kind_and_valid_code() {
        let error = sqlx::Error::Database(Box::new(PrivateDatabaseError("57014")));
        let value = serde_json::to_value(Diagnostic::sql(Stage::SourceBody, &error)).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "kind":"native-context-error", "stage":"source_body",
                "error_kind":"database", "sqlstate":"57014", "os_code":null
            })
        );
        let error = sqlx::Error::Database(Box::new(PrivateDatabaseError("secret-code")));
        assert!(
            Diagnostic::sql(Stage::SourceBody, &error)
                .sqlstate
                .is_none()
        );
    }

    #[test]
    fn io_and_protocol_diagnostics_do_not_format_messages() {
        let error = sqlx::Error::Io(std::io::Error::from_raw_os_error(54));
        let value = serde_json::to_value(Diagnostic::sql(Stage::ContextConnect, &error)).unwrap();
        assert_eq!(value["os_code"], 54);
        assert_eq!(value["error_kind"], "io");
        let error = sqlx::Error::Protocol("private source body; credential".into());
        let value = serde_json::to_value(Diagnostic::sql(Stage::SourceMetadata, &error)).unwrap();
        assert_eq!(value["error_kind"], "protocol");
        assert!(!value.to_string().contains("private"));
        assert!(!value.to_string().contains("credential"));
    }
}
