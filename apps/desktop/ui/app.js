// FastMind's UI. Runs inside the desktop app (Tauri) or in a browser against `fastmind serve`:
// both answer the same commands, so this file doesn't care which.
"use strict";

const tauri = window.__TAURI__;

async function call(name, args = {}) {
  if (tauri) return tauri.core.invoke("call", { name, args });
  const response = await fetch("/api/" + name, { method: "POST", body: JSON.stringify(args) });
  const body = await response.json();
  if (body.error) throw new Error(body.error);
  return body.ok;
}

async function openInEditor(path) {
  try {
    const { abs_path } = await call("open", { path });
    if (tauri) await tauri.core.invoke("open_path", { path: abs_path });
    else await call("os_open", { path: abs_path });
  } catch (error) {
    toast(String(error.message || error), true);
  }
}

// ---------- helpers ----------
const $ = (selector) => document.querySelector(selector);
const esc = (text) =>
  String(text ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

function bytes(n) {
  n = Math.abs(n || 0);
  if (n >= 1048576) return (n / 1048576).toFixed(1) + " MB";
  if (n >= 1024) return (n / 1024).toFixed(1) + " KB";
  return n + " B";
}

function ago(iso) {
  if (!iso) return "";
  const seconds = (Date.now() - new Date(iso).getTime()) / 1000;
  if (seconds < 60) return "just now";
  if (seconds < 3600) return Math.round(seconds / 60) + " min ago";
  if (seconds < 86400) return Math.round(seconds / 3600) + " h ago";
  const days = Math.round(seconds / 86400);
  return days === 1 ? "yesterday" : days + " days ago";
}

let toastTimer;
function toast(message, error = false) {
  const el = $("#toast");
  el.textContent = message;
  el.className = "toast" + (error ? " error" : "");
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), error ? 6000 : 3000);
}

function highlight(text, query) {
  const words = (query || "")
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter((w) => w.length > 2);
  let html = esc(text);
  for (const word of words) {
    const safe = word.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    html = html.replace(new RegExp("(" + safe + "\\p{L}*)", "giu"), "<mark>$1</mark>");
  }
  return html;
}

