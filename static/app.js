"use strict";
const $ = (s) => document.querySelector(s);
const $$ = (s) => Array.from(document.querySelectorAll(s));
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const shortGid = (g) => g ? g.split("|").slice(-1)[0] : "?";
const uriShort = (u) => u.replace("http://opcfoundation.org/UA/", "UA/").replace("urn:factory:", "f:");

let state = { branch: "main", model: null, raw: null };

async function api(path, opts) {
  const res = await fetch(path, opts);
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
  return res.json();
}

function tab(name) {
  $$(".tabs button").forEach((b) => b.classList.toggle("active", b.dataset.tab === name));
  $$(".tab").forEach((t) => t.classList.toggle("hidden", t.id !== `tab-${name}`));
  render();
}
$$(".tabs button").forEach((b) => b.onclick = () => tab(b.dataset.tab));
$("#branch").onchange = (e) => { state.branch = e.target.value; load(); };
$("#btn-refresh").onclick = load;

async function loadOverview() {
  const o = await api("/api/overview");
  $("#rule-version").textContent = o.rule_version;
  $("#schema-version").textContent = o.schema_version;
  $("#head-seq").textContent = o.head;
  const sel = $("#branch");
  const names = o.branches.length ? o.branches.map(([n]) => n) : ["main"];
  if (!names.includes(state.branch)) state.branch = names[0];
  sel.innerHTML = names.map((n) => `<option ${n === state.branch ? "selected" : ""}>${esc(n)}</option>`).join("");
  const st = await api("/api/tour/status");
  $("#codes").textContent = (st.codes_main || []).join(", ") || "—";
}

async function load() {
  await loadOverview();
  state.model = await api(`/api/model/${encodeURIComponent(state.branch)}`);
  render();
}

function sevTag(d) { return `<span class="tag sev-${d.severity}">${d.code}</span>`; }

function render() {
  if (!state.model) return;
  const m = state.model.model;
  renderNamespaces(m, state.model.raw_files);
  renderHierarchy(m);
  renderGraph(m);
  renderUnresolved(m, state.model.quantities);
  renderRaw(state.model);
  renderDiff();
  renderTimeline();
  renderBranches();
  renderRationale();
}

function renderNamespaces(m, files) {
  const rows = m.namespace_uris.map((uri, gi) => {
    const maps = m.namespace_map.map((mp) => {
      const locals = Object.entries(mp.local_to_global).filter(([, g]) => Number(g) === gi).map(([l]) => `ns=${l}`);
      return locals.length ? `<div class="small">${esc(mp.file_id.slice(0, 18))}… → ${locals.join(", ")}</div>` : "";
    }).join("");
    return `<tr><td><code>g${gi}</code></td><td>${esc(uri)}</td><td>${maps || '<span class="small">unused</span>'}</td></tr>`;
  }).join("");
  const fileRows = files.map((f) => `<tr><td><a onclick="showSource('${f.file_id}')">${esc(f.path)}</a></td><td>${f.namespace_count}</td><td>${f.node_count}</td><td><code>${esc(f.file_id)}</code></td></tr>`).join("");
  $("#tab-namespaces").innerHTML = `
    <div class="card"><h3>合并 namespace 表（身份 = URI；数字 index 只在单文件内有效）</h3>
      <table><tr><th>全局 index</th><th>URI</th><th>各文件本地映射</th></tr>${rows}</table></div>
    <div class="card"><h3>来源文件（点击查看原 XML 范围）</h3>
      <table><tr><th>路径</th><th>ns 数</th><th>节点数</th><th>稳定文件身份</th></tr>${fileRows}</table></div>
    <div class="card"><h3>诊断（${m.diagnostics.length}）</h3>${diagList(m.diagnostics)}</div>`;
}

function diagList(diags) {
  if (!diags || !diags.length) return '<p class="small">无诊断</p>';
  return `<table>${diags.map((d) => `<tr><td>${sevTag(d)}</td><td>${esc(d.subject || "")}</td><td>${esc(d.message)}</td>${d.file_id ? `<td><a onclick="showSourceSpan('${d.file_id}',${d.span ? d.span.start : 0},${d.span ? d.span.end : 0})">${d.span ? "L" + d.span.start_line : ""}</a></td>` : "<td></td>"}</tr>`).join("")}</table>`;
}

