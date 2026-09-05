// --- Auth: token management + login ---
let authToken = null;

function authHeaders(extra) {
  const h = extra || {};
  if (authToken) h['Authorization'] = 'Bearer ' + authToken;
  return h;
}

function showLogin() {
  document.getElementById('login-overlay').style.display = 'flex';
}
function hideLogin() {
  document.getElementById('login-overlay').style.display = 'none';
  document.getElementById('sidebar').style.display = 'flex';
  // Start a fresh session; newSession() renders the sidebar (showing the
  // placeholder + any persisted sessions from prior runs).
  newSession();
}

// --- Session management (state machine) ---
//
// Invariants:
//   - `messages[]` is the single source of truth for the *current* session.
//   - Switching sessions is an atomic transaction: persist the old, load the
//     new, then repaint. Nothing is lost.
//   - A session is persisted as soon as it has one message, and re-persisted
//     after every completed reply. Empty sessions stay in-memory only.
//
let currentSessionId = null;
// Track whether the current session exists on the backend yet. An empty
// "new session" has currentSessionId set but isNewSession=true, so the sidebar
// shows a placeholder without hitting the API for it.
let isNewSession = false;

function genSessionId() {
  return 's' + Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
}

/// Derive a human title from the conversation: last user message wins (so the
/// sidebar reflects where the conversation currently is), fallback to '新会话'.
function sessionTitle(msgs) {
  for (let i = msgs.length - 1; i >= 0; i--) {
    const m = msgs[i];
    if (m.role === 'user') {
      const text = typeof m.content === 'string' ? m.content
        : (m.content && m.content.find) ? (m.content.find(p => p.type === 'text') || {}).text || ''
        : '';
      if (text) return text.slice(0, 30);
    }
  }
  return '新会话';
}

/// Extract plain text from a message content (string or multimodal array).
function contentText(m) {
  if (typeof m.content === 'string') return m.content;
  if (Array.isArray(m.content)) {
    const t = m.content.find(p => p.type === 'text');
    return t ? t.text : '';
  }
  return '';
}

function newSession() {
  // Save the outgoing session first (if it has content), then start fresh.
  saveCurrentSession();
  currentSessionId = genSessionId();
  isNewSession = true;
  messages = [];
  chat.innerHTML = '';
  renderSidebar();
}

async function saveCurrentSession() {
  if (!currentSessionId || messages.length === 0) return;
  isNewSession = false;
  const session = {
    id: currentSessionId,
    title: sessionTitle(messages),
    // Persist the FULL message list (incl. tool_calls / tool results) so a
    // session restores with zero context loss on switch.
    messages: messages,
    updated: Date.now(),
  };
  try {
    await fetch('/api/history', {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify(session),
    });
  } catch (e) { console.error('save session:', e); }
  renderSidebar();
}

async function loadSessionList() {
  try {
    const resp = await fetch('/api/history', { headers: authHeaders() });
    if (resp.status === 401) { showLogin(); return; }
    if (!resp.ok) return [];
    const d = await resp.json();
    // Preserve insertion order (as stored on disk). Do NOT sort by `updated`:
    // re-sorting on every render makes sessions jump around when the user just
    // clicks between them, which is confusing. New sessions are prepended at
    // save time, so the most recent naturally lands on top — once.
    return d.sessions || [];
  } catch (e) { return []; }
}

/// Render the entire sidebar from the backend list + in-memory current session.
async function renderSidebar() {
  const sessions = await loadSessionList();
  const list = document.getElementById('session-list');
  list.innerHTML = '';

  // If the current session is new/unsaved, show it as a highlighted placeholder
  // at the very top so the user sees their new conversation exists.
  if (isNewSession && currentSessionId) {
    list.appendChild(makeSessionItem({
      id: currentSessionId, title: '✦ 新会话', updated: Date.now(),
    }, true));
  }

  sessions.forEach(s => list.appendChild(makeSessionItem(s, false)));
}

function makeSessionItem(s, isNew) {
  const item = document.createElement('div');
  const active = s.id === currentSessionId;
  item.style.cssText = 'padding:8px 10px;margin:4px 0;border-radius:6px;cursor:pointer;font-size:13px;color:var(--text);display:flex;justify-content:space-between;align-items:center' +
    (active ? ';background:var(--panel-2)' : '');
  item.onmouseenter = () => {
    if (!active) item.style.background = 'var(--bg)';
    editBtn.style.opacity = '1';
  };
  item.onmouseleave = () => {
    item.style.background = active ? 'var(--panel-2)' : 'transparent';
    editBtn.style.opacity = '0';
  };

  const label = document.createElement('span');
  label.textContent = s.title;
  label.style.overflow = 'hidden'; label.style.textOverflow = 'ellipsis'; label.style.whiteSpace = 'nowrap';
  label.style.color = isNew ? 'var(--accent)' : 'var(--text)';
  label.style.flex = '1';
  label.onclick = () => { if (!active) switchToSession(s.id); };

  // Inline rename: click ✎ → swap label for an <input>, Enter/blur to commit.
  const editBtn = document.createElement('span');
  editBtn.textContent = '✎';
  editBtn.title = '重命名';
  editBtn.style.color = 'var(--muted)'; editBtn.style.marginLeft = '4px';
  editBtn.style.cursor = 'pointer'; editBtn.style.flexShrink = '0';
  editBtn.style.opacity = '0'; editBtn.style.transition = 'opacity .15s';
  editBtn.onclick = async (e) => {
    e.stopPropagation();
    await renameSession(s.id, label, editBtn, del);
  };

  const del = document.createElement('span');
  del.textContent = '✕'; del.style.color = 'var(--muted)'; del.style.marginLeft = '4px'; del.style.flexShrink = '0';
  del.onclick = async (e) => { e.stopPropagation(); await deleteSession(s.id); };

  item.appendChild(label);
  item.appendChild(editBtn);
  item.appendChild(del);
  return item;
}

