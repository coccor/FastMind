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
fillIcons();
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

/// Wikilinks read as their text in snippets: [[Note|shown]] → shown.
const unlink = (text) => String(text ?? "").replace(/\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|([^\]]+))?\]\]/g, (_, target, shown) => shown || target);

function highlight(text, query) {
  text = unlink(text);
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
      .replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (_, alt, src) =>
        /^https?:/.test(src) ? `<span class="muted">[image: ${alt}]</span>` : `<img class="note-image" alt="${alt}" data-src="${src}">`)
      .replace(/!\[\[([^\]|]+\.(?:png|jpe?g|gif|webp|bmp|svg))(?:\|[^\]]*)?\]\]/gi, (_, src) => `<img class="note-image" alt="${src}" data-src="${src}" data-wiki="1">`)
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
      const task = /^\[( |x)\]\s*/i.exec(item[2]);
      const box = task ? icon(task[1].trim() ? "square-check" : "square") + " " : "";
      out.push("<li>" + box + inline(task ? item[2].slice(task[0].length) : item[2]) + "</li>");
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
  // Properties stay out of the way: folded, opened on demand.
  return (front ? `<details class="front"><summary>Properties</summary><div class="front-text">${esc(front)}</div></details>` : "") + out.join("\n");
}

// ---------- navigation ----------
const views = ["home", "remember", "inbox", "dream", "archive", "history", "settings"];
let current = "home";

function show(view) {
  current = view;
  for (const name of views) $("#view-" + name).hidden = name !== view;
  document.querySelectorAll(".nav").forEach((b) => b.classList.toggle("active", b.dataset.view === view));
  ({ home: loadHome, remember: loadRemember, inbox: loadInbox, dream: loadDream, archive: loadArchive, history: loadHistory, settings: loadSettings })[view]();
  try { localStorage.setItem("fastmind.view", view); } catch {}
}

async function refreshStatus() {
  try {
    const status = await call("status");
    $("#badge-inbox").textContent = "";
    $("#badge-dream").textContent = status.dream && status.dream.pending ? status.dream.pending : "";
    const name = String(status.notebook).split(/[\\/]/).filter(Boolean).pop() || String(status.notebook);
    $("#nb-name").textContent = name;
    $("#nb-count").textContent = `${status.notes} notes`;
    $("#notebook-switch").title = `${status.notebook}\nSwitch notebook`;
  } catch (error) {
    $("#nb-name").textContent = "No notebook";
    $("#nb-count").textContent = String(error.message || error);
  }
}

// ---------- Switching notebooks ----------
const notebookMenu = $("#notebook-menu");

function closeNotebookMenu() {
  notebookMenu.hidden = true;
  $("#notebook-switch").setAttribute("aria-expanded", "false");
}

async function openNotebookMenu() {
  let list;
  try { list = await call("notebooks"); } catch (error) { toast(String(error.message || error), true); return; }
  const item = (n, extra = "") => `<button type="button" role="menuitem" class="nb-item" data-path="${esc(n.path)}"${n.exists === false ? " disabled" : ""}>
      <span class="nb-item-name">${esc(n.name)}${extra}</span><span class="nb-item-path">${esc(n.path)}</span></button>`;
  const kind = { obsidian: "Obsidian vault", fastcapture: "FastCapture notebook", fastmind: "FastMind notebook" };
  notebookMenu.innerHTML = `
    ${list.current ? `<div class="nb-section">Using now</div><div class="nb-item current"><span class="nb-item-name">${icon("check")} ${esc(list.current.name)}</span><span class="nb-item-path">${esc(list.current.path)}</span></div>` : ""}
    ${list.recent.length ? `<div class="nb-section">Recent</div>${list.recent.map((n) => `<div class="nb-row">${item(n, n.exists ? "" : " (missing)")}<button type="button" class="nb-forget" data-forget="${esc(n.path)}" title="Remove from this list" aria-label="Remove ${esc(n.name)} from this list">×</button></div>`).join("")}` : ""}
    ${list.found.length ? `<div class="nb-section">Found on this computer</div>${list.found.map((n) => item(n, ` <span class="muted small">· ${esc(kind[n.kind] || "notes")}, ${n.notes} notes</span>`)).join("")}` : ""}
    <div class="nb-section">Another folder</div>
    ${tauri ? `<div class="nb-actions"><button type="button" data-nb="open">Open a folder…</button><button type="button" data-nb="new">New notebook…</button></div>` : ""}
    <form class="nb-path"><input placeholder="Folder path, e.g. C:\\Notes\\Work" aria-label="Folder path"><button>Open</button></form>`;
  notebookMenu.hidden = false;
  $("#notebook-switch").setAttribute("aria-expanded", "true");
  notebookMenu.querySelector(".nb-item:not(.current), input")?.focus();
}

async function switchNotebook(path, create = false) {
  try {
    await call("notebook_switch", { path, create });
    closeNotebookMenu();
    $("#reader").hidden = true;
    selected = null;
    await refreshStatus();
    show(current);
    toast(`Now using ${path.split(/[\\/]/).filter(Boolean).pop()}`);
  } catch (error) {
    const message = String(error.message || error);
    if (!create && message.endsWith("doesn't exist")) {
      // Typed a folder that isn't there: offer to make it, in the menu itself.
      const form = notebookMenu.querySelector(".nb-path");
      form.querySelector("input").value = path;
      form.insertAdjacentHTML("afterend", `<div class="nb-create small">${esc(message)}. <button type="button" data-create="${esc(path)}">Create a new notebook there</button></div>`);
      return;
    }
    toast(message, true);
  }
}

$("#notebook-switch").addEventListener("click", (event) => {
  event.stopPropagation();
  if (notebookMenu.hidden) openNotebookMenu(); else closeNotebookMenu();
});
notebookMenu.addEventListener("click", async (event) => {
  event.stopPropagation();
  const forget = event.target.closest("[data-forget]");
  if (forget) { await call("notebook_forget", { path: forget.dataset.forget }); openNotebookMenu(); return; }
  const create = event.target.closest("[data-create]");
  if (create) { switchNotebook(create.dataset.create, true); return; }
  const action = event.target.closest("[data-nb]");
  if (action) {
    const folder = await tauri.core.invoke("pick_folder").catch(() => null);
    if (folder) switchNotebook(folder, action.dataset.nb === "new");
    return;
  }
  const item = event.target.closest(".nb-item[data-path]");
  if (item) switchNotebook(item.dataset.path);
});
notebookMenu.addEventListener("submit", (event) => {
  event.preventDefault();
  const path = event.target.querySelector("input").value.trim();
  if (path) switchNotebook(path);
});
document.addEventListener("click", () => { if (!notebookMenu.hidden) closeNotebookMenu(); });
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !notebookMenu.hidden) { closeNotebookMenu(); $("#notebook-switch").focus(); }
});

