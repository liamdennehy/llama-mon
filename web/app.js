// ── State ──────────────────────────────────────────────────────
const charts = {};
let summary = null, models = [], scrapes = [], tasks = [];
let autoRefresh = true;
let fetchAll = false;
let taskCountLimit = 200;
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
function formatBytes(bytes, decimals = 1) {
  if (!bytes || bytes === 0) return '—';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  // No decimal places for Bytes; use 1 for all others
  const dec = i === 0 ? 0 : decimals;
  return (bytes / Math.pow(1024, i)).toFixed(dec) + ' ' + units[i];
}

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

// ── Incremental merge helpers ──────────────────────────────────
function mergeRecords(existing, newRecords, keyField) {
  const existingMap = new Map(existing.map(r => [r[keyField], r]));
  for (const rec of newRecords) {
    existingMap.set(rec[keyField], rec);
  }
  // Return sorted by the timestamp field for chronological order
  return [...existingMap.values()];
}

function mergeScrapes(existing, newScrapes) {
  return mergeRecords(existing, newScrapes, 'id');
}

function mergeTasks(existing, newTasks) {
  return mergeRecords(existing, newTasks, 'task_id');
}

// ── Data fetching ────────────────────────────────────────────
async function fetchData() {
  const range = document.getElementById('timeRange').value;
  const model = document.getElementById('modelFilter').value;
  // Map frontend values (1h, 6h, 24h, 7d) to Rust enum variants (H1, H6, H24, D7)
  const rangeMap = { '1h': 'H1', '6h': 'H6', '24h': 'H24', '7d': 'D7' };
  const params = new URLSearchParams({ time_range: rangeMap[range] || 'H24' });
  if (model) params.set('model', model);
  // Only use 'since' on non-initial, non-full-refreshes
  if (lastScrapeAt > 0 && !fetchAll) {
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
    const [newScrapes, newTasks] = await Promise.all([scRes.json(), tRes.json()]);

    if (fetchAll) {
      // One-shot full fetch: replace everything, reset timestamps
      scrapes = newScrapes;
      tasks = newTasks;
      lastScrapeAt = newScrapes.length > 0 ? Math.max(...newScrapes.map(s => s.scraped_at)) : 0;
      lastTaskAt = newTasks.length > 0 ? Math.max(...newTasks.map(t => t.captured_at)) : 0;
    } else {
      // Incremental: merge new records onto existing
      scrapes = mergeScrapes(scrapes, newScrapes);
      tasks = mergeTasks(tasks, newTasks);
      // Update last-seen timestamps if we got new data
      if (newScrapes.length > 0) {
        lastScrapeAt = Math.max(lastScrapeAt, ...newScrapes.map(s => s.scraped_at));
      }
      if (newTasks.length > 0) {
        lastTaskAt = Math.max(lastTaskAt, ...newTasks.map(t => t.captured_at));
      }
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
  mb.innerHTML = filteredModels.map(m => {
    const modelVram = m.memory_model_bytes || 0;
    return `<tr><td>${esc(m.model_name)}</td>` +
    `<td class="${m.status==='loaded'?'status-loaded':'status-unloaded'}">${m.status}</td>` +
    `<td>${new Date(m.created*1000).toLocaleString()}</td>` +
    `<td>${m.scrape_count ?? '—'}</td>` +
    `<td>${m.ctx_size ? m.ctx_size.toLocaleString() : '—'}</td>` +
    `<td>${m.mtp ? 'Yes' : '—'}</td>` +
    `<td>${formatBytes(modelVram)}</td></tr>`;
  }).join('');

  const tb = document.getElementById('tasksBody');
  document.getElementById('taskCount').textContent = `(${tasks.length} shown)`;
  tb.innerHTML = tasks.slice(0, taskCountLimit).map(t => {
    const cached = t.n_prompt_cached || 0;
    const newTokens = t.n_prompt_processed || 0;
    const totalPrompt = cached + newTokens;
    const kvCache = totalPrompt > 0 ? ((cached / totalPrompt) * 100).toFixed(1) + '%' : '—';
    return `<tr><td>${t.task_id}</td><td>${esc(t.model_name)}</td><td>${t.slot_id}</td>` +
    `<td>${t.n_gen}</td><td>${t.gen_tps.toFixed(1)}</td>` +
    `<td>${t.gen_per_token_ms.toFixed(1)}</td>` +
    `<td>${cached.toLocaleString()}</td>` +
    `<td>${newTokens.toLocaleString()}</td>` +
    `<td>${t.prompt_tps.toFixed(1)}</td>` +
    `<td>${kvCache}</td>` +
    `<td>${new Date(t.captured_at*1000).toLocaleString()}</td></tr>`;
  }).join('');

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

  // Task throughput (tasks/min) — compute delta between consecutive scrapes
  if (charts.chartScrapes) charts.chartScrapes.destroy();
  const throughputData = scrapes.map((s, i) => {
    const tasks = s.tasks_completed || 0;
    if (i === 0 || !scrapes[i - 1]) {
      return { x: s.scraped_at, y: 0 };
    }
    const prev = scrapes[i - 1];
    const dtMin = (s.scraped_at - prev.scraped_at) / 60;
    const throughput = dtMin > 0 ? (tasks - (prev.tasks_completed || 0)) / dtMin : 0;
    return { x: s.scraped_at, y: Math.max(0, throughput) };
  });
  const throughputLabels = scrapes.map(s => new Date(s.scraped_at * 1000).toLocaleTimeString());
  charts.chartScrapes = new Chart(document.getElementById('chartScrapes'), {
    type: 'line',
    data: { labels: throughputLabels, datasets: [{ label: 'Tasks/min', data: throughputData.map(d => d.y),
              borderColor: '#58a6ff', fill: true, backgroundColor: '#58a6ff'+fillOpacity,
              tension: 0.3, pointRadius: 1 }] },
    options: {
      ...chartOpts,
      scales: {
        ...chartOpts.scales,
        y: { ...chartOpts.scales.y, beginAtZero: true,
             title: { display: true, text: 'Tasks/min', color: tickColor }
           }
      }
    }
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

  // Context miss rate (misses/min) — log scale to show detail across wide range
  // x-axis is linear epoch time, formatted via tick callback
  if (charts.chartCacheMiss) charts.chartCacheMiss.destroy();
  const cacheMissData = scrapes.map((s, i) => {
    const total = s.prompt_tokens_total || 0;
    const cached = s.prompt_tokens_cached || 0;
    const misses = Math.max(0, total - cached);
    if (i === 0 || !scrapes[i - 1]) {
      return { x: s.scraped_at, y: 0 };
    }
    const prev = scrapes[i - 1];
    const dtMin = (s.scraped_at - prev.scraped_at) / 60;
    const prevMisses = Math.max(0, (prev.prompt_tokens_total || 0) - (prev.prompt_tokens_cached || 0));
    const rate = dtMin > 0 ? (misses - prevMisses) / dtMin : 0;
    return { x: s.scraped_at, y: Math.max(0, rate) };
  });
  charts.chartCacheMiss = new Chart(document.getElementById('chartCacheMiss'), {
    type: 'scatter',
    data: { datasets: [{ label: 'Miss/min', data: cacheMissData,
              backgroundColor: '#f85149', pointRadius: 1 }] },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: { legend: { display: false } },
      scales: {
        x: {
          type: 'linear',
          min: scrapes.length > 0 ? Math.min(...scrapes.map(s => s.scraped_at)) : 0,
          max: scrapes.length > 0 ? Math.max(...scrapes.map(s => s.scraped_at)) : 0,
          ticks: {
            color: tickColor,
            maxTicksLimit: 10,
            callback: function(value) {
              return new Date(value * 1000).toLocaleTimeString();
            }
          },
          grid: { color: gridColor },
          title: { display: true, text: 'Time', color: tickColor }
        },
        y: {
          type: 'logarithmic',
          ticks: { color: tickColor },
          grid: { color: gridColor },
          title: { display: true, text: 'Miss/min (log)', color: tickColor }
        }
      }
    }
  });

  // Throughput vs Context Length — bubble chart
  renderBubbleChart(gridColor, tickColor);
}

function renderBubbleChart(gridColor, tickColor) {
  const metric = document.getElementById('tpsMetric')?.value || 'gen_tps';
  const limit = 500;
  const isGen = metric === 'gen_tps';
  const data = tasks.slice(0, limit).map(t => {
    const ctxLen = (t.n_prompt_cached || 0) + (t.n_prompt_processed || 0);
    const tps = t[metric] || 0;
    // Use the relevant token count for bubble size
    const tokenCount = isGen ? (t.n_gen || 0) : ((t.n_prompt_cached || 0) + (t.n_prompt_processed || 0));
    const r = tokenCount > 0 ? Math.max(3, Math.min(20, Math.sqrt(tokenCount) * 0.7)) : 0;
    return { x: ctxLen, y: tps, r };
  }).filter(d => d.x > 0 && d.r > 0);

  const yLabel = isGen ? 'Gen TPS' : 'Prompt TPS';
  const tokenLabel = isGen ? 'Gen' : 'Prompt';
  const color = isGen ? '#3fb950' : '#58a6ff';

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
      layout: { padding: { right: 70 } },
      plugins: {
        legend: { display: false },
        tooltip: {
          callbacks: {
            label: function(ctx) {
              const d = ctx.raw;
              const tokens = Math.round(d.r * d.r / 0.49);
              return `${yLabel}: ${d.y.toFixed(1)}  ${tokenLabel} tokens: ${tokens}`;
            }
          }
        },
        // Custom plugin: draw bubble size legend on the right side
        _bubbleLegend: {
          beforeDraw: function(chart) {
            const ctx = chart.ctx;
            const area = chart.chartArea;
            if (!area) return;
            const { top, right, bottom } = area;
            const x = right + 10;
            ctx.save();
            ctx.fillStyle = tickColor;
            ctx.font = '11px sans-serif';
            ctx.textAlign = 'left';
            ctx.textBaseline = 'middle';
            const step = Math.max(20, (bottom - top) / (legendSizes.length + 1));
            legendSizes.forEach((count, i) => {
              const cy = top + (i + 1) * step;
              const radius = count > 0 ? Math.max(3, Math.min(20, Math.sqrt(count) * 0.7)) : 0;
              ctx.beginPath();
              ctx.arc(x, cy, radius, 0, Math.PI * 2);
              ctx.fillStyle = color + '99';
              ctx.fill();
              ctx.strokeStyle = color;
              ctx.lineWidth = 1;
              ctx.stroke();
              ctx.fillStyle = tickColor;
              ctx.fillText(count + ' tokens', x + radius + 6, cy);
            });
            ctx.restore();
          }
        },
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

document.getElementById('fetchAllBtn').addEventListener('click', () => {
  fetchAll = !fetchAll;
  document.getElementById('fetchAllBtn').classList.toggle('active', fetchAll);
  fetchData();
});

// Changing time range or model filter invalidates the incremental baseline —
// reset timestamps so the next fetch gets a full dataset for the new scope.
document.getElementById('timeRange').addEventListener('change', () => {
  lastScrapeAt = 0;
  lastTaskAt = 0;
  fetchData();
});
document.getElementById('modelFilter').addEventListener('change', () => {
  lastScrapeAt = 0;
  lastTaskAt = 0;
  fetchData();
});
document.getElementById('tpsMetric')?.addEventListener('change', () => requestAnimationFrame(() => renderCharts()));

document.getElementById('taskCountLimit').addEventListener('change', (e) => {
  taskCountLimit = parseInt(e.target.value, 10);
  render();
});

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
