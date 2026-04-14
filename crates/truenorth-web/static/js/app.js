/**
 * TrueNorth App — Router, State, Page Rendering
 * SPA with hash routing: #/, #/session/:id, #/memory, #/skills, #/tools, #/settings
 */

/* ── App State ──────────────────────────────────────────── */
const AppState = {
  sessions: [],
  activeSessionId: null,
  currentRoute: null,
  health: null,
  baseUrl: 'http://localhost:8080',
  theme: 'dark',
};

/* ── Toast Notifications ───────────────────────────────── */
function showToast(message, type = 'info', duration = 3500) {
  const container = document.getElementById('toast-container');
  if (!container) return;

  const icons = {
    success: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg>',
    error: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>',
    warning: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>',
    info: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>',
  };

  const toast = document.createElement('div');
  toast.className = `toast ${type}`;
  toast.innerHTML = `${icons[type] || icons.info}<span>${escapeHtml(message)}</span>`;
  container.appendChild(toast);

  setTimeout(() => {
    toast.style.transition = 'opacity 0.25s ease, transform 0.25s ease';
    toast.style.opacity = '0';
    toast.style.transform = 'translateX(10px)';
    setTimeout(() => toast.remove(), 300);
  }, duration);
}

function escapeHtml(str) {
  return String(str)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/* ── Theme Toggle ──────────────────────────────────────── */
function initTheme() {
  const preferred = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  AppState.theme = preferred;
  document.documentElement.setAttribute('data-theme', preferred);
  updateThemeToggle();
}

function toggleTheme() {
  AppState.theme = AppState.theme === 'dark' ? 'light' : 'dark';
  document.documentElement.setAttribute('data-theme', AppState.theme);
  updateThemeToggle();
}

function updateThemeToggle() {
  const btn = document.getElementById('theme-toggle');
  if (!btn) return;
  if (AppState.theme === 'dark') {
    btn.innerHTML = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <circle cx="12" cy="12" r="5"/>
      <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/>
    </svg>`;
    btn.setAttribute('aria-label', 'Switch to light mode');
  } else {
    btn.innerHTML = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>
    </svg>`;
    btn.setAttribute('aria-label', 'Switch to dark mode');
  }
}

/* ── Health Check ──────────────────────────────────────── */
async function checkHealth() {
  const badge = document.getElementById('health-badge');
  if (badge) badge.className = 'status-badge checking';

  const { data, error } = await API.getHealth();
  const healthy = !error && data !== null;

  AppState.health = healthy ? 'healthy' : 'unhealthy';

  if (badge) {
    badge.className = `status-badge ${healthy ? 'healthy' : 'unhealthy'}`;
    badge.innerHTML = `
      <span class="dot"></span>
      <span>${healthy ? 'Healthy' : 'Offline'}</span>`;
  }
  return healthy;
}

/* ── Sessions ──────────────────────────────────────────── */
async function loadSessions() {
  const { data, error } = await API.listSessions();
  if (data && Array.isArray(data)) {
    AppState.sessions = data;
  } else if (data && Array.isArray(data.sessions)) {
    AppState.sessions = data.sessions;
  } else {
    AppState.sessions = [];
  }
  renderSessionSidebar();
}

function renderSessionSidebar() {
  const list = document.getElementById('session-list');
  if (!list) return;

  if (!AppState.sessions.length) {
    list.innerHTML = `<div class="sidebar-empty">No sessions yet.<br>Submit a task to start.</div>`;
    return;
  }

  list.innerHTML = AppState.sessions.map(s => {
    const isActive = s.id === AppState.activeSessionId;
    const title = s.title || s.name || (s.id ? s.id.substring(0, 12) + '…' : 'Session');
    const time = s.created_at ? EventsManager.formatTime(s.created_at) : '';
    const statusBadge = s.status
      ? `<span class="tn-badge tn-badge-${statusColor(s.status)}" style="font-size:9px;padding:1px 5px;">${s.status}</span>`
      : '';
    return `
      <a class="session-item ${isActive ? 'active' : ''}" href="#/session/${s.id}" data-session-id="${s.id}" aria-current="${isActive ? 'page' : 'false'}">
        <div class="flex items-center justify-between gap-1">
          <span class="session-item-title">${escapeHtml(title)}</span>
          ${statusBadge}
        </div>
        <span class="session-item-meta">${time}</span>
      </a>`;
  }).join('');
}

function statusColor(status) {
  const map = { running: 'teal', pending: 'amber', completed: 'green', error: 'red', cancelled: 'slate' };
  return map[status] || 'slate';
}

/* ── Router ────────────────────────────────────────────── */
const Router = (() => {
  const routes = {};

  function register(path, handler) { routes[path] = handler; }

  function navigate(hash) {
    window.location.hash = hash.startsWith('#') ? hash : '#' + hash;
  }

  function resolve() {
    const hash = window.location.hash || '#/';
    const path = hash.replace(/^#/, '');

    // Match dynamic routes
    for (const [pattern, handler] of Object.entries(routes)) {
      const params = matchRoute(pattern, path);
      if (params !== null) {
        AppState.currentRoute = path;
        updateNavActive(path);
        handler(params);
        return;
      }
    }

    // 404 fallback
    renderNotFound();
  }

  function matchRoute(pattern, path) {
    const patternParts = pattern.split('/');
    const pathParts = path.split('/');
    if (patternParts.length !== pathParts.length) return null;

    const params = {};
    for (let i = 0; i < patternParts.length; i++) {
      if (patternParts[i].startsWith(':')) {
        params[patternParts[i].slice(1)] = pathParts[i];
      } else if (patternParts[i] !== pathParts[i]) {
        return null;
      }
    }
    return params;
  }

  function updateNavActive(path) {
    document.querySelectorAll('.header-nav a').forEach(a => {
      const href = a.getAttribute('href');
      const isActive = href === '#' + path || (href === '#/' && (path === '/' || path === ''));
      a.classList.toggle('active', isActive);
      a.setAttribute('aria-current', isActive ? 'page' : 'false');
    });
  }

  window.addEventListener('hashchange', resolve);

  return { register, navigate, resolve };
})();

/* ── Page: Dashboard (Home) ────────────────────────────── */
function renderDashboard() {
  const main = document.getElementById('main-content');
  main.innerHTML = `
    <div class="dashboard-layout">
      <!-- Left: Session Sidebar -->
      <aside class="session-sidebar" id="session-sidebar" aria-label="Sessions">
        <div class="sidebar-header">
          <h3>Sessions</h3>
          <button class="tn-btn tn-btn-primary tn-btn-xs" id="new-session-btn" aria-label="New session">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3">
              <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
            </svg>
            New
          </button>
        </div>
        <div class="sidebar-list" id="session-list" role="list"></div>
      </aside>

      <!-- Center: Chat -->
      <main class="chat-pane" aria-label="Chat">
        <div class="chat-messages" id="chat-messages" role="log" aria-live="polite" aria-label="Messages">
          <div class="chat-welcome" id="chat-welcome">
            <svg class="chat-welcome-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5"/>
            </svg>
            <h2>TrueNorth</h2>
            <p>AI orchestration harness. Submit a task to begin reasoning.</p>
          </div>
        </div>
        <div class="chat-input-area">
          <div class="chat-input-row">
            <textarea
              id="chat-input"
              class="chat-textarea"
              placeholder="Enter a task or question…"
              rows="1"
              aria-label="Task input"
              autocomplete="off"
              spellcheck="true"
            ></textarea>
            <button class="chat-send-btn" id="chat-send" aria-label="Send task">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                <line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/>
              </svg>
            </button>
          </div>
          <div class="chat-input-footer">
            <span class="chat-input-hint">⏎ Send &nbsp;·&nbsp; Shift+⏎ Newline</span>
            <span class="chat-input-hint" id="session-id-display"></span>
          </div>
        </div>
      </main>

      <!-- Right: Reasoning Panel -->
      <aside class="reasoning-panel" aria-label="Reasoning">
        <div class="panel-tabs" role="tablist">
          <button class="panel-tab active" role="tab" data-tab="graph" aria-selected="true">Graph</button>
          <button class="panel-tab" role="tab" data-tab="events" aria-selected="false">Events</button>
        </div>
        <div class="panel-body">
          <div class="panel-section active" id="panel-graph" role="tabpanel">
            <div class="graph-container" id="mermaid-graph"></div>
          </div>
          <div class="panel-section" id="panel-events" role="tabpanel">
            <div class="event-timeline" id="event-timeline"></div>
          </div>
        </div>
        <div class="ws-status">
          <span class="ws-dot" id="ws-dot"></span>
          <span class="ws-status-text">WS Disconnected</span>
        </div>
      </aside>
    </div>`;

  // Add backdrop as direct body child (not in grid)
  let backdrop = document.getElementById('sidebar-backdrop');
  if (!backdrop) {
    backdrop = document.createElement('div');
    backdrop.className = 'sidebar-backdrop';
    backdrop.id = 'sidebar-backdrop';
    document.body.appendChild(backdrop);
  }

  // Init Mermaid placeholder
  MermaidManager.init();
  MermaidManager.showPlaceholder('mermaid-graph');

  // Render sessions
  loadSessions();
  renderEventTimeline();

  // Panel tab switching
  document.querySelectorAll('.panel-tab').forEach(tab => {
    tab.addEventListener('click', () => {
      const name = tab.dataset.tab;
      document.querySelectorAll('.panel-tab').forEach(t => {
        t.classList.toggle('active', t === tab);
        t.setAttribute('aria-selected', String(t === tab));
      });
      document.querySelectorAll('.panel-section').forEach(s => {
        s.classList.toggle('active', s.id === `panel-${name}`);
      });
    });
  });

  // Chat send
  const sendBtn = document.getElementById('chat-send');
  const input = document.getElementById('chat-input');

  // Auto-resize textarea
  input.addEventListener('input', () => {
    input.style.height = 'auto';
    input.style.height = Math.min(input.scrollHeight, 160) + 'px';
  });

  // Keyboard shortcut
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendTask();
    }
  });

  sendBtn.addEventListener('click', sendTask);

  // New session button
  document.getElementById('new-session-btn').addEventListener('click', () => {
    AppState.activeSessionId = null;
    document.getElementById('session-id-display').textContent = '';
    renderSessionSidebar();
    const welcome = document.getElementById('chat-welcome');
    if (!welcome) {
      const msgs = document.getElementById('chat-messages');
      msgs.innerHTML = `<div class="chat-welcome" id="chat-welcome">
        <svg class="chat-welcome-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5"/>
        </svg>
        <h2>New Session</h2>
        <p>Submit a task to begin a new session.</p>
      </div>`;
    }
    MermaidManager.showPlaceholder('mermaid-graph');
    document.getElementById('event-timeline').innerHTML = '';
  });

  // Mobile sidebar
  document.getElementById('hamburger-btn')?.addEventListener('click', toggleSidebar);
  document.getElementById('sidebar-backdrop')?.addEventListener('click', closeSidebar);

  // WebSocket enable
  WsManager.enable();

  WsManager.on('event', (ev) => {
    // Store event
    const sid = ev.session_id || AppState.activeSessionId || 'global';
    EventsManager.store(sid, ev);

    // Append to timeline if visible
    const timeline = document.getElementById('event-timeline');
    if (timeline) EventsManager.append(ev, timeline);

    // Update Mermaid if diagram provided
    if (ev.mermaid_diagram) {
      MermaidManager.render(ev.mermaid_diagram);
    } else {
      // Rebuild from stored events
      const allEvents = EventsManager.getEvents(sid);
      const diagram = MermaidManager.buildDiagramFromEvents(allEvents);
      MermaidManager.render(diagram);
    }

    // Update session badge on state transition
    if (ev.type === 'state_transition') {
      const badge = document.querySelector(`.session-item[data-session-id="${sid}"] .tn-badge`);
      if (badge) {
        badge.className = `tn-badge tn-badge-${statusColor(ev.to_state || ev.to)}`;
        badge.textContent = ev.to_state || ev.to;
      }
    }
  });
}

