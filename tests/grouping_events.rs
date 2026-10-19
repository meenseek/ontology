use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use ontology::{
    api::{AppState, router},
    config::Config,
    store::Store,
};
use std::time::Duration;
use tower::ServiceExt;

#[tokio::test]
async fn browser_events_share_the_queue_listener_and_wait_for_committed_changes() {
    let url = std::env::var("TEST_DATABASE_URL").expect("explicit isolated DB");
    assert!(
        ontology::config::database_options(&url)
            .unwrap()
            .get_database()
            .is_some_and(|name| name.starts_with("ontology_test_"))
    );
    let store = Store::connect(&url).await.unwrap();
    store.initialize().await.unwrap();
    sqlx::query("TRUNCATE memory_grouping,document_grouping CASCADE")
        .execute(store.pool())
        .await
        .unwrap();
    let worker = tokio::spawn(ontology::grouping::run_loop(store.clone()));
    // Let the single existing listener register and both empty workers settle.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let listening: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND query LIKE 'LISTEN %ontology_grouping%' AND state='idle')")
                .fetch_one(store.pool()).await.unwrap();
            let before = store.calls();
            tokio::time::sleep(Duration::from_millis(100)).await;
            if listening && before == store.calls() { break; }
        }
    }).await.unwrap();
    let app = router(AppState::new(
        store.clone(),
        Config {
            address: "127.0.0.1:47831".parse().unwrap(),
            web_dist: "/synthetic/static".into(),
        },
    ));
    let request = |cookie: &str, host: &str, origin: &str| {
        Request::builder()
            .uri("/api/grouping/events")
            .header("host", host)
            .header("origin", origin)
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap()
    };
    for cookie in ["", "ontology_session=invalid"] {
        assert_eq!(
            app.clone()
                .oneshot(request(cookie, "127.0.0.1:47831", "http://127.0.0.1:47831"))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    let session = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header("host", "127.0.0.1:47831")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = session.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    for (host, origin) in [
        ("evil.invalid", "http://127.0.0.1:47831"),
        ("127.0.0.1:47831", "http://evil.invalid"),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request(&cookie, host, origin))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    let mut cross_site = request(&cookie, "127.0.0.1:47831", "http://127.0.0.1:47831");
    cross_site
        .headers_mut()
        .insert("sec-fetch-site", "cross-site".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(cross_site).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let connections = store.pool().size();
    let before = store.calls();
    let mut bodies = Vec::new();
    for _ in 0..8 {
        let response = app
            .clone()
            .oneshot(request(
                &cookie,
                "127.0.0.1:47831",
                "http://127.0.0.1:47831",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        assert_eq!(response.headers()["cache-control"], "no-store");
        let mut body = response.into_body();
        assert_eq!(
            body.frame()
                .await
                .unwrap()
                .unwrap()
                .into_data()
                .unwrap()
                .as_ref(),
            b"data: changed\n\n"
        );
        bodies.push(body);
    }
    assert_eq!(
        store.pool().size(),
        connections,
        "clients never acquire a DB connection"
    );
    assert_eq!(store.calls(), before);
    assert!(
        tokio::time::timeout(Duration::from_millis(3100), bodies[0].frame())
            .await
            .is_err(),
        "no timer, heartbeat or empty queue polling"
    );
    assert_eq!(
        store.calls(),
        before,
        "healthy quiet streams perform no store calls"
    );
    let mut tx = store.pool().begin().await.unwrap();
    sqlx::query("SELECT pg_notify('ontology_grouping','')")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), bodies[0].frame())
            .await
            .is_err(),
        "rollback produces no notice"
    );
    let mut tx = store.pool().begin().await.unwrap();
    sqlx::query("SELECT pg_notify('ontology_grouping','')")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    for body in &mut bodies {
        let data = tokio::time::timeout(Duration::from_secs(3), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap();
        assert_eq!(data.as_ref(), b"data: changed\n\n");
    }
    let reconnected: bool = sqlx::query_scalar("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=current_database() AND query LIKE 'LISTEN %ontology_grouping%' AND state='idle' LIMIT 1")
        .fetch_one(store.pool()).await.unwrap();
    assert!(reconnected);
    for body in &mut bodies {
        let data = tokio::time::timeout(Duration::from_secs(10), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap();
        assert_eq!(
            data.as_ref(),
            b"data: changed\n\n",
            "listener reconnect also catches up browser clients"
        );
    }
    // Ending the server must also end active bodies, without waiting for a
    // session deadline or opening another PostgreSQL listener.
    store.close_grouping_events();
    for body in &mut bodies {
        assert!(body.frame().await.is_none());
    }
    worker.abort();
    let _ = worker.await;
    store.pool().close().await;
}
