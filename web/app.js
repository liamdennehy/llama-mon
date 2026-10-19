// ── State ──────────────────────────────────────────────────────
const charts = {};
let summary = null, models = [], scrapes = [], tasks = [];
let autoRefresh = true;
let refreshTimer = null;
// Track the most recent scrape/task timestamps for incremental updates
let lastScrapeAt = 0, lastTaskAt = 0;

// ── Theme ────────────────────────────────────────────────────
function initTheme() {
  const saved = localStorage.getItem('llama-mon-theme');
  const isLight = saved === 'light';
  document.documentElement.setAttribute('data-theme', isLight ? 'light' : 'dark');
  document.getElementById('themeBtn').textContent = isLight ? '☀️' : '🌙';
}
document.getElementById('themeBtn').addEventListener('click', () => {
  const isLight = document.documentElement.getAttribute('data-theme') === 'light';
  const next = isLight ? 'dark' : 'light';
  document.documentElement.setAttribute('data-theme', next);
  localStorage.setItem('llama-mon-theme', next);
  document.getElementById('themeBtn').textContent = isLight ? '🌙' : '☀️';
  // Re-render charts with new colors
  requestAnimationFrame(() => renderCharts());
});
initTheme();

// ── Helpers ──────────────────────────────────────────────────
function showError(msg) {
  const banner = document.getElementById('errorBanner');
  banner.textContent = msg;
  banner.style.display = 'block';
  document.getElementById('status').className = 'error';
}
function hideError() {
  document.getElementById('errorBanner').style.display = 'none';
  document.getElementById('status').className = 'ok';
}
function esc(s) {
  const d = document.createElement('div');
  d.textContent = s ?? '';
  return d.innerHTML;
}

// ── Data fetching ────────────────────────────────────────────
async function fetchData() {
  const range = document.getElementById('timeRange').value;
  const model = document.getElementById('modelFilter').value;
  // Map frontend values (1h, 6h, 24h, 7d) to Rust enum variants (H1, H6, H24, D7)
  const rangeMap = { '1h': 'H1', '6h': 'H6', '24h': 'H24', '7d': 'D7' };
  const params = new URLSearchParams({ time_range: rangeMap[range] || 'H24', limit: 200 });
  if (model) params.set('model', model);
  // Only use 'since' on non-initial refreshes (when we have data to compare against)
  if (lastScrapeAt > 0) {
    params.set('since', String(lastScrapeAt));
  }

  try {
    const [statusRes, scRes, tRes] = await Promise.all([
      fetch('/api/status'),
      fetch('/api/scrapes?' + params),
      fetch('/api/tasks?' + params),
    ]);
    if (!statusRes.ok || !scRes.ok || !tRes.ok) throw new Error('One or more API calls failed');
    const status = await statusRes.json();
    summary = status.summary;
    models = status.model_loads;
    [scrapes, tasks] = await Promise.all([scRes.json(), tRes.json()]);
    // Update last-seen timestamps for incremental fetches
    if (scrapes.length > 0) {
      lastScrapeAt = Math.max(lastScrapeAt, ...scrapes.map(s => s.scraped_at));
    }
    if (tasks.length > 0) {
      lastTaskAt = Math.max(lastTaskAt, ...tasks.map(t => t.captured_at));
    }
    hideError();
    document.getElementById('status').textContent = 'Updated ' + new Date().toLocaleTimeString();
  } catch (e) {
    showError('Connection failed: ' + e.message);
    return;
  }
  render();
}