// ---------- Home ----------
async function loadHome() {
  const hour = new Date().getHours();
  $("#home-greeting").textContent = hour < 5 ? "Still up?" : hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening";
  let home;
  try {
    home = await call("home");
  } catch (error) {
    $("#home-line").textContent = String(error.message || error);
    return;
  }
  $("#home-line").textContent = `${home.notes} notes in your notebook.` + (home.ai_week ? ` ${home.ai_week} AI sessions captured this week.` : "");

  const dream = $("#home-dream");
  const d = home.dream;
  if (!d) {
    dream.innerHTML = `<div class="home-title">${icon("moon")} Dream</div><p class="muted">Dream hasn't looked at this notebook yet. Leave FastMind running tonight, or start it now.</p>
      <div class="row gap"><button class="primary" data-go="dream-now">Dream now</button></div>`;
  } else {
    const kinds = Object.entries(d.kinds || {}).map(([k, n]) => `${n} ${({ merge: "to merge", topic: "to gather", archive: "to archive", fixlink: n === 1 ? "link to fix" : "links to fix", pack: "to pack" })[k] || k}`).join(" · ");
    dream.innerHTML = `<div class="home-title">${icon("moon")} While you slept <span class="muted small">${esc(ago(d.ran_at))}</span></div>
      <div class="stats compact"><div class="stat"><div class="num">${d.before}<span class="arrow">→</span>${d.after}</div><div class="label">${d.pending ? "notes, if you accept everything" : "notes after Dream"}</div></div></div>
      <p class="muted">${d.pending ? `${d.pending} proposals waiting: ${esc(kinds)}.` : "Nothing waiting. The notebook is tidy."}${d.applied ? ` ${d.applied} already applied.` : ""}</p>
      <div class="row gap">${d.pending ? `<button class="primary" data-go="dream">Review ${d.pending}</button>` : `<button data-go="dream">See what Dream did</button>`}</div>`;
  }

  const inbox = $("#home-inbox");
  inbox.innerHTML = `<div class="home-title">${icon("pencil-line")} Fresh memories <span class="muted small">${home.inbox_count ? home.inbox_count + " settling tonight" : ""}</span></div>`
    + (home.inbox.length
      ? `<ul class="mini">${home.inbox.map((n) => `<li><a href="#" data-read="${esc(n.path)}">${esc(n.title)}</a><span class="muted small"> · ${esc(ago(n.captured))}</span></li>`).join("")}</ul>
         <div class="row gap"><button data-go="inbox">See them</button></div>`
      : `<p class="muted">Nothing new. What you capture lands here, and settles overnight.</p>`);

  const back = $("#home-resurfaced");
  back.innerHTML = `<div class="home-title">${icon("rotate-ccw")} From your notebook</div>`
    + (home.resurfaced.length
      ? home.resurfaced.map((n) => `<div class="resurfaced"><a href="#" data-read="${esc(n.path)}">${esc(n.title)}</a>
          <div class="muted small">${n.age_days} days old · ${esc(n.path)}</div><div class="snippet">${esc(unlink(n.snippet))}</div></div>`).join("")
      : `<p class="muted">Notes come back here once they're a month old, so the good ones don't get lost.</p>`);

  const recent = $("#home-recent");
  recent.innerHTML = `<div class="home-title">${icon("clock")} Recently</div>`
    + (home.recent.length
      ? `<ul class="mini">${home.recent.map((n) => `<li><a href="#" data-read="${esc(n.path)}">${esc(n.title)}</a>${n.kind === "ai-session" ? ' <span class="pill ai">AI</span>' : ""}<span class="muted small"> · ${esc(ago(n.touched))}</span></li>`).join("")}</ul>`
      : `<p class="muted">Nothing yet.</p>`);
  if (!home.ai_enabled) {
    recent.innerHTML += `<p class="muted small tip">Tip: turn on <a href="#" data-go="settings">Learn from AI sessions</a> and your Claude Code and Codex sessions become searchable notes.</p>`;
  }
}

$("#view-home").addEventListener("click", async (event) => {
  const link = event.target.closest("[data-read],[data-go]");
  if (!link) return;
  event.preventDefault();
  if (link.dataset.read) { show("remember"); readNote(link.dataset.read); return; }
  const go = link.dataset.go;
  if (go === "dream-now") { show("dream"); $("#dream-now").click(); return; }
  show(go);
});

const homeFiles = attachable($("#home-capture-text"), $("#home-capture-files"));
const clearHomeDraft = keepDraft($("#home-capture-text"), "fastmind.draft.home");
$("#home-capture").addEventListener("submit", async (event) => {
  event.preventDefault();
  const area = $("#home-capture-text");
  if (await saveCapture(area.value, homeFiles.files)) { area.value = ""; homeFiles.clear(); clearHomeDraft(); loadHome(); }
});
$("#home-capture-text").addEventListener("keydown", (event) => {
  if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); $("#home-capture").requestSubmit(); }
});

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
    reader._note = note;
    reader.innerHTML = `<div class="reader-head"><h2>${esc(note.title)}</h2>
        <span class="reader-actions">${note.capture_id ? `<button id="write-note" title="Write the note again from what you captured; History can undo it">${note.written_by ? "Write again" : "Write"}</button><button id="show-original" title="What you captured, as you gave it">Original</button>` : ""}<button id="edit-note" title="Edit (Ctrl+E)">Edit</button><button id="open-editor" title="Open in your editor">Open</button><button id="close-reader" title="Close" aria-label="Close">${icon("x")}</button></span></div>
      <div class="muted small">${esc(note.path)}</div>
      ${note.writing ? `<div class="writing-line"><span class="spinner"></span> Being written from what you captured…</div>` : ""}
      <div class="original" id="original" hidden></div>
      <div class="md" id="reader-body">${renderMarkdown(note.text)}</div>
      <div class="related" id="related"></div>`;
    $("#open-editor").addEventListener("click", () => openInEditor(path));
    $("#edit-note").addEventListener("click", () => editNote());
    $("#write-note")?.addEventListener("click", async (event) => {
      const done = await writeNote(path, event.currentTarget);
      if (done) readNote(done.path);
    });
    $("#show-original")?.addEventListener("click", () => showOriginal(path));
    // The title is in the header already.
    const heading = reader.querySelector("#reader-body > h1");
    if (heading && heading.textContent.trim() === note.title.trim()) heading.remove();
    $("#close-reader").addEventListener("click", () => { reader.hidden = true; selected = null; });
    wireLinks(reader);
    loadImages(reader, path);
    loadRelated(path);
  } catch (error) {
    toast(String(error.message || error), true);
  }
}

