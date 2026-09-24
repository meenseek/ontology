use meenseek_ontology::{
    api::{AppState, router},
    config::Config,
    context::{ContextCommand, ContextOutput},
    context_importer::{ContextReader, validate_paths, validate_store_id},
    domain::{Error, Scope},
    importer::GitReader,
    memory::{BrainCommand, MAX_INPUT_BYTES},
    store::Store,
};
use std::{
    io::{Read, Write},
    path::PathBuf,
    str::FromStr,
};
#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("harness") {
        if let Err(error) =
            meenseek_ontology::native_harness::run(std::env::args().skip(2).collect()).await
        {
            eprintln!("{}", serde_json::json!({"error":error.to_string()}));
            std::process::exit(1);
        }
        return;
    }

    if let Err(error) = run().await {
        if std::env::args().nth(1).as_deref() == Some("brain") {
            println!("{}", serde_json::json!({"error":error.to_string()}));
        } else if std::env::args().nth(1).as_deref() == Some("context") {
            eprintln!("{}", serde_json::json!({"error":error.to_string()}));
        } else {
            eprintln!("{error}");
        }
        std::process::exit(1)
    }
}
async fn run() -> Result<(), Error> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("context") {
        return context(&args).await;
    }
    if args.first().map(String::as_str) == Some("import-vault")
        || args.iter().any(|v| v.starts_with("--vault-"))
    {
        eprintln!(
            "Use import-context --store-id UUID --context-scope SCOPE --scope SCOPE --file PATH; legacy Vault filesystem transport is retired"
        );
        return Err(Error::Invalid);
    }
    let context_import = if args.first().map(String::as_str) == Some("import-context") {
        let (mut store_id, mut context_scope, mut scope, mut paths) =
            (None, None, None, Vec::new());
        let mut iter = args.iter().skip(1);
        while let Some(key) = iter.next() {
            let value = iter.next().ok_or(Error::Invalid)?;
            match key.as_str() {
                "--store-id" if store_id.is_none() => {
                    validate_store_id(value)?;
                    store_id = Some(value.clone());
                }
                "--context-scope" if context_scope.is_none() => {
                    context_scope = Some(value.parse::<meenseek_ontology::context::ContextScope>()?)
                }
                "--scope" if scope.is_none() => scope = Some(Scope::from_str(value)?),
                "--file" => paths.push(value.clone()),
                _ => return Err(Error::Invalid),
            }
        }
        let context_scope = context_scope.ok_or(Error::Invalid)?;
        validate_paths(&context_scope, &paths, true)?;
        Some((
            store_id.ok_or(Error::Invalid)?,
            context_scope,
            scope.ok_or(Error::Invalid)?,
            paths,
        ))
    } else {
        None
    };
    let brain = if args.first().map(String::as_str) == Some("brain") {
        if args.len() != 1 {
            return Err(Error::Invalid);
        }
        let mut bytes = Vec::new();
        std::io::stdin()
            .take((MAX_INPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(Error::Limit);
        }
        Some(serde_json::from_slice::<BrainCommand>(&bytes).map_err(|_| Error::Invalid)?)
    } else {
        None
    };
    let sync_path = if args.first().map(String::as_str) == Some("sync-once") {
        if args.len() != 1 {
            return Err(Error::Invalid);
        }
        let path = std::env::var_os("ONTOLOGY_SYNC_CONFIG")
            .map(PathBuf::from)
            .ok_or(Error::Invalid)?;
        meenseek_ontology::sync::SyncConfig::load(&path).inspect_err(|_| {
            eprintln!("Use bounded Git/Context sync sources; Context requires store_id, context_scope, scope and paths; legacy Vault filesystem sources are retired");
        })?;
        Some(path)
    } else {
        None
    };
    let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
    let store = Store::connect(&url).await?;
    store.initialize().await?;
    if let Some((store_id, context_scope, scope, paths)) = context_import {
        let count = ContextReader::new()
            .import(&store, &store_id, &context_scope, &paths, scope)
            .await?;
        println!("Verify and import {count} registered Context documents");
        return Ok(());
    }
    if let Some(command) = brain {
        let curation_apply = matches!(
            &command,
            BrainCommand::Curation {
                scope: Scope::Personal,
                command: meenseek_ontology::curation::CurationCommand::Apply { .. },
            }
        );
        let wake_grouping = matches!(
            &command,
            BrainCommand::Remember {
                scope: Scope::Personal,
                ..
            } | BrainCommand::Propose {
                scope: Scope::Personal,
                ..
            } | BrainCommand::Correct {
                scope: Scope::Personal,
                ..
            } | BrainCommand::Accept {
                scope: Scope::Personal,
                ..
            } | BrainCommand::GroupingRetry {
                scope: Scope::Personal,
                ..
            } | BrainCommand::GroupingSet {
                scope: Scope::Personal,
                ..
            }
        );
        let value = store.brain(command).await?;
        let bytes = serde_json::to_vec(&value).map_err(|_| Error::Storage)?;
        if bytes.len() > meenseek_ontology::domain::MAX_RESPONSE_BYTES {
            return Err(Error::Limit);
        }
        println!("{}", String::from_utf8(bytes).map_err(|_| Error::Storage)?);
        if ((wake_grouping && value["grouping"]["state"] == "pending")
            || (curation_apply && matches!(value["outcome"].as_str(), Some("created" | "updated"))))
            && let Ok(binary) = std::env::current_exe()
        {
            let _ = std::process::Command::new(binary)
                .arg("grouping-drain")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("grouping-drain") && args.len() == 1 {
        while store.grouping_once().await? {}
        return Ok(());
    }
    if let Some(path) = sync_path {
        let report = meenseek_ontology::sync::refresh(&store, &path).await?;
        println!(
            "{}",
            serde_json::to_string(&report).map_err(|_| Error::Storage)?
        );
        if !report.ok {
            std::process::exit(1);
        }
        return Ok(());
    }
    match args.first().map(String::as_str) {
        Some("init") if args.len() == 1 => {
            println!("Initialize the app database successfully");
            Ok(())
        }
        Some("import") => {
            let mut repo = None;
            let mut commit = None;
            let mut scope = None;
            let mut paths = Vec::new();
            let mut iter = args.iter().skip(1);
            while let Some(key) = iter.next() {
                let value = iter.next().ok_or(Error::Invalid)?;
                match key.as_str() {
                    "--repo" if repo.is_none() => repo = Some(PathBuf::from(value)),
                    "--commit" if commit.is_none() => commit = Some(value.clone()),
                    "--scope" if scope.is_none() => scope = Some(Scope::from_str(value)?),
                    "--file" => paths.push(value.clone()),
                    _ => return Err(Error::Invalid),
                }
            }
            let allowed = std::env::var_os("ONTOLOGY_ALLOWED_REPOSITORIES")
                .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
                .ok_or(Error::Invalid)?;
            let reader = GitReader::new(allowed)?;
            let count = reader
                .import(
                    &store,
                    &repo.ok_or(Error::Invalid)?,
                    &commit.ok_or(Error::Invalid)?,
                    &paths,
                    scope.ok_or(Error::Invalid)?,
                )
                .await?;
            println!("Verify and import {count} registered Git documents");
            Ok(())
        }
        Some("serve") if args.len() == 1 => serve(store).await,
        None => serve(store).await,
        _ => Err(Error::Invalid),
    }
}
async fn serve(store: Store) -> Result<(), Error> {
    let config = Config::from_env()?;
    if !config.web_dist.join("index.html").is_file() {
        return Err(Error::Invalid);
    }
    let listener = tokio::net::TcpListener::bind(config.address)
        .await
        .map_err(|_| Error::Invalid)?;
    println!("Open {}", config.origin());
    let state = AppState::new(store.clone(), config);
    let grouping_task = tokio::spawn(meenseek_ontology::grouping::run_loop(store.clone()));
    let sync_task = std::env::var_os("ONTOLOGY_SYNC_CONFIG").map(|path| {
        tokio::spawn(meenseek_ontology::sync::run_loop(
            store,
            PathBuf::from(path),
            state.sync_status.clone(),
        ))
    });
    let result = axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|_| Error::Storage);
    if let Some(task) = sync_task {
        task.abort();
        let _ = task.await;
    }
    grouping_task.abort();
    let _ = grouping_task.await;
    result
}

// Context inventory is offline. Other context operations require an explicitly initialized
// database; a read or inventory command must never apply an additive migration implicitly.
async fn context(args: &[String]) -> Result<(), Error> {
    use meenseek_ontology::context::{
        ContextScope, MAX_COMMAND_BYTES, MAX_OUTPUT_BYTES, MAX_READ_BYTES, inventory,
    };
    if args.get(1).map(String::as_str) == Some("edit") {
        let (mut scope, mut path, mut revision, mut digest) = (None, None, None, None);
        let mut arguments = args.iter().skip(2);
        while let Some(key) = arguments.next() {
            let value = arguments.next().ok_or(Error::Invalid)?;
            match key.as_str() {
                "--scope" if scope.is_none() => scope = Some(value.parse::<ContextScope>()?),
                "--path" if path.is_none() => path = Some(value.as_str()),
                "--expected-revision" if revision.is_none() => {
                    revision = Some(value.parse::<i64>().map_err(|_| Error::Invalid)?)
                }
                "--expected-digest" if digest.is_none() => digest = Some(value.as_str()),
                _ => return Err(Error::Invalid),
            }
        }
        let mut bytes = Vec::new();
        std::io::stdin()
            .take((MAX_READ_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_READ_BYTES {
            return Err(Error::Limit);
        }
        let content = String::from_utf8(bytes).map_err(|_| Error::Invalid)?;
        let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
        let store = Store::connect(&url).await?;
        let receipt = store
            .edit_context(
                &scope.ok_or(Error::Invalid)?,
                path.ok_or(Error::Invalid)?,
                revision.ok_or(Error::Invalid)?,
                digest.ok_or(Error::Invalid)?,
                &content,
            )
            .await?;
        let output = serde_json::to_vec(&receipt).map_err(|_| Error::Storage)?;
        std::io::stdout()
            .lock()
            .write_all(&output)
            .map_err(|_| Error::Storage)?;
        std::io::stdout()
            .lock()
            .write_all(b"\n")
            .map_err(|_| Error::Storage)?;
        return Ok(());
    }
    if args.len() != 1 {
        return Err(Error::Invalid);
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((MAX_COMMAND_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_COMMAND_BYTES {
        return Err(Error::Limit);
    }
    let command: ContextCommand = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    let result = if let ContextCommand::Inventory { root, scopes } = command {
        ContextOutput::Json(
            serde_json::to_value(inventory(&root, &scopes)?).map_err(|_| Error::Storage)?,
        )
    } else {
        let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
        let store = Store::connect(&url).await?;
        store.context(command).await?
    };
    let output = match result {
        ContextOutput::Json(value) => {
            let mut output = serde_json::to_vec(&value).map_err(|_| Error::Storage)?;
            output.push(b'\n');
            output
        }
        ContextOutput::Text(text) => text.into_bytes(),
    };
    if output.len() > MAX_OUTPUT_BYTES {
        return Err(Error::Limit);
    }
    std::io::stdout()
        .lock()
        .write_all(&output)
        .map_err(|_| Error::Storage)
}
