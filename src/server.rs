use crate::config::default_pid_path;
use crate::process::PidFileGuard;
use crate::proxy::{self, AppState};
use anyhow::Result;
use axum::routing::get;
use axum::Router;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, warn};

pub async fn run(config_path: PathBuf) -> Result<()> {
    let state = Arc::new(AppState::new(config_path.clone()).await?);
    let bind = state.bind().await;

    let listener = TcpListener::bind(bind).await?;
    let _pid_guard = PidFileGuard::create(default_pid_path())?;

    let app = Router::new()
        .route("/health", get(health))
        .fallback(proxy::proxy)
        .with_state(state.clone());

    info!(address = %bind, config = %config_path.display(), "gateway listening");
    spawn_reload_task(state.clone());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

fn spawn_reload_task(state: Arc<AppState>) {
    #[cfg(unix)]
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};

        let mut hangup = match signal(SignalKind::hangup()) {
            Ok(signal) => signal,
            Err(err) => {
                warn!(error = %err, "failed to install SIGHUP handler");
                return;
            }
        };

        while hangup.recv().await.is_some() {
            match state.reload().await {
                Ok(_) => info!(config = %state.config_path().display(), "configuration reloaded"),
                Err(err) => warn!(error = %err, "failed to reload configuration"),
            }
        }
    });

    #[cfg(not(unix))]
    let _ = state;
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
