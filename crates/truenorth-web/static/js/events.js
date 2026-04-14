/**
 * TrueNorth Event Timeline Renderer
 * Handles appending, formatting, and displaying reasoning events
 * in the timeline panel.
 */

const EventsManager = (() => {
  // Stored events per session
  const _events = {}; // sessionId → []
  let _activeSession = null;

  /* ── Event type metadata ─────────────────────────────── */
  const EVENT_META = {
    state_transition: {
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
        <path d="M5 12h14M12 5l7 7-7 7"/>
      </svg>`,
      cls: 'state',
      label: (ev) => `${ev.from_state || ev.from || '?'} → ${ev.to_state || ev.to || '?'}`,
    },
    tool_called: {
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>
      </svg>`,
      cls: 'tool',
      label: (ev) => `Call: ${ev.tool_name || 'unknown tool'}`,
    },
    tool_result: {
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>
      </svg>`,
      cls: 'tool',
      label: (ev) => `Result: ${ev.tool_name || 'unknown'} (${ev.success !== false ? 'ok' : 'error'})`,
    },
    llm_request_sent: {
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M12 2a10 10 0 1 1 0 20 10 10 0 0 1 0-20z"/>
        <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/>
        <path d="M12 17h.01"/>
      </svg>`,
      cls: 'llm',
      label: (ev) => `LLM Request sent${ev.model ? ` → ${ev.model}` : ''}`,
    },
    llm_response_received: {
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M12 2a10 10 0 1 1 0 20 10 10 0 0 1 0-20z"/>
        <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/>
        <path d="M12 17h.01"/>
      </svg>`,
      cls: 'llm',
      label: (ev) => `LLM Response${ev.tokens ? ` (${ev.tokens} tokens)` : ''}`,
    },
    error: {
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/>
        <line x1="12" y1="16" x2="12.01" y2="16"/>
      </svg>`,
      cls: 'error',
      label: (ev) => `Error: ${ev.message || ev.error || 'unknown error'}`,
    },
    plan_created: {
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
        <polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/>
        <line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/>
      </svg>`,
      cls: 'plan',
      label: (ev) => `Plan created${ev.steps ? ` (${ev.steps} steps)` : ''}`,
    },
  };

  // RCS events
  ['rcs_thesis', 'rcs_antithesis', 'rcs_synthesis', 'rcs_start', 'rcs_end'].forEach(t => {
    EVENT_META[t] = {
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <circle cx="12" cy="12" r="3"/><path d="M12 3v2M12 19v2M3 12h2M19 12h2"/>
      </svg>`,
      cls: 'rcs',
      label: (ev) => `RCS: ${(ev.type || '').replace('rcs_', '')} ${ev.iteration ? `(iter ${ev.iteration})` : ''}`,
    };
  });

  function getMeta(type) {
    if (!type) return getDefaultMeta();
    // Check exact match first
    if (EVENT_META[type]) return EVENT_META[type];
    // Prefix match for rcs_*
    if (type.startsWith('rcs_')) return EVENT_META['rcs_thesis'];
    return getDefaultMeta();
  }

  function getDefaultMeta() {
    return {
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <circle cx="12" cy="12" r="3"/>
      </svg>`,
      cls: 'default',
      label: (ev) => ev.type || ev.description || 'Event',
    };
  }

  function formatTime(timestamp) {
    if (!timestamp) return '';
    const d = timestamp instanceof Date ? timestamp : new Date(timestamp);
    if (isNaN(d)) return '';
    return d.toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
  }

  /**
   * Create a DOM element for one event.
   */
  function createEventElement(event) {
    const meta = getMeta(event.type);
    const desc = meta.label(event);
    const time = formatTime(event.timestamp || event.ts || Date.now());

    const el = document.createElement('div');
    el.className = 'tn-event';
    el.setAttribute('data-event-type', event.type || 'unknown');
    el.setAttribute('title', JSON.stringify(event, null, 2));

    el.innerHTML = `
      <div class="tn-event-icon ${meta.cls}">${meta.icon}</div>
      <div class="tn-event-body">
        <div class="tn-event-desc">${escapeHtml(desc)}</div>
        <div class="tn-event-time">${time}</div>
      </div>`;

    return el;
  }

  /**
   * Append an event to the timeline container.
   * @param {Object} event
   * @param {HTMLElement} container
   * @param {boolean} scrollToBottom
   */
  function append(event, container, scrollToBottom = true) {
    if (!container) return;

    const el = createEventElement(event);
    container.appendChild(el);

    // Animate in
    el.style.opacity = '0';
    el.style.transform = 'translateY(4px)';
    requestAnimationFrame(() => {
      el.style.transition = 'opacity 0.15s ease, transform 0.15s ease';
      el.style.opacity = '1';
      el.style.transform = 'translateY(0)';
    });

    if (scrollToBottom) {
      container.scrollTop = container.scrollHeight;
    }
  }

  /**
   * Render a full list of events to a container (clears first).
   */
  function renderAll(events, container) {
    if (!container) return;
    container.innerHTML = '';

    if (!events || events.length === 0) {
      container.innerHTML = `
        <div class="empty-state" style="padding: var(--space-8);">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
            <path d="M12 2a10 10 0 1 1 0 20 10 10 0 0 1 0-20z"/>
            <path d="M8 12h8M12 8v8"/>
          </svg>
          <p>No events yet. Submit a task to see reasoning events here.</p>
        </div>`;
      return;
    }

    const fragment = document.createDocumentFragment();
    events.forEach(ev => fragment.appendChild(createEventElement(ev)));
    container.appendChild(fragment);
    container.scrollTop = container.scrollHeight;
  }

  /**
   * Store an event for a session.
   */
  function store(sessionId, event) {
    if (!_events[sessionId]) _events[sessionId] = [];
    _events[sessionId].push(event);
    if (_activeSession === sessionId) {
      // Trim if too large (keep last 500)
      if (_events[sessionId].length > 500) {
        _events[sessionId] = _events[sessionId].slice(-500);
      }
    }
  }

  function getEvents(sessionId) {
    return _events[sessionId] || [];
  }

  function clearEvents(sessionId) {
    delete _events[sessionId];
  }

  function setActiveSession(sessionId) {
    _activeSession = sessionId;
  }

  function escapeHtml(str) {
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  return {
    append,
    renderAll,
    store,
    getEvents,
    clearEvents,
    setActiveSession,
    createEventElement,
    formatTime,
  };
})();

if (typeof module !== 'undefined' && module.exports) module.exports = EventsManager;