/// Inline rename: replace the label with an input field, persist on Enter/blur.
async function renameSession(id, labelEl, editBtn, delBtn) {
  const oldTitle = labelEl.textContent;
  const input = document.createElement('input');
  input.type = 'text';
  input.value = oldTitle;
  input.style.cssText = 'flex:1;background:var(--bg);border:1px solid var(--accent);color:var(--text);padding:2px 6px;border-radius:4px;font-size:13px;font-family:inherit';
  // Swap the label for the input.
  labelEl.replaceWith(input);
  editBtn.style.display = 'none';
  delBtn.style.display = 'none';
  input.focus();
  input.select();

  let committed = false;
  const commit = async () => {
    if (committed) return;
    committed = true;
    const newTitle = input.value.trim() || oldTitle;
    // Restore the label with the new title.
    labelEl.textContent = newTitle;
    input.replaceWith(labelEl);
    editBtn.style.display = '';
    delBtn.style.display = '';
    if (newTitle === oldTitle) return;
    // Persist: upsert the session with the new title. We need the full session
    // body — fetch it, patch the title, save back.
    try {
      const resp = await fetch('/api/history/' + id, { headers: authHeaders() });
      if (!resp.ok) return;
      const session = await resp.json();
      session.title = newTitle;
      await fetch('/api/history', {
        method: 'POST',
        headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify(session),
      });
    } catch (e) { console.error('rename:', e); }
  };
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); commit(); }
    else if (e.key === 'Escape') { input.value = oldTitle; committed = true; labelEl.textContent = oldTitle; input.replaceWith(labelEl); editBtn.style.display = ''; delBtn.style.display = ''; }
  });
  input.addEventListener('blur', commit);
}

/// Atomic switch: save current → load target → repaint. This is the only
/// function that should change currentSessionId after init.
async function switchToSession(id) {
  if (id === currentSessionId) return;
  // 1. Persist the outgoing session so nothing is lost.
  await saveCurrentSession();
  // 2. Fetch the target.
  try {
    const resp = await fetch('/api/history/' + id, { headers: authHeaders() });
    if (resp.status === 401) { showLogin(); return; }
    if (!resp.ok) return;
    const session = await resp.json();
    // 3. Swap state atomically.
    currentSessionId = session.id;
    isNewSession = false;
    messages = session.messages || [];
    // 4. Repaint the chat area from the full message list.
    renderChatFromMessages();
    renderSidebar();
  } catch (e) { console.error('switch session:', e); }
}

/// Repaint the entire chat area from `messages[]`, reconstructing user text,
/// assistant markdown, AND tool-call cards with their results. This is what
/// makes session switching restore the full conversation visually.
function renderChatFromMessages() {
  chat.innerHTML = '';
  for (const m of messages) {
    if (m.role === 'user') {
      addBubble('user', contentText(m));
    } else if (m.role === 'assistant') {
      const text = contentText(m);
      if (text) {
        const d = addBubble('assistant', '');
        d.innerHTML = renderMarkdown(text);
      }
      // Reconstruct tool-call cards from tool_calls + matching tool results.
      if (m.tool_calls) {
        for (const tc of m.tool_calls) {
          let args = tc.function?.arguments;
          try { args = JSON.parse(args); } catch (_) {}
          const tres = addToolCard(tc.function?.name || 'tool', args, false, null);
          // Find the matching tool-result message for this call.
          const result = messages.find(x => x.role === 'tool' && x.tool_call_id === tc.id);
          if (result && tres) {
            tres.textContent = '✓ ' + contentText(result);
            tres.className = 'tres ok';
          }
        }
      }
    }
    // role === 'tool' messages are rendered inline above (paired with their
    // caller). Don't render them standalone.
  }
  scrollDown();
}

async function deleteSession(id) {
  try {
    const resp = await fetch('/api/history/' + id, { method: 'DELETE', headers: authHeaders() });
    if (resp.status === 401) { showLogin(); return; }
    if (!resp.ok && resp.status !== 404) {
      console.error('delete failed:', resp.status, await resp.text());
    }
    if (id === currentSessionId) {
      // Deleted the active session: start a fresh one without saving (it's
      // being deleted, not switched away from).
      currentSessionId = genSessionId();
      isNewSession = true;
      messages = [];
      chat.innerHTML = '';
    }
    renderSidebar();
  } catch (e) { console.error('delete error:', e); }
}

