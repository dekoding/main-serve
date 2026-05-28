mod support;

use crate::support::helpers::load_yaml;

#[test]
fn test_endpoint_proxy_parses() {
    let yaml = r#"
endpoints:
  - path: "/proxy/*"
    methods: ["get"]
    action: "proxy"
    proxy:
      upstream: "https://example.com"
      timeouts:
        connect: 3
        total: 30
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let proxy = config.endpoints[0].proxy.as_ref().unwrap();
    assert_eq!(proxy.upstream, "https://example.com");
    assert_eq!(proxy.timeouts.connect, 3);
    assert_eq!(proxy.timeouts.total, 30);
    assert_eq!(proxy.timeouts.read, 30); // default
}

#[test]
fn test_proxy_with_path_rewrite_parses() {
    let yaml = r#"
endpoints:
  - path: "/gateway/*"
    methods: ["get"]
    action: "proxy"
    proxy:
      upstream: "http://backend:3000"
      path_rewrite:
        strip_prefix: "/gateway"
        add_prefix: "/api"
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let proxy = config.endpoints[0].proxy.as_ref().unwrap();
    assert_eq!(proxy.upstream, "http://backend:3000");
    let rewrite = proxy.path_rewrite.as_ref().unwrap();
    assert_eq!(rewrite.strip_prefix, "/gateway");
    assert_eq!(rewrite.add_prefix, "/api");
}

#[test]
fn test_proxy_with_custom_headers_parses() {
    let yaml = r#"
endpoints:
  - path: "/proxy/*"
    methods: ["get"]
    action: "proxy"
    proxy:
      upstream: "http://api.example.com"
      headers:
        X-API-Key: "secret-key"
        X-Forwarded-For: "true"
      max_response_size: 5242880
    auth: "none"
"#;
    let config = load_yaml(yaml).unwrap();
    let proxy = config.endpoints[0].proxy.as_ref().unwrap();
    assert_eq!(proxy.upstream, "http://api.example.com");
    assert_eq!(proxy.max_response_size, 5242880);
    assert!(proxy.headers.contains_key("X-API-Key"));
    assert_eq!(proxy.headers.get("X-API-Key").unwrap(), "secret-key");
}

#[test]
fn test_proxy_missing_config_fails_validation() {
    let yaml = r#"
endpoints:
  - path: "/proxy/*"
    methods: ["get"]
    action: "proxy"
    auth: "none"
"#;
    let err = load_yaml(yaml).unwrap_err();
    assert!(
        err.to_string().contains("proxy config"),
        "Should fail validation when proxy action has no proxy config, got: {err}"
    );
}

#[test]
fn test_proxy_missing_upstream_fails_validation() {
    let yaml = r#"
endpoints:
  - path: "/proxy/*"
    methods: ["get"]
    action: "proxy"
    proxy: {}
    auth: "none"
"#;
    let err = load_yaml(yaml).unwrap_err();
    assert!(
        err.to_string().contains("upstream"),
        "Should fail when upstream is missing, got: {err}"
    );
}