// A small Markdown renderer: enough to read a note, never to run anything in it.
function renderMarkdown(text) {
  let front = "";
  let body = text;
  const fm = /^---\r?\n([\s\S]*?)\r?\n(---|\.\.\.)\r?\n?/.exec(text);
  if (fm) {
    front = fm[1];
    body = text.slice(fm[0].length);
  }
  const inline = (line) =>
    esc(line)
      .replace(/`([^`]+)`/g, "<code>$1</code>")
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>")
      .replace(/\[\[([^\]|#]+)(#[^\]|]*)?(\|([^\]]+))?\]\]/g, (_, target, _h, _p, shown) =>
        `<a href="#" class="wikilink" data-target="${esc(target)}">${esc(shown || target)}</a>`)
      .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>')
      .replace(/&lt;(https?:[^&\s]+)&gt;/g, '<a href="$1" target="_blank" rel="noreferrer">$1</a>');
  const out = [];
  let list = null;
  let code = null;
  let para = [];
  const flush = () => {
    if (para.length) out.push("<p>" + para.map(inline).join("<br>") + "</p>");
    para = [];
    if (list) out.push(`</${list}>`);
    list = null;
  };
  for (const line of body.split(/\r?\n/)) {
    if (code !== null) {
      if (/^\s*```/.test(line)) {
        out.push("<pre><code>" + esc(code) + "</code></pre>");
        code = null;
      } else code += line + "\n";
      continue;
    }
    if (/^\s*```/.test(line)) { flush(); code = ""; continue; }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) { flush(); const n = Math.min(heading[1].length, 3); out.push(`<h${n}>${inline(heading[2])}</h${n}>`); continue; }
    const item = /^\s*(?:[-*+]|(\d+)\.)\s+(.*)$/.exec(line);
    if (item) {
      if (para.length) { out.push("<p>" + para.map(inline).join("<br>") + "</p>"); para = []; }
      const kind = item[1] ? "ol" : "ul";
      if (list !== kind) { if (list) out.push(`</${list}>`); out.push(`<${kind}>`); list = kind; }
      out.push("<li>" + inline(item[2].replace(/^\[( |x)\]\s*/i, (m, x) => (x.trim() ? "☑ " : "☐ "))) + "</li>");
      continue;
    }
    const quote = /^>\s?(.*)$/.exec(line);
    if (quote) { flush(); out.push("<blockquote>" + inline(quote[1]) + "</blockquote>"); continue; }
    if (!line.trim()) { flush(); continue; }
    if (list) { out.push(`</${list}>`); list = null; }
    para.push(line);
  }
  if (code !== null) out.push("<pre><code>" + esc(code) + "</code></pre>");
  flush();
  return (front ? `<div class="front">${esc(front)}</div>` : "") + out.join("\n");
}

// ---------- navigation ----------
const views = ["remember", "inbox", "dream", "archive", "history", "settings"];
let current = "remember";

function show(view) {
  current = view;
  for (const name of views) $("#view-" + name).hidden = name !== view;
  document.querySelectorAll(".nav").forEach((b) => b.classList.toggle("active", b.dataset.view === view));
  ({ remember: loadRemember, inbox: loadInbox, dream: loadDream, archive: loadArchive, history: loadHistory, settings: loadSettings })[view]();
  try { localStorage.setItem("fastmind.view", view); } catch {}
}

async function refreshStatus() {
  try {
    const status = await call("status");
    $("#badge-inbox").textContent = status.inbox || "";
    $("#badge-dream").textContent = status.dream && status.dream.pending ? status.dream.pending : "";
    $("#notebook-line").textContent = `${status.notes} notes · ${status.notebook}`;
  } catch (error) {
    $("#notebook-line").textContent = String(error.message || error);
  }
}

// ---------- Remember ----------
let searchTimer;
let lastQuery = null;
let selected = null;

function loadRemember() {
  runSearch($("#query").value, true);
  setTimeout(() => $("#query").focus(), 0);
}

async function runSearch(query, force = false) {
  if (!force && query === lastQuery) return;
  lastQuery = query;
  try {
    const hits = await call("search", { query, limit: 30 });
    if (query !== lastQuery) return;
    const list = $("#results");
    if (!hits.length) {
      list.innerHTML = `<li class="empty">${query.trim() ? "No notes match. Try fewer words, or ask it as a question." : "The notebook is empty. Press Capture to save your first note."}</li>`;
      return;
    }
    list.innerHTML = hits
      .map(
        (hit, i) => `<li class="result" data-i="${i}" data-path="${esc(hit.path)}" data-archived="${hit.archived ? esc(hit.archived) : ""}">
          <div class="title">${highlight(hit.title, query)}${hit.kind === "ai-session" || hit.kind === "ai-thread" ? '<span class="pill ai">AI session</span>' : ""}${hit.kind === "topic" ? '<span class="pill topic">topic</span>' : ""}${hit.archived ? '<span class="pill">archived</span>' : ""}</div>
          <div class="path">${esc(hit.path)}${(hit.tags || []).map((t) => ` <span class="tag">#${esc(t)}</span>`).join("")}</div>
          <div class="snippet">${highlight(hit.snippet, query)}</div>
        </li>`)
      .join("");
  } catch (error) {
    toast(String(error.message || error), true);
  }
}

async function ask() {
  const question = $("#query").value.trim();
  if (!question) return;
  const box = $("#answer");
  box.hidden = false;
  $("#reader").hidden = true;
  box.innerHTML = `<div class="muted">Thinking it over…</div>`;
  try {
    const answer = await call("ask", { question });
    const text = esc(answer.text).replace(/\[(\d+)\]/g, '<button class="link cite" data-n="$1">[$1]</button>');
    box.innerHTML = `<div class="text">${text}</div>
      <div class="sources">${answer.sources
        .map((s) => `<button data-path="${esc(s.path)}" data-archived="${s.archived ? esc(s.archived) : ""}" title="${esc(s.path)}">[${s.n}] ${esc(s.title)}${s.archived ? " (archived)" : ""}</button>`)
        .join("")}</div>
      <div class="muted small" style="margin-top:8px">${answer.by_model ? "Written by the model from these notes." : "Quoted from your notes (no model)."}${answer.note ? " " + esc(answer.note) : ""}</div>`;
    box.querySelectorAll(".sources button").forEach((b) => b.addEventListener("click", () => readNote(b.dataset.path, b.dataset.archived)));
    box.querySelectorAll(".cite").forEach((b) => b.addEventListener("click", () => {
      const source = answer.sources.find((s) => String(s.n) === b.dataset.n);
      if (source) readNote(source.path, source.archived);
    }));
  } catch (error) {
    box.innerHTML = `<div class="muted">${esc(error.message || error)}</div>`;
  }
}

async function readNote(path, archivedId) {
  const reader = $("#reader");
  selected = path;
  document.querySelectorAll(".result").forEach((r) => r.classList.toggle("selected", r.dataset.path === path));
  if (archivedId) {
    reader.hidden = false;
    reader.innerHTML = `<div class="reader-head"><h2>${esc(path)}</h2><button class="primary" id="restore-from-reader">Restore</button></div>
      <p class="muted">This note is in the archive. Restore it to read and edit it in the notebook again.</p>`;
    $("#restore-from-reader").addEventListener("click", () => restore(archivedId));
    return;
  }
  try {
    const note = await call("read", { path });
    reader.hidden = false;
    reader.innerHTML = `<div class="reader-head"><h2>${esc(note.title)}</h2>
        <span><button id="open-editor">Open in editor</button> <button id="close-reader" title="Close">✕</button></span></div>
      <div class="muted small">${esc(note.path)}</div>
      <div class="md">${renderMarkdown(note.text)}</div>`;
    $("#open-editor").addEventListener("click", () => openInEditor(path));
    $("#close-reader").addEventListener("click", () => { reader.hidden = true; selected = null; });
    reader.querySelectorAll(".wikilink").forEach((a) =>
      a.addEventListener("click", (event) => {
        event.preventDefault();
        $("#query").value = a.dataset.target;
        runSearch(a.dataset.target, true).then(() => {
          const first = document.querySelector(".result");
          if (first) readNote(first.dataset.path, first.dataset.archived);
        });
      }));
  } catch (error) {
    toast(String(error.message || error), true);
  }
}

// ---------- Inbox ----------
async function loadInbox() {
  const list = $("#inbox-list");
  try {
    const notes = await call("inbox");
    if (!notes.length) {
      list.innerHTML = `<li class="empty">Inbox zero. Captures land here until you keep or file them.</li>`;
      return;
    }
    list.innerHTML = notes
      .map((n) => `<li class="card" data-path="${esc(n.path)}">
        <div class="card-head"><div class="grow">
          <div class="title">${esc(n.title)}</div>
          <div class="reason">${esc(n.snippet)}</div>
          <div class="meta"><span>${esc(ago(n.captured))}</span><span>${esc(n.path)}</span>${(n.tags || []).map((t) => `<span class="tag">#${esc(t)}</span>`).join("")}</div>
        </div><div class="actions">
          <button data-act="read">Read</button><button data-act="keep">Keep</button><button data-act="archive">Archive</button>
        </div></div></li>`)
      .join("");
  } catch (error) {
    list.innerHTML = `<li class="empty">${esc(error.message || error)}</li>`;
  }
}

$("#inbox-list").addEventListener("click", async (event) => {
  const button = event.target.closest("button");
  const card = event.target.closest(".card");
  if (!button || !card) return;
  const path = card.dataset.path;
  try {
    if (button.dataset.act === "read") { show("remember"); readNote(path); return; }
    if (button.dataset.act === "keep") { await call("inbox_keep", { path }); toast("Kept. It leaves the inbox; History can undo it."); }
    if (button.dataset.act === "archive") { await call("archive_note", { path }); toast("Archived. Restore it from the Archive any time."); }
    loadInbox();
    refreshStatus();
  } catch (error) {
    toast(String(error.message || error), true);
  }
});

// ---------- Dream ----------
const KIND_ICON = { merge: "⧉", topic: "❖", archive: "▤", fix_link: "⛓", pack: "⇲" };
const KIND_NAME = { merge: "Merge copies", topic: "Gather a topic", archive: "Archive", fix_link: "Fix a link", pack: "Pack the archive" };

async function loadDream() {
  try {
    renderDream(await call("dream_latest"));
  } catch (error) {
    toast(String(error.message || error), true);
  }
}

function renderDream(report) {
  const summary = $("#dream-summary");
  const list = $("#proposals");
  if (!report) {
    summary.innerHTML = `<strong>Dream hasn't run on this notebook yet.</strong><p class="muted">Press Dream now, or leave FastMind running and it dreams tonight.</p>`;
    list.innerHTML = "";
    $("#dream-actions").hidden = true;
    return;
  }
  const proposals = report.proposals || [];
  const pending = proposals.filter((p) => p.status === "pending");
  const applied = proposals.filter((p) => p.status === "applied");
  const removedAll = proposals.filter((p) => p.status !== "rejected").reduce((n, p) => n + p.notes_removed, 0);
  const freedAll = proposals.filter((p) => p.status !== "rejected").reduce((n, p) => n + p.bytes_freed + (p.disk_freed || 0), 0);
  const before = report.before;
  summary.innerHTML = `<div class="muted small">Dreamed ${esc(ago(report.ran_at))} · took ${report.took_ms} ms</div>
    <div style="margin-top:4px">${esc(report.summary)}</div>
    <div class="stats">
      <div class="stat"><div class="num">${before.notes}<span class="arrow">→</span>${before.notes - removedAll}</div><div class="label">notes in the notebook</div></div>
      <div class="stat"><div class="num">${bytes(freedAll)}</div><div class="label">lighter</div></div>
      <div class="stat"><div class="num">${applied.length}/${proposals.length}</div><div class="label">proposals applied</div></div>
    </div>
    ${(report.model_notes || []).map((n) => `<div class="muted small" style="margin-top:8px">Model: ${esc(n)}</div>`).join("")}`;
  $("#dream-actions").hidden = pending.length === 0;
  $("#dream-pending-line").textContent = pending.length ? `${pending.length} waiting for you` : "";
  if (!proposals.length) {
    list.innerHTML = `<li class="empty">Nothing to tidy. The notebook is in good shape.</li>`;
    return;
  }
  list.innerHTML = proposals
    .map((p) => {
      const freed = p.bytes_freed + (p.disk_freed || 0);
      const actions =
        p.status === "pending"
          ? `${p.preview ? '<button data-act="preview">Preview</button>' : ""}${p.kind === "archive" && p.notes[0].endsWith(".md") ? '<button data-act="pin" title="Pin it: Dream never touches it again">Keep forever</button>' : '<button data-act="reject" title="Dream won't propose this again">Skip</button>'}<button data-act="apply" class="primary">Accept</button>`
          : p.status === "applied"
            ? `<span class="status applied">✓ Applied</span><button data-act="undo">Undo</button>`
            : `<span class="status ${p.status}">${p.status === "rejected" ? "Skipped" : "Out of date"}</span>`;
      return `<li class="card ${p.status}" data-id="${esc(p.id)}">
        <div class="card-head">
          <div class="kind ${p.kind}" title="${esc(KIND_NAME[p.kind])}">${KIND_ICON[p.kind] || "•"}</div>
          <div class="grow">
            <div class="title">${esc(p.title)}</div>
            <div class="reason">${esc(p.reason)}</div>
            <div class="meta">
              <span>${esc(KIND_NAME[p.kind])}</span>
              ${p.notes_removed ? `<span>−${p.notes_removed} note${p.notes_removed > 1 ? "s" : ""}</span>` : ""}
              ${freed > 0 ? `<span>−${bytes(freed)}</span>` : ""}
              ${p.error ? `<span class="status outdated">${esc(p.error)}</span>` : ""}
            </div>
          </div>
          <div class="actions">${actions}</div>
        </div>
        <div class="preview md" hidden></div>
      </li>`;
    })
    .join("");
  list._report = report;
}

