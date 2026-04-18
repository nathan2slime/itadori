use crate::config::{GatewayConfig, RouteConfig};
use anyhow::Result;
use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderName, Request, Response, StatusCode};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::{debug, warn};

#[derive(Clone)]
pub struct AppState {
    config_path: PathBuf,
    pub config: Arc<RwLock<GatewayConfig>>,
    pub client: reqwest::Client,
}

impl AppState {
    pub async fn new(config_path: PathBuf) -> Result<Self> {
        let config = GatewayConfig::load(&config_path)?;
        let client = reqwest::Client::builder().build()?;
        Ok(Self {
            config_path,
            config: Arc::new(RwLock::new(config)),
            client,
        })
    }

    pub async fn snapshot(&self) -> GatewayConfig {
        self.config.read().await.clone()
    }

    pub async fn bind(&self) -> std::net::SocketAddr {
        self.config.read().await.server.bind
    }

    pub async fn reload(&self) -> Result<()> {
        let mut fresh = GatewayConfig::load(&self.config_path)?;
        let mut guard = self.config.write().await;

        if guard.server.bind != fresh.server.bind {
            anyhow::bail!(
                "server bind changed from {} to {}; restart required",
                guard.server.bind,
                fresh.server.bind
            );
        }

        std::mem::swap(&mut *guard, &mut fresh);
        Ok(())
    }

    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }
}

pub async fn proxy(State(state): State<Arc<AppState>>, req: Request<Body>) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path().to_owned();
    let query = parts.uri.query().map(str::to_owned);
    let config = state.snapshot().await;

    let Some(route) = config.route_for_path(&path) else {
        return error_response(StatusCode::NOT_FOUND, "no route matches this path");
    };

    let max_body = config.server.max_body_bytes;
    let body = match to_bytes(body, max_body).await {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!(error = %err, path = %path, "request body too large or unreadable");
            return error_response(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
        }
    };

    let upstream = match route.upstream_url(&path, query.as_deref()) {
        Ok(url) => url,
        Err(err) => {
            warn!(error = %err, route = %route.name, "failed to build upstream url");
            return error_response(StatusCode::BAD_GATEWAY, "invalid upstream url");
        }
    };

    debug!(route = %route.name, method = %parts.method, path = %path, upstream = %upstream, "proxying request");

    let mut request = state.client.request(parts.method.clone(), upstream);
    request = copy_request_headers(request, &parts.headers, route);
    request = request.body(body);

    let upstream_response = match timeout(
        route_timeout(route, config.server.request_timeout_ms),
        request.send(),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            warn!(error = %err, route = %route.name, "upstream request failed");
            return error_response(StatusCode::BAD_GATEWAY, "upstream request failed");
        }
        Err(_) => {
            warn!(route = %route.name, "upstream request timed out");
            return error_response(StatusCode::GATEWAY_TIMEOUT, "upstream request timed out");
        }
    };

    let status = upstream_response.status();
    let headers = upstream_response.headers().clone();
    let body = match upstream_response.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!(error = %err, route = %route.name, "failed to read upstream response body");
            return error_response(
                StatusCode::BAD_GATEWAY,
                "failed to read upstream response body",
            );
        }
    };

    let mut builder = Response::builder().status(status);
    for (name, value) in headers.iter() {
        if should_forward_header(name) {
            builder = builder.header(name, value);
        }
    }

    builder
        .body(Body::from(body))
        .unwrap_or_else(|_| error_response(StatusCode::BAD_GATEWAY, "failed to build response"))
}

fn copy_request_headers(
    mut request: reqwest::RequestBuilder,
    headers: &HeaderMap,
    route: &RouteConfig,
) -> reqwest::RequestBuilder {
    for (name, value) in headers.iter() {
        if should_forward_header(name) {
            request = request.header(name, value);
        }
    }

    for (name, value) in &route.headers {
        request = request.header(name, value);
    }

    request
}

fn should_forward_header(name: &HeaderName) -> bool {
    !matches!(
        name,
        &header::HOST
            | &header::CONNECTION
            | &header::PROXY_AUTHENTICATE
            | &header::PROXY_AUTHORIZATION
            | &header::TE
            | &header::TRAILER
            | &header::TRANSFER_ENCODING
            | &header::UPGRADE
    )
}

fn route_timeout(route: &RouteConfig, default_timeout_ms: u64) -> std::time::Duration {
    std::time::Duration::from_millis(route.timeout_ms.unwrap_or(default_timeout_ms))
}

fn error_response(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::from(body.to_owned()))
        .unwrap_or_else(|_| Response::new(Body::from(body.to_owned())))
}
