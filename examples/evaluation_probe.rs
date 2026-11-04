//! Executes only synthetic context cases in an explicitly selected, empty test database.
use ontology::{
    context::{ContextCommand, ContextOutput, ContextScope, inventory},
    store::Store,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, io::Read, path::PathBuf, time::Instant};

async fn step(store: &Store, command: &Value) -> Value {
    let result = if command["op"] == "edit" {
        match (
            command["scope"].as_str().and_then(|s| s.parse::<ContextScope>().ok()),
            command["path"].as_str(),
            command["revision"].as_i64(),
            command["digest"].as_str(),
            command["content"].as_str(),
        ) {
            (Some(scope), Some(path), Some(revision), Some(digest), Some(content)) => store
                .edit_context(&scope, path, revision, digest, content)
                .await
                .map(|v| json!({"revision":v.revision,"changed":v.changed,"content_digest":v.content_digest})),
            _ => return json!({"ok":false,"error":"invalid request"}),
        }
    } else {
        let allowed = [
            "read",
            "read-documents",
            "search",
            "semantic-search",
            "ontology-edges",
            "history",
            "version",
        ];
        if !command["op"]
            .as_str()
            .is_some_and(|op| allowed.contains(&op))
        {
            return json!({"ok":false,"error":"invalid request"});
        }
        let Ok(command) = serde_json::from_value::<ContextCommand>(command.clone()) else {
            return json!({"ok":false,"error":"invalid request"});
        };
        store.context(command).await.map(|v| match v {
            ContextOutput::Json(value) => value,
            ContextOutput::Text(value) => Value::String(value),
        })
    };
    match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"error":error.to_string()}),
    }
}

async fn run() -> Result<(), &'static str> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|_| "input read failed")?;
    if bytes.len() > 1_048_576 {
        return Err("suite too large");
    }
    let suite: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid suite")?;
    if suite["origin"] != "synthetic" {
        return Err("synthetic input required");
    }
    let documents = suite["documents"].as_object().ok_or("documents required")?;
    let cases = suite["cases"].as_array().ok_or("cases required")?;
    if documents.is_empty() || documents.len() > 100 || cases.is_empty() || cases.len() > 100 {
        return Err("suite bounds exceeded");
    }
    let directory = PathBuf::from(
        std::env::var("ONTOLOGY_EVALUATION_FIXTURES").map_err(|_| "fixture directory required")?,
    );
    if !directory.is_absolute()
        || directory.canonicalize().ok().as_ref() != Some(&directory)
        || fs::read_dir(&directory)
            .map_err(|_| "fixture directory unavailable")?
            .next()
            .is_some()
    {
        return Err("empty canonical fixture directory required");
    }
    let mut scopes = BTreeSet::new();
    for (path, content) in documents {
        let parts: Vec<_> = path.split('/').collect();
        if parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains('\\'))
            || !path.ends_with(".md")
        {
            return Err("invalid fixture path");
        }
        let depth = if parts.first() == Some(&"work") { 2 } else { 1 };
        if parts.len() <= depth {
            return Err("fixture scope missing");
        }
        let scope: ContextScope = parts[..depth]
            .join("/")
            .parse()
            .map_err(|_| "invalid fixture scope")?;
        scopes.insert(scope);
        let path = directory.join(path);
        fs::create_dir_all(path.parent().ok_or("invalid fixture parent")?)
            .map_err(|_| "fixture directory failed")?;
        fs::write(path, content.as_str().ok_or("fixture content required")?)
            .map_err(|_| "fixture write failed")?;
    }
    let url = std::env::var("TEST_DATABASE_URL").map_err(|_| "test database required")?;
    let name = std::env::var("ONTOLOGY_EVALUATION_DATABASE")
        .map_err(|_| "exact test database required")?;
    let suffix = name
        .strip_prefix("ontology_test_eval_")
        .ok_or("evaluation database name required")?;
    let options = ontology::config::database_options(&url).map_err(|_| "invalid test database")?;
    if suffix.len() != 32
        || !suffix.bytes().all(|b| b.is_ascii_hexdigit())
        || options.get_database() != Some(name.as_str())
        || options.get_host() != "127.0.0.1"
    {
        return Err("evaluation database identity mismatch");
    }
    let store = Store::connect(&url)
        .await
        .map_err(|_| "test database unavailable")?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(store.pool())
    .await
    .map_err(|_| "test database inspection failed")?;
    if count != 0 {
        return Err("refusing a nonempty database");
    }
    store
        .initialize()
        .await
        .map_err(|_| "test schema initialization failed")?;
    let scopes: Vec<_> = scopes.into_iter().collect();
    let manifest = inventory(&directory, &scopes).map_err(|_| "fixture inventory failed")?;
    store
        .import_context(&directory, &scopes, &manifest.inventory_digest)
        .await
        .map_err(|_| "fixture import failed")?;
    let mut results = Vec::new();
    for case in cases {
        let steps = case["steps"].as_array().ok_or("case steps required")?;
        if steps.is_empty() || steps.len() > 16 {
            return Err("case step bounds exceeded");
        }
        let before = store.calls();
        let started = Instant::now();
        let mut observations = Vec::new();
        for command in steps {
            observations.push(step(&store, command).await);
        }
        results.push(json!({"id":case["id"],"observations":observations,
            "duration_ms":started.elapsed().as_secs_f64()*1000.0,"database_calls":store.calls()-before}));
    }
    println!("{}", json!({"cases":results}));
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
