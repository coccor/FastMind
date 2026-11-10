//! `fastmind mcp`: the notebook as an MCP server on stdin/stdout, for coding agents (Claude Code,
//! Codex, Claude Desktop…). Agents search and read notes before they dig into something, and
//! save what they find to the inbox, where you file it like any other capture.

use fastmind_core::api::App;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

const PROTOCOL: &str = "2025-06-18";

fn tools() -> Value {
    json!([
        {
            "name": "search_notes",
            "description": "Search the user's notebook (plain Markdown notes, including notes from past AI coding sessions). Use it before investigating a bug, a subsystem or a decision: the answer may already be written down. Typos are tolerated; #tag filters by tag.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words to look for, or #tag"},
                    "limit": {"type": "integer", "description": "Most results to return (default 8)"}
                },
                "required": ["query"]
            }
        },
        {
            "name": "read_note",
            "description": "Read one note in full, by the path search_notes returned.",
            "inputSchema": {
                "type": "object",
                "properties": {"path": {"type": "string", "description": "The note's path inside the notebook"}},
                "required": ["path"]
            }
        },
        {
            "name": "ask_notebook",
            "description": "Ask the notebook a question. Returns the passages that answer it (or an answer written from them, when the user set up a model), each with the note it came from.",
            "inputSchema": {
                "type": "object",
                "properties": {"question": {"type": "string"}},
                "required": ["question"]
            }
        },
        {
            "name": "save_finding",
            "description": "Save something worth keeping to the user's notebook inbox, for them to review: a root cause, a decision and its reasons, a trap, a procedure that worked, or an approach that failed. Write a title that states the finding, then the details. Never include secrets.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "The finding stated as a claim, e.g. 'Consumer lag comes from the batch job's 30 s poll'"},
                    "body": {"type": "string", "description": "Markdown: what was found, the evidence, what to do"},
                    "tags": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["title", "body"]
            }
        }
    ])
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn call_tool(app: &App, name: &str, args: &Value) -> Value {
    let result = match name {
        "search_notes" => app
            .call(
                "search",
                json!({ "query": args["query"], "limit": args["limit"].as_u64().unwrap_or(8) }),
            )
            .map(|hits| {
                let hits = hits.as_array().cloned().unwrap_or_default();
                if hits.is_empty() {
                    return "No notes match.".to_owned();
                }
                hits.iter()
                    .map(|h| {
                        let archived = if h["archived"].is_null() {
                            ""
                        } else {
                            " (archived)"
                        };
                        format!(
                            "- {}{archived}\n  path: {}\n  {}",
                            h["title"].as_str().unwrap_or(""),
                            h["path"].as_str().unwrap_or(""),
                            h["snippet"].as_str().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
        "read_note" => app
            .call("read", json!({ "path": args["path"] }))
            .map(|note| note["text"].as_str().unwrap_or("").to_owned()),
        "ask_notebook" => app
            .call("ask", json!({ "question": args["question"] }))
            .map(|answer| {
                let sources: Vec<String> = answer["sources"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| {
                        format!(
                            "[{}] {} ({})",
                            s["n"],
                            s["title"].as_str().unwrap_or(""),
                            s["path"].as_str().unwrap_or("")
                        )
                    })
                    .collect();
                format!(
                    "{}\n\n{}",
                    answer["text"].as_str().unwrap_or(""),
                    sources.join("\n")
                )
            }),
        "save_finding" => {
            let title = args["title"].as_str().unwrap_or("").trim();
            let body = args["body"].as_str().unwrap_or("").trim();
            let mut tags: Vec<String> = args["tags"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(str::to_owned))
                .collect();
            tags.push("agent".into());
            let text = fastmind_core::sessions::redact(&format!("{title}\n\n{body}"));
            app.call(
                "capture",
                json!({ "text": text, "tags": tags, "as_written": true }),
            )
            .map(|saved| {
                format!(
                    "Saved to the inbox as {}",
                    saved["path"].as_str().unwrap_or("")
                )
            })
        }
        other => Err(fastmind_core::Error::msg(format!("no tool called {other}"))),
    };
    match result {
        Ok(text) => text_result(text, false),
        Err(error) => text_result(error.0, true),
    }
}

/// Serves MCP over stdin/stdout (one JSON-RPC message per line) until stdin closes.
pub fn serve(app: &App) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            let error = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}});
            writeln!(stdout, "{error}")?;
            stdout.flush()?;
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            continue; // A notification (initialized, cancelled…): nothing to answer.
        };
        let method = message["method"].as_str().unwrap_or("");
        let params = &message["params"];
        let reply = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fastmind", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "The user's notebook. Search it before investigating something that may already be written down; save findings worth keeping with save_finding."
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => Ok(call_tool(
                app,
                params["name"].as_str().unwrap_or(""),
                &params["arguments"],
            )),
            _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
        };
        let response = match reply {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        };
        writeln!(stdout, "{response}")?;
        stdout.flush()?;
    }
    Ok(())
}