$("#proposals").addEventListener("click", async (event) => {
  const button = event.target.closest("button");
  const card = event.target.closest(".card");
  if (!button || !card) return;
  const id = card.dataset.id;
  const act = button.dataset.act;
  if (act === "preview") {
    const pane = card.querySelector(".preview");
    const proposal = ($("#proposals")._report.proposals || []).find((p) => p.id === id);
    pane.innerHTML = renderMarkdown(proposal.preview || "");
    pane.hidden = !pane.hidden;
    button.textContent = pane.hidden ? "Preview" : "Hide preview";
    return;
  }
  button.disabled = true;
  try {
    if (act === "apply") {
      const p = await call("dream_apply", { id });
      toast(p.status === "applied" ? "Done. Undo is one click away." : `Not applied: ${p.error || p.status}`, p.status !== "applied");
    }
    if (act === "reject") await call("dream_reject", { id });
    if (act === "pin") {
      const proposal = ($("#proposals")._report.proposals || []).find((p) => p.id === id);
      await call("pin_note", { path: proposal.notes[0] });
      toast("Pinned. Dream will leave it alone.");
    }
    if (act === "undo") { await call("dream_undo", { id }); toast("Undone. The notes are back as they were."); }
  } catch (error) {
    toast(String(error.message || error), true);
  }
  await loadDream();
  refreshStatus();
});

