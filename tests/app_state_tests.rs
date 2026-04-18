use itadori::proxy::AppState;
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn reload_refuses_bind_changes() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.toml");

    fs::write(&path, config("127.0.0.1:8787")).unwrap();
    let state = AppState::new(path.clone()).await.unwrap();

    fs::write(&path, config("127.0.0.1:8788")).unwrap();
    let error = state.reload().await.unwrap_err();

    assert!(error.to_string().contains("restart required"));
}

#[tokio::test]
async fn reload_accepts_content_changes_without_bind_changes() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.toml");

    fs::write(
        &path,
        config_with_header("127.0.0.1:8787", "x-team", "platform"),
    )
    .unwrap();
    let state = AppState::new(path.clone()).await.unwrap();

    fs::write(
        &path,
        config_with_header("127.0.0.1:8787", "x-team", "infra"),
    )
    .unwrap();
    state.reload().await.unwrap();

    let snapshot = state.snapshot().await;
    assert_eq!(snapshot.routes[0].headers.get("x-team").unwrap(), "infra");
}

fn config(bind: &str) -> String {
    format!(
        r#"[server]
bind = "{bind}"
max_body_bytes = 10485760
request_timeout_ms = 30000

[[routes]]
name = "jira"
prefix = "/jira"
upstream = "https://jira.internal.example.com"
strip_prefix = true
"#
    )
}

fn config_with_header(bind: &str, key: &str, value: &str) -> String {
    format!(
        r#"[server]
bind = "{bind}"
max_body_bytes = 10485760
request_timeout_ms = 30000

[[routes]]
name = "jira"
prefix = "/jira"
upstream = "https://jira.internal.example.com"
strip_prefix = true

[routes.headers]
{key} = "{value}"
"#
    )
}
