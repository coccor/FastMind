//! `fastmind mcp`: the notebook as an MCP server on stdin/stdout, for coding agents (Claude Code,
//! Codex, Claude Desktop…). Agents search and read notes before they dig into something, and
//! save what they find as fresh memories, which Dream settles overnight. Its `note` prompt is a
//! slash command in Claude Code (`/mcp__fastmind__note`): write up this session and save it.

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
            "description": "Save something worth keeping to the user's notebook as a fresh memory (FastMind settles it beside related notes overnight): a root cause, a decision and its reasons, a trap, a procedure that worked, or an approach that failed. Never include secrets, tokens or passwords.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "The finding as a short claim, at most eight words, e.g. 'Batch job's 30 s poll causes consumer lag'"},
                    "summary": {"type": "string", "description": "One sentence that answers the question on its own"},
                    "body": {"type": "string", "description": "Markdown, without the title: only the sections that apply, e.g. ## Problem, ## What was found, ## Decision, ## What worked (commands in fenced blocks), ## Watch out"},
                    "kind": {"type": "string", "enum": ["finding", "decision", "how-to", "dead-end", "gotcha", "investigation"], "description": "What the note is"},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "Up to five lowercase tags"}
                },
                "required": ["title", "summary", "body"]
            }
        }
    ])
}

fn prompts() -> Value {
    json!([{
        "name": "note",
        "title": "Note this session",
        "description": "Write up what we did in this session as a note in your FastMind notebook",
        "arguments": [{
            "name": "focus",
            "description": "What the note should be about, if not the whole session (optional)",
            "required": false
        }]
    }])
}

/// The `note` prompt: write up this session, the way the notebook's ai-session skill says,
/// and save it.
fn note_prompt(app: &App, args: &Value) -> Result<Value, Value> {
    let skill = app
        .call("skill_get", json!({ "name": "ai-session" }))
        .map_err(|e| json!({ "code": -32603, "message": e.0 }))?;
    let text = skill["skill"]["text"].as_str().unwrap_or("");
    let skill_body = fastmind_core::frontmatter::body(text).trim();
    let preferences = app.settings().writing.instructions;
    let focus = args["focus"]
        .as_str()
        .map(str::trim)
        .filter(|f| !f.is_empty());
    let mut prompt = String::from(
        "Write up the work we did in this session as a note for my FastMind notebook, and save it.\n\n\
         1. First call search_notes with a few words about the work, to find notes already about it.\n\
         2. Write the note from this conversation, following the skill below. Write about what was learned, \
         decided and done, not the conversation itself. If the session covered separate pieces of work, write \
         one note for each (at most three).\n\
         3. Save each with the save_finding tool: title, summary, body (without the title), kind and tags. \
         Under `## Related` in the body, link notes search_notes found that are about the same work, as [[Their title]].\n\
         4. Then tell me in one line what you saved.\n",
    );
    if let Some(focus) = focus {
        prompt.push_str(&format!("\nFocus the note on: {focus}\n"));
    }
    if !preferences.trim().is_empty() {
        prompt.push_str(&format!(
            "\nMy preferences for how notes are written: {}\n",
            preferences.trim()
        ));
    }
    prompt.push_str(&format!("\n---\n\n{skill_body}\n"));
    Ok(json!({
        "description": "Write up this session as a FastMind note",
        "messages": [{ "role": "user", "content": { "type": "text", "text": prompt } }]
    }))
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
            let summary = args["summary"]
                .as_str()
                .map(fastmind_core::sessions::redact);
            app.call(
                "capture",
                json!({
                    "text": text, "tags": tags, "as_written": true, "note_type": "finding",
                    "kind": args["kind"], "summary": summary,
                }),
            )
            .map(|saved| {
                format!(
                    "Saved \"{}\" as a fresh memory: {}",
                    saved["title"].as_str().unwrap_or(""),
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
                "capabilities": { "tools": {}, "prompts": {} },
                "serverInfo": { "name": "fastmind", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "The user's notebook. Search it before investigating something that may already be written down; save findings worth keeping with save_finding. The note prompt writes up the session."
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "prompts/list" => Ok(json!({ "prompts": prompts() })),
            "prompts/get" => match params["name"].as_str().unwrap_or("") {
                "note" => note_prompt(app, &params["arguments"]),
                other => {
                    Err(json!({ "code": -32602, "message": format!("no prompt called {other}") }))
                }
            },
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
