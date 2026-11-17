pub fn render_cockpit_html(token: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>AgentGit — Causal Time-Travel Cockpit</title>
  <style>
    :root {{
      --bg: #09090b;
      --surface: #111114;
      --surface-2: #18181b;
      --border: #27272a;
      --text: #fafafa;
      --muted: #a1a1aa;
      --accent: #38bdf8;
      --pass: #10b981;
      --fail: #ef4444;
      --warn: #f59e0b;
    }}
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      background: var(--bg);
      color: var(--text);
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      font-size: 13px;
      line-height: 1.5;
      min-height: 100vh;
      display: flex;
      flex-direction: column;
    }}
    .mono {{ font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-variant-numeric: tabular-nums; }}
    header {{
      border-bottom: 1px solid var(--border);
      background: var(--surface);
      padding: 12px 20px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 16px;
    }}
    .brand {{ display: flex; align-items: center; gap: 10px; font-weight: 600; letter-spacing: -0.01em; }}
    .badge {{
      border: 1px solid var(--border);
      background: var(--surface-2);
      padding: 2px 8px;
      border-radius: 4px;
      font-size: 11px;
      color: var(--muted);
    }}
    .badge.pass {{ color: var(--pass); border-color: rgba(16,185,129,0.35); }}
    .badge.fail {{ color: var(--fail); border-color: rgba(239,68,68,0.35); }}
    .badge.pending {{ color: var(--warn); border-color: rgba(245,158,11,0.35); }}
    nav a {{ color: var(--muted); text-decoration: none; margin-left: 16px; font-size: 12px; }}
    nav a:hover {{ color: var(--text); }}
    main {{
      flex: 1;
      display: grid;
      grid-template-columns: 280px 1fr 420px;
      min-height: 0;
    }}
    @media (max-width: 1100px) {{
      main {{ grid-template-columns: 1fr; }}
    }}
    .panel {{
      border-right: 1px solid var(--border);
      display: flex;
      flex-direction: column;
      min-height: 0;
    }}
    .panel:last-child {{ border-right: none; }}
    .panel-hdr {{
      padding: 10px 14px;
      border-bottom: 1px solid var(--border);
      background: var(--surface);
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 0.06em;
      color: var(--muted);
      display: flex;
      justify-content: space-between;
      align-items: center;
    }}
    .panel-body {{ padding: 12px; overflow-y: auto; flex: 1; }}
    .card {{
      border: 1px solid var(--border);
      background: var(--surface);
      border-radius: 6px;
      padding: 10px 12px;
      margin-bottom: 8px;
      cursor: pointer;
      transition: border-color 120ms ease;
    }}
    .card:hover, .card.active {{ border-color: var(--accent); }}
    .card.rolled-back {{ opacity: 0.48; }}
    .row {{ display: flex; justify-content: space-between; align-items: center; gap: 8px; }}
    .btn {{
      border: 1px solid var(--border);
      background: var(--surface-2);
      color: var(--text);
      padding: 5px 10px;
      border-radius: 5px;
      font-size: 12px;
      cursor: pointer;
    }}
    .btn:hover {{ border-color: var(--accent); }}
    .btn.danger:hover {{ border-color: var(--fail); color: var(--fail); }}
    .btn.primary {{ background: #0c4a6e; border-color: #0284c7; }}
    pre.diff {{
      background: #050507;
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 12px;
      overflow-x: auto;
      font-size: 12px;
      line-height: 1.45;
      white-space: pre-wrap;
      word-break: break-word;
    }}
    .empty {{ color: var(--muted); padding: 24px 8px; text-align: center; }}
    footer {{
      border-top: 1px solid var(--border);
      background: var(--surface);
      padding: 8px 20px;
      font-size: 11px;
      color: var(--muted);
      display: flex;
      justify-content: space-between;
    }}
    footer a {{ color: var(--muted); text-decoration: none; margin-left: 12px; }}
    footer a:hover {{ color: var(--text); }}
  </style>
</head>
<body>
  <header>
    <div class="brand">
      <span>AgentGit</span>
      <span class="badge mono" id="session-branch">loading...</span>
    </div>
    <nav>
      <button class="btn" onclick="loadDag()">Refresh</button>
      <a href="/privacy">Privacy</a>
      <a href="/terms">Terms</a>
      <a href="/delete-account">Purge Data</a>
    </nav>
  </header>

  <main>
    <section class="panel">
      <div class="panel-hdr"><span>Shadow Agents</span><span class="mono" id="agent-count">0</span></div>
      <div class="panel-body" id="agents-list"><div class="empty">Loading agents...</div></div>
    </section>
    <section class="panel">
      <div class="panel-hdr"><span>Causal DAG Timeline</span><span class="mono" id="node-count">0</span></div>
      <div class="panel-body" id="nodes-list"><div class="empty">Loading causal nodes...</div></div>
    </section>
    <section class="panel">
      <div class="panel-hdr"><span>Inspector &amp; Diff</span><span class="mono" id="selected-id">—</span></div>
      <div class="panel-body" id="inspector"><div class="empty">Select a causal node to inspect its diff and actions.</div></div>
    </section>
  </main>

  <footer>
    <span>Local-First Telemetry (127.0.0.1)</span>
    <div>
      <a href="/privacy">Privacy Policy</a>
      <a href="/terms">Terms &amp; Conditions</a>
      <a href="/delete-account">Delete Local Data</a>
    </div>
  </footer>

  <script>
    const TOKEN = "{token}";
    let dag = null;
    let activeAgent = null;
    let selectedNodeId = null;

    async function loadDag() {{
      const res = await fetch('/api/v1/dag');
      dag = await res.json();
      document.getElementById('session-branch').textContent =
        dag.session.base_branch + ' @ ' + dag.session.base_commit_oid.slice(0, 7);
      renderAgents();
      renderNodes();
    }}

    function renderAgents() {{
      const el = document.getElementById('agents-list');
      document.getElementById('agent-count').textContent = dag.agents.length;
      if (!dag.agents.length) {{
        el.innerHTML = '<div class="empty">No shadow agents spawned yet.</div>';
        return;
      }}
      el.innerHTML = dag.agents.map(a => `
        <div class="card ${{activeAgent === a.id ? 'active' : ''}}" onclick="toggleAgent('${{a.id}}')">
          <div class="row">
            <strong>${{a.id}}</strong>
            <span class="badge mono">${{a.status}}</span>
          </div>
          <div style="color:var(--muted);font-size:12px;margin-top:4px">${{a.role}}</div>
          <div class="row" style="margin-top:8px">
            <span class="mono" style="font-size:11px;color:var(--muted)">${{a.ref_name}}</span>
            <button class="btn primary" onclick="event.stopPropagation();promoteAgent('${{a.id}}')">Promote</button>
          </div>
        </div>
      `).join('');
    }}

    function toggleAgent(id) {{
      activeAgent = activeAgent === id ? null : id;
      renderAgents();
      renderNodes();
    }}

    function renderNodes() {{
      const el = document.getElementById('nodes-list');
      const nodes = activeAgent ? dag.nodes.filter(n => n.agent_id === activeAgent) : dag.nodes;
      document.getElementById('node-count').textContent = nodes.length;
      if (!nodes.length) {{
        el.innerHTML = '<div class="empty">No causal checkpoints recorded.</div>';
        return;
      }}
      el.innerHTML = nodes.map(n => `
        <div class="card ${{selectedNodeId === n.id ? 'active' : ''}} ${{n.is_rolled_back ? 'rolled-back' : ''}}"
             onclick="selectNode('${{n.id}}')">
          <div class="row">
            <span class="mono" style="color:var(--accent)">${{n.agent_id}}</span>
            <span class="badge ${{n.gate_status}} mono">${{n.is_rolled_back ? 'rolled_back' : n.gate_status}}</span>
          </div>
          <div style="margin:6px 0;font-weight:500">${{n.summary}}</div>
          <div class="row mono" style="font-size:11px;color:var(--muted)">
            <span>${{n.tool_name}} · ${{n.commit_oid.slice(0, 7)}}</span>
            <span>$$${{n.token_cost_usd.toFixed(4)}}</span>
          </div>
        </div>
      `).join('');
    }}

    async function selectNode(id) {{
      selectedNodeId = id;
      renderNodes();
      document.getElementById('selected-id').textContent = id.slice(0, 8);
      const node = dag.nodes.find(n => n.id === id);
      const res = await fetch('/api/v1/diff?node_id=' + encodeURIComponent(id));
      const data = await res.json();
      const touches = dag.file_touches.filter(t => t.node_id === id);
      document.getElementById('inspector').innerHTML = `
        <div style="margin-bottom:10px">
          <div class="row" style="margin-bottom:6px">
            <strong>${{node.summary}}</strong>
            <button class="btn danger" onclick="rollbackNode('${{node.id}}')">Causal Rollback</button>
          </div>
          <div class="mono" style="font-size:11px;color:var(--muted)">
            Prompt SHA-256: ${{node.prompt_hash.slice(0, 16)}}...<br/>
            Commit: ${{node.commit_oid}}
          </div>
        </div>
        <div style="margin-bottom:10px">
          ${{touches.map(t => `<div class="mono" style="font-size:11px">${{t.change_type}} ${{t.file_path}} (+${{t.lines_added}} / -${{t.lines_deleted}})</div>`).join('')}}
        </div>
        <pre class="diff mono">${{(data.diff || 'No file diff.').replace(/</g, '&lt;')}}</pre>
      `;
    }}

    async function rollbackNode(id) {{
      const res = await fetch('/api/v1/rollback', {{
        method: 'POST',
        headers: {{ 'Content-Type': 'application/json', 'X-AgentGit-Token': TOKEN }},
        body: JSON.stringify({{ node_id: id }})
      }});
      const out = await res.json();
      if (!res.ok) alert(out.error || 'Rollback failed');
      await loadDag();
    }}

    async function promoteAgent(id) {{
      const res = await fetch('/api/v1/promote', {{
        method: 'POST',
        headers: {{ 'Content-Type': 'application/json', 'X-AgentGit-Token': TOKEN }},
        body: JSON.stringify({{ agent_id: id, squash: true }})
      }});
      const out = await res.json();
      if (!res.ok) alert(out.error || 'Promote failed');
      else alert('Promoted ' + id + ' to HEAD: ' + out.human_head_oid.slice(0, 7));
      await loadDag();
    }}

    loadDag();
  </script>
</body>
</html>"##
    )
}