async function doLogin() {
  const user = document.getElementById('login-user').value.trim();
  const pass = document.getElementById('login-pass').value;
  const err = document.getElementById('login-err');
  err.style.display = 'none';
  try {
    const resp = await fetch('/api/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username: user, password: pass }),
    });
    if (resp.ok) {
      const d = await resp.json();
      authToken = d.token;
      hideLogin();
      loadCfg();
    } else {
      err.style.display = 'block';
    }
  } catch (e) {
    err.textContent = '连接失败';
    err.style.display = 'block';
  }
}
document.getElementById('login-btn').onclick = doLogin;
document.getElementById('new-session').onclick = newSession;
document.getElementById('login-pass').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') doLogin();
});

// On page load: show login first. After login, loadCfg is called.
showLogin();

// --- Session heartbeat: check token validity every 5 minutes ---
// If the token expired (24h), proactively return to login instead of waiting
// for the next API call to fail.
setInterval(async () => {
  if (!authToken) return;
  try {
    const resp = await fetch('/api/config', { headers: authHeaders() });
    if (resp.status === 401) {
      authToken = null;
      showLogin();
      const note = document.createElement('div');
      note.style.cssText = 'position:fixed;top:16px;left:50%;transform:translateX(-50%);background:var(--accent);color:#fff;padding:8px 16px;border-radius:6px;z-index:99999;font-size:13px';
      note.textContent = '⏰ 会话已过期,请重新登录';
      document.body.appendChild(note);
      setTimeout(() => note.remove(), 3000);
    }
  } catch (e) {}
}, 300000); // 5 minutes

// --- Config persistence (stored in ~/.liteclaw/config.json via backend) ---
function fillCfg(c) {
  document.getElementById('base_url').value = c.base_url || 'http://172.21.0.1:11434/v1';
  // Respect the saved model; fall back to the fast no-think default on first
  // visit or when the saved model is no longer in the dropdown.
  const wanted = c.model || 'qwen3:30b-a3b-nothink';
  const sel = document.getElementById('model');
  sel.value = [...sel.options].some(o => o.value === wanted) ? wanted : 'qwen3:30b-a3b-nothink';
  document.getElementById('api_key').value = c.api_key || '';
}
async function loadCfg() {
  try {
    const resp = await fetch('/api/config', { headers: authHeaders() });
    if (resp.status === 401) { showLogin(); return; }
    if (resp.ok) {
      const c = await resp.json();
      fillCfg(c);
      return;
    }
  } catch (e) {}
  // Fallback to defaults if backend unreachable.
  fillCfg({});
}
async function saveCfg() {
  const c = {
    base_url: document.getElementById('base_url').value.trim(),
    model: document.getElementById('model').value.trim(),
    api_key: document.getElementById('api_key').value.trim(),
  };
  const btn = document.getElementById('save_cfg');
  const orig = btn.textContent;
  try {
    const resp = await fetch('/api/config', {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify(c),
    });
    if (resp.status === 401) { showLogin(); return; }
    btn.textContent = resp.ok ? '✓ 已保存' : '✗ 失败';
  } catch (e) {
    btn.textContent = '✗ 失败';
  }
  setTimeout(() => { btn.textContent = orig; }, 1500);
}
document.getElementById('save_cfg').onclick = saveCfg;
loadCfg();

// --- <think> tag filter ---
// qwen3/deepseek-r1 emit <think>...</think> reasoning blocks. Instead of
// discarding them, we collect the think content and emit a collapsible
// <details> block so the user can optionally expand it.
class ThinkFilter {
  constructor() { this.inThink = false; this.pending = ''; this.thinkBuf = ''; }
  feed(chunk) {
    let out = '';
    this.pending += chunk;
    while (this.pending.length > 0) {
      if (!this.inThink) {
        const open = this.pending.indexOf('<think>');
        if (open === -1) {
          if (this.pending.length > 7) {
            const cut = this.pending.length - 7;
            out += this.pending.slice(0, cut);
            this.pending = this.pending.slice(cut);
          }
          break;
        }
        out += this.pending.slice(0, open);
        this.pending = this.pending.slice(open + 7);
        this.inThink = true;
        this.thinkBuf = '';
      } else {
        const close = this.pending.indexOf('</think>');
        if (close === -1) {
          // Still inside think: accumulate content, keep tail for partial tag.
          if (this.pending.length > 8) {
            this.thinkBuf += this.pending.slice(0, this.pending.length - 8);
            this.pending = this.pending.slice(this.pending.length - 8);
          }
          break;
        }
        this.thinkBuf += this.pending.slice(0, close);
        this.pending = this.pending.slice(close + 8);
        this.inThink = false;
        // Emit the accumulated think as a collapsible block marker.
        out += '\n\u0002T' + this.thinkBuf.trim() + '\u0002\n';
        this.thinkBuf = '';
      }
    }
    return out;
  }
  flush() {
    let out = '';
    // Unclosed think: emit whatever we have as a block.
    if (this.inThink && this.thinkBuf.trim()) {
      out += '\n\u0002T' + this.thinkBuf.trim() + '\u0002\n';
    } else if (!this.inThink && this.pending.length > 0) {
      out += this.pending;
    }
    this.pending = '';
    this.thinkBuf = '';
    return out;
  }
}