// The second step of a capture: the model writes the note from what was captured.
async function writeNote(path, button) {
  const status = await call("status").catch(() => ({}));
  if (!status.model_on) {
    toast("Choose a model in Settings to write notes (Claude Code uses your Claude plan).", true);
    return null;
  }
  const label = button?.textContent;
  if (button) { button.disabled = true; button.innerHTML = `<span class="spinner"></span> Writing…`; }
  try {
    const done = await call("rewrite_note", { path });
    toast(`Wrote "${done.title}". History can undo it.`);
    refreshStatus();
    return done;
  } catch (error) {
    toast(String(error.message || error), true);
    return null;
  } finally {
    if (button && button.isConnected) { button.disabled = false; button.textContent = label; }
  }
}

async function showOriginal(path) {
  const box = $("#original");
  if (!box.hidden) { box.hidden = true; return; }
  try {
    const raw = await call("capture_original", { path });
    const files = (raw.attachments || []).map((a) => `<li>${esc(a.split("/").pop())}</li>`).join("");
    box.innerHTML = `<div class="muted small">Captured ${esc(raw.captured.replace("T", " "))}${raw.written_by ? ` · written by ${esc(raw.written_by)}` : " · not written by a model"}</div>
      ${raw.text.trim() ? `<pre>${esc(raw.text)}</pre>` : ""}
      ${raw.url ? `<div class="small">Link: ${esc(raw.url)}</div>` : ""}
      ${files ? `<div class="small">Attached:</div><ul class="small">${files}</ul>` : ""}
      ${raw.failed ? `<div class="write-failed small">Last try: ${esc(raw.failed)}</div>` : ""}`;
    box.hidden = false;
  } catch (error) {
    toast(String(error.message || error), true);
  }
}

// Images in a note load through FastMind (the page can't read local files itself).
function loadImages(root, notePath) {
  const folder = notePath.includes("/") ? notePath.slice(0, notePath.lastIndexOf("/") + 1) : "";
  root.querySelectorAll("img.note-image[data-src]").forEach(async (img) => {
    let src = decodeURIComponent(img.dataset.src);
    const parts = (folder + src).split("/");
    const out = [];
    for (const part of parts) { if (part === "..") out.pop(); else if (part && part !== ".") out.push(part); }
    const candidates = [out.join("/"), folder + "attachments/" + src.split("/").pop()];
    for (const path of candidates) {
      try {
        const { data } = await call("image", { path });
        img.src = data;
        return;
      } catch {}
    }
    img.replaceWith(Object.assign(document.createElement("span"), { className: "muted", textContent: `[missing image: ${src}]` }));
  });
}

function wireLinks(root) {
  root.querySelectorAll(".wikilink").forEach((a) =>
    a.addEventListener("click", (event) => {
      event.preventDefault();
      $("#query").value = a.dataset.target;
      runSearch(a.dataset.target, true).then(() => {
        const first = document.querySelector(".result");
        if (first) readNote(first.dataset.path, first.dataset.archived);
      });
    }));
}

async function loadRelated(path) {
  try {
    const { linked_from, similar } = await call("related", { path });
    if (selected !== path) return;
    const item = (n) => `<button class="chip" data-path="${esc(n.path)}" title="${esc(n.path)}">${esc(n.title)}</button>`;
    let html = "";
    if (linked_from.length) html += `<div class="related-row"><span class="muted small">Linked from</span>${linked_from.map(item).join("")}</div>`;
    const linked = new Set(linked_from.map((n) => n.path));
    const more = similar.filter((n) => !linked.has(n.path)).slice(0, 5);
    if (more.length) html += `<div class="related-row"><span class="muted small">Related</span>${more.map(item).join("")}</div>`;
    const box = $("#related");
    box.innerHTML = html;
    box.querySelectorAll("button").forEach((b) => b.addEventListener("click", () => readNote(b.dataset.path)));
  } catch {}
}

function editNote() {
  const reader = $("#reader");
  const note = reader._note;
  if (!note || !$("#reader-body")) return;
  $("#reader-body").outerHTML = `<div id="reader-edit"><textarea id="edit-text" spellcheck="true">${esc(note.text)}</textarea>
    <div class="row gap"><button class="primary" id="save-note">Save</button><button id="cancel-edit">Cancel</button><span class="muted small">Ctrl+S saves · Esc cancels</span></div></div>`;
  $("#edit-note").hidden = true;
  const area = $("#edit-text");
  area.focus();
  const save = async () => {
    try {
      await call("write_note", { path: note.path, text: area.value, base: note.text });
      toast("Saved. History can undo it.");
      readNote(note.path);
      runSearch($("#query").value, true);
    } catch (error) {
      toast(String(error.message || error), true);
    }
  };
  $("#save-note").addEventListener("click", save);
  $("#cancel-edit").addEventListener("click", () => readNote(note.path));
  area.addEventListener("keydown", (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") { event.preventDefault(); save(); }
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); readNote(note.path); }
  });
}

// ---------- Inbox ----------
let folderList = [];

let inboxPoll = null;

