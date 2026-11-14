//! Trusting a backend's certificate is the user's trust decision: no AI or
//! MCP tool fetches or pins one, under any of its names.

const NAMES: [&str; 6] = [
    "backends.trust-certificate",
    "backends.trust_certificate",
    "ui.backends.certificate.fetch",
    "ui.backends.certificate.trust",
    "ui_backends_certificate_fetch",
    "ui_backends_certificate_trust",
];

#[test]
fn no_ai_or_mcp_tool_fetches_or_trusts_backend_certificates() {
    for name in NAMES {
        assert!(kassiber_ai::get_tool(name).is_none(), "AI tool {name}");
        let wire = kassiber_pyjson::PyStr::from(name);
        assert!(
            kassiber_mcp::tools::entry_for(&wire).is_none(),
            "MCP tool {name}"
        );
    }
    let reaches =
        |name: &str, kind: &str| name.contains("certificate") || kind.contains("certificate");
    for tool in kassiber_ai::tools() {
        let kind = tool.daemon_kind.as_deref().unwrap_or_default();
        assert!(
            !reaches(&tool.name, kind),
            "AI tool {} reaches {kind}",
            tool.name
        );
    }
    for tool in kassiber_mcp::tools::mcp_tools() {
        let kind = tool.entry.daemon_kind.as_deref().unwrap_or_default();
        assert!(
            !reaches(&tool.name, kind),
            "MCP tool {} reaches {kind}",
            tool.name
        );
    }
}
