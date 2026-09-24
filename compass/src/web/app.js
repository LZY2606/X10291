"use strict";
const state = { versions: [], current: null, version: null, tab: "namespaces", compareId: null, selected: null };

async function api(path) {
  const res = await fetch(path);
  if (!res.ok) throw new Error(`${path}: ${res.status}`);
  return res.json();
}
function esc(s) { return String(s ?? "").replace(/[&<>"]/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c])); }
function sevClass(s) { return "sev-" + (s || "info"); }
function short(id) { return id ? id.slice(0, 18) : ""; }

async function loadVersions() {
  const data = await api("/api/versions");
  state.versions = data.versions || [];
  state.current = data.current;
  const sel = document.getElementById("versionSelect");
  sel.innerHTML = state.versions.map(v =>
    `<option value="${esc(v.version_id)}">${esc(v.label)} [${esc(v.branch)}]</option>`).join("");
  if (state.current) sel.value = state.current;
  renderVersionList();
  await selectVersion(sel.value);
}

function renderVersionList() {
  const box = document.getElementById("versions");
  const cur = document.getElementById("versionSelect").value;
  box.innerHTML = state.versions.map(v => `
    <div class="ver ${v.version_id === cur ? "active" : ""}" data-id="${esc(v.version_id)}">
      <div>${esc(v.label)} <span class="pill">${esc(v.branch)}</span></div>
      <div class="idcode">${esc(v.version_id)}</div>
      <div class="muted">parent: ${v.parent_id ? short(v.parent_id) : "—"} · #${v.seq}</div>
    </div>`).join("");
  box.querySelectorAll(".ver").forEach(el => el.onclick = () => {
    document.getElementById("versionSelect").value = el.dataset.id;
    selectVersion(el.dataset.id);
  });
}

async function selectVersion(id) {
  state.version = await api(`/api/versions/${id}`);
  renderEvents(); renderTab(); renderPropagation(); renderVersionList();
}

function m() { return state.version.merged || { nodes: {}, edges: [], namespaces: [], quantities: [], dependencies: [], events: [] }; }
function nodeList() { return Object.values(m().nodes); }

function renderEvents() {
  const evs = state.version.events || [];
  document.getElementById("eventLog").innerHTML = evs.map(e => `
    <div class="row">
      <span class="sev-${e.kind === "source_snapshot" ? "info" : "warn"}">#${e.seq} ${esc(e.kind)}</span>
      <span class="tag">${esc(e.event_id)}</span>
    </div>
    <div class="muted" style="padding-bottom:6px">${esc(Object.entries(e.payload || {}).map(([k,v]) => `${k}=${v}`).join(" · "))}</div>
  `).join("") + `<div class="muted" style="margin-top:6px">规则版本: ${esc(state.version.merged?.rule_version || "")}</div>`;
}

function renderTab() {
  const panel = document.getElementById("tabPanel");
  const nodes = nodeList();
  if (state.tab === "namespaces") {
    panel.innerHTML = `<h2>Namespace 映射（数字索引仅文件内有效，按 URI 重映射）</h2>
      <table><thead><tr><th>global</th><th>URI（稳定身份）</th><th>文件内绑定</th></tr></thead><tbody>
      ${m().namespaces.map(n => `<tr>
        <td>${n.global_index}</td>
        <td class="clickable" onclick="showSource('${esc(n.bound[0]?.[0] || "")}')">${esc(n.uri)}</td>
        <td>${n.bound.map(b => `<span class="pill">${esc(b[0])}:ns=${b[1]}</span>`).join(" ")}</td>
      </tr>`).join("")}
      </tbody></table>
      <h2 style="margin-top:14px">模型依赖</h2>
      <table><tbody>${m().dependencies.map(d => `<tr>
        <td>${esc(d.model_uri)}</td><td>v${esc(d.version)}</td>
        <td class="tag">${esc(d.source_id)}</td></tr>`).join("")}</tbody></table>`;
  } else if (state.tab === "hierarchy") {
    const subtype = (m().edges || []).filter(e => e.ref_type.includes("HasSubtype") || e.ref_type.endsWith("s=HasSubtype"));
    const children = {};
    subtype.forEach(e => { (children[e.source.display] ||= []).push(e.target.display); });
    const hasParent = new Set(subtype.map(e => e.target.display));
    const roots = [...new Set(subtype.map(e => e.source.display))].filter(x => !hasParent.has(x));
    function walk(key, depth) {
      const node = nodes.find(n => n.key.display === key);
      const id = node ? node.id : "";
      return `<div style="margin-left:${depth*16}px" class="nodecard clickable" ${id ? `onclick="showNode('${esc(id)}')"` : ""}>
        <span class="badge kind">${esc(node?.kind || "Type")}</span>${esc(key)}
        <div class="idcode">${esc(id)}</div></div>` +
        (children[key] || []).map(k => walk(k, depth + 1)).join("");
    }
    panel.innerHTML = `<h2>类型层次（inverse HasSubtype 已规范化，未制造缺边）</h2>` +
      roots.map(r => walk(r, 0)).join("") +
      nodes.filter(n => n.kind.includes("Object") || n.kind === "Unresolved")
        .map(n => `<div class="nodecard clickable" onclick="showNode('${esc(n.id)}')">
          <span class="badge kind">${esc(n.kind)}</span>${esc(n.key.display)}<div class="idcode">${esc(n.id)}</div></div>`).join("");
  }
  else if (state.tab === "graph") {
    const edges = m().edges || [];
    const keys = [...new Set(edges.flatMap(e => [e.source.display, e.target.display]))];
    const pos = {};
    keys.forEach((k, i) => { const col = i % 4, row = Math.floor(i / 4);
      pos[k] = { x: 60 + col * 190, y: 40 + row * 110 }; });
    const nodeById = Object.fromEntries(nodes.map(n => [n.key.display, n]));
    panel.innerHTML = `<h2>引用图（边均为 forward 规范化；虚线=dangling）</h2>
      <svg width="100%" height="${Math.max(260, 120 + Math.ceil(keys.length/4)*110)}" id="gsvg">
        ${edges.map(e => {
          const a = pos[e.source.display], b = pos[e.target.display];
          if (!a || !b) return "";
          const dangling = e.status !== "ok";
          return `<line x1="${a.x}" y1="${a.y}" x2="${b.x}" y2="${b.y}"
            stroke-dasharray="${dangling ? "5,4" : ""}" stroke="${dangling ? "#f06e6e" : "#4a6b8a"}"
            class="clickable" data-edge="${esc(e.id)}"><title>${esc(e.ref_type)}${dangling ? " (dangling)" : ""}</title></line>`;
        }).join("")}
        ${keys.map(k => {
          const p = pos[k], n = nodeById[k];
          const unres = n && n.unresolved_reason;
          return `<g class="clickable" ${n ? `onclick="showNode('${esc(n.id)}')"` : ""}>
            <rect x="${p.x-8}" y="${p.y-16}" width="170" height="34" rx="6"
              fill="${unres ? "#3a2222" : "#1a2530"}" stroke="${unres ? "#f06e6e" : "#324458"}"/>
            <text x="${p.x}" y="${p.y-2}">${esc(k.split("|").pop().slice(0,26))}</text>
            <text x="${p.x}" y="${p.y+12}" font-size="9" fill="#7f95a8">${esc((n?.kind || "?").slice(0,20))}</text></g>`;
        }).join("")}
      </svg>`;
    panel.querySelectorAll("[data-edge]").forEach(el => el.onclick = () => showEdge(el.dataset.edge));
  } else if (state.tab === "unresolved") {
    const unresolved = nodes.filter(n => n.unresolved_reason);
    const dangling = (m().edges || []).filter(e => e.status !== "ok");
    panel.innerHTML = `<h2>未解析依赖 / 悬空引用（保留 unresolved 节点，不删除）</h2>
      ${unresolved.map(n => `<div class="nodecard clickable" onclick="showNode('${esc(n.id)}')">
        <span class="badge unresolved">${esc(n.unresolved_reason)}</span>${esc(n.key.display)}
        <div class="muted">来源: ${n.source_ids.map(esc).join(", ")}</div></div>`).join("")}
      <h2 style="margin-top:12px">dangling edges</h2>
      ${dangling.map(e => `<div class="row"><span class="clickable sev-warn" onclick="showEdge('${esc(e.id)}')">
        ${esc(e.source.display)} → ${esc(e.target.display)}</span><span class="tag">${esc(e.id)}</span></div>`).join("")}
      <h2 style="margin-top:12px">异常与判定事件</h2>
      ${(m().events || []).filter(e => e.severity !== "info").map(e => `
        <div class="row"><span class="${sevClass(e.severity)}">${esc(e.code)}</span>
        <span class="tag clickable" ${e.object_id ? `onclick="jumpObject('${esc(e.object_id)}')"` : ""}>${esc(e.subject)}</span></div>
        <div class="muted">${esc(e.message)}</div>`).join("")}`;
  } else if (state.tab === "diff") {
    if (!state.compareId) {
      panel.innerHTML = `<h2>语义差异</h2><p class="muted">点击顶部“与父版本比较”，或选择两个版本。</p>`;
    } else {
      renderDiff(); return;
    }
  } else if (state.tab === "quantities") {
    panel.innerHTML = `<h2>领域量（原始表示保留；派生层才规范化，无法转换留在原位）</h2>
      <table><thead><tr><th>稳定身份</th><th>原始值</th><th>单位/基准</th><th>派生值</th><th>诊断</th></tr></thead><tbody>
      ${(m().quantities || []).map(q => `<tr class="clickable" onclick="showSourceSpan('${esc(q.quantity.span.source_id)}',${q.quantity.span.start},${q.quantity.span.end})">
        <td class="idcode">${esc(q.id)}</td>
        <td>${esc(q.quantity.value)}</td>
        <td>${esc(q.quantity.unit || "")} ${q.quantity.coordinate_frame ? `<span class="pill">${esc(q.quantity.coordinate_frame)}</span>` : ""}</td>
        <td>${q.quantity.canonical_value ? esc(q.quantity.canonical_value + " " + (q.quantity.canonical_unit||"")) : '<span class="muted">—</span>'}</td>
        <td>${q.quantity.diagnostic ? `<span class="sev-warn">${esc(q.quantity.diagnostic)}</span>` : ""}</td></tr>`).join("")}
      </tbody></table>`;
  }
}

async function renderDiff() {
  const d = await api(`/api/diff/${encodeURIComponent(state.compareId)}/${encodeURIComponent(state.version.version_id)}`);
  const panel = document.getElementById("tabPanel");
  panel.innerHTML = `<h2>语义差异：${short(state.compareId)} → ${short(state.version.version_id)}</h2>
    ${d.namespace_reorder ? `<p class="sev-warn">检测到 namespace 重排：URI 集合不变，仅文件内索引顺序变化（身份按 URI 稳定）。</p>` : ""}
    <h2>节点</h2>
    <table><tbody>${d.nodes.map(c => `<tr><td><span class="sev-${c.kind === "removed" ? "anomaly" : c.kind === "added" ? "ok" : "warn"}">${esc(c.kind)}</span></td>
      <td class="tag clickable" onclick="jumpObject('${esc(c.id)}')">${esc(c.id)}</td><td>${esc(c.fields.join(", "))}</td></tr>`).join("")}</tbody></table>
    <h2 style="margin-top:10px">边</h2>
    <table><tbody>${d.edges.map(c => `<tr><td class="sev-${c.kind === "removed" ? "anomaly" : "ok"}">${esc(c.kind)}</td>
      <td>${esc(c.ref_type)}</td><td class="tag">${esc(c.source)} → ${esc(c.target)}</td></tr>`).join("")}</tbody></table>`;
}

function findNode(id) { return Object.values(m().nodes).find(n => n.id === id); }
function findEdge(id) { return (m().edges || []).find(e => e.id === id); }

function jumpObject(id) {
  if (findNode(id)) { state.tab = "hierarchy"; setActiveTab(); showNode(id); }
}
function setActiveTab() {
  document.querySelectorAll("#tabs button").forEach(b => b.classList.toggle("active", b.dataset.tab === state.tab));
  renderTab();
}

async function showNode(id) {
  const n = findNode(id); if (!n) return; state.selected = { type: "node", id };
  const events = (m().events || []).filter(e => e.object_id === id);
  const decisions = (state.version.decisions || []);
  document.getElementById("detail").innerHTML = `
    <div><span class="badge kind">${esc(n.kind)}</span><b>${esc(n.display_name || n.key.id)}</b></div>
    <div class="muted">${esc(n.key.ns_uri)} | ${esc(n.key.id)}</div>
    <div class="idcode">${esc(n.id)}</div>
    ${n.browse_name ? `<div class="muted">BrowseName: ${esc(n.browse_name.ns_uri)}#${esc(n.browse_name.name)}</div>` : ""}
    <h2>候选（重复 NodeId 全部保留）</h2>
    ${n.candidates.map(c => `<div class="nodecard clickable" onclick="showSourceSpan('${esc(c.source_id)}',${c.span.start},${c.span.end})">
      <div>${esc(c.source_id)} · ${esc(c.kind)}</div>
      <div class="muted">raw NodeId: ${esc(c.raw_node_id)}</div>
      <div class="idcode">${esc(c.span.source_id)} [${c.span.start}..${c.span.end}]</div></div>`).join("")}
    ${n.unresolved_reason ? `<p class="sev-warn">unresolved: ${esc(n.unresolved_reason)} —— 依赖缺失，节点保留</p>` : ""}
    <h2>关联事件 / 判定依据</h2>
    ${events.map(e => `<div class="${sevClass(e.severity)}"><b>${esc(e.code)}</b>
      <div class="muted">${esc(e.rationale)}</div></div>`).join("")}
    <h2>人工决定</h2>
    ${decisions.length ? decisions.map(d => `<div class="nodecard">
      <div>${esc(d.subject)}</div>
      <div class="sev-ok">采纳: ${esc(d.adopted)}</div>
      <div class="sev-warn">未采纳候选保留: ${(d.rejected||[]).map(esc).join(", ")}</div>
      <div class="muted">${esc(d.reason)}</div></div>`).join("") : '<div class="muted">无</div>'}`;
}

function showEdge(id) {
  const e = findEdge(id); if (!e) return;
  document.getElementById("detail").innerHTML = `
    <b>引用边</b><div class="idcode">${esc(e.id)}</div>
    <div>${esc(e.source.display)} → ${esc(e.target.display)}</div>
    <div class="muted">type: ${esc(e.ref_type)}${e.ref_type_resolved ? "" : " (raw, 未解析)"}</div>
    <div class="muted">raw IsForward=${e.raw_is_forward} → 规范化 forward=${e.forward}</div>
    <div class="muted">状态: <span class="sev-${e.status === "ok" ? "ok" : "warn"}">${esc(e.status)}</span></div>
    <button onclick="showSourceSpan('${esc(e.span.source_id)}',${e.span.start},${e.span.end})">查看源 XML 范围</button>`;
}

async function showSource(sourceId) {
  const version = state.version;
  const src = (version.raw.sources || []).find(s => s.source_id === sourceId);
  if (!src) return;
  showSourceSpan(sourceId, 0, Math.min(400, src.bytes.length));
}

async function showSourceSpan(sourceId, start, end) {
  const data = await api(`/api/versions/${state.version.version_id}/source/${encodeURIComponent(sourceId)}`);
  const content = data.content;
  const pad = 120;
  const a = Math.max(0, start - pad), b = Math.min(content.length, end + pad);
  const snippet = esc(content.slice(a, b));
  const relStart = start - a, relEnd = end - a;
  const before = snippet.slice(0, relStart), marked = snippet.slice(relStart, relEnd), after = snippet.slice(relEnd);
  document.getElementById("detail").innerHTML = `
    <b>源 XML 范围</b> <span class="pill">${esc(data.path)}</span>
    <div class="idcode">${esc(sourceId)} [${start}..${end}]</div>
    <pre>${before}<mark>${marked}</mark>${after}</pre>
    <div class="muted">namespaces: ${(data.namespaces||[]).map(n=>n.uri).join(" · ")}</div>`;
}

function renderPropagation() {
  const p = state.version.propagation;
  if (!p) { document.getElementById("propagation").innerHTML = '<span class="muted">无</span>'; return; }
  document.getElementById("propagation").innerHTML =
    (p.changed_sources.length ? `<div class="muted">变更来源: ${p.changed_sources.map(esc).join(", ")}</div>` : "") +
    (p.changed_parameters.length ? `<div class="muted">变更参数: ${p.changed_parameters.map(esc).join(", ")}</div>` : "") +
    (p.events || []).map(e => `<div class="row"><span class="sev-${e.state === "valid" ? "ok" : "warn"}">${esc(e.state)}</span>
      <span class="tag">${esc(e.reason_code)}</span></div>
      <div class="muted">${short(e.object_id)} · ${esc(e.reason)}</div>`).join("");
}

document.getElementById("tabs").addEventListener("click", (ev) => {
  const b = ev.target.closest("button"); if (!b) return;
  state.tab = b.dataset.tab; setActiveTab();
});
document.getElementById("versionSelect").onchange = (e) => { state.compareId = null; selectVersion(e.target.value); };
document.getElementById("btnDiff").onclick = async () => {
  const parent = state.version.parent_id;
  if (!parent) { alert("基线版本没有父版本"); return; }
  state.compareId = parent; state.tab = "diff"; setActiveTab();
};
document.getElementById("btnExport").onclick = async () => {
  const a = await api("/api/archive");
  const blob = new Blob([JSON.stringify(a, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a"); link.href = url; link.download = "compass-archive.json"; link.click();
  URL.revokeObjectURL(url);
};
document.getElementById("btnReimport").onclick = async () => {
  await api("/api/archive/import");
  alert("档案已重建：身份、顺序与来源关系保持（服务端删库模拟）。");
  await loadVersions();
};

// --- guided 90 second tour ---
const tourSteps = [
  "① 这是基线导入：左侧 main/boundary 两套分支，共 6 个独立版本；规则版本固定为 compass-rules-3。先看 Namespace 映射：同一 URI 在不同文件里数字索引不同，合并后按 URI 重映射。",
  "② 切到「类型层次」：CompressorType→PumpType→EquipmentType。vendor 文件里写的是 IsForward=false 的 HasSubtype，系统交换端点规范化，但不额外制造边。",
  "③ 打开「引用图」：红色虚线是 dangling。切到「未解析依赖」可看到 safety-package 缺失时，目标以 unresolved 节点保留；core 1.1.0 删除 PumpType 后旧引用仍保留为 dangling。",
  "④ 「领域量/单位」：6.8 bar 规范化为 680000 Pa；坐标 Position 保留 PlantSiteFrame 不做跨基准换算；vendorVib-code-9 的 high-ish 无法转换，留在原位并附 UNCONVERTIBLE_VALUE。",
  "⑤ 选 main 的 v3 版本再点「与父版本比较」：只出现 EquipmentTag 新增和 PumpType 相关边删除；传播面板说明 CMP-101 因引用闭包 recomputed，其它节点仍 valid。",
  "⑥ 切到 boundary b1：namespace 重排 + alias 环(CycleA→CycleB)；b2：重复 CMP-101 两个候选都在；b3：core 1.1.0 vs 2.0.0 冲突，人工采纳 2.0.0，未采纳候选保留。点任意异常都能跳到源 XML 高亮范围。"
];
let tourIndex = 0;
function showTour() {
  document.getElementById("tourBox").classList.remove("hidden");
  document.getElementById("tourText").textContent = tourSteps[tourIndex];
  applyTourJump(tourIndex);
}
function applyTourJump(i) {
  const want = ["main#0", "main#0", "main#v3", "main#v3", "main#v3", "boundary#b3"][i];
  const v = state.versions.find(x => want === "main#0" ? x.seq === 0
    : want === "main#v3" ? x.seq === 2 : want === "boundary#b3" && x.label.includes("b3"));
  if (v) { document.getElementById("versionSelect").value = v.version_id; selectVersion(v.version_id); }
  state.tab = ["namespaces","hierarchy","unresolved","quantities","diff","unresolved"][i];
  setActiveTab();
}
document.getElementById("btnTour").onclick = () => { tourIndex = 0; showTour(); };
document.getElementById("tourNext").onclick = () => {
  tourIndex = (tourIndex + 1) % tourSteps.length; showTour();
};
document.getElementById("tourClose").onclick = () => document.getElementById("tourBox").classList.add("hidden");

loadVersions().catch(e => { document.body.insertAdjacentHTML("afterbegin", `<pre>${esc(e.message)}</pre>`); });