async function loadInbox() {
  const list = $("#inbox-list");
  clearTimeout(inboxPoll);
  const focusedPath = list.querySelector(".inbox-card.focused")?.dataset.path;
  try {
    const notes = await call("inbox");
    if (!notes.length) {
      list.innerHTML = `<li class="empty">No fresh memories. What you capture lands here, written up, and settles overnight.</li>`;
      return;
    }
    list.innerHTML = notes
      .map((n, i) => `<li class="card inbox-card${n.writing ? " is-writing" : ""}${(focusedPath ? n.path === focusedPath : i === 0) ? " focused" : ""}" tabindex="0" data-path="${esc(n.path)}">
        <div class="card-head"><div class="grow">
          <div class="title">${esc(n.title)}${n.written_by ? ` <span class="written" title="Written by ${esc(n.written_by)} from what you captured">${icon("sparkles")}</span>` : ""}</div>
          ${n.writing ? `<div class="writing-line"><span class="spinner"></span> Writing the note from what you captured…</div>` : ""}
          <div class="reason">${esc(unlink(n.snippet))}</div>
          ${n.write_failed && !n.writing && !n.written_by ? `<div class="write-failed small">Kept as you typed it: ${esc(n.write_failed)} <button class="chip" data-act="write">Try again</button></div>` : ""}
          <div class="meta"><span>${esc(ago(n.captured))}</span>${(n.tags || []).map((t) => `<span class="tag">#${esc(t)}</span>`).join("")}</div>
          <div class="homes" data-loading="1"><span class="muted small">${n.writing ? "" : "Finding where it belongs…"}</span></div>
        </div><div class="actions">
          <button data-act="read" title="Read (R)">Read</button>${n.can_write && !n.writing && !n.written_by && !n.write_failed ? `<button data-act="write" title="Write the note from what you captured (W)">Write</button>` : ""}<button data-act="keep" title="Keep it where it is; Dream won't move it (K)">Keep</button><button data-act="archive" title="Archive (A)">Archive</button>
        </div></div></li>`)
      .join("");
    // While the model writes, check back: the card turns into the written note.
    if (notes.some((n) => n.writing)) inboxPoll = setTimeout(() => current === "inbox" && loadInbox(), 2000);
    call("folders").then((folders) => (folderList = folders)).catch(() => {});
    for (const card of list.querySelectorAll(".inbox-card:not(.is-writing)")) loadHomes(card);
    if (!list.contains(document.activeElement) || document.activeElement === list) list.querySelector(".inbox-card.focused")?.focus();
  } catch (error) {
    list.innerHTML = `<li class="empty">${esc(error.message || error)}</li>`;
  }
}

async function loadHomes(card) {
  const box = card.querySelector(".homes");
  try {
    const homes = await call("suggest_homes", { path: card.dataset.path });
    const chip = (h, primary) => `<button class="${primary ? "primary file-best" : "chip"}" data-act="file" data-kind="${esc(h.kind)}" data-target="${esc(h.target)}"
        title="${h.because.length ? "Beside " + esc(h.because.join(", ")) : ""}">${primary ? "Settle in " : ""}${h.kind === "note" ? icon("file-text") : icon("folder")} ${esc(h.label)}</button>`;
    const why = (h) => h.score > 1e30 ? h.because.join(" ") : "Beside " + h.because.join(", ");
    box.innerHTML = (homes.length ? chip(homes[0], true) + homes.slice(1).map((h) => chip(h, false)).join("") : `<span class="muted small">No obvious home yet.</span>`)
      + ` <button class="chip" data-act="other">Other folder…</button>`
      + (homes.length && homes[0].because.length ? `<div class="muted small why">${esc(why(homes[0]))}</div>` : "");
  } catch {
    box.innerHTML = `<button class="chip" data-act="other">Settle in a folder…</button>`;
  }
}

async function fileCard(card, kind, target) {
  const path = card.dataset.path;
  const args = kind === "note" ? { path, home: target } : { path, folder: target };
  const { path: filed } = await call("file_note", args);
  toast(`Settled in ${filed.includes("/") ? filed.slice(0, filed.lastIndexOf("/") + 1) : "the notebook root"}. History can undo it.`);
  const next = card.nextElementSibling;
  card.remove();
  if (next) next.classList.add("focused"), next.focus();
  if (!document.querySelector(".inbox-card")) loadInbox();
  refreshStatus();
}

function askFolder(card) {
  const box = card.querySelector(".homes");
  box.innerHTML = `<form class="other-folder"><input list="folder-options" placeholder="Folder, e.g. work/db (new folders are made)" autofocus>
    <datalist id="folder-options">${folderList.map((f) => `<option value="${esc(f)}">`).join("")}</datalist>
    <button class="primary">File</button> <button type="button" data-act="cancel-other">Cancel</button></form>`;
  const form = box.querySelector("form");
  form.querySelector("input").focus();
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const folder = form.querySelector("input").value.trim();
    try { await fileCard(card, "folder", folder); } catch (error) { toast(String(error.message || error), true); }
  });
}

$("#inbox-list").addEventListener("click", async (event) => {
  const button = event.target.closest("button");
  const card = event.target.closest(".card");
  if (!card) return;
  document.querySelectorAll(".inbox-card").forEach((c) => c.classList.toggle("focused", c === card));
  if (!button) return;
  const path = card.dataset.path;
  try {
    const act = button.dataset.act;
    if (act === "read") { show("remember"); readNote(path); return; }
    if (act === "write") { await writeNote(path, button); loadInbox(); return; }
    if (act === "file") { await fileCard(card, button.dataset.kind, button.dataset.target); return; }
    if (act === "other") { askFolder(card); return; }
    if (act === "cancel-other") { loadHomes(card); return; }
    if (act === "keep") { await call("inbox_keep", { path }); toast("Kept where it is; Dream won't move it. History can undo it."); }
    if (act === "archive") { await call("archive_note", { path }); toast("Archived. Restore it from the Archive any time."); }
    if (act === "keep" || act === "archive") { loadInbox(); refreshStatus(); }
  } catch (error) {
    toast(String(error.message || error), true);
  }
});

// Inbox by keyboard: arrows move, Enter files in the suggestion, F another folder, K keeps,
// A archives, R reads.
$("#inbox-list").addEventListener("keydown", (event) => {
  if (event.target.closest("input")) return;
  const cards = [...document.querySelectorAll(".inbox-card")];
  const at = cards.findIndex((c) => c.classList.contains("focused"));
  const card = cards[at];
  const move = (to) => {
    if (!cards[to]) return;
    cards.forEach((c, i) => c.classList.toggle("focused", i === to));
    cards[to].focus();
  };
  const key = event.key.toLowerCase();
  if (key === "arrowdown") { event.preventDefault(); move(at + 1); }
  else if (key === "arrowup") { event.preventDefault(); move(at - 1); }
  else if (!card) return;
  else if (key === "enter") { event.preventDefault(); card.querySelector(".file-best")?.click(); }
  else if (key === "k") card.querySelector('[data-act="keep"]').click();
  else if (key === "a") card.querySelector('[data-act="archive"]').click();
  else if (key === "r") card.querySelector('[data-act="read"]').click();
  else if (key === "w") card.querySelector('[data-act="write"]')?.click();
  else if (key === "f" || key === "o") { event.preventDefault(); askFolder(card); }
});

// ---------- Dream ----------
const KIND_ICON = {
  merge: icon("git-merge"),
  topic: icon("layers"),
  archive: icon("archive"),
  fix_link: icon("link"),
  pack: icon("package"),
  settle: icon("corner-down-right"),
};
const KIND_NAME = { merge: "Merge copies", topic: "Gather a topic", archive: "Archive", fix_link: "Fix a link", pack: "Pack the archive", settle: "Settle fresh memories" };

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
          ? `${p.preview ? '<button data-act="preview">Preview</button>' : ""}${p.kind === "archive" && p.notes[0].endsWith(".md") ? '<button data-act="pin" title="Pin it: Dream never touches it again">Keep forever</button>' : '<button data-act="reject" title="Dream won’t propose this again">Skip</button>'}<button data-act="apply" class="primary">Accept</button>`
          : p.status === "applied"
            ? `<span class="status applied">${icon("check")} Applied</span><button data-act="undo">Undo</button>`
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
          .map((item) => `<li class="card" data-id="${esc(item.id)}"><div class="card-head"><div class="kind ${item.pack ? "pack" : "archive"}">${item.pack ? icon("package") : icon("archive")}</div>
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
// ---------- Hotkeys ----------
// Keys are recorded, not typed: click the button, press the keys. They're stored as accelerators
// ("CommandOrControl+Alt+Space"), the form the desktop app registers.
const isMac = navigator.platform.startsWith("Mac");

function keyLabel(accelerator) {
  if (!accelerator) return "Not set";
  const names = { CommandOrControl: isMac ? "Cmd" : "Ctrl", Control: "Ctrl", Super: isMac ? "Cmd" : "Super", Alt: isMac ? "Option" : "Alt" };
  return accelerator.split("+").map((part) => names[part] || part).join(" + ");
}

const KEY_NAMES = {
  Space: "Space", Enter: "Enter", Tab: "Tab", Insert: "Insert", Delete: "Delete", Home: "Home", End: "End",
  PageUp: "PageUp", PageDown: "PageDown", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
  Minus: "Minus", Equal: "Equal", Comma: "Comma", Period: "Period", Slash: "Slash", Semicolon: "Semicolon",
  Quote: "Quote", BracketLeft: "BracketLeft", BracketRight: "BracketRight", Backslash: "Backslash", Backquote: "Backquote",
};

/** The accelerator for a key press, or null while only modifiers are down. */
function acceleratorFrom(event) {
  const code = event.code;
  let key = /^Key([A-Z])$/.exec(code)?.[1] || /^Digit(\d)$/.exec(code)?.[1] || /^(F\d{1,2})$/.exec(code)?.[1] || KEY_NAMES[code];
  if (!key) return null;
  const parts = [];
  if (isMac ? event.metaKey : event.ctrlKey) parts.push("CommandOrControl");
  if (isMac && event.ctrlKey) parts.push("Control");
  if (!isMac && event.metaKey) parts.push("Super");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  // A key on its own would be taken from every app; function keys are the exception.
  if (!parts.length && !/^F\d/.test(key)) return null;
  return [...parts, key].join("+");
}

function showKeys() {
  const form = $("#settings-form");
  document.querySelectorAll(".keyrec").forEach((button) => {
    const value = form[button.dataset.for].value;
    button.textContent = keyLabel(value);
    button.classList.toggle("unset", !value);
  });
}

let recording = null;
function stopRecording() {
  if (!recording) return;
  window.removeEventListener("keydown", recording.listener, true);
  recording.button.classList.remove("recording");
  recording = null;
  showKeys();
}

function saveKey(name, value) {
  const form = $("#settings-form");
  form[name].value = value;
  showKeys();
  form.requestSubmit();
}

document.querySelectorAll(".keyrec").forEach((button) =>
  button.addEventListener("click", () => {
    if (recording?.button === button) return stopRecording();
    stopRecording();
    button.classList.add("recording");
    button.textContent = "Press the keys…";
    const listener = (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape" && !event.ctrlKey && !event.altKey && !event.metaKey) return stopRecording();
      const accelerator = acceleratorFrom(event);
      if (!accelerator) {
        const held = [event.ctrlKey && "Ctrl", event.metaKey && (isMac ? "Cmd" : "Super"), event.altKey && (isMac ? "Option" : "Alt"), event.shiftKey && "Shift"].filter(Boolean);
        button.textContent = held.length ? held.join(" + ") + " + …" : "Press the keys… (with Ctrl, Alt, Shift or Super)";
        return;
      }
      const name = button.dataset.for;
      stopRecording();
      saveKey(name, accelerator);
    };
    recording = { button, listener };
    window.addEventListener("keydown", listener, true);
  }),
);
document.querySelectorAll(".keyclear").forEach((button) =>
  button.addEventListener("click", () => { stopRecording(); saveKey(button.dataset.for, ""); }),
);

// How the hotkeys are wired. On Wayland the desktop owns global keys: FastMind asks its shortcuts
// service. Hyprland leaves the keys to the user, so there FastMind binds them itself while it
// runs, unless that's switched off.
async function loadHotkeyStatus(status) {
  const note = $("#hotkey-note");
  const row = $("#hypr-bind-row");
  if (!tauri) { note.hidden = row.hidden = true; return; }
  status = status || (await tauri.core.invoke("hotkey_status").catch(() => null));
  const command = "<code>fastmind-desktop --capture</code> (or <code>--remember</code>)";
  const parts = [];
  const hypr = status?.hyprland;
  row.hidden = !hypr;
  if (status?.how === "native") {
    for (const problem of status.problems || []) parts.push(`<p class="bad">${esc(problem)}</p>`);
  } else if (status?.how === "portal" && hypr) {
    if (hypr.off) {
      parts.push(`<p>FastMind registered its shortcuts with Hyprland; bind keys to them in your config with <code>hl.dsp.global("dev.coccor.fastmind:capture")</code> (and <code>:remember</code>).</p>`);
    } else {
      for (const taken of hypr.conflicts || []) parts.push(`<p class="bad">${esc(taken)} in Hyprland, so FastMind left it alone: pick another key here.</p>`);
      for (const problem of hypr.problems || []) parts.push(`<p class="bad">${esc(problem)}</p>`);
      if (hypr.active) parts.push(`<p>${icon("check")} Bound in Hyprland while FastMind runs. Nothing is written to your Hyprland config.</p>`);
    }
  } else if (status?.how === "portal") {
    const keys = (status.bound || []).map((b) => `${esc(b.id)}: ${esc(b.trigger || "not set yet")}`).join(" · ");
    parts.push(`<p>Your desktop owns global keys: FastMind asked it for these${keys ? ` (${keys})` : ""}.</p>`);
    if (status.can_configure) parts.push(`<div class="actions"><button type="button" data-hotkeys="configure">Change in the desktop's settings…</button></div>`);
  } else if (status?.how === "none") {
    parts.push(`<p>This desktop has no global shortcuts service, so FastMind can't set the keys itself. Bind a key in your desktop's settings to run ${command}.</p>`);
  }
  if (parts.length && status?.how !== "native") parts.push(`<p class="muted">Any key binding can also run ${command}.</p>`);
  note.innerHTML = parts.join("");
  note.hidden = !parts.length;
}

$("#settings-form").bind_in_hyprland.addEventListener("change", () => $("#settings-form").requestSubmit());

$("#hotkey-note").addEventListener("click", async (event) => {
  const button = event.target.closest('button[data-hotkeys="configure"]');
  if (!button) return;
  button.disabled = true;
  try {
    await tauri.core.invoke("hotkeys_configure");
  } catch (error) {
    toast(String(error.message || error), true);
  } finally {
    button.disabled = false;
  }
});

async function loadSettings() {
  try {
    const { settings, file } = await call("settings_get");
    $("#settings-file").textContent = "Saved in " + file;
    const form = $("#settings-form");
    form.notebook.value = settings.notebook || "";
    form.inbox.value = settings.inbox;
    form.hotkey_capture.value = settings.hotkeys.capture;
    form.hotkey_remember.value = settings.hotkeys.remember;
    form.bind_in_hyprland.checked = settings.hotkeys.bind_in_hyprland !== false;
    showKeys();
    loadHotkeyStatus();
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
    call("agent_setup").then((agents) => {
      $("#agent-claude").textContent = agents.claude_code;
      $("#agent-codex").textContent = agents.codex;
    }).catch(() => {});
    form.provider.value = settings.model.provider || "claude-code";
    form.model.value = settings.model.model || "";
    form.base_url.value = settings.model.base_url || "";
    loadSkills();
    form.write_instructions.value = settings.writing.instructions || "";
    showModelFields(form);
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
  s.hotkeys = {
    capture: form.hotkey_capture.value.trim(),
    remember: form.hotkey_remember.value.trim(),
    bind_in_hyprland: form.bind_in_hyprland.checked,
  };
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
    provider === "local" ? { provider: "local", model: form.model.value.trim() || "llama3.2", base_url: form.base_url.value.trim() || "http://localhost:11434/v1" }
    : provider === "claude-code" ? { provider: "claude-code", model: form.model.value.trim() || "sonnet" }
    : { provider: "claude", model: form.model.value.trim() || "claude-opus-5-5" };
  s.writing = { enabled: true, instructions: form.write_instructions.value };
  try {
    if (JSON.stringify(s.model) !== JSON.stringify(form._settings.model)) {
      $("#settings-saved").innerHTML = `<span class="spinner"></span> Checking the AI…`;
      await call("model_check", { model: s.model });
    }
    await call("settings_set", { settings: s });
    form._settings = s;
    $("#settings-saved").textContent = "Saved.";
    setTimeout(() => ($("#settings-saved").textContent = ""), 2500);
    refreshStatus();
  } catch (error) {
    $("#settings-saved").textContent = "";
    toast(String(error.message || error), true);
  }
});

// ---------- Skills ----------
const SKILL_NAMES = {
  "write-capture": "Write a capture", merge: "Merge duplicates", "gather-topic": "Gather a topic",
  "gather-thread": "Gather AI sessions", archive: "Decide what to archive", "fix-link": "Mend broken links",
  settle: "Settle fresh memories", answer: "Answer from your notes", "ai-session": "Write up an AI session",
};

async function loadSkills() {
  try {
    const skills = await call("skills_list");
    $("#skills-list").innerHTML = skills.map((s) => `<li class="skill-row">
      <div class="grow"><div><strong>${esc(SKILL_NAMES[s.name] || s.name)}</strong>${s.customized ? ' <span class="tag">edited</span>' : ""}</div>
      <div class="muted small">${esc(s.description)}</div></div>
      <button type="button" class="chip" data-skill="${esc(s.name)}">${s.customized ? "Edit" : "View / edit"}</button></li>`).join("");
  } catch (error) {
    $("#skills-list").innerHTML = `<li class="muted small">${esc(error.message || error)}</li>`;
  }
}

$("#skills-list").addEventListener("click", async (event) => {
  const button = event.target.closest("[data-skill]");
  if (button) openSkill(button.dataset.skill);
});

async function openSkill(name) {
  const { skill, default: original } = await call("skill_get", { name });
  const dialog = $("#skill-dialog");
  $("#skill-title").textContent = SKILL_NAMES[name] || name;
  $("#skill-path").textContent = skill.customized ? `This notebook's version: ${skill.path}` : "FastMind's default. Saving makes a version for this notebook.";
  $("#skill-text").value = skill.text;
  $("#skill-note").textContent = "";
  $("#skill-reset").hidden = !skill.customized;
  $("#skill-reset").onclick = async () => {
    await call("skill_reset", { name });
    toast("Back to FastMind's default. Your version is kept beside it as .bak.");
    dialog.close();
    loadSkills();
  };
  dialog.onclose = async () => {
    if (dialog.returnValue !== "save") return;
    const text = $("#skill-text").value;
    if (text === original && !skill.customized) return;
    try {
      await call("skill_set", { name, text });
      toast("Saved. Dream re-checks notes with this skill tonight.");
      loadSkills();
    } catch (error) {
      toast(String(error.message || error), true);
    }
  };
  dialog.returnValue = "";
  dialog.showModal();
  $("#skill-text").focus();
}

// Only the fields the chosen provider uses, with its own default model as the hint.
function showModelFields(form) {
  const provider = form.provider.value;
  form.base_url.closest("label").hidden = provider !== "local";
  form.model.placeholder = { local: "llama3.2", claude: "claude-opus-5-5", "claude-code": "sonnet" }[provider] || "";
}
$("#settings-form").provider.addEventListener("change", (event) => {
  const form = event.target.form;
  form.model.value = "";
  showModelFields(form);
});

$("#settings-form").addEventListener("click", async (event) => {
  const button = event.target.closest("[data-copy]");
  if (!button) return;
  try {
    await navigator.clipboard.writeText($("#" + button.dataset.copy).textContent);
    toast("Copied");
  } catch {
    toast("Couldn't copy; select the text instead", true);
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
const MAX_FILE = 50 * 1024 * 1024;

// A capture box that takes pasted screenshots and dropped files, shown as chips under it.
function attachable(area, chips) {
  const files = [];
  const render = () => {
    chips.hidden = !files.length;
    chips.innerHTML = files
      .map((f, i) => `<span class="attach">${/\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(f.name) ? icon("image") : icon("paperclip")} ${esc(f.name)} <button type="button" title="Remove" aria-label="Remove" data-i="${i}">${icon("x")}</button></span>`)
      .join("");
  };
  const add = (file) => {
    if (file.size > MAX_FILE) { toast(`${file.name} is over 50 MB`, true); return; }
    const reader = new FileReader();
    reader.onload = () => {
      const ext = (file.type.split("/")[1] || "png").replace("jpeg", "jpg");
      files.push({ name: file.name || `pasted ${new Date().toISOString().slice(0, 19).replace(/[T:]/g, "-")}.${ext}`, data: reader.result });
      render();
    };
    reader.readAsDataURL(file);
  };
  area.addEventListener("paste", (event) => {
    const pasted = [...(event.clipboardData?.files || [])];
    if (pasted.length) { event.preventDefault(); pasted.forEach(add); }
  });
  const target = area.closest("form") || area;
  target.addEventListener("dragover", (event) => { event.preventDefault(); target.classList.add("dropping"); });
  target.addEventListener("dragleave", () => target.classList.remove("dropping"));
  target.addEventListener("drop", (event) => {
    event.preventDefault();
    target.classList.remove("dropping");
    [...(event.dataTransfer?.files || [])].forEach(add);
  });
  chips.addEventListener("click", (event) => {
    const button = event.target.closest("button");
    if (button) { files.splice(Number(button.dataset.i), 1); render(); }
  });
  render();
  return { files, clear() { files.length = 0; render(); } };
}

// What you typed in a capture box survives closing it: Esc keeps a draft for next time.
function keepDraft(area, key) {
  try { area.value = localStorage.getItem(key) || area.value; } catch {}
  area.addEventListener("input", () => { try { localStorage.setItem(key, area.value); } catch {} });
  return () => { try { localStorage.removeItem(key); } catch {} };
}

async function saveCapture(text, files = []) {
  if (!text.trim() && !files.length) return false;
  try {
    const saved = await call("capture", { text, files });
    toast(saved.writing ? "Saved as a fresh memory. Writing it up…" : `Saved "${saved.title}" as a fresh memory`);
    refreshStatus();
    if (current === "inbox") loadInbox();
    if (current === "home") loadHome();
    if (current === "remember") runSearch($("#query").value, true);
    return true;
  } catch (error) {
    toast(String(error.message || error), true);
    return false;
  }
}

const dialogFiles = attachable($("#capture-text"), $("#capture-files"));
const clearDialogDraft = keepDraft($("#capture-text"), "fastmind.draft.dialog");

function openCapture() {
  $("#capture-dialog").showModal();
  $("#capture-text").focus();
}

$("#capture-open").addEventListener("click", openCapture);
$("#capture-cancel").addEventListener("click", () => $("#capture-dialog").close());
$("#capture-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (await saveCapture($("#capture-text").value, dialogFiles.files)) {
    $("#capture-text").value = "";
    dialogFiles.clear();
    clearDialogDraft();
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
  const results = [...document.querySelectorAll(".result")];
  const at = results.findIndex((r) => r.classList.contains("selected"));
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    const next = results[Math.max(0, Math.min(results.length - 1, at + (event.key === "ArrowDown" ? 1 : -1)))];
    if (next) { readNote(next.dataset.path, next.dataset.archived); next.scrollIntoView({ block: "nearest" }); }
    return;
  }
  if (event.key === "Enter") {
    const value = event.target.value.trim();
    if (event.ctrlKey || event.metaKey || value.endsWith("?")) { event.preventDefault(); ask(); return; }
    const pick = results[at] || results[0];
    if (pick) readNote(pick.dataset.path, pick.dataset.archived);
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
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "e" && current === "remember" && !$("#reader").hidden) { event.preventDefault(); editNote(); }
});

function startMain() {
  $("#app").hidden = false;
  // FastMind opens on Home: the morning page.
  show("home");
  refreshStatus();
  if (tauri) {
    tauri.event.listen("focus-remember", () => { show("remember"); $("#query").select(); });
    tauri.event.listen("show-view", (event) => show(event.payload));
    tauri.event.listen("changed", () => { refreshStatus(); show(current); });
    tauri.event.listen("hotkeys-changed", () => { if (current === "settings") loadHotkeyStatus(); });
  }
}

function startCaptureOnly() {
  $("#capture-only").hidden = false;
  const area = $("#capture-text-mini");
  const files = attachable(area, $("#capture-files-mini"));
  const clearDraft = keepDraft(area, "fastmind.draft.window");
  const close = () => (tauri ? tauri.core.invoke("hide_capture") : window.close());
  $("#capture-close-mini").addEventListener("click", () => close());
  area.focus();
  $("#capture-form-mini").addEventListener("submit", async (event) => {
    event.preventDefault();
    if (await saveCapture(area.value, files.files)) {
      area.value = "";
      files.clear();
      clearDraft();
      close();
    }
  });
  area.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); $("#capture-form-mini").requestSubmit(); }
    if (event.key === "Escape") { event.preventDefault(); close(); }
  });
  const notebooks = captureNotebookPicker(area);
  if (tauri) {
    tauri.event.listen("capture-shown", () => { notebooks.close(); notebooks.refresh(); setTimeout(() => area.focus(), 30); });
    // The main window switched notebooks: show where the next capture goes.
    tauri.event.listen("changed", () => notebooks.refresh());
  }
}

