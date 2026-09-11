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

// --- i18n: 中英双语切换 ---
// 语言存在 localStorage；界面文案、建议问题、系统提示词全部联动：
// 英文模式下提示词强制模型用英文回答，中文模式下用中文回答。
const LANG_KEY = 'liteclaw_lang';
let LANG = localStorage.getItem(LANG_KEY) === 'en' ? 'en' : 'zh';

const I18N = {
  zh: {
    title: '车书助手 🚗 v2',
    appName: '🚗 车书助手',
    newSessionShort: '新会话',
    rename: '重命名',
    commonQ: '常见问题',
    sugPlaceholder: '选择问题，点击即发送…',
    selectModel: '选择模型',
    noThink: '禁用思考',
    noThinkTip: '思考开关:勾选后对所有模型发送禁思考指令。会思考的模型(MiniCPM5-2B、Qwen3-30B-A3B 默认思考)将直接回答,更快更省 token;不会思考的模型无变化。',
    autoRag: '强制RAG检索',
    autoRagTip: '开启 = 每轮提问强制先触发一轮 RAG 检索:服务器代查手册原文并注入给模型,回答只依据检索内容,更稳定(检索过程界面不可见)。关闭 = 模型自行决定是否调 skill 检索(界面能看到工具卡片),小模型容易漏检或编造参数,不建议关闭。默认:开启。',
    save: '保存',
    saved: '✓ 已保存(本浏览器)',
    saveFail: '✗ 保存失败',
    helpTip: '怎么提问、能问什么：打开 RAG 问题地图',
    langBtn: 'EN',
    langTip: '切换到英文：界面、建议问题与回答都改为 English',
    inputPlaceholder: '说点什么… (Enter 发送, Shift+Enter 换行, 可粘贴/拖拽图片)',
    send: '发送',
    stop: '⏹ 停止',
    remove: '✕ 移除',
    login: '登录',
    loginUser: '用户名',
    loginPass: '密码',
    loginErr: '用户名或密码错误',
    needConfirm: ' ⚠️ 需确认',
    allow: '✓ 允许',
    deny: '✗ 拒绝',
    executing: '⏳ 执行中…',
    denied: '🚫 已拒绝',
    confirmFail: '⚠️ 确认发送失败',
    chars: ' 字符',
    kChars: 'K 字符',
    result: ' 结果 ',
    clickTo: ' · 点击',
    expand: '展开',
    collapse: '收起',
    thinking: '💭 思考过程',
    truncated: 'ℹ️ (为节省上下文,已截断 {n} 条较早的消息)',
    stopped: '⏹ 已停止',
    connLost: '⚠️ 连接中断: ',
    connFailHint: '⚠️ 无法连接服务器\n请检查:\n1. lc serve 是否在运行\n2. 模型 base_url 是否正确\n3. 模型是否已加载',
    reqFail: '⚠️ 请求失败 (HTTP {code})',
    reqFailCause: '\n可能原因: 模型 API 地址错误、模型未加载、或网络不通',
    ttft: '⚡ 首token ',
    describeImage: '请描述这张图片',
    imgHandled: '[图片已处理]',
    unknownErr: '未知错误',
    errModel: '\n→ 请检查模型名称是否正确,或模型是否已加载',
    errUrl: '\n→ 请检查 base_url 是否正确(应以 /v1 结尾)',
    errTimeout: '\n→ 模型响应超时,可能是推理负载过重',
    errConn: '\n→ 模型服务未运行,请先启动 Ollama/LM Studio',
    connectFail: '连接失败',
    sessionExpired: '⏰ 会话已过期,请重新登录',
  },
  en: {
    title: 'Car Manual Assistant 🚗 v2',
    appName: '🚗 Car Manual Assistant',
    newSessionShort: 'New chat',
    rename: 'Rename',
    commonQ: 'Common questions',
    sugPlaceholder: 'Pick a question, click to send…',
    selectModel: 'Select model',
    noThink: 'No thinking',
    noThinkTip: 'Thinking toggle: sends a no-think directive to all models. Thinking models (MiniCPM5-2B, Qwen3-30B-A3B by default) answer directly — faster, fewer tokens; non-thinking models are unaffected.',
    autoRag: 'Force RAG',
    autoRagTip: 'On = every question triggers one forced RAG retrieval: the server queries the manual and injects the passages, so answers stay grounded (retrieval invisible in the UI). Off = the model decides whether to call the retrieval skill (tool cards visible); small models then tend to skip retrieval or invent parameters — keep it on.',
    save: 'Save',
    saved: '✓ Saved (this browser)',
    saveFail: '✗ Save failed',
    helpTip: 'What to ask and how: open the RAG question map',
    langBtn: '中文',
    langTip: 'Switch to Chinese: UI, suggested questions and answers all in Chinese',
    inputPlaceholder: 'Ask something… (Enter to send, Shift+Enter for newline, paste/drag images)',
    send: 'Send',
    stop: '⏹ Stop',
    remove: '✕ Remove',
    login: 'Sign in',
    loginUser: 'Username',
    loginPass: 'Password',
    loginErr: 'Wrong username or password',
    needConfirm: ' ⚠️ needs confirm',
    allow: '✓ Allow',
    deny: '✗ Deny',
    executing: '⏳ Running…',
    denied: '🚫 Denied',
    confirmFail: '⚠️ Failed to send confirmation',
    chars: ' chars',
    kChars: 'K chars',
    result: ' result ',
    clickTo: ' · click to ',
    expand: 'expand',
    collapse: 'collapse',
    thinking: '💭 Thinking',
    truncated: 'ℹ️ ({n} earlier messages trimmed to save context)',
    stopped: '⏹ Stopped',
    connLost: '⚠️ Connection lost: ',
    connFailHint: '⚠️ Cannot reach the server\nCheck:\n1. Is lc serve running?\n2. Is the model base_url correct?\n3. Is the model loaded?',
    reqFail: '⚠️ Request failed (HTTP {code})',
    reqFailCause: '\nPossible causes: wrong model API address, model not loaded, or network down',
    ttft: '⚡ TTFT ',
    describeImage: 'Please describe this image',
    imgHandled: '[image processed]',
    unknownErr: 'Unknown error',
    errModel: '\n→ Check the model name is correct, or that the model is loaded',
    errUrl: '\n→ Check base_url is correct (should end with /v1)',
    errTimeout: '\n→ Model response timed out; the inference backend may be overloaded',
    errConn: '\n→ Model service not running; start Ollama/LM Studio first',
    connectFail: 'Connect failed',
    sessionExpired: '⏰ Session expired, please sign in again',
  },
};
function t(key) {
  const d = I18N[LANG] || I18N.zh;
  return d[key] !== undefined ? d[key] : (I18N.zh[key] !== undefined ? I18N.zh[key] : key);
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

// --- Session history: per-browser (localStorage), not shared server state ---
// Same rationale as the config store: several people use this deployment and
// chat history should be private to each browser. The server-side
// ~/.liteclaw/history.json is only used once to seed a first-time browser.
const LC_HIST_KEY = 'liteclaw_history';
const LC_HIST_MIGRATED = 'liteclaw_history_migrated';
const MAX_SESSIONS = 30;

function histStore() {
  try { return JSON.parse(localStorage.getItem(LC_HIST_KEY)) || []; }
  catch (e) { return []; }
}
function histSave(arr) {
  // LRU cap + quota guard: on localStorage overflow, shed the oldest half
  // and retry until it fits.
  let arr2 = arr.slice(0, MAX_SESSIONS);
  while (arr2.length) {
    try {
      localStorage.setItem(LC_HIST_KEY, JSON.stringify(arr2));
      return;
    } catch (e) {
      arr2 = arr2.slice(Math.ceil(arr2.length / 2));
    }
  }
  try { localStorage.removeItem(LC_HIST_KEY); } catch (e) {}
}
function histUpsert(session) {
  const arr = histStore();
  const i = arr.findIndex(s => s.id === session.id);
  if (i >= 0) arr[i] = session; else arr.unshift(session);
  histSave(arr);
}
function histGet(id) {
  return histStore().find(s => s.id === id) || null;
}
function histDelete(id) {
  histSave(histStore().filter(s => s.id !== id));
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
  return t('newSessionShort');
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
    histUpsert(session);
  } catch (e) { console.error('save session:', e); }
  renderSidebar();
}