function renderEventTimeline() {
  const container = document.getElementById('event-timeline');
  if (!container) return;
  const sid = AppState.activeSessionId || 'global';
  EventsManager.renderAll(EventsManager.getEvents(sid), container);
}

/* ── Chat / Task Submission ────────────────────────────── */
let _streamingMessageEl = null;
let _isStreaming = false;

async function sendTask() {
  if (_isStreaming) return;

  const input = document.getElementById('chat-input');
  const prompt = input.value.trim();
  if (!prompt) return;

  const msgs = document.getElementById('chat-messages');

  // Remove welcome if present
  const welcome = document.getElementById('chat-welcome');
  if (welcome) welcome.remove();

  // Add user message
  appendMessage(msgs, { role: 'user', content: prompt, timestamp: new Date() });

  // Clear input
  input.value = '';
  input.style.height = 'auto';
  input.focus();

  // Start streaming AI response
  _isStreaming = true;
  document.getElementById('chat-send').disabled = true;

  const aiMsgEl = appendMessage(msgs, { role: 'ai', content: '', timestamp: new Date() });
  const bodyEl = aiMsgEl.querySelector('.message-body');
  bodyEl.innerHTML = '<span class="streaming-cursor"></span>';
  _streamingMessageEl = bodyEl;

  // Submit task
  const payload = {
    prompt,
    session_id: AppState.activeSessionId || undefined,
  };

  let accumulated = '';

  // Try SSE streaming, fall back to single request
  try {
    const { data: taskResp, error: taskErr } = await API.submitTask(payload);

    if (taskErr) {
      showToast(`Error: ${taskErr}`, 'error');
      bodyEl.innerHTML = `<span style="color:var(--color-red-text)">Error: ${escapeHtml(taskErr)}</span>`;
      _isStreaming = false;
      document.getElementById('chat-send').disabled = false;
      return;
    }

    // If we got a session_id back, track it
    if (taskResp && (taskResp.session_id || taskResp.id)) {
      const sid = taskResp.session_id || taskResp.id;
      AppState.activeSessionId = sid;
      document.getElementById('session-id-display').textContent = sid.substring(0, 12) + '…';
      EventsManager.setActiveSession(sid);

      // Add/update session in sidebar
      if (!AppState.sessions.find(s => s.id === sid)) {
        AppState.sessions.unshift({ id: sid, title: prompt.substring(0, 40), status: 'running', created_at: new Date() });
        renderSessionSidebar();
      }
    }

    // If response has content, show it
    if (taskResp && taskResp.response) {
      accumulated = taskResp.response;
      bodyEl.innerHTML = renderMarkdown(accumulated);
    } else if (taskResp && taskResp.content) {
      accumulated = taskResp.content;
      bodyEl.innerHTML = renderMarkdown(accumulated);
    } else {
      // Try SSE stream for response
      API.streamResponse(
        (chunk) => {
          accumulated += chunk;
          bodyEl.innerHTML = renderMarkdown(accumulated) + '<span class="streaming-cursor"></span>';
          msgs.scrollTop = msgs.scrollHeight;
        },
        () => {
          bodyEl.innerHTML = renderMarkdown(accumulated || '(no response)');
          _isStreaming = false;
          document.getElementById('chat-send').disabled = false;
          // Reload sessions
          setTimeout(loadSessions, 500);
        },
        (err) => {
          if (!accumulated) bodyEl.innerHTML = `<span style="color:var(--color-text-muted)">Response received (check terminal)</span>`;
          _isStreaming = false;
          document.getElementById('chat-send').disabled = false;
        }
      );
      return; // wait for SSE to finish
    }
  } catch (e) {
    showToast('Network error', 'error');
    bodyEl.innerHTML = `<span style="color:var(--color-red-text)">Network error</span>`;
  }

  _isStreaming = false;
  document.getElementById('chat-send').disabled = false;
  msgs.scrollTop = msgs.scrollHeight;
  setTimeout(loadSessions, 500);
}

