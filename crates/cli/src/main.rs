//! `fastmind`: the notebook from a terminal or a script. Every command goes through the same
//! command surface as the desktop app.

mod mcp;
mod serve;

use fastmind_core::api::App;
use fastmind_core::config::Settings;
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

const HELP: &str = "FastMind: capture, remember, dream. Plain Markdown, any OS.

Usage: fastmind [--notebook DIR] [--json] <command> [args]

  capture TEXT [--url URL] [--tag TAG]... [--attach FILE]... [--folder DIR] [--raw]
                          Save a note in the inbox (TEXT \"-\" reads stdin). With a
                          model chosen, it writes the note before returning; --raw
                          saves it as typed
  write NOTE              Write a captured note again from what was captured
  search QUERY            Find notes (#tag filters by tag)
  ask QUESTION            Answer from your notes, with sources
  inbox                   Notes waiting in the inbox
  status                  The notebook at a glance

  dream                   Look over the notebook and propose how to compress it
  dream show              The latest proposals
  dream apply ID|all [--kinds merge,topic,archive,fix_link,pack]
  dream reject ID
  dream undo ID           Undo an applied proposal

  history                 Changes FastMind made, newest first
  undo ENTRY              Undo one change (refused if the files were edited since)
  archive                 Archived notes
  restore ID              Put an archived note back

  model claude-code|claude|local [MODEL] [--url URL]
                          Choose the AI FastMind runs on, and check it answers
                          (claude-code uses your Claude plan; claude needs
                          ANTHROPIC_API_KEY; local is Ollama, LM Studio…)
  setup DIR               Use DIR as the notebook (made if missing)
  demo [DIR]              Make a messy demo notebook and use it
  serve [--port N] [--ui DIR]
                          Serve the app's UI and API on localhost (for a browser)
  mcp                     Serve the notebook to coding agents (MCP over stdio)
  settings                Show where settings live and what they are

Options:
  --notebook DIR          Use this notebook instead of the one in settings
  --json                  Print raw JSON

Exit codes: 0 done, 1 failed, 2 bad arguments.";

struct Args {
    rest: Vec<String>,
    json: bool,
    notebook: Option<PathBuf>,
}

fn parse() -> Result<Args, String> {
    let mut rest = Vec::new();
    let mut json = false;
    let mut notebook = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--notebook" | "-n" => {
                notebook = Some(PathBuf::from(
                    args.next().ok_or("--notebook needs a folder")?,
                ))
            }
            _ => rest.push(arg),
        }
    }
    Ok(Args {
        rest,
        json,
        notebook,
    })
}

fn take_flag(rest: &mut Vec<String>, name: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == name && i + 1 < rest.len() {
            values.push(rest.remove(i + 1));
            rest.remove(i);
        } else {
            i += 1;
        }
    }
    values
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let mut settings = Settings::load();
    let persist = args.notebook.is_none();
    if let Some(notebook) = &args.notebook {
        settings.notebook = Some(notebook.clone());
    }
    let app = if persist {
        App::new(settings)
    } else {
        App::ephemeral(settings)
    };
    match run(&app, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("{message}\n\nRun `fastmind help` for the commands.");
            ExitCode::from(2)
        }
        Err(Failure::Failed(message)) => {
            eprintln!("fastmind: {message}");
            ExitCode::from(1)
        }
    }
}

enum Failure {
    Usage(String),
    Failed(String),
}

impl From<fastmind_core::Error> for Failure {
    fn from(error: fastmind_core::Error) -> Self {
        Failure::Failed(error.0)
    }
}

fn usage(message: &str) -> Failure {
    Failure::Usage(message.to_owned())
}