async function loadSessionList() {
  // First visit on this browser: import the existing server-side history once,
  // then diverge — every browser keeps its own private session list.
  if (!localStorage.getItem(LC_HIST_KEY) && !localStorage.getItem(LC_HIST_MIGRATED)) {
    try {
      const resp = await fetch('/api/history', { headers: authHeaders() });
      if (resp.status === 401) { showLogin(); return []; }
      if (resp.ok) histSave((await resp.json()).sessions || []);
    } catch (e) {}
    try { localStorage.setItem(LC_HIST_MIGRATED, '1'); } catch (e) {}
  }
  return histStore();
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
      id: currentSessionId, title: '✦ ' + t('newSessionShort'), updated: Date.now(),
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
  editBtn.title = t('rename');
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
    // Persist: upsert the session with the new title, in place.
    try {
      const session = histGet(id);
      if (!session) return;
      session.title = newTitle;
      histUpsert(session);
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
  // 2. Load the target from this browser's store.
  const session = histGet(id);
  if (!session) return;
  // 3. Swap state atomically.
  currentSessionId = session.id;
  isNewSession = false;
  messages = session.messages || [];
  // 4. Repaint the chat area from the full message list.
  renderChatFromMessages();
  renderSidebar();
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
          if (result && tres && tres.setResult) {
            tres.setResult(true, contentText(result));
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
    histDelete(id);
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
    err.textContent = t('connectFail');
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
      note.textContent = t('sessionExpired');
      document.body.appendChild(note);
      setTimeout(() => note.remove(), 3000);
    }
  } catch (e) {}
}, 300000); // 5 minutes

