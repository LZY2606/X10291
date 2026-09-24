'use strict';
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));

const state = {
  branch: 'main',
  versionId: '',
  model: null,
  stateJson: null,
  caseManifest: null,
  selectedNode: null,
};

function esc(s) {
  return String(s ?? '').replace(/[&<>"]/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
}
function toast(msg, ms = 3500) {
  const t = $('#toast');
  t.textContent = msg;
  t.classList.remove('hidden');
  clearTimeout(toast._t);
  toast._t = setTimeout(() => t.classList.add('hidden'), ms);
}
function chip(text, id) {
  if (id) return `<span class="chip id" data-node="${esc(id)}" title="稳定身份：点击查看来源链">${esc(text)}</span>`;
  return `<span class="chip">${esc(text)}</span>`;
}
async function api(path, opts) {
  const res = await fetch(path, opts);
  const text = await res.text();
  let data;
  try { data = JSON.parse(text); } catch { data = text; }
  if (!res.ok) throw new Error((data && data.error) || res.statusText);
  return data;
}

async function loadCase() {
  try {
    const c = await api('/api/case');
    state.caseManifest = c.manifest;
    renderTour(c);
  } catch (e) {
    $('#tour-status').textContent = '案例包未找到：' + e.message;
  }
}

function renderTour(c) {
  const el = $('#tour-steps');
  el.innerHTML = '';
  c.manifest.steps.forEach((step, i) => {
    const li = document.createElement('li');
    li.dataset.step = step.id;
    li.innerHTML = `<span>${i + 1}. ${esc(step.title)}</span>`;
    const btn = document.createElement('button');
    btn.textContent = '执行/重放';
    btn.onclick = async () => {
      btn.disabled = true;
      try {
        const r = await api(`/api/case/step/${encodeURIComponent(step.id)}`, { method: 'POST' });
        $('#branch').value = step.branch;
        state.branch = step.branch;
        $('#version').value = '';
        await refresh();
        li.classList.add('done');
        const ev = (r.event_kinds || []).join(', ');
        toast(`步骤「${step.title}」完成：版本 ${r.version_id}。事件：${ev || '无'}${r.conflict ? '；冲突：' + r.conflict : ''}`, 6000);
      } catch (e) {
        toast('重放失败：' + e.message, 6000);
      } finally {
        btn.disabled = false;
      }
    };
    li.appendChild(btn);
    el.appendChild(li);
  });
  $('#tour-status').textContent = c.manifest.title + ' — 点击按钮依次重放，可随时从历史版本回到任意旧结论';
}

async function refresh() {
  const q = new URLSearchParams({ branch: state.branch });
  state.stateJson = await api('/api/state?' + q.toString());
  const v = state.stateJson.current;
  state.model = v ? v.model : null;
  state.versionId = state.versionId || '';
  renderVersionSelect();
  renderAll();
}

function renderVersionSelect() {
  const sel = $('#version');
  const cur = state.stateJson.current;
  sel.innerHTML = '<option value="">当前版本</option>';
  (state.stateJson.history || []).slice().reverse().forEach((h) => {
    const o = document.createElement('option');
    o.value = h.id;
    o.textContent = `#${h.seq} ${h.label} (${h.created_at})`;
    sel.appendChild(o);
  });
  if (cur) sel.title = `当前：#${cur.seq} ${cur.label} [${cur.id}] 规则 ${cur.rule_set_version}`;
}

$$('#tabs button').forEach((b) =>
  b.addEventListener('click', () => {
    $$('#tabs button').forEach((x) => x.classList.remove('active'));
    $$('.tab').forEach((x) => x.classList.remove('active'));
    b.classList.add('active');
    $('#tab-' + b.dataset.tab).classList.add('active');
    if (b.dataset.tab === 'graph') drawGraph();
    if (b.dataset.tab === 'diff') loadDiff();
  })
);
$('#branch').addEventListener('change', async (e) => {
  state.branch = e.target.value;
  state.versionId = '';
  $('#version').value = '';
  await refresh();
});
$('#version').addEventListener('change', async (e) => {
  state.versionId = e.target.value;
  if (state.versionId) {
    const v = await api(`/api/branch/${state.branch}/version/${state.versionId}`);
    state.model = v.model;
    renderAll();
  } else {
    await refresh();
  }
});

function nodeById(id) {
  return (state.model?.nodes || []).find((n) => n.id === id);
}
function nodeName(n) {
  if (!n) return '(?)';
  return n.browse_name || n.display_name || n.literal || n.canonical_node_id;
}
function short(s, n = 46) {
  s = String(s ?? '');
  return s.length > n ? s.slice(0, n - 1) + '…' : s;
}

function renderAll() {
  if (!state.model) {
    ['namespaces', 'types', 'unresolved', 'diff', 'events', 'extensions'].forEach((t) => {
      $('#tab-' + t).innerHTML = '<div class="card muted">此分支尚无版本。请从上方执行样例步骤。</div>';
    });
    $('#graph').innerHTML = '';
    return;
  }
  renderNamespaces();
  renderTypes();
  renderUnresolved();
  renderEvents();
  renderExtensions();
  drawGraph();
}

function renderNamespaces() {
  const m = state.model;
  const files = m.files.map((f) => `<div>${chip('file ' + f.path + ' @order ' + f.order, null)} <span class="mono muted">${f.file_id}</span></div>`).join('');
  const rows = m.namespaces.map((ns) => {
    const binds = ns.bindings.map((b) =>
      `<tr><td>${esc(b.path)}</td><td class="mono">ns=${b.index}</td>
       <td><button class="evidence-btn" data-span='${JSON.stringify({ file_id: b.file_id, start: b.span?.[0], end: b.span?.[1] })}'>源 XML</button></td></tr>`
    ).join('');
    return `<div class="card">
      <h3>${chip('uri=' + ns.uri, null)} <span class="muted">canonical index = ${ns.canonical_index}</span>
        <span class="chip id" title="稳定身份">ns-canon-${ns.canonical_index}</span></h3>
      <table><thead><tr><th>文件</th><th>文件内数字 index（仅本文件有效）</th><th>来源</th></tr></thead>
      <tbody>${binds}</tbody></table>
    </div>`;
  }).join('');
  const models = m.models.map((mm) => {
    const reqs = mm.requirements.map((r) =>
      `<tr><td class="mono">${esc(r.uri)}</td><td>${esc(r.requested_version || '—')}</td>
       <td class="status-${esc(r.status.startsWith('accepted') ? 'ok' : r.status)}">${esc(r.status)}</td>
       <td>${esc(r.resolved_by || '—')}</td>
       <td><button class="evidence-btn" data-span='${JSON.stringify({ file_id: r.span.file_id, start: r.span.start, end: r.span.end })}'>源 XML</button></td></tr>`
    ).join('');
    return `<div class="card"><h3>Model ${esc(mm.uri)} ${mm.version ? 'v' + esc(mm.version) : ''}
      <span class="chip id">${mm.id}</span> <span class="muted">provided by ${mm.provided_by.join(', ')}</span></h3>
      <table><thead><tr><th>required URI</th><th>wanted</th><th>status</th><th>resolved</th><th>来源</th></tr></thead><tbody>${reqs}</tbody></table></div>`;
  }).join('');
  const aliases = m.aliases.map((a) =>
    `<tr><td>${esc(a.name)} → ${esc(a.target)}</td><td class="status-${a.status === 'resolved' ? 'ok' : 'dangling'}">${a.status}</td>
     <td class="mono muted">${a.id}</td>
     <td><button class="evidence-btn" data-span='${JSON.stringify({ file_id: a.span.file_id, start: a.span.start, end: a.span.end })}'>源 XML</button></td>
     <td class="muted">${esc(a.chain.join(' → '))}</td></tr>`
  ).join('');
  $('#tab-namespaces').innerHTML =
    `<div class="card"><h3>导入文件（稳定顺序与身份）</h3>${files}</div>
     ${rows}
     <div class="card"><h3>模型与 model dependency</h3>${models || '<span class="muted">无 Models 声明</span>'}</div>
     <div class="card"><h3>Aliases</h3>
       <table><thead><tr><th>alias</th><th>状态</th><th>身份</th><th>来源</th><th>解析链</th></tr></thead>
       <tbody>${aliases || '<tr><td colspan=5 class=muted>无 alias</td></tr>'}</tbody></table></div>`;
  bindEvidence();
}

function renderTypes() {
  const m = state.model;
  // build subtype edges via reference_type containing i=45 (HasSubtype)
  const parentOf = new Map();
  m.edges.filter((e) => e.reference_type.includes('|i=45')).forEach((e) => {
    if (!parentOf.has(e.source)) parentOf.set(e.source, []);
    parentOf.get(e.source).push(e.target);
  });
  const shown = new Set();
  const seenNode = new Set();
  function nodeCard(nid) {
    const n = nodeById(nid);
    if (!n) return '';
    seenNode.add(nid);
    const children = (parentOf.get(nid) || []).map(nodeCard).join('');
    return `<li>${chip(nodeName(n), n.id)} <span class="kind-pill">${esc(n.kind)}</span>
      <span class="chip id">${n.id}</span>
      ${n.status !== 'ok' ? `<span class="status-${esc(n.status)}">${esc(n.status)}</span>` : ''}
      ${children ? `<ul>${children}</ul>` : ''}</li>`;
  }
  const roots = m.nodes.filter((n) => !parentOf.get(n.id) && (parentOf.size)).map((n) => n.id);
  // all nodes that are referenced as child somewhere
  const childSet = new Set();
  parentOf.forEach((v) => v.forEach((c) => childSet.add(c)));
  const topRoots = m.nodes.filter((n) => !childSet.has(n.id) && (m.edges.some((e) => e.reference_type.includes('|i=45') && (e.source === n.id || e.target === n.id)))).map((n) => n.id);
  const tree = `<ul>${topRoots.map(nodeCard).join('')}</ul>`;

  // type hierarchy via HasTypeDefinition (instances -> type)
  const typeDefs = m.edges.filter((e) => e.reference_type.includes('|i=40')).map((e) => {
    const inst = nodeById(e.source); const typ = nodeById(e.target);
    return `<tr><td>${inst ? chip(nodeName(inst), inst.id) : esc(e.source)}</td>
      <td>${typ ? chip(nodeName(typ), typ.id) : esc(e.target)}</td>
      <td class="${esc('status-' + e.status)}">${esc(e.status)}</td></tr>`;
  }).join('');

  // quantities
  const qs = m.quantities.map((q) => {
    const n = nodeById(q.node);
    return `<tr><td>${n ? chip(nodeName(n), n.id) : esc(q.node)}</td>
      <td class="mono">${esc(q.raw_value)} ${esc(q.raw_unit)}</td>
      <td class="mono">${q.converted ? q.canonical_value.toFixed(3) + ' ' + q.canonical_unit : '—'}</td>
      <td>${q.diagnostic ? `<span class="sev-warn">${esc(q.diagnostic)}</span>` : '<span class="status-ok">已规范化</span>'}</td>
      <td class="muted">${esc(q.rule_version)}</td>
      <td><button class="evidence-btn" data-span='${JSON.stringify({ file_id: q.span.file_id, start: q.span.start, end: q.span.end })}'>源 XML</button></td></tr>`;
  }).join('');
  const cs = m.coordinates.map((c) => {
    const n = nodeById(c.node);
    return `<tr><td>${n ? chip(nodeName(n), n.id) : esc(c.node)}</td><td class="mono">${esc(c.frame)}</td>
      <td class="mono">[${esc(c.raw_values.join('; '))}]</td>
      <td>${c.diagnostic ? `<span class="sev-warn">${esc(c.diagnostic)}</span>` : 'ok'}</td>
      <td><button class="evidence-btn" data-span='${JSON.stringify({ file_id: c.span.file_id, start: c.span.start, end: c.span.end })}'>源 XML</button></td></tr>`;
  }).join('');

  $('#tab-types').innerHTML = `<div class="card tree"><h3>类型层次（HasSubtype 闭包）</h3>${tree}</div>
   <div class="card"><h3>实例 → 类型（HasTypeDefinition）</h3>
     <table><thead><tr><th>实例</th><th>类型</th><th>边状态</th></tr></thead><tbody>${typeDefs}</tbody></table></div>
   <div class="card"><h3>领域量（原始表示保留；派生层规范化，无法转换留在原位并附诊断）</h3>
     <table><thead><tr><th>节点</th><th>原始值/单位</th><th>规范化(派生)</th><th>诊断</th><th>规则版本</th><th>来源</th></tr></thead><tbody>${qs}</tbody></table></div>
   <div class="card"><h3>坐标基准</h3>
     <table><thead><tr><th>节点</th><th>坐标基准(frame)</th><th>原始分量</th><th>诊断</th><th>来源</th></tr></thead><tbody>${cs}</tbody></table></div>`;
  bindEvidence();
  bindNodeChips();
  void shown; void seenNode; void roots;
}

let graphLayout = null;
function drawGraph() {
  const m = state.model;
  const svg = $('#graph');
  if (!m) { svg.innerHTML = ''; return; }
  const W = 960, H = 560;
  const nodes = m.nodes;
  const ids = nodes.map((n) => n.id);
  const idx = new Map(ids.map((id, i) => [id, i]));
  // deterministic circular layout seeded by index; force settle few rounds
  const N = nodes.length || 1;
  let pos = nodes.map((n, i) => ({
    x: W / 2 + Math.cos((i / N) * Math.PI * 2) * 220 + (n.status === 'unresolved_stub' ? 0 : 0),
    y: H / 2 + Math.sin((i / N) * Math.PI * 2) * 200,
  }));
  const edges = m.edges.filter((e) => idx.has(e.source) && idx.has(e.target));
  for (let iter = 0; iter < 120; iter++) {
    const force = pos.map(() => ({ x: 0, y: 0 }));
    for (let i = 0; i < N; i++) for (let j = i + 1; j < N; j++) {
      let dx = pos[i].x - pos[j].x, dy = pos[i].y - pos[j].y;
      let d2 = dx * dx + dy * dy + 0.01;
      const k = 14000 / d2;
      force[i].x += dx * k * 0.02; force[i].y += dy * k * 0.02;
      force[j].x -= dx * k * 0.02; force[j].y -= dy * k * 0.02;
    }
    edges.forEach((e) => {
      const a = idx.get(e.source), b = idx.get(e.target);
      let dx = pos[b].x - pos[a].x, dy = pos[b].y - pos[a].y;
      const d = Math.sqrt(dx * dx + dy * dy) || 1;
      const pull = (d - 130) * 0.01;
      force[a].x += dx * pull; force[a].y += dy * pull;
      force[b].x -= dx * pull; force[b].y -= dy * pull;
    });
    pos = pos.map((p, i) => ({
      x: Math.max(40, Math.min(W - 40, p.x + force[i].x)),
      y: Math.max(30, Math.min(H - 30, p.y + force[i].y)),
    }));
  }
  graphLayout = new Map(nodes.map((n, i) => [n.id, pos[i]]));

  const edgeSvg = edges.map((e) => {
    const a = graphLayout.get(e.source), b = graphLayout.get(e.target);
    const cls = e.from_inverse ? 'link inverse' : 'link ' + (e.status !== 'resolved' ? e.status : '');
    return `<line class="${cls}" x1="${a.x.toFixed(1)}" y1="${a.y.toFixed(1)}" x2="${b.x.toFixed(1)}" y2="${b.y.toFixed(1)"><title>${esc(e.id)}\n${esc(e.reference_type)} [${esc(e.status)}]${e.from_inverse ? ' (normalized from inverse)' : ''}</title></line>`;
  }).join('');
  const nodeSvg = nodes.map((n) => {
    const p = graphLayout.get(n.id);
    const fill = n.status === 'unresolved_stub' ? '#5a2330'
      : n.status === 'duplicate_conflict' ? '#5a4a1e'
      : n.kind.includes('Type') ? '#1d3354' : '#233252';
    return `<g class="node" data-node="${esc(n.id)}" transform="translate(${p.x.toFixed(1)},${p.y.toFixed(1)})">
      <circle r="9" fill="${fill}"></circle>
      <text x="12" y="4">${esc(short(nodeName(n), 22))}</text>
      <title>${esc(n.canonical_node_id)} [${esc(n.kind)}] ${esc(n.status)}</title></g>`;
  }).join('');
  svg.innerHTML = `<defs><marker id="arrow" viewBox="0 -5 10 10" refX="14" markerWidth="7" markerHeight="7" orient="auto">
    <path d="M0,-4L8,0L0,4" fill="#7f8db0"></path></marker></defs>${edgeSvg}${nodeSvg}`;
  $$('#graph .node').forEach((g) => g.addEventListener('click', () => showNode(g.dataset.node)));
}

function renderUnresolved() {
  const m = state.model;
  const rows = m.unresolved.map((u) =>
    `<tr><td><span class="sev-warn">${esc(u.kind)}</span></td><td>${esc(u.subject)}</td>
     <td>${esc(u.reason)}</td><td class="mono muted">${u.id}</td>
     <td>${u.evidence.map((s) => `<button class="evidence-btn" data-span='${JSON.stringify({ file_id: s.file_id, start: s.start, end: s.end })}'>${esc(s.label || '源 XML')}</button>`).join(' ')}</td></tr>`
  ).join('');
  const edges = m.edges.filter((e) => e.status !== 'resolved').map((e) =>
    `<tr><td class="status-${esc(e.status)}">${esc(e.status)}</td>
      <td>${chip(nodeName(nodeById(e.source)) || esc(e.source), e.source)} → ${chip(nodeName(nodeById(e.target)) || esc(e.target), e.target)}</td>
      <td class="mono">${esc(e.reference_type)}</td>
      <td>${e.from_inverse ? '<span class="sev-warn">inverse 已规范化，未制造反向边</span>' : ''}</td>
      <td>${e.provenance.map((p) => `<button class="evidence-btn" data-span='${JSON.stringify({ file_id: p.file_id, start: p.span[0], end: p.span[1] })}'>源</button>`).join(' ')}</td></tr>`
  ).join('');
  $('#tab-unresolved').innerHTML =
    `<div class="card"><h3>未解析依赖（保留 unresolved 节点，不吞掉不确定状态）</h3>
      <table><thead><tr><th>种类</th><th>对象</th><th>原因</th><th>身份</th><th>证据</th></tr></thead>
      <tbody>${rows || '<tr><td colspan=5 class="muted">无</td></tr>'}</tbody></table></div>
     <div class="card"><h3>非 resolved 引用边（inverse/forward 规范化但不制造缺边）</h3>
      <table><thead><tr><th>状态</th><th>源 → 目标</th><th>引用类型</th><th>方向</th><th>来源</th></tr></thead>
      <tbody>${edges || '<tr><td colspan=5 class="muted">所有引用均已解析</td></tr>'}</tbody></table></div>`;
  bindEvidence();
  bindNodeChips();
}

function renderEvents() {
  const m = state.model;
  const rows = m.events.map((e) =>
    `<div class="card">
      <div><span class="sev-${esc(e.severity)}">●</span> <strong>#${e.seq} ${esc(e.kind)}</strong>
        <span class="muted mono">${esc(e.id)}</span></div>
      <div>${esc(e.message)}</div>
      <div class="muted">规则版本：${esc(e.rule)} · 分支：${esc(e.branch)} · subject: <span class="mono">${esc(e.subject)}</span></div>
      <div>${(e.candidates || []).map((c) => chip('候选 ' + c, c.startsWith('g-') ? c : null)).join(' ')}</div>
      <div>${e.evidence.map((s) => `<button class="evidence-btn" data-span='${JSON.stringify({ file_id: s.file_id, start: s.start, end: s.end })}'>证据：${esc(s.label || 'XML')}</button>`).join(' ')}</div>
    </div>`
  ).join('');
  const pending = (state.stateJson?.pending_decisions || []).map(([s, c]) =>
    `<li><span class="mono">${esc(s)}</span> → ${esc(c)}</li>`).join('');
  $('#tab-events').innerHTML =
    (pending ? `<div class="card"><h3>待应用裁定</h3><ul>${pending}</ul></div>` : '') + rows;
  bindEvidence();
}

function renderExtensions() {
  const m = state.model;
  const rows = m.extensions.map((x) =>
    `<tr><td>${x.recognized ? '<span class="status-ok">recognized</span>' : '<span class="sev-warn">unknown</span>'}</td>
      <td class="mono">${esc(x.tag)}</td><td>${esc(x.container)}</td>
      <td class="mono muted">${x.id}</td>
      <td><button class="evidence-btn" data-span='${JSON.stringify({ file_id: x.span.file_id, start: x.span.start, end: x.span.end })}'>原始 XML</button></td></tr>`
  ).join('');
  $('#tab-extensions').innerHTML =
    `<div class="card"><h3>未识别扩展均保留（导入即保存原始表示）</h3>
      <table><thead><tr><th>识别状态</th><th>标签</th><th>容器</th><th>身份</th><th>原始片段</th></tr></thead>
      <tbody>${rows || '<tr><td colspan=5 class="muted">无</td></tr>'}</tbody></table></div>`;
  bindEvidence();
}

function bindEvidence() {
  $$('.evidence-btn').forEach((b) =>
    b.onclick = () => {
      let q;
      try { q = JSON.parse(b.dataset.span); } catch { return; }
      if (q.start == null || q.end == null) { toast('该声明无字节范围'); return; }
      showSnippet(q.file_id, q.start, q.end);
    }
  );
}
function bindNodeChips() {
  $$('.chip.id[data-node]').forEach((c) =>
    c.onclick = () => showNode(c.dataset.node)
  );
}

async function showSnippet(fileId, start, end) {
  const q = new URLSearchParams({ file_id: fileId, start, end });
  try {
    const s = await api('/api/snippet?' + q.toString());
    const pad = 400;
    const ctxStart = Math.max(0, start - pad), ctxEnd = end + pad;
    const before = s.text ? '' : '';
    const full = await api('/api/snippet?' + new URLSearchParams({
      file_id: fileId, start: ctxStart, end: ctxEnd,
    }).toString()).catch(() => null);
    openDetail(`<h3>源 XML 范围 <span class="muted mono">${esc(s.path)}:${start}-${end}</span></h3>
      <div class="muted">稳定 file id：<span class="mono">${esc(fileId)}</span></div>
      <div class="snippet">${esc(full ? full.text : s.text)}</div>
      ${full ? `<div class="muted">（展示含上下文；精确范围为字节 ${start}–${end}）</div>` : ''}`);
  } catch (e) { toast('读取片段失败：' + e.message); }
}

async function showNode(id) {
  const n = nodeById(id);
  if (!n) { toast('节点不在当前版本（可能是 unresolved pending 端）'); return; }
  const incoming = state.model.edges.filter((e) => e.target === id);
  const outgoing = state.model.edges.filter((e) => e.source === id);
  function edgeList(list) {
    return list.map((e) => {
      const other = e.source === id ? e.target : e.source;
      const on = nodeById(other);
      return `<li>${e.source === id ? '→' : '←'} ${on ? chip(nodeName(on), on.id) : esc(other)}
        <span class="kind-pill">${esc(short(e.reference_type, 40))}</span>
        <span class="status-${esc(e.status)}">${esc(e.status)}</span>
        ${e.from_inverse ? '<span class="sev-warn">from-inverse</span>' : ''}
        <span class="chip id">${esc(e.id)}</span></li>`;
    }).join('');
  }
  const srcs = n.sources.map((s) =>
    `<div class="card" style="margin:6px 0;padding:8px">
       <div>${esc(s.path)} · <span class="mono">NodeId=${esc(s.raw_node_id_attr)}</span></div>
       <div class="muted mono">raw node ${s.raw_node_id} · bytes ${s.span[0]}-${s.span[1]}</div>
       <button class="evidence-btn" data-span='${JSON.stringify({ file_id: s.file_id, start: s.span[0], end: s.span[1] })}'>跳到源 XML</button>
     </div>`).join('');
  const dup = n.candidates?.length ? `
    <div class="card"><h3>重复 NodeId 候选（多解保留）</h3>
      <div>当前 winner：${n.winner ? '<span class="mono">' + esc(n.winner) + '</span>' : '<span class="sev-warn">未裁定</span>'}</div>
      ${n.sources.map((s, i) => `<button class="decide-btn" data-subject="${esc(n.id)}" data-choice="${esc(s.raw_node_id)}">采用候选 ${i + 1}：${esc(s.path)}</button>`).join(' ')}
    </div>` : '';
  const qty = state.model.quantities.find((q) => q.node === id);
  const crd = state.model.coordinates.find((c) => c.node === id);
  openDetail(`
    <h3>${esc(nodeName(n))} <span class="kind-pill">${esc(n.kind)}</span></h3>
    <div class="muted mono">${esc(n.canonical_node_id)}</div>
    <div>稳定身份：<span class="chip id">${esc(n.id)}</span> 状态：<span class="status-${esc(n.status)}">${esc(n.status)}</span></div>
    <div>指纹：<span class="mono">${esc(n.fingerprint)}</span>（来源变化才会重算）</div>
    <div class="muted">本地 NodeId 表示：${n.local_node_ids.map((x) => chip(x)).join(' ')}</div>
    ${qty ? `<div class="card">量：<span class="mono">${esc(qty.raw_value)} ${esc(qty.raw_unit)}</span>
      → ${qty.converted ? qty.canonical_value.toFixed(3) + ' ' + esc(qty.canonical_unit) : '未转换'}
      <div class="muted">${esc(qty.rule_version)} ${qty.diagnostic ? '· ' + esc(qty.diagnostic) : ''}</div></div>` : ''}
    ${crd ? `<div class="card">坐标基准：<span class="mono">${esc(crd.frame)}</span> [${esc(crd.raw_values.join('; '))}]</div>` : ''}
    ${dup}
    <div class="card"><h3>来源链（可追溯原 XML）</h3>${srcs || '<span class="muted">stub 无来源</span>'}</div>
    <div class="row2">
      <div class="card"><h3>出边</h3><ul>${edgeList(outgoing) || '<li class=muted>无</li>'}</ul></div>
      <div class="card"><h3>入边</h3><ul>${edgeList(incoming) || '<li class=muted>无</li>'}</ul></div>
    </div>`);
  bindEvidence();
  bindNodeChips();
  $$('.decide-btn').forEach((b) => b.onclick = async () => {
    try {
      const r = await api(`/api/branch/${encodeURIComponent(state.branch)}/decision`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ subject_kind: 'duplicate', subject: b.dataset.subject, choice: b.dataset.choice, candidate_ids: [] }),
      });
      toast('已生成新裁定版本：' + r.version_id);
      $('#version').value = '';
      await refresh();
      showNode(b.dataset.subject);
    } catch (e) { toast('裁定冲突（已保留双方，未覆盖）：' + e.message, 6000); }
  });
}