pub fn render_legal_page(title: &str, body_html: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{title} — AgentGit</title>
  <style>
    body {{
      background: #09090b; color: #fafafa;
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      max-width: 720px; margin: 48px auto; padding: 0 20px; line-height: 1.65;
    }}
    h1 {{ font-size: 22px; margin-bottom: 16px; }}
    p, li {{ color: #a1a1aa; font-size: 14px; margin-bottom: 12px; }}
    a {{ color: #38bdf8; text-decoration: none; }}
    .box {{ border: 1px solid #27272a; background: #111114; border-radius: 6px; padding: 18px; margin-top: 20px; }}
    .btn-danger {{
      border: 1px solid #ef4444; background: #450a0a; color: #fecaca;
      padding: 8px 14px; border-radius: 5px; cursor: pointer; font-size: 13px;
    }}
  </style>
</head>
<body>
  <p><a href="/">&larr; Back to AgentGit Cockpit</a></p>
  <h1>{title}</h1>
  <div class="box">{body_html}</div>
</body>
</html>"##
    )
}

pub fn render_privacy_html() -> String {
    render_legal_page(
        "Privacy Policy (Local-First)",
        "<p>AgentGit operates 100% locally on your machine (<code>127.0.0.1</code>). All causal DAG metadata, prompt SHA-256 hashes, file diffs, and Git shadow worktrees reside strictly inside your repository's <code>.agentgit/</code> directory.</p>
         <p>Zero telemetry, source code, or prompts are transmitted to external servers. Compliant with UU PDP No. 27/2022 and GDPR Art. 13.</p>",
    )
}

pub fn render_terms_html() -> String {
    render_legal_page(
        "Terms & Conditions",
        "<p>AgentGit is distributed under the MIT License. Automated shadow worktree operations (<code>rollback</code>, <code>promote</code>, <code>gc</code>) modify local Git refs and working trees within your repository.</p>
         <p>Users are responsible for reviewing diffs and running verification gates before promoting agent changes to production branches.</p>",
    )
}

pub fn render_delete_account_html(token: &str) -> String {
    let body = format!(
        r##"<p>In accordance with GDPR Art. 17 and UU PDP No. 27/2022, you can permanently erase all AgentGit local state (<code>.agentgit/state.db</code>, shadow worktrees, and <code>refs/agentgit/*</code>) from this repository.</p>
        <p>Your main Git history and human working tree will remain untouched.</p>
        <button class="btn-danger" onclick="purgeAll()">Permanently Purge .agentgit/ Data</button>
        <p id="purge-status" style="margin-top:12px"></p>
        <script>
          async function purgeAll() {{
            if (!confirm('Permanently delete all .agentgit/ shadow worktrees and causal DAG history?')) return;
            const res = await fetch('/api/v1/purge', {{
              method: 'POST',
              headers: {{ 'X-AgentGit-Token': '{token}' }}
            }});
            const data = await res.json();
            document.getElementById('purge-status').textContent = res.ok ? data.message : (data.error || 'Failed');
          }}
        </script>"##
    );
    render_legal_page("User Data Deletion", &body)
}