// The capture window's notebook: its name in the title, and a menu to change it there.
// Changing it switches FastMind's notebook, the same as the switcher in the main window.
function captureNotebookPicker(area) {
  const button = $("#capture-nb");
  const menu = $("#capture-nb-menu");
  const refresh = async () => {
    try {
      const status = await call("status");
      $("#capture-nb-name").textContent = String(status.notebook).split(/[\\/]/).filter(Boolean).pop() || String(status.notebook);
      button.title = `Saving to ${status.notebook}\nChange notebook`;
    } catch {
      $("#capture-nb-name").textContent = "No notebook";
      button.title = "Choose a notebook";
    }
  };
  const close = () => { menu.hidden = true; button.setAttribute("aria-expanded", "false"); };
  const open = async () => {
    let list;
    try { list = await call("notebooks"); } catch (error) { toast(String(error.message || error), true); return; }
    const item = (n, current = false) => `<button type="button" role="menuitem" class="nb-item${current ? " current" : ""}" data-path="${esc(n.path)}"${n.exists === false ? " disabled" : ""}>
        <span class="nb-item-name">${current ? icon("check") + " " : ""}${esc(n.name)}${n.exists === false ? " (missing)" : ""}</span><span class="nb-item-path">${esc(n.path)}</span></button>`;
    menu.innerHTML = `
      ${list.current ? item(list.current, true) : ""}
      ${list.recent.map((n) => item(n)).join("")}
      ${list.found.length ? `<div class="nb-section">Found on this computer</div>${list.found.map((n) => item(n)).join("")}` : ""}
      ${tauri ? `<div class="nb-actions"><button type="button" data-nb="open">Open a folder…</button></div>` : ""}`;
    menu.hidden = false;
    button.setAttribute("aria-expanded", "true");
    (menu.querySelector(".nb-item:not(.current):not(:disabled)") || menu.querySelector(".nb-item"))?.focus();
  };
  const choose = async (path) => {
    close();
    try {
      await call("notebook_switch", { path });
      await refresh();
    } catch (error) {
      toast(String(error.message || error), true);
    }
    area.focus();
  };
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    if (menu.hidden) open(); else { close(); area.focus(); }
  });
  menu.addEventListener("click", async (event) => {
    event.stopPropagation();
    if (event.target.closest("[data-nb]")) {
      close();
      const folder = await tauri.core.invoke("pick_folder").catch(() => null);
      if (folder) choose(folder); else area.focus();
      return;
    }
    const pick = event.target.closest(".nb-item[data-path]");
    if (!pick) return;
    if (pick.classList.contains("current")) { close(); area.focus(); return; }
    choose(pick.dataset.path);
  });
  menu.addEventListener("keydown", (event) => {
    // Esc closes only the menu, not the window; arrows move through the notebooks.
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(); button.focus(); return; }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const items = [...menu.querySelectorAll("button:not(:disabled)")];
      const at = items.indexOf(document.activeElement);
      items[(at + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
    }
  });
  document.addEventListener("click", () => { if (!menu.hidden) close(); });
  refresh();
  return { refresh, close };
}