function renderHierarchy(m) {
  const edges = m.edges.filter((e) => e.ref_type.id === "45" && e.ref_type.ns_uri.includes("opcfoundation.org"));
  const childrenOf = {};
  edges.forEach((e) => { (childrenOf[e.source.render()] ||= []).push(e); });
  const nodeById = Object.fromEntries(m.nodes.map((n) => [n.gid.render(), n]));
  const roots = new Set(m.nodes.map((n) => n.gid.render()));
  edges.forEach((e) => roots.delete(e.target.render()));
  function walk(gid, depth) {
    const n = nodeById[gid];
    if (!n) return "";
    const kids = (childrenOf[gid] || []).map((e) => walk(e.target.render(), depth + 1)).join("");
    return `<div style="margin-left:${depth * 18}px">${n.placeholder ? "⌀ " : "▸ "}<a onclick="nodeDrawer('${esc(gid)}')">${esc(n.browse_name || shortGid(n.gid))}</a> ${n.duplicate ? '<span class="tag sev-Warning pill">duplicate</span>' : ""}${n.placeholder ? '<span class="tag sev-Error pill">unresolved</span>' : ""} <code class="small">${esc(n.stable_id)}</code>${kids}</div>`;
  }
  const typed = m.edges.filter((e) => e.ref_type.id === "40" && e.ref_type.ns_uri.includes("opcfoundation.org"))
    .map((e) => `<tr><td><a onclick="nodeDrawer('${esc(e.source.render())}')">${gname(m, e.source)}</a></td><td>${gname(m, e.target)}</td><td>${e.forward_in_source ? "forward" : "inverse→normalized"}</td></tr>`).join("");
  $("#tab-hierarchy").innerHTML = `
    <div class="card"><h3>类型层次（HasSubtype 闭包）</h3>${Array.from(roots).map((r) => walk(r, 0)).join("") || '<p class="small">无</p>'}</div>
    <div class="card"><h3>实例类型（HasTypeDefinition）</h3><table><tr><th>实例</th><th>类型</th><th>规范化</th></tr>${typed}</table></div>`;
}
function gname(m, gid) { const n = m.nodes.find((x) => x.gid.render() === gid.render()); return n ? esc(n.browse_name || shortGid(gid)) : esc(shortGid(gid)); }

function renderGraph(m) {
  const W = 1200, H = 620;
  const ids = m.nodes.map((n) => n.gid.render());
  const idx = Object.fromEntries(ids.map((g, i) => [g, i]));
  const N = Math.max(ids.length, 1);
  const pos = ids.map((g, i) => {
    const ring = 250, a = (i / N) * Math.PI * 2 - Math.PI / 2;
    return [600 + ring * Math.cos(a), 310 + ring * Math.sin(a)];
  });
  const edgeSvg = m.edges.map((e) => {
    const a = idx[e.source.render()], b = idx[e.target.render()];
    if (a == null || b == null) return "";
    const cls = `edge ${e.forward_in_source ? "" : "inv"} ${e.state === "Dangling" ? "dangling" : ""}`;
    return `<path class="${cls}" d="M${pos[a][0]},${pos[a][1]} L${pos[b][0]},${pos[b][1]}"><title>${esc(e.stable_id)} ${e.forward_in_source ? "forward" : "inverse-normalized"}</title></path>`;
  }).join("");
  const nodeSvg = ids.map((g, i) => {
    const n = m.nodes[idx[g]];
    const cls = `node-rect ${n.placeholder ? "placeholder" : ""} ${n.duplicate ? "duplicate" : ""}`;
    return `<g class="node" onclick="nodeDrawer('${esc(g)}')" style="cursor:pointer"><rect class="${cls}" x="${pos[i][0] - 8}" y="${pos[i][1] - 12}" width="16" height="24" rx="5"/><text x="${pos[i][0] + 12}" y="${pos[i][1] + 4}" font-size="11" fill="#cfe3ff">${esc((n.browse_name || shortGid(n.gid)).slice(0, 22))}</text></g>`;
  }).join("");
  $("#tab-graph").innerHTML = `
    <div class="card"><h3>引用图（实线 forward；虚线 inverse 已规范化，无反向边；红边=悬空；红框=unresolved 占位；黄框=重复 NodeId）</h3>
    <svg class="graph" viewBox="0 0 ${W} ${H}" width="100%" height="${H}">
      <defs><marker id="arrow" markerWidth="8" markerHeight="8" refX="7" refY="3" orient="auto"><path d="M0,0 L7,3 L0,6 Z" fill="#76a9d6"/></marker></defs>
      ${edgeSvg}${nodeSvg}
    </svg></div>`;
}

