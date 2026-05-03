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
