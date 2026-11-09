//! `fastmind mcp` speaks MCP to a coding agent: handshake, tools, search, saving a finding.

use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn an_agent_can_search_and_save_findings() {
    let dir = tempfile::tempdir().unwrap();
    let notebook = dir.path().join("nb");
    std::fs::create_dir_all(&notebook).unwrap();
    std::fs::write(
        notebook.join("Ledger lag.md"),
        "# Ledger lag\n\nThe nightly batch lags because entries.posted_at had no index.\n",
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_fastmind"))
        .args(["--notebook", notebook.to_str().unwrap(), "mcp"])
        .env("FASTMIND_CONFIG", dir.path().join("settings.json"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "search_notes", "arguments": {"query": "ledger batch lag"}}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "save_finding", "arguments": {"title": "Index posted_at to fix the ledger lag", "body": "Added migration 42. password=hunter22"}}}),
        json!({"jsonrpc": "2.0", "id": 5, "method": "nope"}),
    ];
    {
        let stdin = child.stdin.as_mut().unwrap();
        for request in &requests {
            writeln!(stdin, "{request}").unwrap();
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        replies.len(),
        5,
        "one reply per request, none for the notification"
    );
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "fastmind");
    assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 4);
    assert!(
        replies[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("path: Ledger lag.md")
    );
    assert_eq!(replies[3]["result"]["isError"], false);
    assert_eq!(replies[4]["error"]["code"], -32601);

    let saved =
        std::fs::read_to_string(notebook.join("inbox/Index posted_at to fix the ledger lag.md"))
            .unwrap();
    assert!(
        saved.contains("tags: [agent]") && !saved.contains("hunter22"),
        "{saved}"
    );
}