function renderUnresolved(m, quantities) {
  const dangling = m.edges.filter((e) => e.state === "Dangling");
  const placeholders = m.nodes.filter((n) => n.placeholder);
  const deps = m.diagnostics.filter((d) => d.code.includes("DEP")).map((d) => `<tr><td>${sevTag(d)}</td><td>${esc(d.subject || "")}</td><td>${esc(d.message)}</td></tr>`).join("");
  const qrows = (quantities || []).map((q) => `<tr><td>${esc(q.label)}</td><td>${esc(q.value)}</td><td><code>${esc(q.stable_id)}</code></td></tr>`).join("");
  $("#tab-unresolved").innerHTML = `
    <div class="grid2">
      <div class="card"><h3>悬空引用（${dangling.length}，边与节点均保留）</h3>
        <table>${dangling.map((e) => `<tr><td><a onclick="nodeDrawer('${esc(e.source.render())}')">${esc(shortGid(e.source))}</a></td><td>→</td><td><a onclick="nodeDrawer('${esc(e.target.render())}')">${esc(shortGid(e.target))}</a></td><td><a onclick="showSourceSpan('${e.file_id}',${e.span.start},${e.span.end})">L${e.span.start_line}</a></td></tr>`).join("") || '<p class="small">无</p>'}</table></div>
      <div class="card"><h3>unresolved 占位节点（${placeholders.length}）</h3>
        <table>${placeholders.map((n) => `<tr><td>⌀ <a onclick="nodeDrawer('${esc(n.gid.render())}')">${esc(n.gid.render())}</a></td><td><code>${esc(n.stable_id)}</code></td></tr>`).join("") || '<p class="small">无</p>'}</table></div>
    </div>
    <div class="card"><h3>模型依赖与版本冲突</h3><table>${deps || '<p class="small">无</p>'}</table></div>
    <div class="card"><h3>领域量（原值/单位/坐标基准保留；无法转换留原位）</h3><table><tr><th>变量</th><th>派生值（含原表示）</th><th>稳定身份</th></tr>${qrows}</table></div>`;
}