// ── Rendering ────────────────────────────────────────────────
function render() {
  if (!summary) return;
  document.getElementById('scrapes').textContent = summary.total_scrapes.toLocaleString();
  document.getElementById('tasks').textContent = summary.total_tasks.toLocaleString();
  document.getElementById('loads').textContent = summary.total_model_loads.toLocaleString();
  document.getElementById('reloads').textContent = summary.total_reloads.toLocaleString();

  const mb = document.getElementById('modelsBody');
  const filterModel = document.getElementById('modelFilter').value;
  const filteredModels = filterModel ? models.filter(m => m.model_name === filterModel) : models;
  mb.innerHTML = filteredModels.map(m =>
    `<tr><td>${esc(m.model_name)}</td>` +
    `<td class="${m.status==='loaded'?'status-loaded':'status-unloaded'}">${m.status}</td>` +
    `<td>${new Date(m.created*1000).toLocaleString()}</td>` +
    `<td>${m.scrape_count}</td>` +
    `<td>${m.ctx_size ? m.ctx_size.toLocaleString() : '—'}</td></tr>`
  ).join('');

  const tb = document.getElementById('tasksBody');
  document.getElementById('taskCount').textContent = `(${tasks.length} shown)`;
  tb.innerHTML = tasks.slice(0, 50).map(t =>
    `<tr><td>${t.task_id}</td><td>${esc(t.model_name)}</td><td>${t.slot_id}</td>` +
    `<td>${t.n_gen}</td><td>${t.gen_tps.toFixed(1)}</td>` +
    `<td>${t.gen_per_token_ms.toFixed(1)}</td><td>${t.n_prompt_cached+t.n_prompt_processed}</td>` +
    `<td>${t.prompt_tps.toFixed(1)}</td>` +
    `<td>${new Date(t.captured_at*1000).toLocaleString()}</td></tr>`
  ).join('');

  // Defer chart rendering to ensure the browser has painted canvas dimensions.
  // On initial load the DOM layout may not be ready by the time fetchData resolves.
  requestAnimationFrame(() => renderCharts());
}

// ── Charts ───────────────────────────────────────────────────
function renderCharts() {
  const labels = scrapes.map(s => new Date(s.scraped_at*1000).toLocaleTimeString());
  const theme = document.documentElement.getAttribute('data-theme');
  const gridColor = theme === 'light' ? '#e8e8e8' : '#30363d';
  const tickColor = theme === 'light' ? '#555' : '#8b949e';
  const fillOpacity = theme === 'light' ? '0.08' : '0.22';

  const chartOpts = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: { legend: { display: false } },
    scales: {
      x: { ticks: { color: tickColor, maxTicksLimit: 10 }, grid: { color: gridColor } },
      y: { ticks: { color: tickColor }, grid: { color: gridColor } }
    }
  };

  // Tasks completed (cumulative)
  if (charts.chartScrapes) charts.chartScrapes.destroy();
  charts.chartScrapes = new Chart(document.getElementById('chartScrapes'), {
    type: 'line',
    data: { labels, datasets: [{ label: 'Completed', data: scrapes.map(s => s.tasks_completed),
              borderColor: '#58a6ff', fill: true, backgroundColor: '#58a6ff'+fillOpacity,
              tension: 0.3, pointRadius: 1 }] },
    options: chartOpts
  });

  // Gen TPS
  const tasksSlice = tasks.slice(0, 100);
  if (charts.chartTps) charts.chartTps.destroy();
  charts.chartTps = new Chart(document.getElementById('chartTps'), {
    type: 'scatter',
    data: { datasets: [{ label: 'Gen TPS', data: labels.slice(0, tasksSlice.length).map((l,i) => ({x:l, y:tasksSlice[i].gen_tps})),
              backgroundColor: '#3fb950', pointRadius: 1.5 }] },
    options: chartOpts
  });

  // Latency
  if (charts.chartLatency) charts.chartLatency.destroy();
  charts.chartLatency = new Chart(document.getElementById('chartLatency'), {
    type: 'scatter',
    data: { datasets: [{ label: 'Gen ms/token', data: labels.slice(0, tasksSlice.length).map((l,i) => ({x:l, y:tasksSlice[i].gen_per_token_ms})),
              backgroundColor: '#d29922', pointRadius: 1.5 }] },
    options: chartOpts
  });

  // Memory
  if (charts.chartMemory) charts.chartMemory.destroy();
  charts.chartMemory = new Chart(document.getElementById('chartMemory'), {
    type: 'line',
    data: { labels, datasets: [{ label: 'Memory', data: scrapes.map(s => +(((s.memory_context_bytes||0)+(s.memory_model_bytes||0))/1e9).toFixed(2)),
              borderColor: '#f85149', fill: true, backgroundColor: '#f85149'+fillOpacity,
              tension: 0.3, pointRadius: 1 }] },
    options: chartOpts
  });

  // Throughput vs Context Length — bubble chart
  renderBubbleChart(gridColor, tickColor);
}

