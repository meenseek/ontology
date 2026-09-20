use crate::{
    config::Config,
    context::{ContextScope, MAX_READ_BYTES},
    domain::{AREAS, Classification, Error, LinkChange, MAX_RESPONSE_BYTES, Scope},
    graph::GraphQuery,
    memory::BrainCommand,
    store::Store,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Path, Query, Request, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub sync_status: crate::sync::SharedStatus,
    config: Config,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
}
struct Session {
    csrf: String,
    expires: Instant,
}
impl AppState {
    pub fn new(store: Store, config: Config) -> Self {
        Self {
            store,
            sync_status: Arc::new(tokio::sync::RwLock::new(crate::sync::SyncStatus::default())),
            config,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
pub fn router(state: AppState) -> Router {
    let files = ServeDir::new(&state.config.web_dist)
        .not_found_service(ServeFile::new(state.config.web_dist.join("index.html")));
    Router::new()
        .route("/api/session", get(session))
        .route("/api/brain", post(brain))
        .route("/api/graph", get(graph))
        .route("/api/sync", get(sync_status))
        .route("/api/context/scopes", get(context_scopes))
        .route("/api/context", get(context_search))
        .route("/api/context/read", get(context_read))
        .route("/api/context/download", get(context_download))
        .route("/api/records", get(list))
        .route("/api/records/{id}", get(detail))
        .route("/api/records/{id}/classification", post(classify))
        .route("/api/records/{id}/links", post(link))
        .route(
            "/api/{*path}",
            get(|| async { Error::NotFound.into_response() }),
        )
        .fallback_service(files)
        .layer(DefaultBodyLimit::max(16_384))
        .layer(middleware::from_fn_with_state(state.clone(), protect))
        .with_state(state)
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Invalid => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Gone => StatusCode::GONE,
            Self::Conflict | Self::ContextPending => StatusCode::CONFLICT,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::Limit => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({"error":self.to_string()}))).into_response()
    }
}
fn json_response(value: Value) -> Result<Response, Error> {
    let bytes = serde_json::to_vec(&value).map_err(|_| Error::Storage)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(Error::Limit);
    }
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert("content-type", HeaderValue::from_static("application/json"));
    Ok(response)
}
fn one_header<'a>(headers: &'a HeaderMap, key: &str) -> Option<&'a str> {
    if headers.get_all(key).iter().count() != 1 {
        return None;
    }
    headers.get(key)?.to_str().ok()
}
fn cookie(headers: &HeaderMap) -> Option<&str> {
    let raw = one_header(headers, "cookie")?;
    let mut values = raw
        .split(';')
        .filter_map(|part| part.trim().strip_prefix("ontology_session="));
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}
async fn protect(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    if one_header(headers, "host") != Some(state.config.host().as_str())
        || headers.get_all("origin").iter().count() > 1
        || headers.contains_key("origin")
            && one_header(headers, "origin") != Some(state.config.origin().as_str())
        || one_header(headers, "sec-fetch-site") == Some("cross-site")
    {
        return Error::Forbidden.into_response();
    }
    let path = request.uri().path();
    let api = path.starts_with("/api/");
    if api && path != "/api/session" {
        let sessions = state.sessions.lock().await;
        let Some(session) = cookie(headers)
            .and_then(|id| sessions.get(id))
            .filter(|s| s.expires > Instant::now())
        else {
            return Error::Forbidden.into_response();
        };
        if request.method() != Method::GET
            && request.method() != Method::HEAD
            && (one_header(headers, "origin") != Some(state.config.origin().as_str())
                || one_header(headers, "x-csrf-token") != Some(session.csrf.as_str()))
        {
            return Error::Forbidden.into_response();
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    response
}
async fn session(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, Error> {
    let mut sessions = state.sessions.lock().await;
    let now = Instant::now();
    sessions.retain(|_, session| session.expires > now);
    let existing =
        cookie(&headers).and_then(|id| sessions.get(id).map(|s| (id.to_owned(), s.csrf.clone())));
    let (id, csrf) = if let Some(pair) = existing {
        pair
    } else {
        if sessions.len() >= 64 {
            return Err(Error::Limit);
        }
        let id = Uuid::new_v4().to_string();
        let csrf = Uuid::new_v4().to_string();
        sessions.insert(
            id.clone(),
            Session {
                csrf: csrf.clone(),
                expires: now + Duration::from_secs(8 * 60 * 60),
            },
        );
        (id, csrf)
    };
    let mut response = json_response(
        json!({"csrf":csrf,"areas":AREAS.iter().map(|(id,label)|json!({"id":id,"label":label})).collect::<Vec<_>>()}),
    )?;
    response.headers_mut().insert(
        "set-cookie",
        HeaderValue::from_str(&format!(
            "ontology_session={id}; Path=/; HttpOnly; SameSite=Strict; Max-Age=28800"
        ))
        .map_err(|_| Error::Storage)?,
    );
    Ok(response)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    scope: Scope,
    #[serde(default)]
    q: String,
    #[serde(default)]
    unclassified: bool,
    area: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scoped {
    scope: Scope,
}
async fn list(
    State(state): State<AppState>,
    query: Result<Query<Search>, QueryRejection>,
) -> Result<Response, Error> {
    let Query(query) = query.map_err(|_| Error::Invalid)?;
    json_response(
        state
            .store
            .list(
                query.scope,
                &query.q,
                query.unclassified,
                query.area.as_deref(),
            )
            .await?,
    )
}
async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
    query: Result<Query<Scoped>, QueryRejection>,
) -> Result<Response, Error> {
    let Query(query) = query.map_err(|_| Error::Invalid)?;
    json_response(state.store.detail(query.scope, &id).await?)
}
async fn classify(
    State(state): State<AppState>,
    Path(id): Path<String>,
    query: Result<Query<Scoped>, QueryRejection>,
    body: Result<Json<Classification>, JsonRejection>,
) -> Result<Response, Error> {
    let Query(query) = query.map_err(|_| Error::Invalid)?;
    let Json(change) = body.map_err(|_| Error::Invalid)?;
    state.store.classify(query.scope, &id, change).await?;
    json_response(json!({"saved":true}))
}
async fn link(
    State(state): State<AppState>,
    Path(id): Path<String>,
    query: Result<Query<Scoped>, QueryRejection>,
    body: Result<Json<LinkChange>, JsonRejection>,
) -> Result<Response, Error> {
    let Query(query) = query.map_err(|_| Error::Invalid)?;
    let Json(change) = body.map_err(|_| Error::Invalid)?;
    state.store.link(query.scope, &id, change).await?;
    json_response(json!({"saved":true}))
}

async fn brain(
    State(state): State<AppState>,
    body: Result<Json<BrainCommand>, JsonRejection>,
) -> Result<Response, Error> {
    let Json(command) = body.map_err(|_| Error::Invalid)?;
    json_response(state.store.brain(command).await?)
}

async fn sync_status(State(state): State<AppState>) -> Result<Response, Error> {
    json_response(
        serde_json::to_value(&*state.sync_status.read().await).map_err(|_| Error::Storage)?,
    )
}

async fn graph(
    State(state): State<AppState>,
    query: Result<Query<GraphQuery>, QueryRejection>,
) -> Result<Response, Error> {
    let Query(query) = query.map_err(|_| Error::Invalid)?;
    json_response(state.store.graph(query).await?)
}

// This surface deliberately accepts no ContextCommand or archive/write options.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextSearch {
    scope: ContextScope,
    #[serde(default)]
    q: String,
    after: Option<String>,
    #[serde(default = "context_limit")]
    limit: usize,
}
fn context_limit() -> usize {
    20
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextSelection {
    scope: ContextScope,
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextDiscovery {}

async fn context_scopes(
    State(state): State<AppState>,
    query: Result<Query<ContextDiscovery>, QueryRejection>,
) -> Result<Response, Error> {
    query.map_err(|_| Error::Invalid)?;
    bounded_context_json(
        &json!({ "scopes": state.store.context_scopes().await? }),
        MAX_RESPONSE_BYTES,
    )
}
async fn context_search(
    State(state): State<AppState>,
    query: Result<Query<ContextSearch>, QueryRejection>,
) -> Result<Response, Error> {
    let Query(query) = query.map_err(|_| Error::Invalid)?;
    if query.limit == 0 || query.limit > context_limit() {
        return Err(Error::Invalid);
    }
    let mut page = state
        .store
        .search_context(&query.scope, &query.q, query.after.as_deref(), query.limit)
        .await?;
    // The CLI retains complete transport provenance; HTTP never returns server locations.
    if let Some(items) = page["items"].as_array_mut() {
        for item in items {
            if let Some(item) = item.as_object_mut() {
                item.remove("source_root");
            }
        }
    }
    bounded_context_json(&page, MAX_RESPONSE_BYTES)
}
async fn context_read(
    State(state): State<AppState>,
    query: Result<Query<ContextSelection>, QueryRejection>,
) -> Result<Response, Error> {
    let Query(query) = query.map_err(|_| Error::Invalid)?;
    let material = state
        .store
        .read_context_material(&query.scope, &query.path)
        .await?;
    // One byte may require six JSON escape bytes. Other API response limits stay unchanged.
    bounded_context_json(&material, 6 * MAX_READ_BYTES + 16 * 1024)
}
async fn context_download(
    State(state): State<AppState>,
    query: Result<Query<ContextSelection>, QueryRejection>,
) -> Result<Response, Error> {
    let Query(query) = query.map_err(|_| Error::Invalid)?;
    let material = state
        .store
        .download_context(&query.scope, &query.path)
        .await?;
    let filename = material
        .metadata
        .path
        .rsplit('/')
        .next()
        .ok_or(Error::Invalid)?;
    let disposition = attachment_disposition(filename)?;
    let length =
        HeaderValue::from_str(&material.bytes.len().to_string()).map_err(|_| Error::Storage)?;
    let mut response = Response::new(Body::from(material.bytes));
    let headers = response.headers_mut();
    headers.insert(
        "content-type",
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert("content-disposition", disposition);
    headers.insert("content-length", length);
    Ok(response)
}
fn attachment_disposition(filename: &str) -> Result<HeaderValue, Error> {
    use std::fmt::Write;
    let mut fallback = String::new();
    let mut encoded = String::new();
    for character in filename.chars() {
        match character {
            '\"' | '\\' => {
                fallback.push('\\');
                fallback.push(character);
            }
            c if c.is_ascii() && !c.is_control() => fallback.push(c),
            _ => fallback.push('_'),
        }
    }
    // Percent-encoding every UTF-8 byte is valid RFC 8187 and cannot inject a header delimiter.
    for byte in filename.bytes() {
        write!(&mut encoded, "%{byte:02X}").map_err(|_| Error::Storage)?;
    }
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .map_err(|_| Error::Storage)
}
fn bounded_context_json<T: Serialize>(value: &T, limit: usize) -> Result<Response, Error> {
    struct Buffer {
        bytes: Vec<u8>,
        limit: usize,
        exceeded: bool,
    }
    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self
                .bytes
                .len()
                .checked_add(bytes.len())
                .is_none_or(|len| len > self.limit)
            {
                self.exceeded = true;
                return Err(std::io::Error::other("Response limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Buffer {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    if serde_json::to_writer(&mut buffer, value).is_err() {
        return Err(if buffer.exceeded {
            Error::Limit
        } else {
            Error::Storage
        });
    }
    let mut response = Response::new(Body::from(buffer.bytes));
    response
        .headers_mut()
        .insert("content-type", HeaderValue::from_static("application/json"));
    Ok(response)
}

#[cfg(test)]
mod context_response_tests {
    use super::*;
    #[test]
    fn context_serialization_stops_at_its_own_limit() {
        assert!(bounded_context_json(&"\0".repeat(100), 602).is_ok());
        assert!(matches!(
            bounded_context_json(&"\0".repeat(100), 601),
            Err(Error::Limit)
        ));
        assert!(bounded_context_json(&"", 2).is_ok());
        assert!(matches!(bounded_context_json(&"", 1), Err(Error::Limit)));
    }
    #[test]
    fn attachment_headers_preserve_unicode_and_quote_delimiters() {
        let header = attachment_disposition("한글 \";name.html").expect("validated filename");
        let value = header.to_str().expect("ASCII header");
        assert!(value.starts_with("attachment; filename=\"__ \\\";name.html\"; filename*=UTF-8''"));
        assert!(value.ends_with("%ED%95%9C%EA%B8%80%20%22%3B%6E%61%6D%65%2E%68%74%6D%6C"));
    }
}