function appendMessage(container, { role, content, timestamp }) {
  const isUser = role === 'user';
  const time = EventsManager.formatTime(timestamp || Date.now());

  const el = document.createElement('div');
  el.className = `message-bubble ${isUser ? 'user' : 'ai'}`;
  el.innerHTML = `
    <div class="message-avatar ${isUser ? '' : 'ai'}">${isUser ? 'You' : 'AI'}</div>
    <div class="message-content">
      <div class="message-body">${renderMarkdown(content)}</div>
      <div class="message-meta"><span>${time}</span></div>
    </div>`;

  container.appendChild(el);
  container.scrollTop = container.scrollHeight;
  return el;
}

function renderMarkdown(text) {
  if (!text) return '';
  // Basic markdown: code blocks, inline code, bold, italic
  return text
    .replace(/```(\w*)\n([\s\S]*?)```/g, (_, lang, code) =>
      `<pre><code class="lang-${lang || 'text'}">${escapeHtml(code.trim())}</code></pre>`)
    .replace(/`([^`]+)`/g, (_, c) => `<code>${escapeHtml(c)}</code>`)
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/\*([^*]+)\*/g, '<em>$1</em>')
    .replace(/\n\n/g, '</p><p>')
    .replace(/\n/g, '<br>')
    .replace(/^/, '<p>')
    .replace(/$/, '</p>')
    .replace(/<p><\/p>/g, '');
}

/* ── Sidebar Toggle (mobile) ────────────────────────────── */
function toggleSidebar() {
  const sidebar = document.getElementById('session-sidebar');
  const backdrop = document.getElementById('sidebar-backdrop');
  if (!sidebar) return;
  const isOpen = sidebar.classList.toggle('open');
  backdrop.classList.toggle('open', isOpen);
  document.getElementById('hamburger-btn')?.setAttribute('aria-expanded', String(isOpen));
}

function closeSidebar() {
  document.getElementById('session-sidebar')?.classList.remove('open');
  document.getElementById('sidebar-backdrop')?.classList.remove('open');
}

/* ── Page: Session Detail ────────────────────────────────── */
async function renderSessionDetail({ id }) {
  const main = document.getElementById('main-content');
  main.innerHTML = `<div class="loading-overlay"><div class="spinner"></div><span>Loading session…</span></div>`;

  const { data, error } = await API.getSession(id);

  if (error) {
    main.innerHTML = `<div class="empty-state" style="padding:var(--space-12)">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
      </svg>
      <p>Failed to load session: ${escapeHtml(error)}</p>
      <a href="#/" class="tn-btn tn-btn-ghost tn-btn-sm">← Back to Dashboard</a>
    </div>`;
    return;
  }

  const session = data || {};
  const events = session.events || EventsManager.getEvents(id) || [];
  const messages = session.messages || session.conversation || [];
  const tokenUsed = session.tokens_used || session.token_count || 0;
  const tokenMax = session.token_limit || session.context_limit || 128000;
  const tokenPct = Math.min((tokenUsed / tokenMax) * 100, 100);
  const tokenWarn = tokenPct > 85 ? 'danger' : tokenPct > 65 ? 'warn' : '';

  const toolCalls = events.filter(e => e.type === 'tool_called');

  main.innerHTML = `
    <div class="page-content" style="height:100%;overflow-y:auto;">
      <div class="session-detail-layout">
        <!-- Header -->
        <div class="flex items-center justify-between gap-4">
          <div>
            <div class="flex items-center gap-2 mb-2">
              <a href="#/" class="tn-btn tn-btn-ghost tn-btn-xs">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                  <path d="M19 12H5M12 19l-7-7 7-7"/>
                </svg> Back
              </a>
              <span class="tn-badge tn-badge-${statusColor(session.status)}">${session.status || 'unknown'}</span>
            </div>
            <h1 class="page-title">${escapeHtml(session.title || session.name || id)}</h1>
            <p class="text-sm text-muted mono" style="margin-top:4px;">${id}</p>
          </div>
          <button class="tn-btn tn-btn-danger tn-btn-sm" id="cancel-session-btn" data-id="${id}">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/>
            </svg>
            Cancel
          </button>
        </div>

        <!-- Meta grid -->
        <div class="session-meta-grid">
          <div class="session-meta-item">
            <div class="session-meta-label">Created</div>
            <div class="session-meta-value">${session.created_at ? new Date(session.created_at).toLocaleString() : '—'}</div>
          </div>
          <div class="session-meta-item">
            <div class="session-meta-label">Duration</div>
            <div class="session-meta-value">${session.duration || '—'}</div>
          </div>
          <div class="session-meta-item">
            <div class="session-meta-label">Tokens Used</div>
            <div class="session-meta-value">${tokenUsed.toLocaleString()} / ${tokenMax.toLocaleString()}</div>
            <div class="token-bar" style="margin-top:8px;">
              <div class="token-bar-fill ${tokenWarn}" style="width:${tokenPct}%"></div>
            </div>
          </div>
          <div class="session-meta-item">
            <div class="session-meta-label">Events</div>
            <div class="session-meta-value">${events.length}</div>
          </div>
        </div>

        <!-- Conversation -->
        ${messages.length ? `
        <div class="tn-card">
          <div class="tn-card-header">
            <h3 class="tn-card-title">Conversation</h3>
          </div>
          <div style="display:flex;flex-direction:column;gap:var(--space-3);">
            ${messages.map(m => `
              <div class="message-bubble ${m.role === 'user' ? 'user' : 'ai'}">
                <div class="message-avatar ${m.role === 'user' ? '' : 'ai'}">${m.role === 'user' ? 'You' : 'AI'}</div>
                <div class="message-content">
                  <div class="message-body">${renderMarkdown(m.content || m.text || '')}</div>
                </div>
              </div>`).join('')}
          </div>
        </div>` : ''}

        <!-- Reasoning Graph -->
        <div class="tn-card">
          <div class="tn-card-header">
            <h3 class="tn-card-title">Reasoning Graph</h3>
          </div>
          <div id="mermaid-graph" class="graph-container"></div>
        </div>

        <!-- Tool Calls -->
        ${toolCalls.length ? `
        <div>
          <h3 class="tn-card-title" style="margin-bottom:var(--space-3);">Tool Calls (${toolCalls.length})</h3>
          <div style="display:flex;flex-direction:column;gap:var(--space-2);">
            ${toolCalls.map((tc, i) => `
              <div class="tool-call-card">
                <div class="tool-call-header" onclick="toggleToolCall(this)" role="button" tabindex="0" aria-expanded="false" data-collapse-toggle>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>
                  </svg>
                  <span class="tool-call-name">${escapeHtml(tc.tool_name || 'unknown')}</span>
                  <span class="tn-badge tn-badge-amber" style="font-size:9px;">called</span>
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <polyline points="9 18 15 12 9 6"/>
                  </svg>
                </div>
                <div class="tool-call-body" id="tool-body-${i}">
                  <p class="text-xs text-muted" style="margin-bottom:var(--space-2);">Input</p>
                  <pre>${escapeHtml(JSON.stringify(tc.input || tc.arguments || {}, null, 2))}</pre>
                </div>
              </div>`).join('')}
          </div>
        </div>` : ''}

        <!-- Event Timeline -->
        ${events.length ? `
        <div class="tn-card">
          <div class="tn-card-header">
            <h3 class="tn-card-title">Event Timeline</h3>
            <span class="tn-badge tn-badge-slate">${events.length} events</span>
          </div>
          <div id="event-timeline-detail" style="max-height:400px;overflow-y:auto;"></div>
        </div>` : ''}
      </div>
    </div>`;

  // Render graph from events
  MermaidManager.init();
  if (events.length > 0) {
    const diagram = session.mermaid_diagram || MermaidManager.buildDiagramFromEvents(events);
    MermaidManager.render(diagram, null, true);
  } else {
    MermaidManager.showPlaceholder('mermaid-graph');
  }

  // Render event timeline
  if (events.length) {
    const tlContainer = document.getElementById('event-timeline-detail');
    if (tlContainer) EventsManager.renderAll(events, tlContainer);
  }

  // Cancel session
  document.getElementById('cancel-session-btn')?.addEventListener('click', async () => {
    const { error } = await API.cancelSession(id);
    if (!error) { showToast('Session cancelled', 'warning'); Router.navigate('/'); }
    else showToast(`Failed: ${error}`, 'error');
  });
}

function toggleToolCall(header) {
  const id = header.closest('.tool-call-card').querySelector('[id^="tool-body-"]').id;
  const body = document.getElementById(id);
  if (!body) return;
  const open = body.classList.toggle('open');
  header.setAttribute('aria-expanded', String(open));
  header.classList.toggle('open', open);
}
window.toggleToolCall = toggleToolCall;

/* ── Page: Memory ────────────────────────────────────────── */
async function renderMemory() {
  const main = document.getElementById('main-content');
  main.innerHTML = `
    <div class="page-content" style="height:100%;overflow-y:auto;">
      <div class="memory-layout">
        <div class="page-header">
          <div>
            <h1 class="page-title">Memory Browser</h1>
            <p class="page-subtitle">Search and explore the 3-tier memory store</p>
          </div>
        </div>

        <div class="memory-search-bar">
          <input type="search" id="memory-search-input" class="tn-input" placeholder="Search memory… (e.g. 'project goals', 'user preferences')" aria-label="Search memory" />
          <button class="tn-btn tn-btn-primary" id="memory-search-btn">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
              <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
            </svg>
            Search
          </button>
        </div>

        <div class="memory-tabs" role="tablist">
          <button class="memory-tab active" data-scope="" role="tab" aria-selected="true">All</button>
          <button class="memory-tab" data-scope="session" role="tab" aria-selected="false">Session</button>
          <button class="memory-tab" data-scope="project" role="tab" aria-selected="false">Project</button>
          <button class="memory-tab" data-scope="identity" role="tab" aria-selected="false">Identity</button>
        </div>

        <div id="memory-results" class="memory-results">
          <div class="empty-state">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/>
              <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/>
            </svg>
            <p>Search memory above, or browse by tier using the tabs.</p>
          </div>
        </div>
      </div>
    </div>`;

  let currentScope = '';

  async function doSearch() {
    const query = document.getElementById('memory-search-input').value.trim();
    const resultsEl = document.getElementById('memory-results');
    resultsEl.innerHTML = `<div class="loading-overlay"><div class="spinner"></div><span>Searching…</span></div>`;

    const { data, error } = await API.searchMemory(query, currentScope, 50);

    if (error) {
      resultsEl.innerHTML = `<div class="empty-state"><p>Error: ${escapeHtml(error)}</p></div>`;
      return;
    }

    const items = Array.isArray(data) ? data : (data?.results || data?.memories || []);

    if (!items.length) {
      resultsEl.innerHTML = `<div class="empty-state">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"/>
          <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/>
        </svg>
        <p>No results found${query ? ` for "${escapeHtml(query)}"` : ''}.</p>
      </div>`;
      return;
    }

    resultsEl.innerHTML = items.map(item => {
      const scope = item.scope || item.tier || 'unknown';
      const importance = item.importance || item.score || 0;
      const pips = Array.from({ length: 5 }, (_, i) =>
        `<span class="importance-pip ${i < Math.round(importance * 5) ? 'filled' : ''}"></span>`).join('');
      const timestamp = item.created_at || item.timestamp || item.updated_at;

      return `
        <div class="memory-card" onclick="this.classList.toggle('expanded')" role="button" tabindex="0" aria-expanded="false">
          <div class="memory-card-header">
            <span class="tn-badge tn-badge-${scopeColor(scope)}">${scope}</span>
            ${item.key ? `<span class="tn-badge tn-badge-slate mono">${escapeHtml(item.key)}</span>` : ''}
          </div>
          <div class="memory-card-content">${escapeHtml(item.content || item.value || item.text || '')}</div>
          <div class="memory-card-footer">
            <div class="importance-bar">${pips}</div>
            <span>importance: ${(importance * 100).toFixed(0)}%</span>
            ${timestamp ? `<span>${new Date(timestamp).toLocaleDateString()}</span>` : ''}
          </div>
        </div>`;
    }).join('');
  }

  document.getElementById('memory-search-btn').addEventListener('click', doSearch);
  document.getElementById('memory-search-input').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') doSearch();
  });

  document.querySelectorAll('.memory-tab').forEach(tab => {
    tab.addEventListener('click', () => {
      currentScope = tab.dataset.scope;
      document.querySelectorAll('.memory-tab').forEach(t => {
        t.classList.toggle('active', t === tab);
        t.setAttribute('aria-selected', String(t === tab));
      });
      doSearch();
    });
  });

  // Auto-load on open
  doSearch();
}

function scopeColor(scope) {
  const map = { session: 'teal', project: 'blue', identity: 'purple' };
  return map[scope] || 'slate';
}

/* ── Page: Skills ────────────────────────────────────────── */
async function renderSkills() {
  const main = document.getElementById('main-content');
  main.innerHTML = `<div class="loading-overlay"><div class="spinner"></div><span>Loading skills…</span></div>`;

  const { data, error } = await API.listSkills();

  const skills = Array.isArray(data) ? data : (data?.skills || []);

  main.innerHTML = `
    <div class="page-content" style="height:100%;overflow-y:auto;">
      <div class="skills-layout">
        <div class="page-header">
          <div>
            <h1 class="page-title">Skills</h1>
            <p class="page-subtitle">${error ? `Error: ${escapeHtml(error)}` : `${skills.length} skill${skills.length !== 1 ? 's' : ''} registered`}</p>
          </div>
        </div>
        ${!skills.length ? `
        <div class="empty-state">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
            <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5"/>
          </svg>
          <p>${error ? escapeHtml(error) : 'No skills loaded yet.'}</p>
        </div>` : `
        <div class="skills-grid">
          ${skills.map(skill => `
            <div class="skill-card">
              <div class="skill-card-top">
                <span class="skill-name">${escapeHtml(skill.name || skill.id || 'Unknown')}</span>
                <span class="tn-badge tn-badge-${skill.status === 'loaded' ? 'teal' : 'slate'}">
                  ${skill.status || 'available'}
                </span>
              </div>
              ${skill.version ? `<span class="tn-badge tn-badge-slate" style="margin-bottom:var(--space-2);font-size:9px;">v${escapeHtml(skill.version)}</span>` : ''}
              <p class="skill-desc">${escapeHtml(skill.description || 'No description.')}</p>
              ${skill.triggers?.length ? `
              <div class="skill-triggers">
                ${skill.triggers.slice(0, 6).map(t => `<span class="skill-trigger">${escapeHtml(t)}</span>`).join('')}
                ${skill.triggers.length > 6 ? `<span class="skill-trigger">+${skill.triggers.length - 6} more</span>` : ''}
              </div>` : ''}
              ${skill.required_tools?.length ? `
              <p class="skill-tools">Tools: ${escapeHtml(skill.required_tools.join(', '))}</p>` : ''}
            </div>`).join('')}
        </div>`}
      </div>
    </div>`;
}

/* ── Page: Tools ─────────────────────────────────────────── */
async function renderTools() {
  const main = document.getElementById('main-content');
  main.innerHTML = `<div class="loading-overlay"><div class="spinner"></div><span>Loading tools…</span></div>`;

  const { data, error } = await API.listTools();
  const tools = Array.isArray(data) ? data : (data?.tools || []);

  main.innerHTML = `
    <div class="page-content" style="height:100%;overflow-y:auto;">
      <div class="tools-layout">
        <div class="page-header">
          <div>
            <h1 class="page-title">Tools</h1>
            <p class="page-subtitle">${error ? escapeHtml(error) : `${tools.length} tool${tools.length !== 1 ? 's' : ''} registered`}</p>
          </div>
        </div>
        ${!tools.length ? `
        <div class="empty-state">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
            <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>
          </svg>
          <p>${error ? escapeHtml(error) : 'No tools registered.'}</p>
        </div>` : `
        <div class="tools-list">
          ${tools.map(tool => {
            const permColor = { safe: 'green', restricted: 'amber', dangerous: 'red' }[tool.permission_level] || 'slate';
            return `
              <div class="tool-card">
                <div class="tool-card-header">
                  <div class="tool-icon-wrap">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>
                    </svg>
                  </div>
                  <div class="tool-card-info">
                    <div class="tool-card-name">${escapeHtml(tool.name || tool.id || 'Unknown')}</div>
                    <div class="tool-card-desc">${escapeHtml(tool.description || 'No description')}</div>
                  </div>
                  <span class="tn-badge tn-badge-${permColor}">${tool.permission_level || 'unknown'}</span>
                </div>
                ${tool.schema ? `
                <div class="tool-schema">
                  <pre>${escapeHtml(typeof tool.schema === 'string' ? tool.schema : JSON.stringify(tool.schema, null, 2))}</pre>
                </div>` : ''}
              </div>`;
          }).join('')}
        </div>`}
      </div>
    </div>`;
}

/* ── Page: Settings ──────────────────────────────────────── */
async function renderSettings() {
  const main = document.getElementById('main-content');

  const savedUrl = getStoredBaseUrl();
  const displayUrl = savedUrl || AppState.baseUrl;

  main.innerHTML = `
    <div class="page-content" style="height:100%;overflow-y:auto;">
      <div class="settings-layout">
        <div class="page-header">
          <h1 class="page-title">Settings</h1>
        </div>

        <!-- API Configuration -->
        <div class="settings-section">
          <h2 class="settings-section-title">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="12" cy="12" r="3"/><path d="M19.07 4.93l-1.41 1.41M5.34 18.66l-1.41 1.41M22 12h-2M4 12H2M19.07 19.07l-1.41-1.41M5.34 5.34L3.93 3.93"/>
            </svg>
            API Configuration
          </h2>
          <div class="tn-form-group">
            <label class="tn-label" for="base-url-input">TrueNorth Server URL</label>
            <input type="url" id="base-url-input" class="tn-input" value="${escapeHtml(displayUrl)}" placeholder="http://localhost:8080" />
          </div>
          <div class="flex gap-2">
            <button class="tn-btn tn-btn-primary" id="save-url-btn">Save & Reconnect</button>
            <button class="tn-btn tn-btn-outline" id="test-url-btn">Test Connection</button>
          </div>
          <div id="url-test-result" style="margin-top:var(--space-3);"></div>
        </div>

        <!-- Health Status -->
        <div class="settings-section">
          <h2 class="settings-section-title">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>
            </svg>
            Health Status
          </h2>
          <div id="health-detail" class="tn-card">
            <div class="loading-overlay" style="padding:var(--space-6)"><div class="spinner"></div><span>Checking…</span></div>
          </div>
        </div>

        <!-- Agent Card -->
        <div class="settings-section">
          <h2 class="settings-section-title">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>
            </svg>
            Agent Card
          </h2>
          <div id="agent-card-detail">
            <div class="loading-overlay" style="padding:var(--space-6)"><div class="spinner"></div><span>Loading…</span></div>
          </div>
        </div>
      </div>
    </div>`;

  // Load health & agent card
  loadHealthDetail();
  loadAgentCard();

  // Save URL
  document.getElementById('save-url-btn').addEventListener('click', () => {
    const url = document.getElementById('base-url-input').value.trim();
    if (!url) { showToast('Enter a valid URL', 'warning'); return; }
    setStoredBaseUrl(url);
    AppState.baseUrl = url;
    API.setBaseUrl(url);
    WsManager.disable();
    WsManager.enable();
    showToast('URL saved. Reconnecting…', 'success');
    checkHealth();
  });

  // Test connection
  document.getElementById('test-url-btn').addEventListener('click', async () => {
    const url = document.getElementById('base-url-input').value.trim();
    if (!url) return;
    const resultEl = document.getElementById('url-test-result');
    resultEl.innerHTML = '<span class="text-sm text-muted">Testing…</span>';

    const oldUrl = API.getBaseUrl();
    API.setBaseUrl(url);
    const { data, error } = await API.getHealth();
    API.setBaseUrl(oldUrl);

    if (!error) {
      resultEl.innerHTML = `<span class="tn-badge tn-badge-green">● Connected — server is healthy</span>`;
    } else {
      resultEl.innerHTML = `<span class="tn-badge tn-badge-red">● Failed: ${escapeHtml(error)}</span>`;
    }
  });
}

async function loadHealthDetail() {
  const { data, error } = await API.getHealth();
  const el = document.getElementById('health-detail');
  if (!el) return;

  if (error) {
    el.innerHTML = `<div class="flex items-center gap-2">
      <span class="status-badge unhealthy"><span class="dot"></span>Unhealthy</span>
      <span class="text-sm text-muted">${escapeHtml(error)}</span>
    </div>`;
    return;
  }

  const health = typeof data === 'object' && data ? data : { status: 'ok' };
  el.innerHTML = `
    <div class="flex items-center gap-3 mb-3">
      <span class="status-badge healthy"><span class="dot"></span>Healthy</span>
      <span class="text-xs text-muted mono">${new Date().toLocaleTimeString()}</span>
    </div>
    ${Object.keys(health).length > 0 ? `<pre class="code-block">${escapeHtml(JSON.stringify(health, null, 2))}</pre>` : ''}`;
}

async function loadAgentCard() {
  const { data, error } = await API.getAgentCard();
  const el = document.getElementById('agent-card-detail');
  if (!el) return;

  if (error) {
    el.innerHTML = `<div class="empty-state" style="padding:var(--space-6);">
      <p>Agent card unavailable: ${escapeHtml(error)}</p>
    </div>`;
    return;
  }

  const card = typeof data === 'object' && data ? data : {};
  const entries = Object.entries(card);

  el.innerHTML = `
    <div class="agent-card-grid">
      ${entries.map(([k, v]) => `
        <div class="agent-card-item">
          <div class="agent-card-key">${escapeHtml(k)}</div>
          <div class="agent-card-val">${escapeHtml(typeof v === 'string' ? v : JSON.stringify(v))}</div>
        </div>`).join('')}
    </div>
    ${!entries.length ? `<pre class="code-block">${escapeHtml(JSON.stringify(card, null, 2))}</pre>` : ''}`;
}

/* ── Storage helpers (in-memory for settings) ────────── */
// Using in-memory storage — settings persist during session.
let _storedBaseUrl = '';

function getStoredBaseUrl() {
  return _storedBaseUrl;
}

function setStoredBaseUrl(url) {
  _storedBaseUrl = url;
}

/* ── Not Found ───────────────────────────────────────────── */
function renderNotFound() {
  const main = document.getElementById('main-content');
  main.innerHTML = `
    <div class="empty-state" style="padding:var(--space-16);">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <circle cx="12" cy="12" r="10"/><path d="M16 16s-1.5-2-4-2-4 2-4 2"/>
        <line x1="9" y1="9" x2="9.01" y2="9"/><line x1="15" y1="9" x2="15.01" y2="9"/>
      </svg>
      <p>Page not found.</p>
      <a href="#/" class="tn-btn tn-btn-ghost">← Back to Dashboard</a>
    </div>`;
}

/* ── Bootstrap ───────────────────────────────────────────── */
function bootstrap() {
  // Restore saved base URL
  const savedUrl = getStoredBaseUrl();
  if (savedUrl) {
    AppState.baseUrl = savedUrl;
    API.setBaseUrl(savedUrl);
  }

  // Theme
  initTheme();

  // Register routes
  Router.register('/', renderDashboard);
  Router.register('/session/:id', renderSessionDetail);
  Router.register('/memory', renderMemory);
  Router.register('/skills', renderSkills);
  Router.register('/tools', renderTools);
  Router.register('/settings', renderSettings);

  // Nav
  document.querySelectorAll('.header-nav a').forEach(a => {
    a.addEventListener('click', (e) => {
      // Let hash routing handle it
      setTimeout(() => Router.resolve(), 0);
    });
  });

  // Theme toggle
  document.getElementById('theme-toggle')?.addEventListener('click', toggleTheme);

  // Hamburger
  document.getElementById('hamburger-btn')?.addEventListener('click', toggleSidebar);

  // Health check (initial + periodic)
  checkHealth();
  setInterval(checkHealth, 30000);

  // Initial route
  Router.resolve();
}

// Start when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', bootstrap);
} else {
  bootstrap();
}