// --- Lightweight Markdown renderer (no external deps) ---
// Renders common markdown into safe HTML. Pipeline: escape HTML first (XSS),
// extract fenced code blocks (protect from inline processing), then apply
// block + inline transforms.
function escapeHtml(s) {
  return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}

function renderMarkdown(md) {
  // 1. Escape all HTML.
  let text = escapeHtml(md);
  // 1b. Extract <think> blocks (marker: \u0002T...\u0002) → collapsible details.
  const thinkBlocks = [];
  text = text.replace(/\u0002T([\s\S]*?)\u0002/g, (m, content) => {
    const idx = thinkBlocks.length;
    thinkBlocks.push('<details style="margin:6px 0;border:1px solid var(--border);border-radius:6px;padding:4px 10px"><summary style="cursor:pointer;color:var(--muted);font-size:13px">💭 思考过程</summary><div style="margin-top:6px;color:var(--muted);font-size:13px;white-space:pre-wrap">' + content + '</div></details>');
    return '\u0000TB' + idx + '\u0000';
  });
  // 2. Extract fenced code blocks (```lang\n...\n```) → placeholders.
  const codeBlocks = [];
  text = text.replace(/```(\w*)\n([\s\S]*?)```/g, (m, lang, code) => {
    const idx = codeBlocks.length;
    codeBlocks.push('<pre><code>' + code.replace(/\n$/, '') + '</code></pre>');
    return '\u0000CB' + idx + '\u0000';
  });
  // 3. Split into lines for block-level processing.
  const lines = text.split('\n');
  let html = '';
  let inList = false;
  let inBlockquote = false;
  const closeList = () => { if (inList) { html += '</ul>'; inList = false; } };
  const closeBq = () => { if (inBlockquote) { html += '</blockquote>'; inBlockquote = false; } };
  for (let raw of lines) {
    const line = raw;
    // Code-block placeholder on its own line.
    const cbMatch = line.match(/^\u0000CB(\d+)\u0000$/);
    if (cbMatch) { closeList(); closeBq(); html += codeBlocks[parseInt(cbMatch[1])]; continue; }
    // Think-block placeholder on its own line.
    const tbMatch = line.match(/^\u0000TB(\d+)\u0000$/);
    if (tbMatch) { closeList(); closeBq(); html += thinkBlocks[parseInt(tbMatch[1])]; continue; }
    // Headings.
    const h = line.match(/^(#{1,3})\s+(.*)/);
    if (h) { closeList(); closeBq(); html += '<h' + h[1].length + '>' + inline(h[2]) + '</h' + h[1].length + '>'; continue; }
    // Unordered list.
    if (/^\s*[-*]\s+/.test(line)) { closeBq(); if (!inList) { html += '<ul>'; inList = true; } html += '<li>' + inline(line.replace(/^\s*[-*]\s+/, '')) + '</li>'; continue; }
    // Ordered list.
    if (/^\s*\d+\.\s+/.test(line)) { closeBq(); if (!inList) { html += '<ul>'; inList = true; } html += '<li>' + inline(line.replace(/^\s*\d+\.\s+/, '')) + '</li>'; continue; }
    // Blockquote.
    if (/^&gt;\s?/.test(line)) { closeList(); if (!inBlockquote) { html += '<blockquote>'; inBlockquote = true; } html += inline(line.replace(/^&gt;\s?/, '')); continue; }
    // Blank line.
    if (line.trim() === '') { closeList(); closeBq(); html += '\n'; continue; }
    // Regular paragraph.
    closeList(); closeBq();
    html += '<p>' + inline(line) + '</p>';
  }
  closeList(); closeBq();
  // Merge consecutive <p> (a paragraph split across lines).
  html = html.replace(/<\/p>\n?<p>/g, '<br>');
  // Restore code-block placeholders embedded in lines (rare).
  html = html.replace(/\u0000CB(\d+)\u0000/g, (m, i) => codeBlocks[parseInt(i)]);
  // Restore think-block placeholders.
  html = html.replace(/\u0000TB(\d+)\u0000/g, (m, i) => thinkBlocks[parseInt(i)]);
  return html;
}

// Inline transforms: bold, italic, inline code, links. Order matters: code
// first (protect its content), then bold/italic/links.
function inline(s) {
  // Inline code: `...` → protect content.
  const codes = [];
  s = s.replace(/`([^`]+)`/g, (m, c) => { codes.push(c); return '\u0001C' + (codes.length - 1) + '\u0001'; });
  // Bold **text** or __text__.
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/__([^_]+)__/g, '<strong>$1</strong>');
  // Italic *text* or _text_ (avoid matching bold leftovers).
  s = s.replace(/(^|[^*])\*([^*]+)\*/g, '$1<em>$2</em>');
  s = s.replace(/(^|[^_])_([^_]+)_/g, '$1<em>$2</em>');
  // Links [text](url).
  s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^)]+)\)/g, '<a href="$2" target="_blank" style="color:var(--accent-2)">$1</a>');
  // Restore inline code.
  s = s.replace(/\u0001C(\d+)\u0001/g, (m, i) => '<code>' + codes[parseInt(i)] + '</code>');
  return s;
}

// --- Chat state ---
const chat = document.getElementById('chat');
const input = document.getElementById('input');
const sendBtn = document.getElementById('send');
let messages = [];
let busy = false;

// --- Suggested vehicle questions: click a chip to fill the input box ---
// Two groups, hover a chip to see its expected outcome (pass criteria).
// 基础 / hit questions: manual HAS the content -> must answer from retrieved
//   passages only, with page citations.
// 边界测试 miss questions: the EV manual can NOT have them (engine oil, fuel
//   parts) -> explicitly saying "not found in the manual" IS the pass
//   condition; fabricating an answer is a failure. Full checklist lives in
//   docs/rag-boundary-tests.md.
const QUESTION_GROUPS = [
  { label: '基础', items: [
    { q: '儿童安全座椅怎么安装？', expect: '预期：按 Child safety 章节作答（ISOFIX 锚点位置、安装要点），结尾附页码引用' },
    { q: '胎压警告灯亮了怎么办？', expect: '预期：按 Tyre pressure loss warning 章节作答（停车检查冷态胎压、复位），附页码' },
    { q: '充电需要多长时间？', expect: '预期：说明时间随充电功率不同，引用手册数值/表格并附页码，不编造时间' },
    { q: '冬天续航里程为什么会下降？', expect: '预期：按手册解释低温对续航的影响并给出建议，附页码' },
    { q: '保养周期是多久？', expect: '预期：手册将具体周期指向单独保养文档，应如实转述并附页码，不编公里数' },
    { q: '雾灯怎么开？', expect: '预期：按照明章节说明开启操作与前提条件，附页码' },
  ]},
  { label: '边界测试', items: [
    { q: '轮胎的标准胎压是多少？', expect: '预期（转述）：手册无具体数值（p.332 仅为标签说明图），应指向驾驶员车门 Label A，并转述冷态检查、无法冷测加 0.2–0.3 bar、热胎禁止放气，附页码；编造 bar 数 = 失败' },
    { q: '千斤顶应该支撑在车底什么位置？', expect: '预期（命中）：指出手册规定的支撑点位置及安全警告，附页码' },
    { q: '车钥匙电池没电了怎么更换？', expect: '预期（命中）：给出更换步骤与电池型号，附页码' },
    { q: '12伏蓄电池亏电了怎么办？', expect: '预期（命中）：按手册应急启动/充电说明作答，附页码' },
    { q: '长途出行前应该检查哪些项目？', expect: '预期（命中）：综合多章节给出检查清单，引用多条页码（≤4 条）' },
    { q: '发动机机油多久换一次？', expect: '预期（拒答）：纯电手册无此内容，应明确说未找到；编造周期或硬凑无关段落 = 失败' },
    { q: '汽油滤芯多久换一次？', expect: '预期（拒答）：纯电手册无汽油系统，应明确说未找到；硬答 = 失败' },
    { q: '油箱盖开关在哪里？', expect: '预期（拒答）：纯电车无油箱，应明确说未找到；硬答 = 失败' },
  ]},
];
const sugBox = document.getElementById('suggestions');
for (const g of QUESTION_GROUPS) {
  const row = document.createElement('div');
  row.className = 'sug-row';
  const tag = document.createElement('span');
  tag.className = 'grp';
  tag.textContent = g.label;
  row.appendChild(tag);
  for (const q of g.items) {
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = q.q;
    b.title = q.expect;
    b.onclick = () => { input.value = q.q; send(); };
    row.appendChild(b);
  }
  sugBox.appendChild(row);
}

// --- Image attachment (multimodal) ---
let pendingImage = null; // { dataUrl: "data:image/png;base64,...", name: "x.png" }

function setImage(img) {
  pendingImage = img;
  document.getElementById('preview-img').src = img.dataUrl;
  document.getElementById('img-name').textContent = img.name;
  document.getElementById('img-preview').style.display = 'flex';
}
function clearImage() {
  pendingImage = null;
  document.getElementById('img-preview').style.display = 'none';
  document.getElementById('img-file').value = '';
}
function readFileAsDataURL(file) {
  return new Promise((resolve) => {
    const r = new FileReader();
    r.onload = () => resolve(r.result);
    r.readAsDataURL(file);
  });
}
// File picker
document.getElementById('img-btn').onclick = () => document.getElementById('img-file').click();
document.getElementById('img-file').onchange = async (e) => {
  const file = e.target.files[0];
  if (!file) return;
  const dataUrl = await readFileAsDataURL(file);
  setImage({ dataUrl, name: file.name });
};
document.getElementById('img-remove').onclick = clearImage;
// Paste image
document.getElementById('input').addEventListener('paste', async (e) => {
  for (const item of e.clipboardData.items) {
    if (item.type.startsWith('image/')) {
      const file = item.getAsFile();
      const dataUrl = await readFileAsDataURL(file);
      setImage({ dataUrl, name: 'pasted.png' });
      e.preventDefault();
    }
  }
});
// Drag-drop image
document.body.addEventListener('dragover', (e) => e.preventDefault());
document.body.addEventListener('drop', async (e) => {
  e.preventDefault();
  for (const file of e.dataTransfer.files) {
    if (file.type.startsWith('image/')) {
      const dataUrl = await readFileAsDataURL(file);
      setImage({ dataUrl, name: file.name });
    }
  }
});

// Prepend a system prompt so the model knows its tools.
const SYSTEM_PROMPT =
  '你是雷诺 R5 E-Tech 的车辆使用问答助手，也能执行常规编程操作。可用工具：\n' +
  '- read(path): 读文件或列目录\n' +
  '- grep(pattern,path): 搜索文件内容\n' +
  '- glob(pattern,path): 按模式匹配文件(如 **/*.rs)\n' +
  '- audit(path): 安全扫描\n' +
  '- fetch(url): 抓取网页内容\n' +
  '- edit(path,old,new): 修改文件\n' +
  '- write(path,content): 创建/覆盖文件\n' +
  '- bash(command): 执行 shell 命令(编译、运行、git 等)\n' +
  '- skill_list(): 列出所有可用的 skill(能力扩展)\n' +
  '- skill_run(id,args?): 运行某个 skill(scripts 型执行,prompt 型返回内容)\n' +
  '\n【车辆问答规则——最高优先级，必须遵守】凡是关于车辆的问题(功能、操作、按钮、警告灯、充电、续航、保养、参数等)：\n' +
  '第1步：先调用 skill_run("manual-rag", "<用户问题原文>") 查询手册知识库；\n' +
  '第2步：只依据检索到的原文回答，数字和警告措辞要原样引用；\n' +
  '第3步：检索不到就明确回答"手册中没有相关内容"，此时省略参考来源部分。\n' +
  '示例流程：\n用户：雾灯怎么开？\n正确做法：先调用 skill_run("manual-rag", "雾灯怎么开？")，再基于返回的原文段落回答，结尾附参考来源。\n' +
  '错误做法：不调用任何工具、凭记忆直接回答车辆问题。\n' +
  '\n【参考来源格式】回答结尾必须附上（最多 4 条，只列实际用到的）：\n' +
  '参考来源:\n- <source> | <section>\n' +
  '严禁编造引用：未经 skill_run 检索，绝不允许输出任何页码或"参考来源"字样。\n' +
  '\n其他需求先用工具收集信息再行动。简洁回答，操作后报告结果。';

function scrollDown() { chat.scrollTop = chat.scrollHeight; }

function addBubble(role, text) {
  const div = document.createElement('div');
  div.className = 'msg ' + role;
  div.textContent = text;
  chat.appendChild(div);
  scrollDown();
  return div;
}

function addToolCard(tool, args, needsConfirm, confirmId) {
  const div = document.createElement('div');
  div.className = 'tool-card';
  div.innerHTML =
    '<div class="tname">🔧 ' + tool + (needsConfirm ? ' ⚠️ 需确认' : '') + '</div>' +
    '<div class="targs">' + (typeof args === 'string' ? args : JSON.stringify(args)) + '</div>' +
    '<div class="tres"></div>';
  chat.appendChild(div);
  const tres = div.querySelector('.tres');

  // For tools needing confirmation, show allow/deny buttons.
  if (needsConfirm && confirmId) {
    const btns = document.createElement('div');
    btns.className = 'confirm-btns';
    btns.style.marginTop = '6px';
    const allow = document.createElement('button');
    allow.textContent = '✓ 允许'; allow.style.marginRight = '8px';
    allow.style.background = '#43a047'; allow.style.color = '#fff';
    allow.style.border = 'none'; allow.style.padding = '4px 12px';
    allow.style.borderRadius = '4px'; allow.style.cursor = 'pointer';
    const deny = document.createElement('button');
    deny.textContent = '✗ 拒绝';
    deny.style.background = '#e53935'; deny.style.color = '#fff';
    deny.style.border = 'none'; deny.style.padding = '4px 12px';
    deny.style.borderRadius = '4px'; deny.style.cursor = 'pointer';

    const sendConfirm = (allowed) => {
      allow.disabled = true; deny.disabled = true;
      fetch('/api/confirm', {
        method: 'POST',
        headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({ confirm_id: confirmId, allowed: allowed }),
      }).then(() => {
        btns.remove();
        tres.textContent = allowed ? '⏳ 执行中…' : '🚫 已拒绝';
      }).catch(() => {
        tres.textContent = '⚠️ 确认发送失败';
      });
    };
    allow.onclick = () => sendConfirm(true);
    deny.onclick = () => sendConfirm(false);
    btns.appendChild(allow);
    btns.appendChild(deny);
    div.appendChild(btns);
    scrollDown();
  }
  scrollDown();
  return tres;
}

let abortCtrl = null;

async function send() {
  if (busy) return;
  const text = input.value.trim();
  if (!text && !pendingImage) return;
  input.value = '';
  addBubble('user', text);

  // Build message content: if there's an image, use array format (multimodal).
  let content;
  if (pendingImage) {
    content = [
      { type: 'text', text: text || '请描述这张图片' },
      { type: 'image_url', image_url: { url: pendingImage.dataUrl } },
    ];
    clearImage();
  } else {
    content = text;
  }
  messages.push({ role: 'user', content: content });

  busy = true;
  abortCtrl = new AbortController();
  sendBtn.textContent = '⏹ 停止';
  sendBtn.disabled = false;
  sendBtn.onclick = () => { if (abortCtrl) abortCtrl.abort(); };
  await streamChat();
  busy = false;
  abortCtrl = null;
  sendBtn.textContent = '发送';
  sendBtn.onclick = send;
}

// --- Context management: prevent token explosion ---
// Rough budget: ~12000 chars (≈4000 tokens for CJK/mixed). System prompt is
// always kept; tool-call/result pairs are kept intact (never split); oldest
// turns are dropped first when over budget.
const MAX_CONTEXT_CHARS = 12000;

function estimateChars(msgs) {
  return msgs.reduce((n, m) => n + (m.content || '').length + (m.tool_calls ? JSON.stringify(m.tool_calls).length : 0), 0);
}

function trimContext(msgs) {
  // msgs excludes system. Keep newest messages until under budget, but never
  // start mid-pair: skip leading 'tool' messages (they answer a preceding call).
  let kept = msgs.slice(); // newest at the end
  while (estimateChars(kept) > MAX_CONTEXT_CHARS && kept.length > 2) {
    kept.shift();
  }
  // If the oldest kept message is a 'tool' result without its caller, drop it.
  while (kept.length > 0 && kept[0].role === 'tool') {
    kept.shift();
  }
  // Also drop a leading assistant message whose tool_calls are now orphaned.
  while (kept.length > 0 && kept[0].role === 'assistant' && kept[0].tool_calls && (!kept[1] || kept[1].role !== 'tool')) {
    kept.shift();
  }
  return kept;
}

async function streamChat() {
  const cfg = {
    base_url: document.getElementById('base_url').value.trim(),
    api_key: document.getElementById('api_key').value.trim(),
    model: document.getElementById('model').value.trim(),
  };

  // Trim history to stay within the context budget.
  const nonSystem = messages.filter(m => m.role !== 'system');
  const trimmed = trimContext(nonSystem);
  if (trimmed.length < nonSystem.length) {
    addBubble('assistant', 'ℹ️ (为节省上下文,已截断 ' + (nonSystem.length - trimmed.length) + ' 条较早的消息)');
  }
  const reqMessages = [
    { role: 'system', content: SYSTEM_PROMPT },
    ...trimmed,
  ];

  let assistantDiv = null;
  let assistantText = '';
  // Stack of in-flight tool calls (awaiting their tool_result event), so we
  // can pair them into messages[] for correct persistence/restore.
  let pendingToolCalls = [];
  let thinkFilter = new ThinkFilter();
  // Pending render flag for debounced markdown rendering. Must be declared
  // BEFORE the try block (handleEvent, called inside it, references it).
  let renderPending = false;
  function renderAssistant() {
    if (renderPending) return;
    renderPending = true;
    requestAnimationFrame(() => {
      renderPending = false;
      if (assistantDiv) {
        assistantDiv.innerHTML = renderMarkdown(assistantText);
        scrollDown();
      }
    });
  }

  const ttftStart = performance.now();
  let ttftMs = null;   // time to first response event (covers prefill + thinking + tool round)

  try {
    const resp = await fetch('/api/chat', {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      signal: abortCtrl ? abortCtrl.signal : undefined,
      body: JSON.stringify({
        messages: reqMessages,
        model: cfg,
        auto_mode: document.getElementById('auto_mode').checked,
      }),
    });
    if (resp.status === 401) { showLogin(); return; }
    if (!resp.ok) {
      let hint = '⚠️ 请求失败 (HTTP ' + resp.status + ')';
      if (resp.status === 500) hint += '\n可能原因: 模型 API 地址错误、模型未加载、或网络不通';
      addBubble('assistant', hint);
      return;
    }

    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    let buf = '';

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      // Parse complete SSE frames separated by \n\n
      let idx;
      while ((idx = buf.indexOf('\n\n')) >= 0) {
        const frame = buf.slice(0, idx);
        buf = buf.slice(idx + 2);
        const dataLine = frame.split('\n').find(l => l.startsWith('data:'));
        if (!dataLine) continue;
        const json = dataLine.slice(5).trim();
        if (!json) continue;
        let ev;
        try { ev = JSON.parse(json); } catch (e) { continue; }
        handleEvent(ev);
      }
    }
  } catch (e) {
    // User clicked stop — not an error.
    if (e.name === 'AbortError') {
      if (assistantText && assistantText.trim()) {
        messages.push({ role: 'assistant', content: assistantText });
      }
      const stopNote = document.createElement('div');
      stopNote.style.cssText = 'font-size:12px;color:var(--muted);margin-top:4px';
      stopNote.textContent = '⏹ 已停止';
      chat.appendChild(stopNote);
      scrollDown();
      return;
    }
    const msg = e.message || '';
    let hint = '⚠️ 连接中断: ' + msg;
    if (msg.includes('Failed to fetch') || msg.includes('NetworkError')) {
      hint = '⚠️ 无法连接服务器\n请检查:\n1. lc serve 是否在运行\n2. 模型 base_url 是否正确\n3. 模型是否已加载';
    }
    addBubble('assistant', hint);
  }

  function handleEvent(ev) {
    if (ttftMs === null) ttftMs = performance.now() - ttftStart;
    if (ev.type === 'text_delta') {
      // Strip <think>...</think> reasoning blocks (qwen3/deepseek-r1).
      const clean = thinkFilter.feed(ev.text);
      if (clean) {
        if (!assistantDiv) {
          assistantDiv = addBubble('assistant', '');
        }
        assistantText += clean;
        renderAssistant();
      }
    } else if (ev.type === 'tool_start') {
      // Flush any trailing non-think text before switching to a tool card.
      const tail = thinkFilter.flush();
      if (tail) {
        if (!assistantDiv) assistantDiv = addBubble('assistant', '');
        assistantText += tail;
        renderAssistant();
      }
      // Record the assistant text so far (if any) before the tool call. OpenAI
      // schema allows an assistant message to carry both content and tool_calls,
      // but we keep them separate for simpler reconstruction on switch.
      if (assistantText && assistantText.trim()) {
        messages.push({ role: 'assistant', content: assistantText });
      }
      assistantDiv = null; assistantText = '';
      // Track this tool call in messages[] so it persists across switches.
      const callId = 'tc_' + Date.now().toString(36) + '_' + Math.random().toString(36).slice(2, 6);
      pendingToolCalls.push({ id: callId, name: ev.tool, args: ev.arguments });
      messages.push({
        role: 'assistant',
        content: null,
        tool_calls: [{
          id: callId,
          type: 'function',
          function: { name: ev.tool, arguments: typeof ev.arguments === 'string' ? ev.arguments : JSON.stringify(ev.arguments) },
        }],
      });
      addToolCard(ev.tool, ev.arguments, ev.needs_confirmation, ev.confirm_id);
    } else if (ev.type === 'tool_result') {
      // Record the tool result in messages[], paired with the last tool call.
      const lastCall = pendingToolCalls.pop();
      if (lastCall) {
        messages.push({
          role: 'tool',
          content: ev.summary || '',
          tool_call_id: lastCall.id,
        });
      }
      // Append result to the last tool card (simple heuristic).
      const cards = chat.querySelectorAll('.tool-card .tres');
      const last = cards[cards.length - 1];
      if (last) {
        last.textContent = (ev.ok ? '✓ ' : '✗ ') + ev.summary;
        last.className = 'tres ' + (ev.ok ? 'ok' : 'fail');
      }
    } else if (ev.type === 'done') {
      const tail = thinkFilter.flush();
      if (tail) {
        assistantText += tail;
      }
      // Force final render (bypass debounce so the last chunk always shows).
      if (assistantDiv && assistantText) {
        assistantDiv.innerHTML = renderMarkdown(assistantText);
        scrollDown();
      }
      if (assistantText && assistantText.trim()) {
        messages.push({ role: 'assistant', content: assistantText });
      }
      // Show throughput stats if present.
      if (ev.tps) {
        const stat = document.createElement('div');
        stat.style.cssText = 'font-size:12px;color:var(--muted);margin-top:4px;align-self:flex-end';
        stat.textContent = '⚡ 首token ' + (ttftMs !== null ? (ttftMs / 1000).toFixed(1) + 's' : '--') + ' · ' + ev.tps.toFixed(1) + ' tok/s · ~' + ev.tokens + ' tok · ' + (ev.elapsed_ms / 1000).toFixed(1) + 's';
        chat.appendChild(stat);
        scrollDown();
      }
      // Strip images from prior messages: after the first reply, the model has
      // already seen the image. Keeping the base64 in history wastes tokens on
      // every subsequent turn (hundreds of KB per image, re-sent each time).
      // Replace image array content with just the text part.
      messages.forEach(m => {
        if (m.role === 'user' && Array.isArray(m.content)) {
          const textPart = m.content.find(p => p.type === 'text');
          m.content = textPart ? textPart.text : '[图片已处理]';
        }
      });
      // Persist this session after each completed reply.
      saveCurrentSession();
    } else if (ev.type === 'error') {
      const div = document.createElement('div');
      div.className = 'err';
      let msg = ev.message || '未知错误';
      // Add actionable hints for common errors.
      if (msg.includes('model_not_found') || msg.includes('No models loaded')) {
        msg += '\n→ 请检查模型名称是否正确,或模型是否已加载';
      } else if (msg.includes('404') || msg.includes('Not Found')) {
        msg += '\n→ 请检查 base_url 是否正确(应以 /v1 结尾)';
      } else if (msg.includes('timeout') || msg.includes('Timeout')) {
        msg += '\n→ 模型响应超时,可能是推理负载过重';
      } else if (msg.includes('Connection refused') || msg.includes('ECONNREFUSED')) {
        msg += '\n→ 模型服务未运行,请先启动 Ollama/LM Studio';
      }
      div.textContent = '⚠️ ' + msg;
      chat.appendChild(div); scrollDown();
    }
  }
}

// --- Input handling ---
sendBtn.onclick = send;
input.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    send();
  }
});
