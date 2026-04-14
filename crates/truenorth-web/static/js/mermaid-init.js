/**
 * TrueNorth Mermaid Integration
 * Handles Mermaid.js initialization, theme configuration,
 * and real-time diagram update logic.
 */

const MermaidManager = (() => {
  let _initialized = false;
  let _currentDiagram = null;
  let _renderQueue = null;
  let _renderTimer = null;

  const DARK_THEME_VARS = {
    primaryColor: '#1e2130',
    primaryTextColor: '#e2e5ec',
    primaryBorderColor: '#2a2d37',
    lineColor: '#4d5168',
    secondaryColor: '#151820',
    tertiaryColor: '#12141e',
    background: '#0f1117',
    mainBkg: '#1a1d27',
    nodeBorder: '#2a2d37',
    clusterBkg: '#151820',
    titleColor: '#e2e5ec',
    edgeLabelBackground: '#1a1d27',
    attributeBackgroundColorEven: '#1a1d27',
    attributeBackgroundColorOdd: '#151820',
    activeTaskBkgColor: '#0d2b28',
    activeTaskBorderColor: '#14b8a6',
    doneTaskBkgColor: '#052e16',
    doneTaskBorderColor: '#22c55e',
    critBkgColor: '#2d0a0a',
    critBorderColor: '#ef4444',
    taskTextColor: '#e2e5ec',
    taskTextOutsideColor: '#e2e5ec',
    taskTextLightColor: '#8891a8',
    nodeTextColor: '#e2e5ec',
    fontFamily: 'Inter, sans-serif',
    fontSize: '12px',
  };

  const LIGHT_THEME_VARS = {
    primaryColor: '#f0f2f5',
    primaryTextColor: '#111827',
    primaryBorderColor: '#d1d5de',
    lineColor: '#6b7280',
    secondaryColor: '#edf0f4',
    tertiaryColor: '#e2e5ec',
    background: '#ffffff',
    mainBkg: '#ffffff',
    nodeBorder: '#d1d5de',
    clusterBkg: '#f7f8fa',
    nodeTextColor: '#111827',
    fontFamily: 'Inter, sans-serif',
    fontSize: '12px',
  };

  function getThemeVars() {
    const theme = document.documentElement.getAttribute('data-theme') || 'dark';
    return theme === 'dark' ? DARK_THEME_VARS : LIGHT_THEME_VARS;
  }

  function init() {
    if (_initialized || typeof mermaid === 'undefined') return;
    _initialized = true;

    mermaid.initialize({
      startOnLoad: false,
      theme: 'base',
      themeVariables: getThemeVars(),
      flowchart: {
        curve: 'basis',
        padding: 16,
        nodeSpacing: 40,
        rankSpacing: 50,
        htmlLabels: true,
        diagramPadding: 8,
      },
      sequence: {
        diagramMarginX: 16,
        diagramMarginY: 16,
        actorMargin: 40,
        noteMargin: 8,
        messageMargin: 20,
      },
      securityLevel: 'loose',
      fontFamily: 'Inter, sans-serif',
      fontSize: 12,
    });

    // Re-init on theme change
    const observer = new MutationObserver(() => {
      if (_currentDiagram) render(_currentDiagram, null, true);
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  }

  /**
   * Render a Mermaid diagram string into a container element.
   * @param {string} diagramText
   * @param {HTMLElement|null} container - defaults to #mermaid-graph
   * @param {boolean} force - bypass debounce
   */
  async function render(diagramText, container = null, force = false) {
    if (!diagramText) return;

    _currentDiagram = diagramText;

    if (!force) {
      // Debounce rapid updates (e.g. streaming)
      if (_renderTimer) clearTimeout(_renderTimer);
      _renderQueue = { diagramText, container };
      _renderTimer = setTimeout(() => {
        _renderTimer = null;
        const q = _renderQueue;
        _renderQueue = null;
        render(q.diagramText, q.container, true);
      }, 200);
      return;
    }

    const target = container || document.getElementById('mermaid-graph');
    if (!target) return;

    if (typeof mermaid === 'undefined') {
      target.innerHTML = '<div class="graph-placeholder"><p>Mermaid not loaded</p></div>';
      return;
    }

    if (!_initialized) init();

    // Reinitialize with current theme vars
    mermaid.initialize({
      startOnLoad: false,
      theme: 'base',
      themeVariables: getThemeVars(),
      flowchart: { curve: 'basis', padding: 16, htmlLabels: true },
      securityLevel: 'loose',
      fontFamily: 'Inter, sans-serif',
      fontSize: 12,
    });

    try {
      const id = 'mermaid-' + Date.now();
      const { svg } = await mermaid.render(id, diagramText);
      target.innerHTML = `<div class="mermaid-wrap">${svg}</div>`;

      // Make SVG responsive
      const svgEl = target.querySelector('svg');
      if (svgEl) {
        svgEl.style.maxWidth = '100%';
        svgEl.style.height = 'auto';
        svgEl.removeAttribute('height');
      }
    } catch (err) {
      console.warn('[Mermaid] Render error:', err);
      target.innerHTML = `
        <div class="graph-placeholder">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="32" height="32">
            <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/>
            <line x1="12" y1="16" x2="12.01" y2="16"/>
          </svg>
          <span>Graph render error</span>
          <code style="font-size:10px;opacity:0.5">${err.message || 'Unknown error'}</code>
        </div>`;
    }
  }

  /**
   * Build a Mermaid flowchart from an array of reasoning events.
   * Each state_transition event adds a node; tool_called adds a subgraph.
   */
  function buildDiagramFromEvents(events) {
    if (!events || events.length === 0) {
      return buildEmptyDiagram();
    }

    const lines = ['flowchart TD'];
    const nodeIds = new Set();
    const edges = [];
    let prevNode = null;

    // Map state transitions to nodes
    events.forEach((ev, i) => {
      const type = ev.type || '';

      if (type === 'state_transition') {
        const from = sanitizeId(ev.from_state || ev.from || 'start');
        const to = sanitizeId(ev.to_state || ev.to || `state_${i}`);
        const label = sanitizeLabel(ev.to_state || ev.to || 'State');

        if (!nodeIds.has(from)) {
          nodeIds.add(from);
          lines.push(`  ${from}["${sanitizeLabel(ev.from_state || ev.from || 'Start')}"]`);
        }
        if (!nodeIds.has(to)) {
          nodeIds.add(to);
          lines.push(`  ${to}["${label}"]`);
        }

        const trigger = ev.trigger ? sanitizeLabel(ev.trigger) : '';
        edges.push(`  ${from} -->|"${trigger}"| ${to}`);
        prevNode = to;
      } else if (type === 'tool_called' || type === 'tool_result') {
        const toolId = sanitizeId('tool_' + (ev.tool_name || `tool${i}`));
        const toolLabel = sanitizeLabel(ev.tool_name || 'tool');

        if (!nodeIds.has(toolId)) {
          nodeIds.add(toolId);
          lines.push(`  ${toolId}(["⚙ ${toolLabel}"])`);
        }
        if (prevNode && type === 'tool_called') {
          edges.push(`  ${prevNode} -.->|"call"| ${toolId}`);
        }
      } else if (type === 'plan_created') {
        const planId = sanitizeId('plan_' + i);
        nodeIds.add(planId);
        lines.push(`  ${planId}{{"📋 Plan Created"}}`);
        if (prevNode) edges.push(`  ${prevNode} --> ${planId}`);
        prevNode = planId;
      } else if (type === 'llm_request_sent') {
        const llmId = sanitizeId('llm_' + i);
        nodeIds.add(llmId);
        lines.push(`  ${llmId}[/"🧠 LLM Request"/]`);
        if (prevNode) edges.push(`  ${prevNode} --> ${llmId}`);
        prevNode = llmId;
      }
    });

    // Add edges
    lines.push(...edges);

    // Apply styles
    lines.push('  classDef default fill:#1a1d27,stroke:#2a2d37,color:#e2e5ec');
    lines.push('  classDef tool fill:#2d1f00,stroke:#f59e0b,color:#fcd34d');
    lines.push('  classDef llm fill:#1e1040,stroke:#a78bfa,color:#c4b5fd');
    lines.push('  classDef plan fill:#052e16,stroke:#22c55e,color:#86efac');

    return lines.join('\n');
  }

  function buildEmptyDiagram() {
    return `flowchart TD
  idle(["● Idle"])
  idle -->|"waiting"| wait["Waiting for task..."]
  classDef default fill:#1a1d27,stroke:#2a2d37,color:#e2e5ec`;
  }

  function sanitizeId(s) {
    return String(s).replace(/[^a-zA-Z0-9_]/g, '_').replace(/^[0-9]/, '_$&');
  }

  function sanitizeLabel(s) {
    return String(s).replace(/"/g, "'").replace(/[<>]/g, '').substring(0, 50);
  }

  function showPlaceholder(containerId = 'mermaid-graph') {
    const el = document.getElementById(containerId);
    if (!el) return;
    el.innerHTML = `
      <div class="graph-placeholder">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="36" height="36">
          <path d="M3 12h18M3 6h18M3 18h18"/>
        </svg>
        <span>Waiting for reasoning events…</span>
      </div>`;
  }

  return {
    init,
    render,
    buildDiagramFromEvents,
    buildEmptyDiagram,
    showPlaceholder,
  };
})();

if (typeof module !== 'undefined' && module.exports) module.exports = MermaidManager;