function openDetail(html) {
  $('#detail-body').innerHTML = html;
  $('#detail').classList.remove('hidden');
}
$('#detail-close').onclick = () => $('#detail').classList.add('hidden');

async function loadDiff() {
  try {
    const d = await api('/api/diff?' + new URLSearchParams({
      left_branch: 'main', right_branch: state.branch === 'main' ? 'boundary' : 'main',
    }).toString());
    const rows = d.entries.map((e) => {
      const cls = e.kind.includes('added') ? 'diff-add' : e.kind.includes('removed') || e.kind.includes('resolved') ? 'diff-del' : '';
      const ev = [...(e.evidence_before || []), ...(e.evidence_after || [])]
        .map((s) => `<button class="evidence-btn" data-span='${JSON.stringify({ file_id: s.file_id, start: s.start, end: s.end })}'>源</button>`).join(' ');
      return `<tr><td class="${cls}">${esc(e.kind)}</td><td>${esc(short(e.subject, 80))}</td>
        <td class="muted">${esc(short(e.before, 70))}</td>
        <td>${esc(short(e.after, 70))}</td>
        <td>${e.before_id ? chip(e.before_id, e.before_id) : ''} ${e.after_id ? chip(e.after_id, e.after_id) : ''}</td>
        <td>${ev}</td></tr>`;
    }).join('');
    $('#tab-diff').innerHTML = `<div class="card"><h3>语义差异：${esc(d.left_branch)} ⇄ ${esc(d.right_branch)}</h3>
      <div class="muted">每条差异带稳定身份与来源证据，可反查材料、规则版本与决定事件。</div>
      <table><thead><tr><th>类型</th><th>对象</th><th>before</th><th>after</th><th>身份</th><th>来源</th></tr></thead>
      <tbody>${rows || '<tr><td colspan=6 class=muted>两版本语义一致</td></tr>'}</tbody></table></div>`;
    bindEvidence();
    bindNodeChips();
  } catch (e) {
    $('#tab-diff').innerHTML = `<div class="card sev-error">差异加载失败：${esc(e.message)}</div>`;
  }
}
$('#btn-diff').onclick = () => {
  $$('#tabs button').forEach((x) => x.classList.remove('active'));
  $$('.tab').forEach((x) => x.classList.remove('active'));
  $('[data-tab=diff]').classList.add('active');
  $('#tab-diff').classList.add('active');
  loadDiff();
};

$('#btn-import').onclick = () => $('#import-file').click();
$('#import-file').addEventListener('change', async (e) => {
  const f = e.target.files[0];
  if (!f) return;
  const text = await f.text();
  try {
    const r = await api('/api/import', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: text });
    toast(`导入完成：${r.imported_versions} 个版本。身份/顺序/来源关系保持不变。`, 6000);
    state.versionId = '';
    $('#version').value = '';
    await refresh();
  } catch (er) { toast('导入失败：' + er.message, 6000); }
});

(async function init() {
  await loadCase();
  await refresh().catch((e) => toast('初始状态加载失败：' + e.message));
  bindNodeChips();
})();