fn s(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

fn run(app: &App, args: Args) -> Result<(), Failure> {
    let mut rest = args.rest;
    if rest.is_empty() {
        println!("{HELP}");
        return Ok(());
    }
    let command = rest.remove(0);
    let out = |value: &Value, pretty: &dyn Fn(&Value)| {
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(value).unwrap_or_default()
            );
        } else {
            pretty(value);
        }
    };
    match command.as_str() {
        "help" | "--help" | "-h" => println!("{HELP}"),
        "capture" | "add" | "learn" => {
            let url = take_flag(&mut rest, "--url").pop();
            let tags = take_flag(&mut rest, "--tag");
            let attachments: Vec<PathBuf> = take_flag(&mut rest, "--attach")
                .into_iter()
                .map(PathBuf::from)
                .collect();
            let folder = take_flag(&mut rest, "--folder").pop();
            let raw = if let Some(at) = rest.iter().position(|a| a == "--raw") {
                rest.remove(at);
                true
            } else {
                false
            };
            let mut text = rest.join(" ");
            if text == "-" {
                text.clear();
                std::io::stdin()
                    .read_to_string(&mut text)
                    .map_err(|e| Failure::Failed(e.to_string()))?;
            }
            let writes = !raw && app.settings().model.is_on() && app.settings().writing.enabled;
            if writes && !args.json {
                eprintln!("Writing the note with {}…", app.settings().model.label());
            }
            let saved = app.call("capture", json!({"text": text, "url": url, "tags": tags, "attachments": attachments, "folder": folder, "wait": writes, "as_written": raw}))?;
            out(&saved, &|v| {
                println!("Saved \"{}\" as {}", s(&v["title"]), s(&v["path"]));
                if let Some(error) = v["write_error"].as_str() {
                    println!("  Kept as you typed it: {error}");
                }
            });
        }
        "write" | "rewrite" => {
            let path = rest.join(" ");
            if path.is_empty() {
                return Err(usage("write needs a note's path"));
            }
            if !args.json {
                eprintln!("Writing the note with {}…", app.settings().model.label());
            }
            let done = app.call("rewrite_note", json!({"path": path}))?;
            out(&done, &|v| {
                println!(
                    "Wrote \"{}\" as {} (undo it from `fastmind history`)",
                    s(&v["title"]),
                    s(&v["path"])
                )
            });
        }
        "search" | "find" => {
            if rest.is_empty() {
                return Err(usage("search needs a query"));
            }
            let hits = app.call("search", json!({"query": rest.join(" "), "limit": 10}))?;
            out(&hits, &|v| {
                let hits = v.as_array().cloned().unwrap_or_default();
                if hits.is_empty() {
                    println!("No notes match.");
                }
                for hit in hits {
                    let archived = if hit["archived"].is_null() {
                        ""
                    } else {
                        " (archived)"
                    };
                    println!(
                        "{}{archived}\n  {}:{}\n  {}\n",
                        s(&hit["title"]),
                        s(&hit["path"]),
                        hit["line"],
                        s(&hit["snippet"])
                    );
                }
            });
        }
        "ask" | "remember" => {
            if rest.is_empty() {
                return Err(usage("ask needs a question"));
            }
            let answer = app.call("ask", json!({"question": rest.join(" ")}))?;
            out(&answer, &|v| {
                println!("{}\n", s(&v["text"]));
                for source in v["sources"].as_array().cloned().unwrap_or_default() {
                    println!(
                        "[{}] {} ({}:{})",
                        source["n"],
                        s(&source["title"]),
                        s(&source["path"]),
                        source["line"]
                    );
                }
                if let Some(note) = v["note"].as_str() {
                    println!("\n{note}");
                }
            });
        }
        "inbox" => {
            let notes = app.call("inbox", json!({}))?;
            out(&notes, &|v| {
                let notes = v.as_array().cloned().unwrap_or_default();
                println!("{} in the inbox", notes.len());
                for note in notes {
                    println!("- {} ({})", s(&note["title"]), s(&note["path"]));
                }
            });
        }
        "status" => {
            let status = app.call("status", json!({}))?;
            out(&status, &|v| {
                println!("Notebook  {}", s(&v["notebook"]));
                println!(
                    "Notes     {} ({} in the inbox, {} archived)",
                    v["notes"], v["inbox"], v["archived"]
                );
                println!("Model     {}", s(&v["model"]));
                match v["dream"].as_object() {
                    Some(dream) => println!(
                        "Dream     {} ({} waiting)",
                        s(&dream["summary"]),
                        dream["pending"]
                    ),
                    None => println!("Dream     hasn't run yet"),
                }
            });
        }
        "dream" => dream(app, rest, args.json)?,
        "history" => {
            let entries = app.call("history", json!({}))?;
            out(&entries, &|v| {
                for entry in v.as_array().cloned().unwrap_or_default() {
                    let state = if entry["undone"].as_bool() == Some(true) {
                        " (undone)"
                    } else if entry["can_undo"].as_bool() == Some(false) {
                        " (edited since)"
                    } else {
                        ""
                    };
                    println!("{}  {}{state}", s(&entry["id"]), s(&entry["label"]));
                }
            });
        }
        "undo" => {
            let id = rest
                .first()
                .ok_or_else(|| usage("undo needs a history entry id"))?;
            let done = app.call("undo", json!({"id": id}))?;
            out(&done, &|v| println!("Undid: {}", s(&v["undone"])));
        }
        "archive" => {
            let list = app.call("archive_list", json!({}))?;
            out(&list, &|v| {
                let items = v["items"].as_array().cloned().unwrap_or_default();
                println!(
                    "{} archived ({} on disk)",
                    items.len(),
                    human(v["disk_bytes"].as_u64().unwrap_or(0))
                );
                for item in items {
                    let packed = if item["pack"].is_null() {
                        ""
                    } else {
                        " [packed]"
                    };
                    println!(
                        "{}  {} — {}{packed}",
                        s(&item["id"]),
                        s(&item["original"]),
                        s(&item["reason"])
                    );
                }
            });
        }
        "restore" => {
            let id = rest
                .first()
                .ok_or_else(|| usage("restore needs an archive id"))?;
            let done = app.call("archive_restore", json!({"id": id}))?;
            out(&done, &|v| println!("Restored to {}", s(&v["path"])));
        }
        "model" | "ai" => {
            let url = take_flag(&mut rest, "--url").pop();
            let provider = rest.first().map(String::as_str).unwrap_or("");
            let name = rest.get(1).cloned();
            let model = match provider {
                "claude-code" | "claudecode" => {
                    json!({"provider": "claude-code", "model": name.unwrap_or_else(|| "sonnet".into())})
                }
                "claude" => {
                    json!({"provider": "claude", "model": name.unwrap_or_else(|| "claude-opus-5-5".into())})
                }
                "local" => {
                    json!({"provider": "local", "model": name.unwrap_or_else(|| "llama3.2".into()), "base_url": url.unwrap_or_else(|| "http://localhost:11434/v1".into())})
                }
                "" => {
                    let settings = app.call("settings_get", json!({}))?;
                    println!(
                        "FastMind runs on: {}",
                        serde_json::to_string(&settings["settings"]["model"]).unwrap_or_default()
                    );
                    return Ok(());
                }
                other => {
                    return Err(usage(&format!(
                        "unknown provider \"{other}\": use claude-code, claude or local"
                    )));
                }
            };
            eprintln!("Checking it answers…");
            let check = app.call("model_check", json!({"model": model}))?;
            let mut settings = app.call("settings_get", json!({}))?["settings"].clone();
            settings["model"] = model;
            app.call("settings_set", json!({"settings": settings}))?;
            println!(
                "FastMind now runs on it (answered in {:.1} s).",
                check["ms"].as_f64().unwrap_or(0.0) / 1000.0
            );
        }
        "setup" => {
            let folder = rest
                .first()
                .ok_or_else(|| usage("setup needs the folder for your notes"))?;
            let folder = std::path::absolute(folder).map_err(|e| Failure::Failed(e.to_string()))?;
            app.call("setup", json!({"notebook": folder}))?;
            println!("FastMind now uses {}", folder.display());
        }
        "demo" => {
            let done = app.call("demo_create", json!({"path": rest.first()}))?;
            out(&done, &|v| {
                println!(
                    "Made a demo notebook with {} notes at {}",
                    v["notes"],
                    s(&v["path"])
                );
                println!("It's now your notebook in FastMind's settings. Try: fastmind dream");
            });
        }
        "settings" => {
            let settings = app.call("settings_get", json!({}))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&settings).unwrap_or_default()
            );
        }
        "mcp" => mcp::serve(app).map_err(|e| Failure::Failed(e.to_string()))?,
        "serve" => {
            let port: u16 = take_flag(&mut rest, "--port")
                .pop()
                .and_then(|p| p.parse().ok())
                .unwrap_or(7878);
            let ui = take_flag(&mut rest, "--ui").pop().map(PathBuf::from);
            serve::serve(app, port, ui).map_err(Failure::Failed)?;
        }
        other => return Err(usage(&format!("unknown command: {other}"))),
    }
    Ok(())
}

