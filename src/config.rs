use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    pub name: String,
    pub prefix: String,
    pub upstream: Url,
    #[serde(default = "default_true")]
    pub strip_prefix: bool,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            max_body_bytes: default_max_body_bytes(),
            request_timeout_ms: default_request_timeout_ms(),
        }
    }
}

impl GatewayConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let mut config: GatewayConfig = toml::from_str(&raw)
            .with_context(|| format!("failed to parse TOML in {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&mut self) -> Result<()> {
        let mut route_names = std::collections::HashSet::new();
        let mut route_prefixes = std::collections::HashSet::new();

        for route in &mut self.routes {
            route.validate()?;
            if !route_names.insert(route.name.clone()) {
                bail!("duplicate route name: {}", route.name);
            }

            let prefix = normalized_prefix(&route.prefix).to_string();
            if !route_prefixes.insert(prefix.clone()) {
                bail!("duplicate route prefix: {}", prefix);
            }
        }
        self.routes
            .sort_by_key(|route| std::cmp::Reverse(route.prefix.len()));
        Ok(())
    }

    pub fn route_for_path(&self, path: &str) -> Option<&RouteConfig> {
        self.routes
            .iter()
            .filter(|route| route.matches(path))
            .max_by_key(|route| route.prefix.len())
    }

    pub fn example_toml() -> &'static str {
        include_str!("../gateway.example.toml")
    }
}

impl RouteConfig {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("route name cannot be empty");
        }
        if self.prefix.trim().is_empty() {
            bail!("route prefix cannot be empty");
        }
        if !self.prefix.starts_with('/') {
            bail!("route prefix must start with '/': {}", self.prefix);
        }
        if self.prefix != "/" && normalized_prefix(&self.prefix).is_empty() {
            bail!("route prefix must contain a path segment: {}", self.prefix);
        }
        Ok(())
    }

    pub fn matches(&self, path: &str) -> bool {
        prefix_matches(normalized_prefix(&self.prefix), path)
    }

    pub fn upstream_url(&self, path: &str, query: Option<&str>) -> Result<Url> {
        let prefix = normalized_prefix(&self.prefix);
        let suffix = if self.strip_prefix {
            path.strip_prefix(prefix).unwrap_or(path)
        } else {
            path
        };

        let suffix = suffix.trim_start_matches('/');
        let mut url = self.upstream.clone();
        let base_path = url.path().trim_end_matches('/');

        let merged_path = match (base_path.is_empty() || base_path == "/", suffix.is_empty()) {
            (true, true) => "/".to_string(),
            (true, false) => format!("/{}", suffix),
            (false, true) => base_path.to_string(),
            (false, false) => format!("{}/{}", base_path, suffix),
        };

        url.set_path(&merged_path);
        url.set_query(query);
        Ok(url)
    }
}

fn prefix_matches(prefix: &str, path: &str) -> bool {
    if prefix == "/" {
        return true;
    }

    path == prefix || path.starts_with(&format!("{}/", prefix.trim_end_matches('/')))
}

fn normalized_prefix(prefix: &str) -> &str {
    if prefix == "/" {
        "/"
    } else {
        prefix.trim_end_matches('/')
    }
}

fn default_bind() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8787))
}

fn default_max_body_bytes() -> usize {
    10 * 1024 * 1024
}

fn default_request_timeout_ms() -> u64 {
    30_000
}

fn default_true() -> bool {
    true
}

pub fn config_dir() -> PathBuf {
    home_dir().join(".config")
}

pub fn default_config_path() -> PathBuf {
    config_dir().join("itadori.toml")
}

pub fn default_pid_path() -> PathBuf {
    config_dir().join("itadori.pid")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