function renderRaw(view) {
  const aliases = view.model.aliases.map((a) => `<tr><td>${esc(a.file_id.slice(0, 18))}…</td><td>${esc(a.alias)}</td><td>${esc(a.target_text)}</td><td>${a.unresolved ? (a.cycle.length ? '<span class="tag sev-Error">cycle ' + esc(a.cycle.join("→")) + "</span>" : '<span class="tag sev-Warning">unresolved</span>') : esc(shortGid(a.target_gid))}</td></tr>`).join("");
  const deps = view.model.dependencies.map((d) => `<tr><td>${esc(d.file_id.slice(0, 18))}…</td><td>${esc(d.model_uri)}</td><td>${esc(d.version || "")}</td><td>${esc(d.publication_date || "")}</td></tr>`).join("");
  const exts = view.model.extensions.map((x) => `<tr><td><a onclick="showSourceSpan('${x.file_id}',${x.span.start},${x.span.end})">${esc(x.file_id.slice(0, 18))}…</a></td><td>${esc(x.tag)}</td><td><code>${esc(x.stable_id)}</code></td><td><pre class="xml">${esc(x.raw_xml.slice(0, 220))}</pre></td></tr>`).join("");
  const nodeRows = view.model.nodes.map((n) => `<tr><td><span class="kind">${n.kind}</span></td><td><a onclick="nodeDrawer('${esc(n.gid.render())}')">${esc(n.browse_name || "(unresolved)")}</a></td><td><code>${esc(shortGid(n.gid))}</code></td><td>${esc(uriShort(n.gid.ns_uri))}</td><td>${n.origins.length}${n.duplicate ? ' <span class="tag sev-Warning">dup</span>' : ""}${n.placeholder ? ' <span class="tag sev-Error">unresolved</span>' : ""}</td><td><code class="small">${esc(n.stable_id)}</code></td></tr>`).join("");
  $("#tab-raw").innerHTML = `
    <div class="card"><h3>节点（${view.model.nodes.length}；重复来源全部保留）</h3><table><tr><th>类</th><th>BrowseName</th><th>NodeId</th><th>namespace URI</th><th>来源数</th><th>稳定身份</th></tr>${nodeRows}</table></div>
    <div class="grid2">
      <div class="card"><h3>Alias（含循环链）</h3><table><tr><th>文件</th><th>alias</th><th>目标原文</th><th>解析</th></tr>${aliases}</table></div>
      <div class="card"><h3>Model dependency（全记录）</h3><table><tr><th>文件</th><th>ModelUri</th><th>Version</th><th>PublicationDate</th></tr>${deps}</table></div>
    </div>
    <div class="card"><h3>未识别扩展（原样保留，可追源 XML 范围）</h3><table>${exts}</table></div>`;
}

async function renderDiff() {
  const d = await api(`/api/diff/${encodeURIComponent(state.branch)}`);
  const rows = d.changes.map((c) => `<tr><td><span class="tag sev-${c.kind === "Removed" ? "Error" : c.kind === "Added" ? "Info" : "Warning"}">${c.kind}</span></td><td>${esc(c.subject)}</td><td>${esc(c.detail)}</td><td><code class="small">${esc(c.stable_id)}</code></td></tr>`).join("");
  $("#tab-diff").innerHTML = `<div class="card"><h3>空起点 → 当前版本的语义差异（${d.changes.length}）</h3><table><tr><th>变化</th><th>对象</th><th>详情</th><th>稳定身份</th></tr>${rows || '<p class="small">无</p>'}</table></div>`;
}

async function renderTimeline() {
  const data = await api(`/api/events/${encodeURIComponent(state.branch)}`);
  const items = data.events.map((e) => `<div class="timeline-item ${e.branch !== "main" ? "branch" : ""}" onclick="restoreFrame(${e.seq}, '${esc(e.branch)}')">
      <b>#${e.seq}</b> [${esc(e.branch)}] ${esc(e.kind)} — ${esc(e.title)}
      <div class="small">${esc(e.detail)} · 规则 ${esc(e.rule_version)}</div></div>`).join("");
  $("#tab-timeline").innerHTML = `<div class="card"><h3>事件流（点击任一旧事件恢复当时的依赖版本与可见结论；不会用最新结果覆盖历史）</h3>${items}</div>
    <div class="card"><h3>历史帧查看</h3><div id="frame-view"><span class="small">点击上方事件…</span></div></div>`;
}

async function restoreFrame(seq, branch) {
  const m = await api(`/api/model/${encodeURIComponent(branch)}/${seq}`);
  $("#frame-view").innerHTML = `<p>事件 #${seq} 时的模型：${m.nodes.length} 节点 / ${m.edges.length} 边 / ${m.diagnostics.length} 诊断 · rule ${esc(m.rule_version)}</p>${diagList(m.diagnostics)}`;
}

