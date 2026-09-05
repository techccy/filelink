/* filelink desktop — popover UI. Plain JS over window.__TAURI__ (withGlobalTauri). */

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let state = { configured: false, base_url: "", autostart: false, uploads: [], history: [] };
let currentView = "main";

const $ = (id) => document.getElementById(id);

/* ---------- helpers ---------- */

function fmtSize(n) {
  if (n == null) return "";
  if (n < 1024) return n + " B";
  if (n < 1048576) return (n / 1024).toFixed(1) + " KB";
  if (n < 1073741824) return (n / 1048576).toFixed(1) + " MB";
  return (n / 1073741824).toFixed(2) + " GB";
}

function fmtCountdown(expiresAt, revokedAt) {
  const now = Date.now();
  if (revokedAt) return "已撤销";
  if (now >= expiresAt) return "已过期";
  let s = Math.max(0, Math.floor((expiresAt - now) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(sec).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
}

function hostOf(url) {
  try { return new URL(url).host; } catch { return url; }
}

function showView(name) {
  currentView = name;
  for (const v of ["main", "settings", "onboarding"]) {
    $("view-" + v).classList.toggle("hidden", v !== name);
  }
  if (name === "settings") {
    $("in-base-url").value = state.base_url || "";
    $("in-token").value = "";
    setSwitch($("btn-autostart"), state.autostart);
    $("test-result").textContent = "";
    $("save-result").textContent = "";
  }
}

/* ---------- rendering ---------- */

function render() {
  $("host-badge").textContent = state.configured ? hostOf(state.base_url) : "";
  $("host-badge").title = state.base_url || "";
  if (!state.configured && currentView !== "onboarding") {
    showView("onboarding");
  } else if (state.configured && currentView === "onboarding") {
    showView("main");
  }
  renderUploads();
  renderHistory();
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function renderUploads() {
  const list = $("upload-list");
  list.textContent = "";
  list.classList.toggle("hidden", state.uploads.length === 0);
  for (const u of state.uploads) {
    const row = el("div", "row-item " + u.status);

    const top = el("div", "row-top");
    top.append(el("div", "row-name", u.filename));
    if (u.status === "done") top.append(el("div", "row-meta", "✓"));
    row.append(top);

    if (u.status === "uploading") {
      const bar = el("div", "bar");
      const fill = el("div");
      const pct = u.size ? Math.min(100, (u.sent / u.size) * 100) : 0;
      fill.style.width = pct.toFixed(1) + "%";
      bar.append(fill);
      row.append(bar);
      row.append(el("div", "row-status",
        u.size ? `${fmtSize(u.sent)} / ${fmtSize(u.size)}` : fmtSize(u.sent)));
    } else if (u.status === "failed") {
      const st = el("div", "row-status");
      st.append(el("span", "err", u.error || "上传失败"));
      row.append(st);
      const acts = el("div", "row-actions");
      const retry = el("button", "act", "重试");
      retry.onclick = () => invoke("retry_upload", { key: u.key }).catch(showToastError);
      const clear = el("button", "act", "清除");
      clear.onclick = () => invoke("clear_upload", { key: u.key }).catch(showToastError);
      acts.append(retry, clear);
      row.append(acts);
    } else if (u.status === "done") {
      row.append(el("div", "row-status", "链接已复制到剪贴板"));
    }
    list.append(row);
  }
}

function renderHistory() {
  const list = $("history-list");
  const empty = $("history-empty");
  list.textContent = "";
  const items = [...state.history].sort((a, b) => b.created_at - a.created_at);
  empty.classList.toggle("hidden", items.length > 0);
  for (const it of items) {
    const expired = !it.revoked_at && Date.now() >= it.expires_at;
    const revoked = !!it.revoked_at;

    const row = el("div", "row-item" + (revoked ? " revoked" : expired ? " expired" : ""));
    row.dataset.id = it.id;

    const top = el("div", "row-top");
    top.append(el("div", "row-name", it.filename));
    const meta = el("div", "row-meta");
    meta.dataset.expires = it.expires_at;
    meta.dataset.revoked = it.revoked_at || "";
    updateMeta(meta);
    top.append(meta);
    row.append(top);

    const acts = el("div", "row-actions");
    if (!expired && !revoked) {
      const copy = el("button", "act", "复制");
      copy.onclick = () => invoke("copy_link", { url: it.url });
      const renew = el("button", "act", "+2h");
      renew.title = "续期一个有效期";
      renew.onclick = () => invoke("renew", { id: it.id })
        .catch((e) => { meta.textContent = String(e); meta.classList.add("gone"); });
      const revoke = el("button", "act danger", "撤销");
      revoke.onclick = () => {
        if (revoke.classList.contains("confirm")) {
          revoke.disabled = true;
          invoke("revoke", { id: it.id }).catch((e) => { meta.textContent = String(e); });
        } else {
          revoke.classList.add("confirm");
          revoke.textContent = "确认？";
          setTimeout(() => {
            revoke.classList.remove("confirm");
            revoke.textContent = "撤销";
          }, 3000);
        }
      };
      acts.append(copy, renew, revoke);
    } else {
      acts.append(el("span", "row-meta gone", revoked ? "已被你撤销" : "链接已过期，7 天后清除"));
    }
    row.append(acts);
    list.append(row);
  }
}

function updateMeta(meta) {
  const expires = Number(meta.dataset.expires);
  const revoked = meta.dataset.revoked ? Number(meta.dataset.revoked) : null;
  meta.textContent = fmtCountdown(expires, revoked);
  meta.className = "row-meta";
  if (!revoked && expires - Date.now() < 10 * 60 * 1000 && Date.now() < expires) {
    meta.classList.add("soon");
  }
  if (revoked || Date.now() >= expires) meta.classList.add("gone");
}

setInterval(() => {
  document.querySelectorAll("#history-list .row-meta[data-expires]").forEach(updateMeta);
}, 500);

function showToastError(e) {
  console.error(e);
}

/* ---------- drag & drop (webview) ---------- */

async function setupDragDrop() {
  try {
    const webview = window.__TAURI__.webview.getCurrentWebview();
    await webview.onDragDropEvent((event) => {
      const p = event.payload;
      const dz = $("dropzone");
      if (p.type === "enter" || p.type === "over") {
        dz.classList.add("drag");
      } else if (p.type === "leave") {
        dz.classList.remove("drag");
      } else if (p.type === "drop") {
        dz.classList.remove("drag");
        if (p.paths && p.paths.length) invoke("upload_paths", { paths: p.paths });
      }
    });
  } catch (e) {
    console.error("drag-drop setup failed", e);
  }
}

/* ---------- wiring ---------- */

function setSwitch(btn, on) {
  btn.setAttribute("aria-checked", on ? "true" : "false");
}

async function doTest(baseUrlInput, tokenInput, resultEl, btn) {
  const base = baseUrlInput.value.trim().replace(/\/+$/, "");
  const token = tokenInput.value.trim();
  if (!base) { resultEl.textContent = "请填写服务地址"; resultEl.className = "hint err"; return; }
  btn.disabled = true;
  resultEl.textContent = "测试中…";
  resultEl.className = "hint";
  try {
    await invoke("test_connection", { baseUrl: base, token: token || null });
    resultEl.textContent = "✓ 连接成功，令牌有效";
    resultEl.className = "hint ok";
  } catch (e) {
    resultEl.textContent = String(e);
    resultEl.className = "hint err";
  }
  btn.disabled = false;
}

async function doSave(baseUrlInput, tokenInput, resultEl, onboarding) {
  const base = baseUrlInput.value.trim().replace(/\/+$/, "");
  const token = tokenInput.value.trim();
  if (!base) { resultEl.textContent = "请填写服务地址"; resultEl.className = "hint err"; return; }
  if (onboarding && !token) { resultEl.textContent = "请填写上传令牌"; resultEl.className = "hint err"; return; }
  try {
    await invoke("save_config", { baseUrl: base, token: token || null });
    resultEl.textContent = "✓ 已保存";
    resultEl.className = "hint ok";
    tokenInput.value = "";
  } catch (e) {
    resultEl.textContent = String(e);
    resultEl.className = "hint err";
  }
}

function wire() {
  $("dropzone").onclick = () => invoke("pick_and_upload").catch(showToastError);
  $("btn-settings").onclick = () => showView("settings");
  $("btn-back").onclick = () => showView("main");

  $("btn-test").onclick = () => doTest($("in-base-url"), $("in-token"), $("test-result"), $("btn-test"));
  $("btn-save").onclick = () => doSave($("in-base-url"), $("in-token"), $("save-result"), false);
  $("ob-test").onclick = () => doTest($("ob-base-url"), $("ob-token"), $("ob-test-result"), $("ob-test"));
  $("ob-done").onclick = () => doSave($("ob-base-url"), $("ob-token"), $("ob-result"), true);

  $("btn-autostart").onclick = async () => {
    try {
      const enabled = await invoke("set_autostart", { enable: !(state.autostart) });
      state.autostart = enabled;
      setSwitch($("btn-autostart"), enabled);
    } catch (e) { showToastError(e); }
  };

  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") invoke("hide_popover").catch(() => {});
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "v" && currentView === "main") {
      invoke("paste_upload").catch((msg) => flashDropzone(String(msg)));
    }
  });
}

let flashTimer = null;
function flashDropzone(msg) {
  const sub = document.querySelector(".dz-sub");
  if (!sub) return;
  const original = "或 点击选择 · ⌘V 粘贴";
  sub.textContent = msg;
  sub.style.color = "var(--danger)";
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => {
    sub.textContent = original;
    sub.style.color = "";
  }, 2500);
}

async function main() {
  wire();
  await listen("state://changed", (event) => {
    state = event.payload;
    render();
  });
  await listen("ui://view", (event) => {
    if (event.payload === "settings") showView("settings");
  });
  await setupDragDrop();
  try {
    state = await invoke("get_state");
  } catch (e) {
    console.error("get_state failed", e);
  }
  render();
}

main();
