use meenseek_ontology::{
    api::{AppState, router},
    config::Config,
    domain::{Error, Scope},
    importer::GitReader,
    memory::{BrainCommand, MAX_INPUT_BYTES},
    store::Store,
    vault_importer::{VaultReader, VaultScope},
};
use std::{io::Read, path::PathBuf, str::FromStr};
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        if std::env::args().nth(1).as_deref() == Some("brain") {
            println!("{}", serde_json::json!({"error":error.to_string()}));
        } else {
            eprintln!("{error}");
        }
        std::process::exit(1)
    }
}
async fn run() -> Result<(), Error> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
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
    let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
    let store = Store::connect(&url).await?;
    store.initialize().await?;
    if let Some(command) = brain {
        let value = store.brain(command).await?;
        let bytes = serde_json::to_vec(&value).map_err(|_| Error::Storage)?;
        if bytes.len() > meenseek_ontology::domain::MAX_RESPONSE_BYTES {
            return Err(Error::Limit);
        }
        println!("{}", String::from_utf8(bytes).map_err(|_| Error::Storage)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("sync-once") && args.len() == 1 {
        let path = std::env::var_os("ONTOLOGY_SYNC_CONFIG")
            .map(PathBuf::from)
            .ok_or(Error::Invalid)?;
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
        Some("import-vault") => {
            let mut binary = None;
            let mut root = None;
            let mut vault_scope = None;
            let mut scope = None;
            let mut paths = Vec::new();
            let mut iter = args.iter().skip(1);
            while let Some(key) = iter.next() {
                let value = iter.next().ok_or(Error::Invalid)?;
                match key.as_str() {
                    "--vault-binary" if binary.is_none() => binary = Some(PathBuf::from(value)),
                    "--vault-root" if root.is_none() => root = Some(PathBuf::from(value)),
                    "--vault-scope" if vault_scope.is_none() => {
                        vault_scope = Some(VaultScope::from_str(value)?)
                    }
                    "--scope" if scope.is_none() => scope = Some(Scope::from_str(value)?),
                    "--file" => paths.push(value.clone()),
                    _ => return Err(Error::Invalid),
                }
            }
            if paths.is_empty() {
                return Err(Error::Invalid);
            }
            let allowed = std::env::var_os("ONTOLOGY_ALLOWED_VAULT_ROOTS")
                .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
                .ok_or(Error::Invalid)?;
            let reader = VaultReader::new(binary.ok_or(Error::Invalid)?, allowed)?;
            let count = reader
                .import(
                    &store,
                    &root.ok_or(Error::Invalid)?,
                    vault_scope.ok_or(Error::Invalid)?,
                    &paths,
                    scope.ok_or(Error::Invalid)?,
                )
                .await?;
            println!("Verify and import {count} registered Vault documents");
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
    result
}