$("#dream-now").addEventListener("click", async () => {
  const button = $("#dream-now");
  button.disabled = true;
  button.textContent = "Dreaming…";
  try {
    const result = await call("dream_run");
    renderDream(result.report);
    if (result.auto_applied) toast(`${result.auto_applied} applied on their own (see Settings › Dream).`);
  } catch (error) {
    toast(String(error.message || error), true);
  }
  button.disabled = false;
  button.textContent = "Dream now";
  refreshStatus();
});

$("#dream-accept-all").addEventListener("click", async () => {
  const button = $("#dream-accept-all");
  button.disabled = true;
  try {
    const done = await call("dream_apply_all", { kinds: [] });
    const ok = done.filter((p) => p.status === "applied").length;
    toast(`${ok} applied${done.length > ok ? `, ${done.length - ok} out of date` : ""}. Each can be undone.`);
  } catch (error) {
    toast(String(error.message || error), true);
  }
  button.disabled = false;
  await loadDream();
  refreshStatus();
});

// ---------- Archive ----------
async function loadArchive() {
  try {
    const { items, disk_bytes } = await call("archive_list");
    $("#archive-line").textContent = items.length ? `${items.length} archived · ${bytes(disk_bytes)} on disk` : "";
    $("#archive-list").innerHTML = items.length
      ? items
          .map((item) => `<li class="card" data-id="${esc(item.id)}"><div class="card-head"><div class="kind ${item.pack ? "pack" : "archive"}">${item.pack ? "⇲" : "▤"}</div>
            <div class="grow"><div class="title">${esc(item.title)}${item.pack ? '<span class="pill">packed</span>' : ""}</div>
            <div class="reason">${esc(item.reason)}</div>
            <div class="meta"><span>was ${esc(item.original)}</span><span>archived ${esc(ago(item.archived_at))}</span><span>${bytes(item.size)}</span></div></div>
            <div class="actions"><button data-act="restore">Restore</button></div></div></li>`)
          .join("")
      : `<li class="empty">The archive is empty.</li>`;
  } catch (error) {
    toast(String(error.message || error), true);
  }
}

