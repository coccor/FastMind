# FastMind reference

- [Keys](#keys)
- [Choosing the AI](#choosing-the-ai)
- [The command line](#the-command-line)
- [Settings](#settings)

## Keys

| Key | |
|---|---|
| **Ctrl+Alt+Space** (anywhere) | Capture. **Cmd+Option+Space** on macOS. |
| **Ctrl+Alt+R** (anywhere) | Remember: search and ask |
| **Ctrl+Enter** | Save a capture, or ask the question in the search box |
| **Esc** | Close the capture box, keeping a draft |
| **↑ ↓ / Enter** | Move through results / read |
| **Ctrl+E**, **Ctrl+S** | Edit the open note, save it |
| Fresh memories: **Enter**, **F**, **K**, **A** | Settle in the suggestion, another folder, keep where it is, archive |
| **Ctrl+K** / **Ctrl+N** | Search / capture, inside the app |

Hotkeys can be changed in Settings and apply at once. Closing the window keeps FastMind in the
tray, where it dreams at night.

### Hotkeys on Linux (Wayland)

Wayland doesn't let an app take keys for the whole desktop, so there FastMind asks the desktop's
global shortcuts service (the XDG portal) for them, and Settings › Hotkeys says how that went:

- **KDE, GNOME**: the desktop may ask you to confirm the keys; change them later in its keyboard
  settings.
- **Hyprland** registers FastMind's shortcuts but leaves the keys to you. Settings shows the lines
  to add to `~/.config/hypr/hyprland.conf`, for the default keys:

  ```
  bind = CTRL ALT, SPACE, global, dev.coccor.fastmind:capture
  bind = CTRL ALT, R, global, dev.coccor.fastmind:remember
  ```

- **Anywhere else**, bind a key in your desktop to run `fastmind-desktop --capture` (or
  `--remember`): it opens the box in the FastMind that's already running.

To be known to the portal, FastMind writes a hidden desktop entry,
`~/.local/share/applications/dev.coccor.fastmind.desktop`, pointing at its binary. On X11 the
hotkeys work as on Windows and macOS.

## Choosing the AI

FastMind runs on an AI you choose, and won't start without one:

| | |
|---|---|
| **Claude Code** | Your Claude plan, no API key. `fastmind model claude-code [MODEL]` |
| **Claude API** | Needs `ANTHROPIC_API_KEY`. `fastmind model claude [MODEL]` |
| **A model on your computer** | Ollama, LM Studio, llama.cpp: nothing leaves the machine. `fastmind model local MODEL --url URL` |

The first run, and each `fastmind model …`, checks the AI answers before anything else.

## The command line

`fastmind` does everything the app does:

```
fastmind setup ~/Notes                    # choose the notebook
fastmind capture "lag spikes in the ledger batch" --tag perf --attach ./ledger.log
fastmind write "Fresh memories/Lag.md"    # write a captured note (again) with the model
fastmind search "#perf ledger"
fastmind ask "why did the ledger batch lag?"
fastmind inbox | status
fastmind dream | dream show | dream apply all | dream reject p3 | dream undo p3
fastmind history | undo <entry> | archive | restore <id>
fastmind skills | skills show|edit|reset NAME
fastmind model claude-code|claude|local [MODEL] [--url URL]
fastmind demo [DIR]                       # a messy demo notebook to try Dream on
fastmind mcp                              # the notebook for coding agents (MCP over stdio)
fastmind agent install | uninstall | status
fastmind import-session FILE              # write up one Claude Code or Codex transcript now
fastmind serve [--port N]                 # the app's UI in a browser, on localhost only
fastmind settings
```

Options: `--notebook DIR` uses another notebook for one command, `--json` prints raw JSON.
Exit codes: 0 done, 1 failed, 2 bad arguments. `fastmind --help` lists everything.

## Settings

Settings live in `settings.json` in your config folder:

| | |
|---|---|
| Windows | `%AppData%\coccor\FastMind\config` |
| macOS | `~/Library/Application Support/dev.coccor.FastMind` |
| Linux | `~/.config/fastmind` |

`FASTMIND_CONFIG` points elsewhere, for example a scratch file while you try the demo.
