/**
 * TrueNorth WebSocket Manager
 * Manages the connection to /api/v1/events/ws with:
 *   - Auto-reconnect with exponential backoff
 *   - Event subscription system
 *   - Connection state tracking
 *   - UI status indicator updates
 */

const WsManager = (() => {
  let ws = null;
  let _state = 'disconnected'; // 'connecting' | 'connected' | 'disconnected'
  let _retryCount = 0;
  let _retryTimer = null;
  let _listeners = {};
  let _enabled = false;

  const MAX_RETRY_DELAY = 30000; // 30s
  const BASE_RETRY_DELAY = 1000; // 1s

  function getRetryDelay() {
    return Math.min(BASE_RETRY_DELAY * Math.pow(2, _retryCount), MAX_RETRY_DELAY);
  }

  function setState(state) {
    _state = state;
    emit('_state', state);
    updateStatusIndicator();
  }

  function updateStatusIndicator() {
    // Update all .ws-dot elements
    document.querySelectorAll('.ws-dot').forEach(dot => {
      dot.className = `ws-dot ${_state}`;
    });
    // Update status text
    document.querySelectorAll('.ws-status-text').forEach(el => {
      el.textContent = {
        connected: 'WS Connected',
        connecting: 'Connecting…',
        disconnected: 'WS Disconnected',
      }[_state] || _state;
    });
  }

  function connect() {
    if (!_enabled) return;
    if (ws && (ws.readyState === WebSocket.CONNECTING || ws.readyState === WebSocket.OPEN)) return;

    setState('connecting');
    const url = API.getWsUrl();

    try {
      ws = new WebSocket(url);
    } catch (e) {
      console.warn('[WS] Failed to create WebSocket:', e.message);
      scheduleReconnect();
      return;
    }

    ws.onopen = () => {
      _retryCount = 0;
      setState('connected');
      console.log('[WS] Connected to', url);
      emit('connected', {});
    };

    ws.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data);
        emit('event', event);
        // Emit by event type too
        if (event.type) emit(event.type, event);
      } catch (err) {
        console.warn('[WS] Failed to parse message:', e.data);
        emit('raw', e.data);
      }
    };

    ws.onerror = (e) => {
      console.warn('[WS] Error:', e);
      emit('error', e);
    };

    ws.onclose = (e) => {
      console.log(`[WS] Closed (code=${e.code})`);
      ws = null;
      setState('disconnected');
      emit('disconnected', { code: e.code, reason: e.reason });
      if (_enabled) scheduleReconnect();
    };
  }

  function scheduleReconnect() {
    if (_retryTimer) clearTimeout(_retryTimer);
    const delay = getRetryDelay();
    _retryCount++;
    console.log(`[WS] Reconnecting in ${delay}ms (attempt ${_retryCount})`);
    _retryTimer = setTimeout(() => {
      if (_enabled) connect();
    }, delay);
  }

  function disconnect() {
    _enabled = false;
    if (_retryTimer) { clearTimeout(_retryTimer); _retryTimer = null; }
    if (ws) { ws.close(1000, 'User disconnected'); ws = null; }
    setState('disconnected');
  }

  function enable() {
    _enabled = true;
    connect();
  }

  function disable() {
    disconnect();
  }

  /** Send a message (if connected) */
  function send(data) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(typeof data === 'string' ? data : JSON.stringify(data));
      return true;
    }
    return false;
  }

  /** Subscribe to events */
  function on(event, handler) {
    if (!_listeners[event]) _listeners[event] = new Set();
    _listeners[event].add(handler);
    return () => off(event, handler);
  }

  function off(event, handler) {
    if (_listeners[event]) _listeners[event].delete(handler);
  }

  function emit(event, data) {
    if (_listeners[event]) {
      _listeners[event].forEach(fn => {
        try { fn(data); } catch (e) { console.error('[WS] Listener error:', e); }
      });
    }
  }

  function getState() { return _state; }
  function isConnected() { return _state === 'connected'; }

  return { enable, disable, connect, disconnect, send, on, off, getState, isConnected };
})();

if (typeof module !== 'undefined' && module.exports) module.exports = WsManager;