async function restore(id) {
  try {
    const { path } = await call("archive_restore", { id });
    toast(`Restored to ${path}`);
    if (current === "archive") loadArchive();
    if (current === "remember") { runSearch($("#query").value, true); readNote(path); }
    refreshStatus();
  } catch (error) {
    toast(String(error.message || error), true);
  }
}

$("#archive-list").addEventListener("click", (event) => {
  const card = event.target.closest(".card");
  if (event.target.closest("button") && card) restore(card.dataset.id);
});

// ---------- History ----------
async function loadHistory() {
  try {
    const entries = await call("history");
    $("#history-list").innerHTML = entries.length
      ? entries
          .map((e) => `<li class="card" data-id="${esc(e.id)}"><div class="card-head"><div class="grow">
            <div class="title">${esc(e.label)}</div>
            <div class="meta"><span>${esc(ago(e.at))}</span><span>${esc(e.source)}</span><span>${e.files.length} file${e.files.length === 1 ? "" : "s"}</span></div></div>
            <div class="actions">${e.undone ? '<span class="muted small">Undone</span>' : e.can_undo ? '<button data-act="undo">Undo</button>' : '<span class="muted small" title="A file was edited since, so undoing would lose that edit">Edited since</span>'}</div>
            </div></li>`)
          .join("")
      : `<li class="empty">No changes yet.</li>`;
  } catch (error) {
    toast(String(error.message || error), true);
  }
}

