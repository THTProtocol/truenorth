/**
 * TrueNorth API Client
 * Wraps all Axum server endpoints with typed fetch helpers.
 * Base URL is configurable and stored in the app state.
 */

const API = (() => {
  // Default base URL — overridden by Settings
  let _baseUrl = 'http://localhost:8080';

  function getBaseUrl() { return _baseUrl; }

  function setBaseUrl(url) {
    _baseUrl = url.replace(/\/$/, ''); // strip trailing slash
  }

  /**
   * Core fetch wrapper with error handling.
   * Returns { data, error } — never throws.
   */
  async function request(path, options = {}) {
    const url = `${_baseUrl}${path}`;
    try {
      const resp = await fetch(url, {
        headers: { 'Content-Type': 'application/json', ...options.headers },
        ...options,
      });

      if (!resp.ok) {
        let msg = `HTTP ${resp.status}`;
        try { const j = await resp.json(); msg = j.message || j.error || msg; } catch {}
        return { data: null, error: msg, status: resp.status };
      }

      const contentType = resp.headers.get('content-type') || '';
      const data = contentType.includes('application/json') ? await resp.json() : await resp.text();
      return { data, error: null, status: resp.status };
    } catch (err) {
      return { data: null, error: err.message || 'Network error', status: 0 };
    }
  }

  /* ── Health ─────────────────────────────────────────── */
  async function getHealth() {
    return request('/health');
  }

  /* ── Tasks ──────────────────────────────────────────── */
  /**
   * Submit a task. Returns session info.
   * @param {Object} payload - { prompt, session_id?, context? }
   */
  async function submitTask(payload) {
    return request('/api/v1/task', {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  }

  /* ── Sessions ───────────────────────────────────────── */
  async function listSessions() {
    return request('/api/v1/sessions');
  }

  async function getSession(id) {
    return request(`/api/v1/sessions/${encodeURIComponent(id)}`);
  }

  async function cancelSession(id) {
    return request(`/api/v1/sessions/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    });
  }

  /* ── SSE Streaming ──────────────────────────────────── */
  /**
   * Open an SSE connection to stream a response.
   * Returns an EventSource-like object.
   * @param {Function} onChunk - called with each text chunk
   * @param {Function} onDone - called when stream completes
   * @param {Function} onError - called on error
   */
  function streamResponse(onChunk, onDone, onError) {
    const url = `${_baseUrl}/api/v1/events/sse`;
    const es = new EventSource(url);

    es.onmessage = (e) => {
      try {
        const data = JSON.parse(e.data);
        if (data.done) {
          onDone && onDone(data);
          es.close();
        } else if (data.chunk) {
          onChunk && onChunk(data.chunk, data);
        } else {
          onChunk && onChunk(e.data, data);
        }
      } catch {
        onChunk && onChunk(e.data, {});
      }
    };

    es.onerror = (err) => {
      onError && onError(err);
      es.close();
    };

    return es;
  }

  /* ── Skills ─────────────────────────────────────────── */
  async function listSkills() {
    return request('/api/v1/skills');
  }

  /* ── Tools ──────────────────────────────────────────── */
  async function listTools() {
    return request('/api/v1/tools');
  }

  /* ── Memory ─────────────────────────────────────────── */
  /**
   * Search memory.
   * @param {string} query
   * @param {string} scope - 'session' | 'project' | 'identity'
   * @param {number} limit
   */
  async function searchMemory(query = '', scope = '', limit = 20) {
    const params = new URLSearchParams();
    if (query) params.set('q', query);
    if (scope) params.set('scope', scope);
    if (limit) params.set('limit', String(limit));
    return request(`/api/v1/memory/search?${params}`);
  }

  /* ── Agent Card ─────────────────────────────────────── */
  async function getAgentCard() {
    return request('/.well-known/agent.json');
  }

  /* ── WebSocket URL ──────────────────────────────────── */
  function getWsUrl() {
    const wsBase = _baseUrl.replace(/^http/, 'ws').replace(/^https/, 'wss');
    return `${wsBase}/api/v1/events/ws`;
  }

  return {
    getBaseUrl,
    setBaseUrl,
    getHealth,
    submitTask,
    listSessions,
    getSession,
    cancelSession,
    streamResponse,
    listSkills,
    listTools,
    searchMemory,
    getAgentCard,
    getWsUrl,
  };
})();

// Export for modules or global use
if (typeof module !== 'undefined' && module.exports) module.exports = API;
