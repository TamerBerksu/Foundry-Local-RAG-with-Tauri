const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);

const ui = {
  stalk: document.querySelector(".stalk"),
  shelf: $("shelf"),
  tally: $("tally"),
  drop: $("drop"),
  picker: $("picker"),
  replant: $("replant"),
  sprout: $("sprout"),
  phase: $("sprout-phase"),
  message: $("sprout-message"),
  sparse: $("sparse"),
  clear: $("clear"),
  thread: $("thread"),
  stillness: $("stillness"),
  ground: $("ground"),
  question: $("question"),
  send: $("send"),
  hush: $("hush"),
  tpl: $("exchange-tpl"),
};

const state = {
  ready: false,
  busy: false,
  history: [],
};

const PHASE_WORDS = {
  waking: "waking",
  seeking: "seeking",
  fetching: "drawing water",
  loading: "rooting",
  ready: "ready",
  error: "withered",
};

const ACCEPTED = [".md", ".markdown", ".txt", ".text"];

function escape(text) {
  return text.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

function inline(text) {
  return escape(text)
    .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
    .replace(/__(.+?)__/g, "<strong>$1</strong>")
    .replace(/`([^`]+)`/g, "<code>$1</code>");
}

function shape(text) {
  let html = "";
  let list = null;
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    const bullet = line.match(/^[-*•]\s+(.*)$/);
    const numbered = line.match(/^\d+[.)]\s+(.*)$/);
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    const kind = bullet ? "ul" : numbered ? "ol" : null;

    if (list && kind !== list) {
      html += `</${list}>`;
      list = null;
    }
    if (kind) {
      if (!list) {
        html += `<${kind}>`;
        list = kind;
      }
      html += `<li>${inline((bullet || numbered)[1])}</li>`;
      continue;
    }
    if (!line) continue;
    html += heading ? `<h4>${inline(heading[1])}</h4>` : `<p>${inline(line)}</p>`;
  }
  if (list) html += `</${list}>`;
  return html;
}

function whisper(text, kind = "") {
  document.querySelectorAll(".whisper").forEach((n) => n.remove());
  const note = document.createElement("div");
  note.className = `whisper ${kind}`.trim();
  note.textContent = text;
  document.querySelector(".clearing").appendChild(note);
  setTimeout(() => note.remove(), 3600);
}

function paintStatus(status) {
  if (!status || !status.phase) return;
  ui.sprout.dataset.phase = status.phase;
  ui.phase.textContent = PHASE_WORDS[status.phase] || status.phase;
  ui.message.textContent = status.phase === "ready" && status.model ? status.model : status.message;

  const segments = ui.sprout.querySelectorAll(".shoot span");
  const filled = typeof status.progress === "number" ? Math.round((status.progress / 100) * segments.length) : 0;
  segments.forEach((s, i) => s.classList.toggle("on", status.phase === "fetching" && i < filled));
  if (status.phase === "fetching" && typeof status.progress === "number") {
    ui.message.textContent = `${status.message} · ${status.progress.toFixed(0)}%`;
  }

  state.ready = status.phase === "ready";
  syncSend();
}

function syncSend() {
  ui.send.disabled = !state.ready || state.busy || !ui.question.value.trim();
  ui.hush.hidden = !state.busy;
}

async function refreshShelf() {
  let docs = [];
  try {
    docs = await invoke("documents");
  } catch (e) {
    whisper(String(e));
  }
  ui.tally.textContent = docs.length;
  ui.shelf.replaceChildren();

  if (!docs.length) {
    const bare = document.createElement("li");
    bare.className = "bare";
    bare.textContent = "Nothing planted yet.";
    ui.shelf.appendChild(bare);
    return;
  }

  docs.forEach((doc, i) => {
    const li = document.createElement("li");
    li.style.animationDelay = `${Math.min(i, 12) * 40}ms`;

    const title = document.createElement("span");
    title.className = "t";
    title.textContent = doc.title;

    const meta = document.createElement("span");
    meta.className = "m";
    const segs = `${doc.chunks} segment${doc.chunks === 1 ? "" : "s"}`;
    meta.textContent = doc.category ? `${doc.category} · ${segs}` : segs;

    const cut = document.createElement("button");
    cut.type = "button";
    cut.className = "cut";
    cut.textContent = "cut";
    cut.setAttribute("aria-label", `Remove ${doc.title}`);
    cut.addEventListener("click", async () => {
      try {
        await invoke("remove_document", { docId: doc.docId });
        await refreshShelf();
      } catch (e) {
        whisper(String(e));
      }
    });

    li.append(title, meta, cut);
    ui.shelf.appendChild(li);
  });
}

async function plant(files) {
  const chosen = [...files].filter((f) => ACCEPTED.some((ext) => f.name.toLowerCase().endsWith(ext)));
  if (!chosen.length) {
    whisper("Only .txt and .md texts can be planted.");
    return;
  }
  let planted = 0;
  for (const file of chosen) {
    try {
      await invoke("add_document", { name: file.name, content: await file.text() });
      planted += 1;
    } catch (e) {
      whisper(String(e));
    }
  }
  await refreshShelf();
  if (planted) whisper(`${planted} text${planted === 1 ? "" : "s"} planted.`, "kind");
}

function renderStrips(list, sources) {
  list.replaceChildren();
  sources.forEach((src, i) => {
    const li = document.createElement("li");
    li.style.animationDelay = `${i * 70}ms`;
    li.title = src.excerpt;
    const t = document.createElement("span");
    t.className = "st";
    t.textContent = src.title;
    const s = document.createElement("span");
    s.className = "ss";
    s.textContent = src.category ? `${src.category} · ${src.score.toFixed(2)}` : src.score.toFixed(2);
    li.append(t, s);
    list.appendChild(li);
  });
}

function settle(node) {
  const near = ui.thread.scrollHeight - ui.thread.scrollTop - ui.thread.clientHeight < 140;
  if (near || node) ui.thread.scrollTop = ui.thread.scrollHeight;
}

async function ask(question) {
  ui.stillness.remove();
  const piece = ui.tpl.content.firstElementChild.cloneNode(true);
  const asked = piece.querySelector(".asked");
  const told = piece.querySelector(".told");
  const strips = piece.querySelector(".strips");
  asked.textContent = question;
  ui.thread.appendChild(piece);
  settle(piece);

  state.busy = true;
  syncSend();

  let answer = "";
  let failure = "";
  let frame = 0;
  const paint = () => {
    frame = 0;
    told.innerHTML = shape(answer);
    settle();
  };

  const channel = new Channel();
  channel.onmessage = (msg) => {
    if (msg.kind === "sources") renderStrips(strips, msg.data);
    if (msg.kind === "text") {
      answer += msg.data;
      if (!frame) frame = requestAnimationFrame(paint);
    }
    if (msg.kind === "error") failure = msg.data;
  };

  try {
    await invoke("ask", { question, history: state.history, channel });
  } catch (e) {
    failure = String(e);
  }

  if (frame) cancelAnimationFrame(frame);
  told.innerHTML = shape(answer);
  told.classList.remove("growing");
  if (failure) {
    const note = document.createElement("p");
    note.className = "withered";
    note.textContent = failure;
    told.appendChild(note);
  }
  if (answer.trim()) {
    state.history.push({ role: "user", content: question }, { role: "assistant", content: answer.trim() });
    state.history = state.history.slice(-12);
  }

  state.busy = false;
  syncSend();
  settle();
}

function fit() {
  ui.question.style.height = "auto";
  ui.question.style.height = `${Math.min(ui.question.scrollHeight, 180)}px`;
}

ui.ground.addEventListener("submit", (e) => {
  e.preventDefault();
  const question = ui.question.value.trim();
  if (!question || !state.ready || state.busy) return;
  ui.question.value = "";
  fit();
  ask(question);
});

ui.question.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    ui.ground.requestSubmit();
  }
});

ui.question.addEventListener("input", () => {
  fit();
  syncSend();
});

ui.hush.addEventListener("click", () => invoke("stop"));

ui.sparse.addEventListener("click", async () => {
  const on = ui.sparse.getAttribute("aria-pressed") !== "true";
  ui.sparse.setAttribute("aria-pressed", String(await invoke("set_compact", { on })));
});

ui.clear.addEventListener("click", () => {
  if (state.busy) return;
  state.history = [];
  ui.thread.replaceChildren(ui.stillness);
});

ui.replant.addEventListener("click", async () => {
  try {
    const n = await invoke("replant");
    await refreshShelf();
    whisper(`${n} sample text${n === 1 ? "" : "s"} replanted.`, "kind");
  } catch (e) {
    whisper(String(e));
  }
});

ui.picker.addEventListener("change", async () => {
  await plant(ui.picker.files);
  ui.picker.value = "";
});

let depth = 0;
window.addEventListener("dragenter", (e) => {
  e.preventDefault();
  depth += 1;
  ui.stalk.classList.add("thirsty");
});
window.addEventListener("dragleave", () => {
  depth = Math.max(0, depth - 1);
  if (!depth) ui.stalk.classList.remove("thirsty");
});
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", (e) => {
  e.preventDefault();
  depth = 0;
  ui.stalk.classList.remove("thirsty");
  if (e.dataTransfer?.files?.length) plant(e.dataTransfer.files);
});

listen("status", (event) => paintStatus(event.payload));
invoke("status").then(paintStatus);
refreshShelf();
syncSend();
ui.question.focus();