// --- Config persistence: per-browser (localStorage), not shared server state ---
// config.json via /api/config only seeds a first-time browser. Each user's
// saved model/checkboxes live in their own browser, so user A saving a model
// never changes what user B sees. Model endpoints and keys are a fixed
// deployment value, kept out of the UI on purpose.
const LC_CFG_KEY = 'liteclaw_cfg';
const DEFAULT_BASE_URL = 'http://172.21.0.1:11434/v1';
// Gateway-hosted models: base_url/api_key resolve server-side from
// config.json's model_endpoints map — credentials never live in the
// frontend. This list decides the protocol and the 禁思考 parameter:
// gateway → /v1 + enable_thinking via extra_body; every other model is
// local Ollama → NATIVE /api/chat + think:false + per-request num_ctx.
const GATEWAY_MODELS = ['qwen3.8-flash', 'deepseek-flash'];
function fillCfg(c) {
  // Respect the saved model; fall back to the fast default on first visit or
  // when the saved model is no longer in the dropdown.
  const wanted = c.model || 'openbmb/minicpm5-2b:latest';
  const sel = document.getElementById('model');
  sel.value = [...sel.options].some(o => o.value === wanted) ? wanted : 'openbmb/minicpm5-2b:latest';
  document.getElementById('no_think').checked = !!c.no_think;
  // 自动检索默认开启: MiniCPM5-2B 自主调工具不可靠(幻觉路径/参数格式错),
  // 服务端注入检索才是稳定路径。显式保存过 false 的老配置予以尊重。
  document.getElementById('auto_rag').checked = c.auto_rag === undefined ? true : !!c.auto_rag;
}
async function loadCfg() {
  // Per-browser saved config wins; the server config only bootstraps a
  // first visit on a new browser.
  try {
    const raw = localStorage.getItem(LC_CFG_KEY);
    if (raw) { fillCfg(JSON.parse(raw)); return; }
  } catch (e) {}
  try {
    const resp = await fetch('/api/config', { headers: authHeaders() });
    if (resp.status === 401) { showLogin(); return; }
    if (resp.ok) {
      fillCfg(await resp.json());
      return;
    }
  } catch (e) {}
  // Fallback to defaults if backend unreachable.
  fillCfg({});
}
async function saveCfg() {
  const c = {
    model: document.getElementById('model').value.trim(),
    no_think: document.getElementById('no_think').checked,
    auto_rag: document.getElementById('auto_rag').checked,
  };
  const btn = document.getElementById('save_cfg');
  const orig = btn.textContent;
  try {
    localStorage.setItem(LC_CFG_KEY, JSON.stringify(c));
    btn.textContent = t('saved');
  } catch (e) {
    btn.textContent = t('saveFail');
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
    thinkBlocks.push('<details style="margin:6px 0;border:1px solid var(--border);border-radius:6px;padding:4px 10px"><summary style="cursor:pointer;color:var(--muted);font-size:13px">' + t('thinking') + '</summary><div style="margin-top:6px;color:var(--muted);font-size:13px;white-space:pre-wrap">' + content + '</div></details>');
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
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    // Code-block placeholder on its own line.
    const cbMatch = line.match(/^\u0000CB(\d+)\u0000$/);
    if (cbMatch) { closeList(); closeBq(); html += codeBlocks[parseInt(cbMatch[1])]; continue; }
    // Think-block placeholder on its own line.
    const tbMatch = line.match(/^\u0000TB(\d+)\u0000$/);
    if (tbMatch) { closeList(); closeBq(); html += thinkBlocks[parseInt(tbMatch[1])]; continue; }
    // Headings.
    const h = line.match(/^(#{1,3})\s+(.*)/);
    if (h) { closeList(); closeBq(); html += '<h' + h[1].length + '>' + inline(h[2]) + '</h' + h[1].length + '>'; continue; }
    // GFM pipe table: a | row followed by a delimiter row (|---|:--:|).
    // Consumes all consecutive | rows so cells never leak into paragraphs.
    if (/^\s*\|/.test(line) && i + 1 < lines.length && isDelimRow(lines[i + 1])) {
      closeList(); closeBq();
      const aligns = splitRow(lines[i + 1]).map(c =>
        /^:-+:$/.test(c) ? 'center' : /-+:$/.test(c) ? 'right' : /^:-+/.test(c) ? 'left' : '');
      const cellTag = (c, tag, j) => {
        const a = aligns[j] ? ' style="text-align:' + aligns[j] + '"' : '';
        return '<' + tag + a + '>' + inline(c) + '</' + tag + '>';
      };
      html += '<table><thead><tr>' +
        splitRow(line).map((c, j) => cellTag(c, 'th', j)).join('') + '</tr></thead><tbody>';
      i += 2;
      while (i < lines.length && /^\s*\|/.test(lines[i])) {
        html += '<tr>' + splitRow(lines[i]).map((c, j) => cellTag(c, 'td', j)).join('') + '</tr>';
        i++;
      }
      html += '</tbody></table>';
      i--; // compensate for the for-loop's i++ landing past the table
      continue;
    }
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

// GFM table helpers: the delimiter row (|---|:--:|-—:|) marks a table; rows
// split on unescaped pipes. \| inside a cell is a literal pipe.
function isDelimRow(line) {
  return /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/.test(line);
}
function splitRow(line) {
  return line.trim().replace(/^\|/, '').replace(/\|\s*$/, '')
    .replace(/\\\|/g, '\u0002').split('|').map(c => c.trim().replace(/\u0002/g, '|'));
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
  { label: '常见问题', label_en: 'Common', items: [
    { q: '车辆如何启动？仪表上出现 READY 代表什么？', expect: '预期：启动步骤与 READY 指示含义，引用 driving 章节，附页码',
      q_en: 'How do I start the vehicle? What does READY on the cluster mean?', expect_en: 'Expect: starting steps and the READY indicator meaning, citing the Driving chapter, with page numbers' },
    { q: 'R5 E-Tech 支持哪些充电方式？如何开始和结束充电？', expect: '预期：按 EV 章节说明 AC/DC 充电方式与插拔流程，附页码',
      q_en: 'What charging methods does the R5 E-Tech support? How do I start and stop charging?', expect_en: 'Expect: AC/DC charging modes and the plug-in/out procedure per the EV chapter, with page numbers' },
    { q: 'D 挡和 B 挡有什么区别？如何切换？', expect: '预期：按 Gear control 章节说明 D/B 挡差异与能量回收关联，附页码',
      q_en: 'What is the difference between D and B gears? How do I switch?', expect_en: 'Expect: D/B differences and their regen linkage per the Gear control chapter, with page numbers' },
    { q: '仪表上的警告灯是什么意思？', expect: '预期：说明警告灯分类并指向 Warning lights 章节，附页码',
      q_en: 'What do the warning lights on the instrument cluster mean?', expect_en: 'Expect: warning-light categories, pointing to the Warning lights chapter, with page numbers' },
    { q: 'V2L 对外放电如何使用？', expect: '预期：按 V2L 章节说明用法与限制，附页码',
      q_en: 'How do I use the V2L external power supply?', expect_en: 'Expect: V2L usage and limits per the V2L chapter, with page numbers' },
  ]},
  { label: '基础', label_en: 'Basics', items: [
    { q: '儿童安全座椅怎么安装？', expect: '预期：按 Child safety 章节作答（ISOFIX 锚点位置、安装要点），结尾附页码引用',
      q_en: 'How do I install a child seat?', expect_en: 'Expect: answer per the Child safety chapter (ISOFIX anchor positions, fitting points), pages cited at the end' },
    { q: '胎压警告灯亮了怎么办？', expect: '预期：按 Tyre pressure loss warning 章节作答（停车检查冷态胎压、复位），附页码',
      q_en: 'The tyre pressure warning light is on — what should I do?', expect_en: 'Expect: Tyre pressure loss warning chapter (stop, check cold pressures, reset), with page numbers' },
    { q: '充电需要多长时间？', expect: '预期：说明时间随充电功率不同，引用手册数值/表格并附页码，不编造时间',
      q_en: 'How long does charging take?', expect_en: 'Expect: time varies with charging power; cite manual values/tables with pages, no made-up durations' },
    { q: '冬天续航里程为什么会下降？', expect: '预期：按手册解释低温对续航的影响并给出建议，附页码',
      q_en: 'Why does the range drop in winter?', expect_en: 'Expect: manual-based explanation of cold-weather range loss plus advice, with page numbers' },
    { q: '保养周期是多久？', expect: '预期：手册将具体周期指向单独保养文档，应如实转述并附页码，不编公里数',
      q_en: 'What is the service interval?', expect_en: 'Expect: the manual defers to a separate maintenance document — relay honestly with pages, no invented mileage' },
    { q: '雾灯怎么开？', expect: '预期：按照明章节说明开启操作与前提条件，附页码',
      q_en: 'How do I turn on the fog lights?', expect_en: 'Expect: operation and preconditions per the lighting chapter, with page numbers' },
  ]},
  { label: '边界测试', label_en: 'Boundary tests', items: [
    { q: '轮胎的标准胎压是多少？', expect: '预期（转述）：手册无具体数值（p.332 仅为标签说明图），应指向驾驶员车门 Label A，并转述冷态检查、无法冷测加 0.2–0.3 bar、热胎禁止放气，附页码；编造 bar 数 = 失败',
      q_en: 'What is the standard tyre pressure?', expect_en: 'Expect (paraphrase): no concrete value in the manual (p.332 is only the label legend); point to driver-door Label A and relay the cold-check rule, +0.2–0.3 bar when a cold check is impossible, never deflate hot tyres, with pages; inventing bar values = fail' },
    { q: '千斤顶应该支撑在车底什么位置？', expect: '预期（命中）：指出手册规定的支撑点位置及安全警告，附页码',
      q_en: 'Where under the car should the jack be positioned?', expect_en: 'Expect (hit): the manual-specified jacking points and the safety warning, with page numbers' },
    { q: '车钥匙电池没电了怎么更换？', expect: '预期（命中）：给出更换步骤与电池型号，附页码',
      q_en: 'The key card battery is dead — how do I replace it?', expect_en: 'Expect (hit): replacement steps and the battery type, with page numbers' },
    { q: '12伏蓄电池亏电了怎么办？', expect: '预期（命中）：按手册应急启动/充电说明作答，附页码',
      q_en: 'The 12 V battery is flat — what should I do?', expect_en: 'Expect (hit): the manual jump-start/charging procedure (EV precautions included), with page numbers' },
    { q: '长途出行前应该检查哪些项目？', expect: '预期（命中）：综合多章节给出检查清单，引用多条页码（≤4 条）',
      q_en: 'What should I check before a long trip?', expect_en: 'Expect (hit): a multi-chapter checklist citing several pages (≤4)' },
    { q: '发动机机油多久换一次？', expect: '预期（拒答）：纯电手册无此内容，应明确说未找到；编造周期或硬凑无关段落 = 失败',
      q_en: 'How often should the engine oil be changed?', expect_en: 'Expect (refuse): the EV manual has no such content; explicitly say not found; inventing an interval or forcing unrelated passages = fail' },
    { q: '汽油滤芯多久换一次？', expect: '预期（拒答）：纯电手册无汽油系统，应明确说未找到；硬答 = 失败',
      q_en: 'How often should the fuel filter be replaced?', expect_en: 'Expect (refuse): an EV has no petrol system; explicitly say not found; answering anyway = fail' },
    { q: '油箱盖开关在哪里？', expect: '预期（拒答）：纯电车无油箱，应明确说未找到；硬答 = 失败',
      q_en: 'Where is the fuel filler release?', expect_en: 'Expect (refuse): an EV has no fuel tank; explicitly say not found; answering anyway = fail' },
  ]},
  { label: '驾驶辅助', label_en: 'Driving aids', items: [
    { q: '自适应巡航在堵车时能用吗？', expect: '预期：按 Stop and Go 章节说明跟车/停走功能与激活限制，附页码',
      q_en: 'Can adaptive cruise control be used in traffic jams?', expect_en: 'Expect: Stop and Go chapter — following/stop-and-go capability and activation limits, with page numbers' },
    { q: '车道保持辅助怎么开启？', expect: '预期：按 Active driver assist 章节说明开启方式与工作条件，附页码',
      q_en: 'How do I enable lane keeping assist?', expect_en: 'Expect: Active driver assist chapter — how to enable and operating conditions, with page numbers' },
    { q: '倒车雷达和倒车影像怎么用？', expect: '预期：按 Parking aids 章节说明雷达提示音与影像使用，附页码',
      q_en: 'How do the parking sensors and the reversing camera work?', expect_en: 'Expect: Parking aids chapter — sensor tones and camera use, with page numbers' },
    { q: '自动泊车功能怎么触发？', expect: '预期：按 Parking aids 章节说明触发条件与操作步骤，附页码',
      q_en: 'How do I trigger the automatic parking feature?', expect_en: 'Expect: Parking aids chapter — trigger conditions and steps, with page numbers' },
    { q: '能量回收强度怎么调节？', expect: '预期：按 Regenerative braking 章节说明换挡拨片/模式调节，附页码',
      q_en: 'How do I adjust the regenerative braking level?', expect_en: 'Expect: Regenerative braking chapter — paddle/mode adjustment, with page numbers' },
  ]},
  { label: '车辆功能', label_en: 'Features', items: [
    { q: '车窗起雾怎么快速除雾？', expect: '预期：按空调/除雾章节说明除雾按钮与风量设置，附页码',
      q_en: 'The windows fog up — how do I demist them quickly?', expect_en: 'Expect: HVAC/demisting chapter — the demisting button and airflow settings, with page numbers' },
    { q: '补胎工具包怎么使用？', expect: '预期：按 Tyre repair kit 章节说明打胶步骤与 15 分钟/1.8 bar 判定阈值语境，附页码',
      q_en: 'How do I use the tyre repair kit?', expect_en: 'Expect: Tyre repair kit chapter — sealing steps and the 15 min / 1.8 bar threshold context, with page numbers' },
    { q: '后排童锁怎么设置？', expect: '预期：按 Child safety 章节说明童锁位置与操作，附页码',
      q_en: 'How do I set the rear door child locks?', expect_en: 'Expect: Child safety chapter — lock location and operation, with page numbers' },
    { q: '洗车需要注意什么？', expect: '预期：按 Cleaning 章节说明高压水枪距离与禁止事项，附页码',
      q_en: 'What should I watch out for when washing the car?', expect_en: 'Expect: Cleaning chapter — pressure-washer distance and prohibitions, with page numbers' },
    { q: '紧急呼叫 SOS 是怎么工作的？', expect: '预期：按 Emergency call 章节说明触发方式与工作原理，附页码',
      q_en: 'How does the emergency call (SOS) work?', expect_en: 'Expect: Emergency call chapter — triggering and how it works, with page numbers' },
  ]},
  { label: '扩展边界', label_en: 'Extended boundary', items: [
    { q: '火花塞多久换一次？', expect: '预期（拒答）：纯电车无火花塞，应明确说未找到；硬答 = 失败',
      q_en: 'How often do the spark plugs need replacing?', expect_en: 'Expect (refuse): an EV has no spark plugs; explicitly say not found; answering anyway = fail' },
    { q: '正时皮带多少公里换一次？', expect: '预期（拒答）：纯电车无正时皮带，应明确说未找到；硬答 = 失败',
      q_en: 'After how many km should the timing belt be replaced?', expect_en: 'Expect (refuse): an EV has no timing belt; explicitly say not found; answering anyway = fail' },
    { q: '变速箱油需要更换吗？', expect: '预期（谨慎）：电驱减速器油如手册未提及更换周期应如实说明；编造公里数 = 失败',
      q_en: 'Does the gearbox oil need changing?', expect_en: 'Expect (caution): if the manual gives no interval for the drive-reducer oil, say so honestly; inventing mileage = fail' },
    { q: '电池质保是多少年？', expect: '预期（转述）：手册通常指向单独质保文档，应如实转述不编年限',
      q_en: 'How many years does the battery warranty last?', expect_en: 'Expect (paraphrase): the manual defers to a separate warranty document; relay honestly, no invented years' },
    { q: '百公里加速需要几秒？', expect: '预期（边界）：按手册技术规格如实回答，规格无此数据应明确说明；编造秒数 = 失败',
      q_en: 'What is the 0–100 km/h acceleration time?', expect_en: 'Expect (boundary): answer only from the manual tech specs; if absent, say so explicitly; inventing seconds = fail' },
  ]},
];
// Suggested questions render as ONE dropdown (grouped by optgroup): picks a
// question, sends it, then resets so the same question can be picked again.
// Bilingual: rebuilt by applyLang() on language switch.
const sugSelect = document.getElementById('sug-select');
function renderSuggestions() {
  sugSelect.innerHTML = '';
  const ph = document.createElement('option');
  ph.value = '';
  ph.textContent = t('sugPlaceholder');
  sugSelect.appendChild(ph);
  for (const g of QUESTION_GROUPS) {
    const og = document.createElement('optgroup');
    og.label = LANG === 'en' ? (g.label_en || g.label) : g.label;
    for (const q of g.items) {
      const o = document.createElement('option');
      o.value = LANG === 'en' ? (q.q_en || q.q) : q.q;
      o.textContent = LANG === 'en' ? (q.q_en || q.q) : q.q;
      o.title = LANG === 'en' ? (q.expect_en || q.expect) : q.expect;
      og.appendChild(o);
    }
    sugSelect.appendChild(og);
  }
}
renderSuggestions();
sugSelect.addEventListener('change', () => {
  if (!sugSelect.value) return;
  input.value = sugSelect.value;
  sugSelect.selectedIndex = 0;
  send();
});

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
  '\n【回答风格——你是车主的随车助手，不是知识库查询界面】\n' +
  '- 口吻亲切自然、通俗易懂，像懂车的朋友坐在旁边讲解，少用专业术语；\n' +
  '- 回答务必简短：一般问题控制在 300 字以内，直接给结论和操作步骤，不铺垫、不总结、不说"希望对您有帮助"之类的客套话；\n' +
  '- 不要用表格和多级标题，用短句；复杂操作用简短的编号步骤；安全警告必须保留，不为凑字数删减；\n' +
  '- 绝对不要出现"根据手册""第X页""章节""检索""chunk""知识库"这类词，也不要复述检索过程；\n' +
  '- 直接说操作步骤和注意事项；内容只覆盖部分时，自然回答已覆盖的部分，不要声明"检索未覆盖"。\n' +
  '\n【参考来源格式】回答结尾附上（最多 4 条，只列实际用到的，面向车主的简洁写法）：\n' +
  '参考来源:\n- 用户手册 p.<页码>（<中文主题>）\n' +
  '示例：- 用户手册 p.148（灯光与信号）\n' +
  '严禁编造引用：未经 skill_run 检索，绝不允许输出任何页码或"参考来源"字样；\n' +
  '禁止把文件路径、chunk 编号写进参考来源。\n' +
  '【数值转述纪律】引用任何数值时，必须连同原文的限定条件（所属章节、场景、前提）一起原样转述；' +
  '不得把特定场景的数值泛化为通用参数（例如补胎流程中的压力阈值不是标准胎压）；' +
  '凡手册写明以车门标签(Label A)为准的参数，必须提示用户查看车门标签。\n' +
  '\n其他需求先用工具收集信息再行动。简洁回答，操作后报告结果。';

// SYSTEM_PROMPT 的英文版：界面切到 English 时使用。内容与中文版逐条对应，
// 参考来源、拒答、数值转述纪律完全一致，只是语言换成英文。
const SYSTEM_PROMPT_EN =
  'You are the vehicle Q&A assistant for the Renault 5 E-Tech (Car Manual Assistant), and you can also handle routine coding tasks. Available tools:\n' +
  '- read(path): read a file or list a directory\n' +
  '- grep(pattern,path): search file contents\n' +
  '- glob(pattern,path): match files by pattern (e.g. **/*.rs)\n' +
  '- audit(path): security scan\n' +
  '- fetch(url): fetch web content\n' +
  '- edit(path,old,new): modify a file\n' +
  '- write(path,content): create/overwrite a file\n' +
  '- bash(command): run a shell command (build, run, git, etc.)\n' +
  '- skill_list(): list all available skills (capability extensions)\n' +
  '- skill_run(id,args?): run a skill (script-type executes, prompt-type returns content)\n' +
  '\n[VEHICLE Q&A RULES — HIGHEST PRIORITY, MUST FOLLOW] For ANY vehicle question (features, operations, buttons, warning lights, charging, range, maintenance, specs):\n' +
  'Step 1: first call skill_run("manual-rag", "<the user question verbatim>") to query the manual knowledge base;\n' +
  'Step 2: answer ONLY from the retrieved passages; quote numbers and warning wording exactly;\n' +
  'Step 3: if the retrieval finds nothing, explicitly say "the manual does not cover this" and omit the sources section.\n' +
  'Example flow:\nUser: How do I turn on the fog lights?\nCorrect: first call skill_run("manual-rag", "How do I turn on the fog lights?"), then answer from the returned passages and end with sources.\n' +
  'Wrong: calling no tool and answering vehicle questions from memory.\n' +
  "\n[ANSWER STYLE — you are the owner's in-car assistant, not a knowledge-base query UI]\n" +
  '- Warm, plain, conversational tone — like a car-savvy friend sitting next to the owner; avoid jargon;\n' +
  '- Keep it short: under 150 words for normal questions; lead with the conclusion and the steps; no preamble, no summary, no closing pleasantries;\n' +
  '- No tables or multi-level headings; short sentences; numbered steps for complex operations; safety warnings must be kept, never trimmed for brevity;\n' +
  '- Never say "according to the manual", "page X", "chapter", "retrieval", "chunk" or "knowledge base"; never narrate the retrieval process;\n' +
  '- State the steps and precautions directly; when the content only partially covers the question, answer the covered part naturally without declaring "not covered".\n' +
  '\n[SOURCE FORMAT] End every answer with (max 4 entries, only those actually used, owner-friendly wording):\n' +
  'Sources:\n- Owner manual p.<page> (<topic>)\n' +
  'Example: - Owner manual p.148 (Lighting and signals)\n' +
  'Fabricating citations is strictly forbidden: without a skill_run retrieval, never output any page number or the word "Sources";\n' +
  'Never put file paths or chunk ids into the sources.\n' +
  '[NUMBER DISCIPLINE] When quoting any number, relay its original qualifiers verbatim (chapter, scenario, precondition);\n' +
  'do not generalise scenario-specific numbers into universal specs (e.g. the pressure threshold in the tyre-repair flow is not the standard tyre pressure);\n' +
  'wherever the manual defers to the driver-door label (Label A), tell the user to check that label.\n' +
  '\nFor other requests, gather information with tools first, then act. Answer concisely and report results after actions.';
// 无工具模型的系统提示词(服务端 config.json 对该模型 tools:false):
// 不提供工具列表、不要求调用 skill —— 检索由服务端自动完成并以
// 【本轮手册检索结果】消息注入, 模型只需读材料答题。
// 注意: NO_TOOL_MODELS 需与服务端 config.json 的 tools:false 配置保持同步。
const NO_TOOL_SYSTEM_PROMPT =
  '你是雷诺 R5 E-Tech 的车辆使用问答助手（车书助手）。\n' +
  '\n【回答依据】每个问题的官方手册检索结果会以【本轮手册检索结果】消息提供：\n' +
  '- 只依据检索到的原文回答，数字和警告措辞要原样引用；\n' +
  '- 检索结果未覆盖的问题，明确回答"手册中没有相关内容"，此时省略参考来源；\n' +
  '- 严禁尝试调用任何工具，也严禁输出 function、tool_calls 之类的调用指令文字。\n' +
  '\n【回答风格——面向车主】\n' +
  '- 口吻亲切自然、通俗易懂，像懂车的朋友坐在旁边讲解，少用专业术语；\n' +
  '- 回答务必简短：一般问题控制在 300 字以内，直接给结论和操作步骤，不铺垫、不总结、不说"希望对您有帮助"之类的客套话；\n' +
  '- 不要用表格和多级标题，用短句；复杂操作用简短的编号步骤；安全警告必须保留，不为凑字数删减；\n' +
  '- 不要出现"根据手册""第X页""检索""知识库"这类词，也不要复述检索过程；\n' +
  '- 内容只覆盖部分时，自然回答已覆盖的部分，不要声明"检索未覆盖"。\n' +
  '\n【参考来源格式】回答结尾附上（最多 4 条，只列实际用到的）：\n' +
  '参考来源:\n- 用户手册 p.<页码>（<中文主题>）\n' +
  '示例：- 用户手册 p.148（灯光与信号）\n' +
  '严禁编造引用：未经检索，绝不允许输出任何页码或"参考来源"字样。\n' +
  '【数值转述纪律】引用数值时必须连同原文的限定条件（场景、前提）一起转述；手册写明以车门标签(Label A)为准的参数，必须提示用户查看车门标签。';

// NO_TOOL_SYSTEM_PROMPT 的英文版（MiniCPM5-2B 等无工具模型，服务端自动注入检索）。
const NO_TOOL_SYSTEM_PROMPT_EN =
  'You are the vehicle Q&A assistant for the Renault 5 E-Tech (Car Manual Assistant).\n' +
  '\n[ANSWER BASIS] The official-manual retrieval results for each question are provided as a [Manual retrieval results for this turn] message:\n' +
  '- Answer ONLY from the retrieved passages; quote numbers and warning wording exactly;\n' +
  '- If the retrieval does not cover the question, explicitly say "the manual does not cover this" and omit the sources section;\n' +
  '- Never attempt to call any tool, and never output function / tool_calls style invocation text.\n' +
  "\n[ANSWER STYLE — for car owners]\n" +
  '- Warm, plain, conversational tone — like a car-savvy friend sitting next to the owner; avoid jargon;\n' +
  '- Keep it short: under 150 words for normal questions; lead with the conclusion and the steps; no preamble, no summary, no closing pleasantries;\n' +
  '- No tables or multi-level headings; short sentences; numbered steps for complex operations; safety warnings must be kept;\n' +
  '- Never say "according to the manual", "page X", "retrieval" or "knowledge base"; never narrate the retrieval process;\n' +
  '- When the content only partially covers the question, answer the covered part naturally without declaring "not covered".\n' +
  '\n[SOURCE FORMAT] End every answer with (max 4 entries, only those actually used):\n' +
  'Sources:\n- Owner manual p.<page> (<topic>)\n' +
  'Example: - Owner manual p.148 (Lighting and signals)\n' +
  'Fabricating citations is strictly forbidden: without retrieval, never output any page number or the word "Sources".\n' +
  '[NUMBER DISCIPLINE] When quoting numbers, relay their original qualifiers (scenario, precondition) verbatim; wherever the manual defers to the driver-door label (Label A), tell the user to check that label.';

// 与服务端 config.json 的 tools:false 配置保持同步
const NO_TOOL_MODELS = ['openbmb/minicpm5-2b:latest'];
// 语言强制规则：拼在系统提示词末尾（服务端注入的 AGENTS.md 在最前），
// 英文会话强制英文回答，中文会话强制中文回答——覆盖 AGENTS.md 里的默认语言。
const LANG_RULE = {
  zh: '\n【回答语言——最高优先级】必须始终用中文回答，即使用户问题或本提示词的其他部分是英文；按钮名、功能名等专有名词可保留英文。\n',
  en: '\n[RESPONSE LANGUAGE — HIGHEST PRIORITY] You must ALWAYS respond in English, even if the user question or other parts of this prompt are in Chinese; proper nouns (button names, feature names) may stay in their official form.\n',
};
function systemPromptFor(model) {
  const base = NO_TOOL_MODELS.includes(model)
    ? (LANG === 'en' ? NO_TOOL_SYSTEM_PROMPT_EN : NO_TOOL_SYSTEM_PROMPT)
    : (LANG === 'en' ? SYSTEM_PROMPT_EN : SYSTEM_PROMPT);
  return base + LANG_RULE[LANG];
}

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
    '<div class="tname">🔧 ' + tool + (needsConfirm ? t('needConfirm') : '') + '</div>' +
    '<div class="targs">' + (typeof args === 'string' ? args : JSON.stringify(args)) + '</div>' +
    '<div class="tres-toggle" hidden></div>' +
    '<div class="tres"></div>';
  chat.appendChild(div);
  const tres = div.querySelector('.tres');
  const toggle = div.querySelector('.tres-toggle');

  // Results longer than this start folded behind a "click to expand" line,
  // so a full RAG retrieval doesn't flood the chat with JSON passages.
  const FOLD_THRESHOLD = 200;
  let folded = true;
  let lastOk = true, lastText = '';
  const fmtChars = (n) => (n >= 1000 ? (n / 1000).toFixed(1) + t('kChars') : n + t('chars'));
  toggle.onclick = () => { folded = !folded; tres.setResult(lastOk, lastText); };
  tres.setResult = (ok, text) => {
    lastOk = ok; lastText = text || '';
    const cls = ' ' + (ok ? 'ok' : 'fail');
    if (lastText.length > FOLD_THRESHOLD) {
      toggle.hidden = false;
      toggle.className = 'tres-toggle' + cls;
      toggle.textContent = (folded ? '▶' : '▼') + ' ' + (ok ? '✓' : '✗') + t('result') +
        fmtChars(lastText.length) + t('clickTo') + (folded ? t('expand') : t('collapse'));
      tres.hidden = folded;
      tres.textContent = lastText;
    } else {
      toggle.hidden = true;
      tres.hidden = false;
      tres.textContent = (ok ? '✓ ' : '✗ ') + lastText;
    }
    tres.className = 'tres' + cls;
  };

  // For tools needing confirmation, show allow/deny buttons.
  if (needsConfirm && confirmId) {
    const btns = document.createElement('div');
    btns.className = 'confirm-btns';
    btns.style.marginTop = '6px';
    const allow = document.createElement('button');
    allow.textContent = t('allow'); allow.style.marginRight = '8px';
    allow.style.background = '#43a047'; allow.style.color = '#fff';
    allow.style.border = 'none'; allow.style.padding = '4px 12px';
    allow.style.borderRadius = '4px'; allow.style.cursor = 'pointer';
    const deny = document.createElement('button');
    deny.textContent = t('deny');
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
        tres.textContent = allowed ? t('executing') : t('denied');
      }).catch(() => {
        tres.textContent = t('confirmFail');
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
      { type: 'text', text: text || t('describeImage') },
      { type: 'image_url', image_url: { url: pendingImage.dataUrl } },
    ];
    clearImage();
  } else {
    content = text;
  }
  messages.push({ role: 'user', content: content });

  busy = true;
  abortCtrl = new AbortController();
  sendBtn.textContent = t('stop');
  sendBtn.disabled = false;
  sendBtn.onclick = () => { if (abortCtrl) abortCtrl.abort(); };
  await streamChat();
  busy = false;
  abortCtrl = null;
  sendBtn.textContent = t('send');
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

// Slim old tool results (full RAG passages, hundreds of tokens each) to a
// one-line stub. The newest SLIM_KEEP_MESSAGES messages stay verbatim (~2
// exchanges), so the model always answers from the CURRENT turn's retrieval
// instead of being diluted by stale passages from earlier turns.
const SLIM_KEEP_MESSAGES = 8;
function slimOldToolResults(msgs) {
  const cut = Math.max(0, msgs.length - SLIM_KEEP_MESSAGES);
  return msgs.map((m, i) =>
    (i < cut && m.role === 'tool')
      ? { ...m, content: '[已检索: manual-rag 结果(略)]' }
      : m
  );
}

function trimContext(msgs) {
  // msgs excludes system. Slim stale tool results first, then keep newest
  // messages until under budget, but never start mid-pair: skip leading 'tool'
  // messages (they answer a preceding call).
  let kept = slimOldToolResults(msgs);
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
  const model = document.getElementById('model').value.trim();
  // base_url/api_key: Ollama is a fixed deployment value; gateway models are
  // resolved server-side from config.json (model_endpoints). Nothing to send.
  const cfg = {
    base_url: DEFAULT_BASE_URL,
    model,
  };
  // 禁思考 + 协议选择: gateway models get the enable_thinking body param on
  // /v1 (server resolves their real endpoint); every other model is local
  // Ollama and goes over its NATIVE /api/chat protocol — that carries
  // per-request num_ctx (stock official models get full context without
  // custom tags) and think:false for 禁思考. num_ctx=32768 fits this box's
  // GPU; lower it there if a bigger model needs the VRAM.
  if (GATEWAY_MODELS.includes(model)) {
    if (document.getElementById('no_think').checked) {
      cfg.extra_body = { enable_thinking: false };
    }
  } else {
    cfg.native = true;
    cfg.num_ctx = 32768;
    if (document.getElementById('no_think').checked) {
      cfg.no_think = true;
    }
  }

  // Trim history to stay within the context budget.
  const nonSystem = messages.filter(m => m.role !== 'system');
  const trimmed = trimContext(nonSystem);
  if (trimmed.length < nonSystem.length) {
    addBubble('assistant', t('truncated').replace('{n}', nonSystem.length - trimmed.length));
  }
  const selectedModel = document.getElementById('model').value.trim();
  const reqMessages = [
    { role: 'system', content: systemPromptFor(selectedModel) },
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
        // 无可见内容(还在检索/思考)时隐藏气泡, 避免空白占位框
        assistantDiv.style.display = assistantText.trim() ? '' : 'none';
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
        auto_mode: true,
        auto_rag: document.getElementById('auto_rag').checked,
      }),
    });
    if (resp.status === 401) { showLogin(); return; }
    if (!resp.ok) {
      let hint = t('reqFail').replace('{code}', resp.status);
      if (resp.status === 500) hint += t('reqFailCause');
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
      stopNote.textContent = t('stopped');
      chat.appendChild(stopNote);
      scrollDown();
      return;
    }
    const msg = e.message || '';
    let hint = t('connLost') + msg;
    if (msg.includes('Failed to fetch') || msg.includes('NetworkError')) {
      hint = t('connFailHint');
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
      // 纯占位气泡(只有飘动圆点、还没产生内容)直接移除,不能留在页面上
      if (assistantDiv && !assistantText.trim()) {
        assistantDiv.remove();
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
      if (last && last.setResult) {
        last.setResult(ev.ok, ev.summary);
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
        stat.textContent = t('ttft') + (ttftMs !== null ? (ttftMs / 1000).toFixed(1) + 's' : '--') + ' · ' + ev.tps.toFixed(1) + ' tok/s · ' + ev.tokens + ' tok · ' + (ev.elapsed_ms / 1000).toFixed(1) + 's';
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
          m.content = textPart ? textPart.text : t('imgHandled');
        }
      });
      // Persist this session after each completed reply.
      saveCurrentSession();
    } else if (ev.type === 'error') {
      const div = document.createElement('div');
      div.className = 'err';
      let msg = ev.message || t('unknownErr');
      // Add actionable hints for common errors.
      if (msg.includes('model_not_found') || msg.includes('No models loaded')) {
        msg += t('errModel');
      } else if (msg.includes('404') || msg.includes('Not Found')) {
        msg += t('errUrl');
      } else if (msg.includes('timeout') || msg.includes('Timeout')) {
        msg += t('errTimeout');
      } else if (msg.includes('Connection refused') || msg.includes('ECONNREFUSED')) {
        msg += t('errConn');
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

// --- Language toggle: UI text, suggested questions, help page and the system
// prompt (answer language) all follow. Persisted per browser; the help page
// reads the same localStorage key.
function applyLang() {
  document.documentElement.lang = LANG === 'en' ? 'en' : 'zh';
  document.title = t('title');
  document.querySelector('header h1').textContent = t('appName');
  const loginTitle = document.querySelector('#login-overlay h2');
  if (loginTitle) loginTitle.textContent = t('appName');
  document.getElementById('login-user').placeholder = t('loginUser');
  document.getElementById('login-pass').placeholder = t('loginPass');
  document.getElementById('login-btn').textContent = t('login');
  document.getElementById('login-err').textContent = t('loginErr');
  document.getElementById('new-session').textContent = '＋ ' + t('newSessionShort');
  document.getElementById('sug-title').textContent = t('commonQ');
  document.getElementById('model').title = t('selectModel');
  document.getElementById('no_think_label').textContent = t('noThink');
  document.getElementById('no_think_wrap').title = t('noThinkTip');
  document.getElementById('auto_rag_label').textContent = t('autoRag');
  document.getElementById('auto_rag_wrap').title = t('autoRagTip');
  document.getElementById('save_cfg').textContent = t('save');
  document.getElementById('lang_btn').textContent = t('langBtn');
  document.getElementById('lang_btn').title = t('langTip');
  document.getElementById('help_btn').title = t('helpTip');
  document.getElementById('input').placeholder = t('inputPlaceholder');
  document.getElementById('send').textContent = t('send');
  document.getElementById('img-remove').textContent = t('remove');
  renderSuggestions();
}
document.getElementById('lang_btn').onclick = () => {
  LANG = LANG === 'zh' ? 'en' : 'zh';
  try { localStorage.setItem(LANG_KEY, LANG); } catch (e) {}
  applyLang();
};
applyLang();
