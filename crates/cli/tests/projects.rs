//! A project with a notebook of its own: `fastmind project set`, then commands and the MCP
//! server run inside the project use that notebook, and everything else the usual one.

use serde_json::json;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn fastmind(config: &Path, cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fastmind"));
    command.env("FASTMIND_CONFIG", config).current_dir(cwd);
    command
}

#[test]
fn work_inside_a_project_goes_to_its_notebook() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let usual = root.join("usual");
    let project = root.join("pz-connector-kafka");
    let planning = root.join("pz-planning/pz-connector-kafka/notes");
    std::fs::create_dir_all(&usual).unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    let config = root.join("settings.json");
    std::fs::write(
        &config,
        json!({"notebook": usual, "model": {"provider": "claude-code", "model": "sonnet"}})
            .to_string(),
    )
    .unwrap();

    let set = fastmind(&config, &project)
        .args(["project", "set", planning.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(set.status.success(), "{set:?}");
    assert!(planning.is_dir(), "the notebook is made");
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(
        settings["notebook"],
        json!(usual),
        "the usual notebook stays"
    );

    // The MCP server, started in a subfolder of the project, saves into the project's notebook.
    let mut mcp = fastmind(&config, &project.join("src"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let save = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "save_finding", "arguments": {"title": "Kafka offsets commit after the batch", "summary": "Offsets are committed once the batch is written.", "body": "## What was found\nCommitting first lost rows."}}});
    writeln!(mcp.stdin.as_mut().unwrap(), "{save}").unwrap();
    drop(mcp.stdin.take());
    let reply = String::from_utf8(mcp.wait_with_output().unwrap().stdout).unwrap();
    assert!(reply.contains("Saved"), "{reply}");
    let saved = planning.join("Fresh memories/Kafka offsets commit after the batch.md");
    assert!(saved.is_file(), "{reply}");

    // Outside the project, the usual notebook.
    let status = |cwd: &Path| {
        let out = fastmind(&config, cwd).arg("status").output().unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    assert!(status(&project).contains(planning.to_str().unwrap()));
    assert!(status(root).contains(usual.to_str().unwrap()));

    let unset = fastmind(&config, &project)
        .args(["project", "unset"])
        .output()
        .unwrap();
    assert!(unset.status.success(), "{unset:?}");
    assert!(status(&project).contains(usual.to_str().unwrap()));
}