$("#history-list").addEventListener("click", async (event) => {
  const card = event.target.closest(".card");
  if (!event.target.closest("button") || !card) return;
  try {
    const { undone } = await call("undo", { id: card.dataset.id });
    toast(`Undone: ${undone}`);
  } catch (error) {
    toast(String(error.message || error), true);
  }
  loadHistory();
  refreshStatus();
});

// ---------- Settings ----------
async function loadSettings() {
  try {
    const { settings, file } = await call("settings_get");
    $("#settings-file").textContent = "Saved in " + file;
    const form = $("#settings-form");
    form.notebook.value = settings.notebook || "";
    form.inbox.value = settings.inbox;
    form.hotkey_capture.value = settings.hotkeys.capture;
    form.hotkey_remember.value = settings.hotkeys.remember;
    form.autostart.checked = !!settings.autostart;
    const d = settings.dream;
    form.nightly.checked = d.nightly;
    form.window_start.value = d.window_start;
    form.window_end.value = d.window_end;
    form.stale_days.value = d.stale_days;
    form.done_days.value = d.done_days;
    form.pack_after_days.value = d.pack_after_days;
    form.undo_days.value = d.undo_days;
    form.querySelectorAll('input[name="auto"]').forEach((box) => (box.checked = d.auto_apply.includes(box.value)));
    form.ai_enabled.checked = settings.ai.enabled;
    form.ai_folder.value = settings.ai.folder;
    form.ai_automated.checked = settings.ai.include_automated;
    call("ai_status").then((ai) => {
      const total = ai.claude_code + ai.codex;
      $("#ai-status").textContent = total
        ? `Found ${ai.claude_code} Claude Code and ${ai.codex} Codex sessions on this computer.`
        : "No Claude Code or Codex sessions found on this computer yet.";
    }).catch(() => {});
    form.provider.value = settings.model.provider || "none";
    form.model.value = settings.model.model || "";
    form.base_url.value = settings.model.base_url || "";
    form._settings = settings;
  } catch (error) {
    toast(String(error.message || error), true);
  }
}

$("#settings-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = event.target;
  const s = structuredClone(form._settings);
  s.notebook = form.notebook.value.trim() || null;
  s.inbox = form.inbox.value.trim() || "inbox";
  s.hotkeys = { capture: form.hotkey_capture.value.trim(), remember: form.hotkey_remember.value.trim() };
  s.autostart = form.autostart.checked;
  Object.assign(s.dream, {
    nightly: form.nightly.checked,
    window_start: form.window_start.value.trim(),
    window_end: form.window_end.value.trim(),
    stale_days: Number(form.stale_days.value),
    done_days: Number(form.done_days.value),
    pack_after_days: Number(form.pack_after_days.value),
    undo_days: Number(form.undo_days.value),
    auto_apply: [...form.querySelectorAll('input[name="auto"]:checked')].map((b) => b.value),
  });
  Object.assign(s.ai, {
    enabled: form.ai_enabled.checked,
    folder: form.ai_folder.value.trim() || "AI sessions",
    include_automated: form.ai_automated.checked,
  });
  const provider = form.provider.value;
  s.model =
    provider === "none" ? { provider: "none" }
    : provider === "local" ? { provider: "local", model: form.model.value.trim() || "llama3.2", base_url: form.base_url.value.trim() || "http://localhost:11434/v1" }
    : { provider: "claude", model: form.model.value.trim() || "claude-opus-5-5" };
  try {
    await call("settings_set", { settings: s });
    $("#settings-saved").textContent = "Saved.";
    setTimeout(() => ($("#settings-saved").textContent = ""), 2500);
    refreshStatus();
  } catch (error) {
    toast(String(error.message || error), true);
  }
});