// First run: choose where the notes live before anything touches the disk.
async function startWelcome(first) {
  $("#welcome").hidden = false;
  // The AI: Claude Code when it's installed, else whatever is set up.
  const models = [
    { provider: "claude-code", model: "sonnet", name: "Claude Code", detail: first.claude_code ? "Found on this computer. Uses your Claude plan, no API key." : "Not found. Install it from claude.com/code, sign in, then come back.", ready: !!first.claude_code },
    { provider: "claude", model: "claude-opus-5-5", name: "Claude API", detail: first.anthropic_key ? "ANTHROPIC_API_KEY is set." : "Needs ANTHROPIC_API_KEY set before FastMind starts.", ready: first.anthropic_key },
    { provider: "local", model: "llama3.2", base_url: "http://localhost:11434/v1", name: "A model on this computer", detail: "Ollama, LM Studio or llama.cpp. Nothing leaves this computer.", ready: true },
  ];
  let model = models.find((m) => m.provider === first.model?.provider) || models.find((m) => m.ready) || models[0];
  const modelBox = $("#welcome-models");
  const showModels = () => {
    modelBox.innerHTML = models.map((m, i) => `<button type="button" class="option${m === model ? " chosen" : ""}" data-i="${i}"${m.ready ? "" : ' aria-disabled="true"'}>
      <span class="option-kind">${esc(m.name)}</span><span class="option-path muted">${esc(m.detail)}</span><span class="small">${m.ready ? "" : "not ready"}</span></button>`).join("");
    $("#welcome-local").hidden = model.provider !== "local";
  };
  modelBox.addEventListener("click", (event) => {
    const option = event.target.closest(".option");
    if (!option) return;
    model = models[Number(option.dataset.i)];
    $("#welcome-check").textContent = "";
    showModels();
  });
  showModels();
  const chosenModel = () => model.provider === "local"
    ? { provider: "local", model: $("#welcome-local-model").value.trim() || "llama3.2", base_url: $("#welcome-local-url").value.trim() || "http://localhost:11434/v1" }
    : { provider: model.provider, model: model.model };
  // Says whether the AI answers before anything is saved.
  const checkModel = async () => {
    const box = $("#welcome-check");
    box.innerHTML = `<span class="spinner"></span> Checking ${esc(model.name)}…`;
    try {
      const { ms } = await call("model_check", { model: chosenModel() });
      box.innerHTML = `${icon("check")} ${esc(model.name)} answered in ${(ms / 1000).toFixed(1)} s.`;
      return true;
    } catch (error) {
      box.textContent = `${model.name} didn't answer: ${error.message || error}`;
      return false;
    }
  };
  // A notebook is already chosen: only the AI is missing.
  $("#welcome-notebook").hidden = !first.needs_notebook;
  $("#welcome-demo").hidden = !first.needs_notebook;
  const options = $("#welcome-options");
  let chosen = null;
  const pick = (path) => {
    chosen = path;
    options.querySelectorAll(".option").forEach((o) => o.classList.toggle("chosen", o.dataset.path === path));
    $("#welcome-path").value = path;
  };
  options.innerHTML = first.candidates
    .map((c) => `<button type="button" class="option" data-path="${esc(c.path)}">
        <span class="option-kind">${c.kind === "obsidian" ? "Obsidian vault" : c.kind === "fastcapture" ? "FastCapture notebook" : c.kind === "fastmind" ? "Your FastMind notebook" : c.notes ? "Notebook" : "New notebook"}</span>
        <span class="option-path">${esc(c.path)}</span>
        <span class="muted small">${c.notes ? c.notes + " notes" : c.kind === "fastmind" ? "used before" : "empty, ready for your first note"}</span></button>`)
    .join("");
  options.addEventListener("click", (event) => {
    const option = event.target.closest(".option");
    if (option) pick(option.dataset.path);
  });
  const best = first.candidates.find((c) => c.kind === "fastmind") || first.candidates.find((c) => c.notes > 0) || first.candidates[first.candidates.length - 1];
  if (best) pick(best.path);
  $("#welcome-path").addEventListener("input", (event) => { chosen = event.target.value.trim(); options.querySelectorAll(".option").forEach((o) => o.classList.remove("chosen")); });
  if (tauri) {
    $("#welcome-browse").hidden = false;
    $("#welcome-browse").addEventListener("click", async () => {
      const folder = await tauri.core.invoke("pick_folder").catch(() => null);
      if (folder) pick(folder);
    });
  }
  if (first.ai_sessions) {
    $("#welcome-ai-row").hidden = false;
    $("#welcome-ai-text").textContent = `Turn my ${first.ai_sessions} Claude Code / Codex sessions into notes (secrets removed, read on this computer only)`;
  }
  const key = (k) => k.replace("CommandOrControl", navigator.platform.startsWith("Mac") ? "Cmd" : "Ctrl").replace(/\+/g, " + ");
  $("#welcome-keys").innerHTML = `From anywhere: <kbd>${esc(key(first.hotkeys.capture))}</kbd> captures, <kbd>${esc(key(first.hotkeys.remember))}</kbd> searches your notes.`;
  const finish = async (path) => {
    try {
      if (!(await checkModel())) return;
      if (!first.needs_notebook) {
        const { settings } = await call("settings_get");
        settings.model = chosenModel();
        await call("settings_set", { settings });
        $("#welcome").hidden = true;
        startMain();
        return;
      }
      await call("setup", { notebook: path, model: chosenModel(), ai: !$("#welcome-ai-row").hidden && $("#welcome-ai").checked });
      $("#welcome").hidden = true;
      startMain();
      if (!$("#welcome-ai-row").hidden && $("#welcome-ai").checked) call("ai_import").then(() => refreshStatus()).catch(() => {});
    } catch (error) {
      toast(String(error.message || error), true);
    }
  };
  $("#welcome-start").addEventListener("click", () => {
    const path = ($("#welcome-path").value || chosen || "").trim();
    if (!path && first.needs_notebook) { toast("Choose a folder for your notes", true); return; }
    finish(path);
  });
  $("#welcome-demo").addEventListener("click", async () => {
    try {
      if (!(await checkModel())) return;
      const { settings } = await call("settings_get");
      settings.model = chosenModel();
      await call("settings_set", { settings });
      await call("demo_create", {});
      $("#welcome").hidden = true;
      startMain();
      toast("This is a messy demo notebook. Open Dream and press Dream now. Switch to your own notes in Settings.");
    } catch (error) {
      toast(String(error.message || error), true);
    }
  });
}

async function start() {
  if (new URLSearchParams(location.search).get("mode") === "capture") { startCaptureOnly(); return; }
  try {
    const first = await call("first_run");
    if (first.needs_setup) { startWelcome(first); return; }
  } catch {}
  startMain();
}

start();