fn dream(app: &App, mut rest: Vec<String>, json_out: bool) -> Result<(), Failure> {
    let kinds: Vec<String> = take_flag(&mut rest, "--kinds")
        .iter()
        .flat_map(|list| list.split(',').map(str::to_owned).collect::<Vec<_>>())
        .collect();
    let sub = if rest.is_empty() {
        "run".to_owned()
    } else {
        rest.remove(0)
    };
    let print = |value: &Value| {
        if json_out {
            println!(
                "{}",
                serde_json::to_string_pretty(value).unwrap_or_default()
            );
        }
    };
    match sub.as_str() {
        "run" => {
            eprintln!("Dreaming…");
            let result = app.call("dream_run", json!({}))?;
            print(&result);
            if !json_out {
                show_report(&result["report"]);
                let auto = result["auto_applied"].as_u64().unwrap_or(0);
                if auto > 0 {
                    println!(
                        "\n{auto} applied on their own (Settings › Dream › apply without asking)."
                    );
                }
            }
        }
        "show" => {
            let report = app.call("dream_latest", json!({}))?;
            print(&report);
            if !json_out {
                show_report(&report);
            }
        }
        "apply" => {
            let id = rest
                .first()
                .ok_or_else(|| usage("dream apply needs a proposal id or all"))?;
            let result = if id == "all" {
                app.call("dream_apply_all", json!({"kinds": kinds}))?
            } else {
                json!([app.call("dream_apply", json!({"id": id}))?])
            };
            print(&result);
            if !json_out {
                for proposal in result.as_array().cloned().unwrap_or_default() {
                    let extra = proposal["error"]
                        .as_str()
                        .map(|e| format!(" — {e}"))
                        .unwrap_or_default();
                    println!(
                        "{} {}: {}{extra}",
                        s(&proposal["id"]),
                        s(&proposal["status"]),
                        s(&proposal["title"])
                    );
                }
            }
        }
        "reject" => {
            let id = rest
                .first()
                .ok_or_else(|| usage("dream reject needs a proposal id"))?;
            let result = app.call("dream_reject", json!({"id": id}))?;
            print(&result);
            if !json_out {
                println!("Rejected {}", s(&result["title"]));
            }
        }
        "undo" => {
            let id = rest
                .first()
                .ok_or_else(|| usage("dream undo needs a proposal id"))?;
            let result = app.call("dream_undo", json!({"id": id}))?;
            print(&result);
            if !json_out {
                println!("Undid {}", s(&result["title"]));
            }
        }
        other => return Err(usage(&format!("unknown dream command: {other}"))),
    }
    Ok(())
}