async function renderBranches() {
  const data = await api("/api/branches");
  const rows = data.branches.map(([name, base, note, accepted]) => `<tr><td><b>${esc(name)}</b></td><td>#${base}</td><td>${esc(note)}</td><td>${accepted ? "已接受过（候选仍保留）" : "候选"}</td><td><button onclick="state.branch='${esc(name)}';$('#branch').value='${esc(name)}';load()">查看</button></td></tr>`).join("");
  const dec = await api(`/api/decisions/${encodeURIComponent(state.branch)}`);
  const drows = dec.decisions.map((d) => `<tr><td>#${d[1]}</td><td>${esc(d[2])}</td><td>${esc(d[3] || "（无选择）")}</td><td><pre class="xml">${esc(d[4])}</pre></td><td>${esc(d[5])}</td></tr>`).join("");
  $("#tab-branches").innerHTML = `
    <div class="card"><h3>分支（比较两个分支后继续其一；未选候选不删除）</h3><table><tr><th>分支</th><th>base</th><th>说明</th><th>状态</th><th></th></tr>${rows}</table></div>
    <div class="card"><h3>人工裁定记录（已选 + 未选均保留）</h3><table><tr><th>事件</th><th>议题</th><th>采纳</th><th>未选候选</th><th>理由</th></tr>${drows || '<p class="small">无</p>'}</table></div>
    <div class="card"><h3>增量重算验证（依赖图传播）</h3>
      <p class="small">输入一个稳定身份（如 <code>span:…</code> 或 <code>node_…</code>），查看哪些派生项失效、哪些仍有效。</p>
      <input id="impact-input" type="text" placeholder="node_... 或 span:fileid:offset"/>
      <button class="primary" onclick="runImpact()">传播重算</button><div id="impact-out"></div></div>`;
}

async function runImpact() {
  const v = encodeURIComponent($("#impact-input").value.trim());
  if (!v) return;
  const r = await api(`/api/impact/${encodeURIComponent(state.branch)}?changed=${v}`);
  const rows = Object.entries(r.report.reasons).map(([id, why]) => `<tr><td><code>${esc(id)}</code></td><td>${r.report.invalidated.includes(id) ? '<span class="tag sev-Warning">需重算</span>' : '<span class="tag sev-Info">仍有效</span>'}</td><td>${esc(why)}</td></tr>`).join("");
  $("#impact-out").innerHTML = `<table><tr><th>派生项</th><th>状态</th><th>判断依据</th></tr>${rows}</table>`;
}

async function renderRationale() {
  const o = await api("/api/rationale");
  $("#tab-rationale").innerHTML = `<div class="card"><h3>机器可读判据（随模型/导出一起保存）</h3><table>${o.rationale.map((r) => `<tr><td><code>${r.code}</code></td><td>${esc(r.statement)}</td><td>${esc(r.rule_version)}</td></tr>`).join("")}</table></div>`;
}

// ---------- source drawer ----------
async function showSource(fileId) {
  const f = await api(`/api/source/${fileId}`);
  openDrawer(`来源文件：${f.path}`, sourceHtml(f));
}
async function showSourceSpan(fileId, start, end) {
  const f = await api(`/api/source/${fileId}`);
  openDrawer(`来源范围：${f.path} L${lineOf(f.content, start)}`, sourceHtml(f, start, end));
}
function sourceHtml(f, markStart, markEnd) {
  const ns = f.namespaces.map((n) => `<tr><td>ns=${n.index}</td><td>${esc(n.uri)}</td></tr>`).join("");
  const aliases = f.aliases.map((a) => `<tr><td>${esc(a.alias)}</td><td>${esc(a.node_id_text)}</td></tr>`).join("");
  let xml = esc(f.content);
  if (markStart != null && markEnd != null && markEnd > markStart) {
    const lo = xml.lastIndexOf("&lt;", markStart), hi = xml.indexOf("/&gt;", markEnd);
    const L = lo >= 0 ? lo : markStart, H = hi >= 0 ? hi + 5 : markEnd;
    xml = xml.slice(0, L) + "<mark>" + xml.slice(L, H) + "</mark>" + xml.slice(H);
  }
  return `<div class="grid2">
    <div><h3>namespace table</h3><table>${ns}</table><h3>aliases</h3><table>${aliases || "<p class=small>无</p>"}</table></div>
    <div><h3>节点 ${f.nodes.length} / 顶层未知扩展 ${f.top_extensions.length}</h3>
      ${f.nodes.map((n) => `<div><a onclick="showSourceSpan('${f.file_id}',${n.element.span.start},${n.element.span.end})">${esc(n.node_class)} ${esc(n.node_id_text)} ${esc(n.browse_name_text)}</a> <span class="small">L${n.element.span.start_line}</span></div>`).join("")}
      ${f.top_extensions.map((x) => `<div class="small">⌀ ${esc(x.tag)} L${x.span.start_line}</div>`).join("")}
    </div></div>
    <h3>原 XML（可定位范围）</h3><pre class="xml">${xml}</pre>`;
}
function lineOf(content, offset) { return content.slice(0, offset).split("\n").length; }