function renderBubbleChart(gridColor, tickColor) {
  const metric = document.getElementById('tpsMetric')?.value || 'gen_tps';
  const limit = 500;
  const data = tasks.slice(0, limit).map(t => {
    const ctxLen = (t.n_prompt_cached || 0) + (t.n_prompt_processed || 0);
    const tps = t[metric] || 0;
    const r = ctxLen > 0 ? Math.max(3, Math.min(20, Math.sqrt(t.n_gen || 1) * 0.7)) : 0;
    return { x: ctxLen, y: tps, r };
  }).filter(d => d.x > 0 && d.r > 0);

  const yLabel = metric === 'gen_tps' ? 'Gen TPS' : 'Prompt TPS';
  const color = metric === 'gen_tps' ? '#3fb950' : '#58a6ff';

  if (charts.chartBubble) charts.chartBubble.destroy();
  charts.chartBubble = new Chart(document.getElementById('chartBubble'), {
    type: 'bubble',
    data: {
      datasets: [{
        label: yLabel,
        data,
        backgroundColor: color + '99',
        borderColor: color,
        borderWidth: 1,
      }]
    },
    options: {
      responsive: true,
      plugins: {
        legend: { display: false },
        tooltip: {
          callbacks: {
            label: function(ctx) {
              const d = ctx.raw;
              return `${yLabel}: ${d.y.toFixed(1)}  Tokens: ${Math.round(d.r*d.r/0.49)}`;
            }
          }
        }
      },
      scales: {
        x: {
          title: { display: true, text: 'Context Length (tokens)', color: tickColor },
          ticks: { color: tickColor },
          grid: { color: gridColor },
          min: 0,
        },
        y: {
          title: { display: true, text: yLabel + ' (tokens/sec)', color: tickColor },
          ticks: { color: tickColor },
          grid: { color: gridColor },
          min: 0,
        }
      }
    }
  });
}

// ── CSV Export ───────────────────────────────────────────────
document.getElementById('exportBtn').addEventListener('click', () => {
  if (scrapes.length === 0) { showError('No scrape data to export'); return; }
  const headers = ['timestamp','scraped_at','model_name','tasks_completed','tasks_duration_s',
    'context_max','prompt_tokens_total','prediction_tokens_total','decode_total',
    'memory_context_bytes','memory_model_bytes'];
  const rows = scrapes.map(s => headers.map(h => {
    if (h === 'timestamp') return new Date(s.scraped_at * 1000).toISOString();
    return s[h] ?? '';
  }).join(','));
  const csv = [headers.join(','), ...rows].join('\n');
  const blob = new Blob([csv], { type: 'text/csv' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url; a.download = `llama-mon-scrapes-${new Date().toISOString().slice(0,10)}.csv`;
  a.click(); URL.revokeObjectURL(url);
});

// ── Controls ─────────────────────────────────────────────────
document.getElementById('refreshBtn').addEventListener('click', () => { fetchData(); });

document.getElementById('autoRefreshBtn').addEventListener('click', function() {
  autoRefresh = !autoRefresh;
  this.classList.toggle('active', autoRefresh);
  this.textContent = autoRefresh ? '●' : '○';
  if (autoRefresh) { startAutoRefresh(); } else { stopAutoRefresh(); }
});

function startAutoRefresh() { stopAutoRefresh(); refreshTimer = setInterval(fetchData, 30000); }
function stopAutoRefresh() { if (refreshTimer) { clearInterval(refreshTimer); refreshTimer = null; } }

document.getElementById('timeRange').addEventListener('change', fetchData);
document.getElementById('modelFilter').addEventListener('change', fetchData);
document.getElementById('tpsMetric')?.addEventListener('change', () => requestAnimationFrame(() => renderCharts()));

// ── Init ─────────────────────────────────────────────────────
async function initModelFilter() {
  try {
    const r = await fetch('/api/status');
    const d = await r.json();
    const sel = document.getElementById('modelFilter');
    // Keep the "All Models" option, clear the rest
    sel.innerHTML = '<option value="">All Models</option>';
    const names = [...new Set(d.model_loads.map(m => m.model_name))];
    names.forEach(m => { const o = document.createElement('option'); o.value = m; o.textContent = m; sel.appendChild(o); });
  } catch { /* models not available yet */ }
}

initModelFilter().then(fetchData);
startAutoRefresh();