$("#ai-import").addEventListener("click", async () => {
  const button = $("#ai-import");
  button.disabled = true;
  button.textContent = "Importing…";
  try {
    const form = $("#settings-form");
    if (!form._settings.ai.enabled) {
      form.ai_enabled.checked = true;
      form.requestSubmit();
      await new Promise((r) => setTimeout(r, 300));
    }
    const report = await call("ai_import");
    const parts = [`${report.added.length} new`];
    if (report.updated.length) parts.push(`${report.updated.length} updated`);
    toast(`AI sessions: ${parts.join(", ")}. Find them in Remember; Dream gathers them tonight.`);
    refreshStatus();
  } catch (error) {
    toast(String(error.message || error), true);
  }
  button.disabled = false;
  button.textContent = "Import now";
});

$("#make-demo").addEventListener("click", async () => {
  try {
    const { path, notes } = await call("demo_create", {});
    toast(`Demo notebook with ${notes} notes at ${path}. Open Dream and press Dream now.`);
    loadSettings();
    refreshStatus();
  } catch (error) {
    toast(String(error.message || error), true);
  }
});

// ---------- Capture ----------
async function saveCapture(text) {
  if (!text.trim()) return false;
  try {
    const saved = await call("capture", { text });
    toast(`Saved "${saved.title}"`);
    refreshStatus();
    if (current === "inbox") loadInbox();
    if (current === "remember") runSearch($("#query").value, true);
    return true;
  } catch (error) {
    toast(String(error.message || error), true);
    return false;
  }
}

function openCapture() {
  $("#capture-dialog").showModal();
  $("#capture-text").focus();
}

$("#capture-open").addEventListener("click", openCapture);
$("#capture-cancel").addEventListener("click", () => $("#capture-dialog").close());
$("#capture-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (await saveCapture($("#capture-text").value)) {
    $("#capture-text").value = "";
    $("#capture-dialog").close();
  }
});
$("#capture-text").addEventListener("keydown", (event) => {
  if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
    event.preventDefault();
    $("#capture-form").requestSubmit();
  }
});

// ---------- wiring ----------
document.querySelectorAll(".nav").forEach((button) => button.addEventListener("click", () => show(button.dataset.view)));
$("#query").addEventListener("input", (event) => {
  clearTimeout(searchTimer);
  const value = event.target.value;
  searchTimer = setTimeout(() => runSearch(value), 120);
  if (!value.trim()) $("#answer").hidden = true;
});
$("#query").addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    const value = event.target.value.trim();
    if (event.ctrlKey || event.metaKey || value.endsWith("?")) { event.preventDefault(); ask(); return; }
    const first = document.querySelector(".result");
    if (first) readNote(first.dataset.path, first.dataset.archived);
  }
  if (event.key === "Escape") { event.target.value = ""; runSearch("", true); $("#answer").hidden = true; }
});
$("#ask-button").addEventListener("click", ask);
$("#results").addEventListener("click", (event) => {
  const item = event.target.closest(".result");
  if (item) readNote(item.dataset.path, item.dataset.archived);
});
document.addEventListener("keydown", (event) => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") { event.preventDefault(); show("remember"); }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "n") { event.preventDefault(); openCapture(); }
});

function startMain() {
  $("#app").hidden = false;
  let first = "remember";
  try { first = localStorage.getItem("fastmind.view") || first; } catch {}
  show(views.includes(first) ? first : "remember");
  refreshStatus();
  if (tauri) {
    tauri.event.listen("focus-remember", () => { show("remember"); $("#query").select(); });
    tauri.event.listen("show-view", (event) => show(event.payload));
    tauri.event.listen("changed", () => { refreshStatus(); show(current); });
  }
}

function startCaptureOnly() {
  $("#capture-only").hidden = false;
  const area = $("#capture-text-mini");
  const close = () => (tauri ? tauri.core.invoke("hide_capture") : window.close());
  area.focus();
  $("#capture-form-mini").addEventListener("submit", async (event) => {
    event.preventDefault();
    if (await saveCapture(area.value)) {
      area.value = "";
      close();
    }
  });
  area.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); $("#capture-form-mini").requestSubmit(); }
    if (event.key === "Escape") { event.preventDefault(); close(); }
  });
  if (tauri) tauri.event.listen("capture-shown", () => setTimeout(() => area.focus(), 30));
}

if (new URLSearchParams(location.search).get("mode") === "capture") startCaptureOnly();
else startMain();