async function nodeDrawer(gidStr) {
  const m = state.model.model;
  const n = m.nodes.find((x) => x.gid.render() === gidStr);
  if (!n) return;
  const incoming = m.edges.filter((e) => e.target.render() === gidStr);
  const outgoing = m.edges.filter((e) => e.source.render() === gidStr);
  const edgeRows = (es, dir) => es.map((e) => `<tr><td>${dir}</td><td><code>${esc(shortGid(e.ref_type))}</code></td><td>${e.forward_in_source ? "forward" : "inverse→norm"}</td><td>${e.state}</td><td><a onclick="showSourceSpan('${e.file_id}',${e.span.start},${e.span.end})">L${e.span.start_line}</a></td><td><a onclick="nodeDrawer('${esc(dir === "in" ? e.source.render() : e.target.render())}')">${esc(shortGid(dir === "in" ? e.source : e.target))}</a></td></tr>`).join("");
  const origins = n.origins.map((o) => `<li><a onclick="showSourceSpan('${o.file_id}',${o.span.start},${o.span.end})">${esc(o.node_id_text)} @ ${esc(o.file_id.slice(0, 20))}… L${o.span.start_line}</a></li>`).join("");
  openDrawer(`节点：${n.browse_name || shortGid(n.gid)}`, `
    <p class="small"><code>${esc(n.stable_id)}</code> · ${n.kind} · ${n.placeholder ? "unresolved 占位 · " : ""}${n.duplicate ? "重复 NodeId · " : ""}${esc(uriShort(n.gid.ns_uri))}</p>
    <h3>来源链（${n.origins.length}）</h3><ul>${origins || "<li>无声明来源（缺失依赖占位）</li>"}</ul>
    <h3>引用</h3><table><tr><th></th><th>ref</th><th>方向</th><th>状态</th><th>源</th><th>对端</th></tr>${edgeRows(incoming, "in")}${edgeRows(outgoing, "out")}</table>`);
}

function openDrawer(title, body) { $("#drawer-title").textContent = title; $("#drawer-body").innerHTML = body; $("#drawer").classList.remove("hidden"); }
$("#drawer-close").onclick = () => $("#drawer").classList.add("hidden");

// ---------- tour ----------
$("#btn-tour").onclick = async () => {
  const bar = $("#tourbar");
  bar.classList.remove("hidden");
  bar.textContent = "▶ 正在重放离线案例（导入 → 参数修订 → 局部改动 → 冲突分支 → 删除悬空）…";
  const r = await api("/api/tour/play", { method: "POST" });
  bar.innerHTML = r.steps.map((s, i) => `<div>#${s.seq} ${esc(s.title)}${s.expected_code ? ` → <code>${s.expected_code}</code>` : ""}</div>`).join("");
  await load();
};

// ---------- export/import ----------
$("#btn-export").onclick = () => { window.location = "/api/export"; };
$("#file-import").onchange = async (e) => {
  const text = await e.target.files[0].text();
  const r = await fetch("/api/import", { method: "POST", headers: { "Content-Type": "application/json" }, body: text });
  if (!r.ok) return alert(await r.text());
  alert("导入完成：" + (await r.json()).branches.join(", "));
  load();
};

load().catch((e) => { $("#tab-namespaces").innerHTML = `<pre>${esc(e.stack)}</pre>`; });
