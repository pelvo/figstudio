//! Pairing banner and default origins: no deploy domain is built in (docs/WEB-MCP-PLAN.md R9).
use figstudio_mcp::bridge::{DEFAULT_ORIGINS, web_banner};

#[test]
fn only_local_origins_are_allowed_by_default() {
    assert!(DEFAULT_ORIGINS.iter().all(|o| o.starts_with("http://")));
    assert!(web_banner(&[], "tok").is_empty());
}

#[test]
fn each_extra_https_origin_gets_a_web_line() {
    let extra = vec![
        "https://example.test/".to_owned(),
        "http://lan.test:1".to_owned(),
        "https://b.test".to_owned(),
    ];
    assert_eq!(
        web_banner(&extra, "tok"),
        [
            "Web: https://example.test/#mcp=tok",
            "Web: https://b.test/#mcp=tok"
        ]
    );
}