fn show_report(report: &Value) {
    if report.is_null() {
        println!("Dream hasn't run on this notebook yet. Run: fastmind dream");
        return;
    }
    println!("{}\n", s(&report["summary"]));
    for proposal in report["proposals"].as_array().cloned().unwrap_or_default() {
        let mark = match s(&proposal["status"]) {
            "applied" => "✓",
            "rejected" => "✗",
            "outdated" => "!",
            _ => "•",
        };
        let freed = proposal["bytes_freed"].as_i64().unwrap_or(0)
            + proposal["disk_freed"].as_i64().unwrap_or(0);
        let freed = if freed > 0 {
            format!("  [-{}]", human(freed as u64))
        } else {
            String::new()
        };
        println!(
            "{mark} {:<4} {}{freed}",
            s(&proposal["id"]),
            s(&proposal["title"])
        );
        println!("       {}", s(&proposal["reason"]));
    }
    for note in report["model_notes"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        println!("\nModel: {}", s(&note));
    }
    let pending = report["proposals"].as_array().map_or(0, |all| {
        all.iter().filter(|p| p["status"] == "pending").count()
    });
    if pending > 0 {
        println!(
            "\nApply one with `fastmind dream apply p1`, or all with `fastmind dream apply all`."
        );
    }
}

fn human(bytes: u64) -> String {
    fastmind_core::dream::human_bytes(bytes)
}
