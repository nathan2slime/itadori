use itadori::config::{GatewayConfig, RouteConfig};
use std::fs;
use tempfile::tempdir;
use url::Url;

#[test]
fn longest_matching_route_wins() {
    let config = GatewayConfig {
        server: Default::default(),
        routes: vec![
            route("root", "/", "https://example.com/"),
            route("jira", "/jira", "https://example.com/jira"),
            route("jira-api", "/jira/api", "https://example.com/api"),
        ],
    };

    assert_eq!(
        config.route_for_path("/jira/api/issues").unwrap().name,
        "jira-api"
    );
    assert_eq!(config.route_for_path("/jira/boards").unwrap().name, "jira");
}

#[test]
fn builds_upstream_url_with_prefix_stripping() {
    let route = route("jira", "/jira", "https://example.com/api");
    let url = route
        .upstream_url("/jira/issues/123", Some("expand=all"))
        .unwrap();

    assert_eq!(
        url.as_str(),
        "https://example.com/api/issues/123?expand=all"
    );
}

#[test]
fn validates_duplicate_prefixes() {
    let mut config = GatewayConfig {
        server: Default::default(),
        routes: vec![
            route("one", "/a", "https://example.com/a"),
            route("two", "/a/", "https://example.com/b"),
        ],
    };

    assert!(config.validate().is_err());
}

#[test]
fn loads_example_config_from_disk() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("gateway.toml");
    fs::write(&path, GatewayConfig::example_toml()).unwrap();

    let config = GatewayConfig::load(&path).unwrap();

    assert_eq!(config.server.bind.to_string(), "127.0.0.1:8787");
    assert_eq!(config.routes.len(), 2);
    assert!(config.route_for_path("/jira/issues").is_some());
    assert!(config.route_for_path("/slack/messages").is_some());
}

#[test]
fn keeps_prefix_when_strip_prefix_is_disabled() {
    let mut route = route("docs", "/docs", "https://example.com/api");
    route.strip_prefix = false;

    let url = route
        .upstream_url("/docs/guide/getting-started", None)
        .unwrap();

    assert_eq!(
        url.as_str(),
        "https://example.com/api/docs/guide/getting-started"
    );
}

fn route(name: &str, prefix: &str, upstream: &str) -> RouteConfig {
    RouteConfig {
        name: name.to_string(),
        prefix: prefix.to_string(),
        upstream: Url::parse(upstream).unwrap(),
        strip_prefix: true,
        headers: Default::default(),
        timeout_ms: None,
    }
}
