/* Pharus default theme — host detail page (single machine + diagnostics). */
(function () {
  'use strict';

  var P = window.Pharus;
  var t = P.t;

  var hostId = parseInt(new URLSearchParams(location.search).get('id'), 10);
  if (!Number.isFinite(hostId)) {
    window.location.href = 'index.html';
    return;
  }

  /* ---------- DOM ---------- */
  var hostName = document.getElementById('host-name');
  var hostOs = document.getElementById('host-os');
  var hostCard = document.getElementById('host-card');
  var tpl = document.getElementById('card-tpl');
  var adminLink = document.getElementById('admin-link');
  var pingTask = document.getElementById('ping-task');
  var pingRange = document.getElementById('ping-range');
  var pingMessage = document.getElementById('ping-message');
  var pingChart = document.getElementById('ping-chart');
  var pingLegend = document.getElementById('ping-legend');
  var pingLoss = document.getElementById('ping-loss');
  var diagTarget = document.getElementById('diag-target');
  var diagCycles = document.getElementById('diag-cycles');
  var diagPing = document.getElementById('diag-ping');
  var diagTraceroute = document.getElementById('diag-traceroute');
  var diagMtr = document.getElementById('diag-mtr');
  var diagError = document.getElementById('diag-error');
  var diagSessions = document.getElementById('diag-sessions');
  var diagEmpty = document.getElementById('diag-empty');
  var streamingMessage = document.getElementById('streaming-message');
  var streamingResults = document.getElementById('streaming-results');
  var editModeBtn = document.getElementById('edit-mode-btn');
  var tokenModal = document.getElementById('token-modal');
  var tokenInput = document.getElementById('token-input');
  var tokenErr = document.getElementById('token-err');
  var editModal = document.getElementById('edit-modal');
  var editTitle = document.getElementById('edit-title');
  var editErr = document.getElementById('edit-err');
  var fResetDay = document.getElementById('f-reset-day');
  var fQuotaGb = document.getElementById('f-quota-gb');
  var fExpiresOn = document.getElementById('f-expires-on');
  var fPrice = document.getElementById('f-price');
  var fCurrency = document.getElementById('f-currency');
  var fCycle = document.getElementById('f-cycle');
  var addtaskPanel = document.getElementById('host-addtask');
  var atLabel = document.getElementById('at-label');
  var atKind = document.getElementById('at-kind');
  var atTarget = document.getElementById('at-target');
  var atPortField = document.getElementById('at-port-field');
  var atPort = document.getElementById('at-port');
  var atInterval = document.getElementById('at-interval');
  var atSubmit = document.getElementById('at-submit');
  var atStatus = document.getElementById('at-status');

  var GAUGE_LEN = 251.33;
  var CURRENCY_SYMBOL = { CNY: '¥', USD: '$', EUR: '€' };
  var CYCLE_DIVISOR = { monthly: 1, quarterly: 3, yearly: 12 };
  var CHART_COLORS = ['#fbbf24', '#38bdf8', '#a78bfa', '#34d399', '#f87171', '#fb7185'];

  var entry = null;
  var card = null;
  var diagRequests = new Map();
  var pingChartData = { tasks: [], points: [] };
  var pingLoadId = 0;
  var streamingLoadId = 0;
  var adminToken = null;
  var adminOn = false;
  var editingId = null;

  /* ---------- host card ---------- */
  function buildCard() {
    var node = tpl.content.firstElementChild.cloneNode(true);
    hostCard.appendChild(node);
    var card = {
      el: node,
      name: P.field(node, 'name'),
      os: P.field(node, 'os'),
      status: P.field(node, 'status'),
      cpuArc: P.field(node, 'cpuArc'),
      cpuVal: P.field(node, 'cpuVal'),
      memFill: P.field(node, 'memFill'),
      memVal: P.field(node, 'memVal'),
      diskFill: P.field(node, 'diskFill'),
      diskVal: P.field(node, 'diskVal'),
      rx: P.field(node, 'rx'),
      tx: P.field(node, 'tx'),
      load: P.field(node, 'load'),
      uptime: P.field(node, 'uptime'),
      billing: P.field(node, 'billing'),
      trafficVal: P.field(node, 'trafficVal'),
      trafficFill: P.field(node, 'trafficFill'),
      expires: P.field(node, 'expires'),
      price: P.field(node, 'price'),
      resetDay: P.field(node, 'resetDay'),
      region: P.field(node, 'region'),
      pingSection: P.field(node, 'pingSection'),
      pings: P.field(node, 'pings'),
      unlockSection: P.field(node, 'unlockSection'),
      unlock: P.field(node, 'unlock'),
      editBtn: P.field(node, 'editBtn')
    };
    card.editBtn.addEventListener('click', function () { openEdit(hostId); });
    return card;
  }

  function renderRegion() {
    if (!entry.region) {
      card.region.hidden = true;
      card.region.textContent = '';
      return;
    }
    card.region.hidden = false;
    card.region.textContent = entry.region.code || entry.region.name || '—';
    card.region.title = entry.region.name || entry.region.code || '';
  }

  function renderPings() {
    card.pingSection.hidden = !entry.pings || !entry.pings.length;
    card.pings.innerHTML = '';
    if (card.pingSection.hidden) return;
    entry.pings.forEach(function (result) {
      var loss = result.loss == null ? 0 : Math.max(0, Math.min(1, result.loss));
      var value = result.rtt_ms == null
        ? t('ping.unreachable')
        : Number(result.rtt_ms).toFixed(1) + ' ms';
      value += ' · ' + Math.round(loss * 100) + '% ' + t('ping.lossShort');
      P.chip(card.pings, result.label || ('#' + (result.task_id == null ? '—' : result.task_id)), value,
        result.rtt_ms == null || loss > 0.2 ? 'crit' : loss > 0 ? 'warn' : 'ok');
    });
  }

  function renderUnlock() {
    card.unlockSection.hidden = !entry.unlock || !entry.unlock.length;
    card.unlock.innerHTML = '';
    if (card.unlockSection.hidden) return;
    entry.unlock.forEach(function (result) {
      var status = P.serviceStatus(result);
      var label = result.service || result.name || t('streaming.service');
      var unlocked = P.statusClass(status) === 'ok';
      var detail = result.region || result.detail
        || (unlocked && entry.region ? (entry.region.code || entry.region.name) : t('streaming.status.' + status));
      P.chip(card.unlock, label, detail, P.statusClass(status));
    });
  }

  function hasBilling(b) {
    return b && (b.reset_day != null || b.quota_bytes != null || b.expires_at != null || b.price != null);
  }

  function renderHardware() {
    var info = entry.info;
    var panel = document.getElementById('hw-panel');
    var gridEl = document.getElementById('hw-grid');
    panel.hidden = !info;
    gridEl.innerHTML = '';
    if (!info) return;
    var items = [
      [t('host.cpu'), info.cpu_model],
      [t('host.cores'), String(info.cpu_cores)],
      [t('host.memory'), info.mem_desc || (entry.data ? P.fmtBytes(entry.data.mem_total) : '—')],
      [t('host.system'), info.os],
      [t('host.kernel'), info.kernel],
      [t('host.arch'), info.arch]
    ];
    if (info.virtualization) items.push([t('host.virt'), info.virtualization]);
    items.forEach(function (item) {
      var cell = document.createElement('div');
      cell.className = 'hw-item';
      var label = document.createElement('span');
      label.className = 'hw-label';
      label.textContent = item[0];
      var value = document.createElement('span');
      value.className = 'hw-value num';
      value.textContent = item[1] || '—';
      cell.appendChild(label);
      cell.appendChild(value);
      gridEl.appendChild(cell);
    });
  }

  function renderBilling() {
    var b = entry.billing;
    var tr = entry.traffic;
    card.billing.hidden = !(hasBilling(b) || adminOn || (tr && (tr.rx_bytes || tr.tx_bytes)));
    card.editBtn.hidden = !adminOn;
    if (card.billing.hidden) return;
    b = b || {};
    tr = tr || {};

    var used = tr ? (tr.rx_bytes || 0) + (tr.tx_bytes || 0) : 0;
    if (b.quota_bytes != null) {
      card.trafficVal.textContent = P.fmtBytes(used) + ' / ' + P.fmtBytes(b.quota_bytes);
      var p = P.pct(used, b.quota_bytes);
      card.trafficFill.style.width = p.toFixed(1) + '%';
      card.trafficFill.classList.toggle('warn', p > 80 && p <= 95);
      card.trafficFill.classList.toggle('crit', p > 95);
    } else {
      card.trafficVal.textContent = P.fmtBytes(used);
      card.trafficFill.style.width = '0%';
      card.trafficFill.classList.remove('warn');
      card.trafficFill.classList.remove('crit');
    }

    if (b.expires_at != null) {
      var days = Math.ceil((b.expires_at * 1000 - Date.now()) / 86400000);
      if (days < 0) {
        card.expires.textContent = t('billing.expired');
        card.expires.className = 'v crit';
      } else {
        card.expires.textContent = P.fmtDate(b.expires_at) + ' · ' + t('billing.daysLeft').replace('{n}', days);
        card.expires.className = 'v' + (days <= 7 ? ' warn' : '');
      }
    } else {
      card.expires.textContent = '—';
      card.expires.className = 'v';
    }

    if (b.price != null && b.currency) {
      var sym = CURRENCY_SYMBOL[b.currency] || (b.currency + ' ');
      card.price.textContent = sym + P.fmtAmount(b.price) + (b.cycle ? ' · ' + t('billing.cycle.' + b.cycle) : '');
    } else {
      card.price.textContent = '—';
    }
    card.resetDay.textContent = b.reset_day != null ? String(b.reset_day) : '—';
  }

  function setStatus(online) {
    card.status.innerHTML = '';
    var dot = document.createElement('span');
    dot.className = 'dot';
    card.status.appendChild(dot);
    card.status.appendChild(document.createTextNode(online ? t('status.online') : t('status.offline')));
    card.status.classList.toggle('online', online);
    card.el.classList.toggle('is-online', online);
    card.el.classList.toggle('is-offline', !online);
  }

  function renderMetrics() {
    if (!entry.data) return;
    var d = entry.data;
    var cpu = Math.max(0, Math.min(100, d.cpu_usage));
    card.cpuArc.style.strokeDashoffset = (GAUGE_LEN * (1 - cpu / 100)).toFixed(2);
    card.cpuVal.textContent = cpu.toFixed(1) + '%';
    card.memFill.style.width = P.pct(d.mem_used, d.mem_total).toFixed(1) + '%';
    card.memVal.textContent = P.fmtBytes(d.mem_used) + ' / ' + P.fmtBytes(d.mem_total);
    card.diskFill.style.width = P.pct(d.disk_used, d.disk_total).toFixed(1) + '%';
    card.diskVal.textContent = P.fmtBytes(d.disk_used) + ' / ' + P.fmtBytes(d.disk_total);
    card.rx.textContent = P.fmtRate(d.net_rx_bps);
    card.tx.textContent = P.fmtRate(d.net_tx_bps);
    card.load.textContent = d.load1.toFixed(2);
    card.uptime.textContent = P.fmtUptime(d.uptime);
  }

  function renderAll() {
    if (!card) card = buildCard();
    hostName.textContent = entry.name || ('Agent #' + hostId);
    hostOs.textContent = entry.info ? entry.info.os + ' · ' + entry.info.kernel + ' · ' + entry.info.cpu_cores + 'C' : '—';
    setStatus(entry.online);
    renderMetrics();
    renderBilling();
    renderHardware();
    renderRegion();
    renderPings();
    renderUnlock();
    updatePingGate();
    updateDiagGate();
    updateStreamingGate();
  }

  /* ---------- ping history ---------- */
  function updatePingGate() {
    var enabled = !!entry && P.hasFeature(entry, 'ping');
    pingTask.disabled = !enabled;
    pingRange.disabled = !enabled;
    if (entry && !enabled) {
      pingMessage.textContent = t('feature.disabled').replace('{feature}', t('feature.ping'));
      pingChartData = { tasks: [], points: [] };
      drawPingChart();
    }
  }

  function populatePingTasks(tasks, selected) {
    pingTask.innerHTML = '';
    var all = document.createElement('option');
    all.value = '';
    all.textContent = t('ping.allTasks');
    pingTask.appendChild(all);
    (tasks || []).forEach(function (task) {
      var option = document.createElement('option');
      option.value = task.id;
      option.textContent = task.label || (task.kind + ' · ' + task.target);
      pingTask.appendChild(option);
    });
    if (selected && Array.from(pingTask.options).some(function (option) { return option.value === selected; })) {
      pingTask.value = selected;
    }
  }

  function loadPingHistory(resetTask) {
    if (!entry || !P.hasFeature(entry, 'ping')) {
      updatePingGate();
      return;
    }
    if (resetTask) pingTask.value = '';
    var selectedTask = pingTask.value;
    var seq = ++pingLoadId;
    var url = '/api/agents/' + hostId + '/ping?range=' + encodeURIComponent(pingRange.value);
    if (selectedTask) url += '&task_id=' + encodeURIComponent(selectedTask);
    pingMessage.textContent = t('common.loading');
    P.requestJson(url).then(function (body) {
      if (seq !== pingLoadId) return;
      pingChartData = {
        tasks: Array.isArray(body.tasks) ? body.tasks : [],
        points: Array.isArray(body.points) ? body.points : []
      };
      populatePingTasks(pingChartData.tasks, selectedTask);
      pingMessage.textContent = pingChartData.points.length ? '' : t('ping.noData');
      drawPingChart();
    }).catch(function (error) {
      if (seq !== pingLoadId) return;
      pingChartData = { tasks: [], points: [] };
      pingMessage.textContent = t('common.error') + ': ' + error.message;
      drawPingChart();
    });
  }

  function formatChartTime(ts) {
    var date = new Date(ts * 1000);
    if (pingRange.value === '7d') {
      return (date.getMonth() + 1) + '/' + date.getDate();
    }
    var h = date.getHours();
    var m = date.getMinutes();
    return (h < 10 ? '0' : '') + h + ':' + (m < 10 ? '0' : '') + m;
  }

  function drawPingChart() {
    var rect = pingChart.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    var ratio = window.devicePixelRatio || 1;
    pingChart.width = Math.round(rect.width * ratio);
    pingChart.height = Math.round(rect.height * ratio);
    var ctx = pingChart.getContext('2d');
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    ctx.clearRect(0, 0, rect.width, rect.height);
    pingLegend.innerHTML = '';
    pingLoss.textContent = '';

    var points = pingChartData.points.filter(function (point) {
      return Number.isFinite(Number(point.ts));
    });
    if (!points.length) return;
    var tasks = pingChartData.tasks.slice();
    if (!tasks.length) {
      var seen = {};
      points.forEach(function (point) {
        if (!seen[point.task_id]) {
          seen[point.task_id] = true;
          tasks.push({ id: point.task_id, label: '#' + point.task_id });
        }
      });
    }
    var minTs = Math.min.apply(null, points.map(function (point) { return Number(point.ts); }));
    var maxTs = Math.max.apply(null, points.map(function (point) { return Number(point.ts); }));
    if (minTs === maxTs) maxTs = minTs + 1;
    var rtts = points.map(function (point) { return Number(point.rtt_avg); }).filter(Number.isFinite);
    var maxRtt = rtts.length ? Math.max.apply(null, rtts) : 1;
    maxRtt = Math.max(10, Math.ceil(maxRtt / 10) * 10);
    var left = 42, right = 12, top = 12, plotBottom = rect.height - 52;
    var plotWidth = Math.max(1, rect.width - left - right);
    var plotHeight = Math.max(1, plotBottom - top);
    var xAt = function (ts) { return left + (Number(ts) - minTs) / (maxTs - minTs) * plotWidth; };
    var yAt = function (rtt) { return top + (1 - Number(rtt) / maxRtt) * plotHeight; };

    ctx.lineWidth = 1;
    ctx.font = '10px ' + getComputedStyle(document.documentElement).getPropertyValue('--font-mono');
    ctx.textBaseline = 'middle';
    for (var line = 0; line <= 4; line++) {
      var y = top + plotHeight * line / 4;
      ctx.strokeStyle = 'rgba(148, 163, 184, 0.10)';
      ctx.beginPath();
      ctx.moveTo(left, y);
      ctx.lineTo(rect.width - right, y);
      ctx.stroke();
      ctx.fillStyle = '#64748b';
      ctx.textAlign = 'right';
      ctx.fillText(Math.round(maxRtt * (1 - line / 4)) + ' ms', left - 6, y);
    }
    ctx.fillStyle = '#64748b';
    ctx.textBaseline = 'alphabetic';
    ctx.textAlign = 'left';
    ctx.fillText(formatChartTime(minTs), left, rect.height - 5);
    ctx.textAlign = 'right';
    ctx.fillText(formatChartTime(maxTs), rect.width - right, rect.height - 5);
    ctx.textAlign = 'left';
    ctx.fillText(t('ping.lossAxis'), left, rect.height - 30);

    var summaries = [];
    tasks.forEach(function (task, taskIndex) {
      var color = CHART_COLORS[taskIndex % CHART_COLORS.length];
      var series = points.filter(function (point) { return String(point.task_id) === String(task.id); })
        .sort(function (a, b) { return Number(a.ts) - Number(b.ts); });
      if (!series.length) return;
      ctx.strokeStyle = color;
      ctx.lineWidth = 1.7;
      ctx.beginPath();
      var drawing = false;
      series.forEach(function (point) {
        if (!Number.isFinite(Number(point.rtt_avg))) {
          drawing = false;
          return;
        }
        var x = xAt(point.ts), y = yAt(point.rtt_avg);
        if (!drawing) { ctx.moveTo(x, y); drawing = true; } else { ctx.lineTo(x, y); }
      });
      ctx.stroke();
      series.forEach(function (point) {
        var loss = Math.max(0, Math.min(1, Number(point.loss) || 0));
        if (!loss) return;
        var x = xAt(point.ts);
        ctx.fillStyle = loss >= 0.5 ? 'rgba(248, 113, 113, 0.85)' : 'rgba(251, 191, 36, 0.75)';
        ctx.fillRect(x - 1.5, rect.height - 20 - loss * 14, 3, Math.max(2, loss * 14));
      });
      var legend = document.createElement('span');
      legend.className = 'legend-item';
      var swatch = document.createElement('span');
      swatch.className = 'legend-swatch';
      swatch.style.background = color;
      var label = document.createElement('span');
      label.textContent = task.label || ('#' + task.id);
      legend.appendChild(swatch);
      legend.appendChild(label);
      pingLegend.appendChild(legend);
      var averageLoss = series.reduce(function (sum, point) { return sum + (Number(point.loss) || 0); }, 0) / series.length;
      summaries.push((task.label || ('#' + task.id)) + ': ' + (averageLoss * 100).toFixed(1) + '%');
    });
    pingLoss.textContent = t('ping.avgLoss') + ' · ' + summaries.join('  |  ');
  }

  /* ---------- diagnostics ---------- */
  function updateDiagGate() {
    var online = !!entry && !!entry.online;
    var lg = !!entry && P.hasFeature(entry, 'lg');
    var mtr = !!entry && P.hasFeature(entry, 'mtr');
    diagPing.hidden = !lg;
    diagTraceroute.hidden = !lg;
    diagMtr.hidden = !mtr;
    diagPing.disabled = !online;
    diagTraceroute.disabled = !online;
    diagMtr.disabled = !online;
    document.querySelector('.cycles-field').hidden = !mtr;
    if (entry && !online) {
      diagError.textContent = t('diag.offline');
      diagError.hidden = false;
    } else if (entry && !lg && !mtr) {
      diagError.textContent = t('feature.disabled').replace('{feature}', t('feature.diagnostics'));
      diagError.hidden = false;
    } else {
      diagError.hidden = true;
    }
  }

  function diagnosticLabel(kind) {
    return kind === 'traceroute' ? t('diag.traceroute') : kind === 'mtr' ? t('diag.mtr') : t('diag.ping');
  }

  function setDiagnosticState(request, stateName, exitCode) {
    request.state.className = 'run-state ' + stateName;
    var label = t('diag.state.' + stateName);
    if (exitCode != null) label += ' · ' + t('diag.exitCode').replace('{code}', exitCode);
    request.state.textContent = label;
  }

  function createDiagnosticSession(requestId, agentId, kind, target) {
    if (diagRequests.has(requestId)) return diagRequests.get(requestId);
    var shell = document.createElement('article');
    shell.className = 'diag-session';
    var head = document.createElement('div');
    head.className = 'terminal-head';
    var title = document.createElement('span');
    title.className = 'terminal-title';
    title.textContent = P.entryName(entry, hostId) + ' · ' + diagnosticLabel(kind) + ' ' + target;
    var requestLabel = document.createElement('span');
    requestLabel.className = 'terminal-id';
    requestLabel.textContent = '#' + String(requestId).slice(0, 8);
    requestLabel.title = String(requestId);
    var runState = document.createElement('span');
    var output = document.createElement('pre');
    output.className = 'terminal-output';
    head.appendChild(title);
    head.appendChild(requestLabel);
    head.appendChild(runState);
    shell.appendChild(head);
    shell.appendChild(output);
    diagSessions.insertBefore(shell, diagSessions.firstChild);
    diagEmpty.hidden = true;
    var request = { id: requestId, agentId: agentId, kind: kind, target: target, el: shell, state: runState, output: output };
    diagRequests.set(requestId, request);
    setDiagnosticState(request, 'running', null);
    return request;
  }

  function appendDiagnosticPart(request, text, className) {
    if (text == null || text === '') return;
    var part = document.createElement('span');
    part.className = className || '';
    part.textContent = String(text);
    request.output.appendChild(part);
    request.output.scrollTop = request.output.scrollHeight;
  }

  function handleDiagnosticFrame(message) {
    // Frames are broadcast to every browser. Only locally-originated IDs are trusted for display.
    var request = diagRequests.get(message.request_id);
    if (!request) return;
    if (message.data != null) appendDiagnosticPart(request, message.data, message.stream === 'stderr' ? 'stderr' : '');
    if (message.result != null) {
      appendDiagnosticPart(request, JSON.stringify(message.result, null, 2) + '\n', 'structured');
    }
    if (message.done) {
      setDiagnosticState(request, message.exit_code == null || message.exit_code === 0 ? 'finished' : 'failed', message.exit_code);
    }
  }

  function runDiagnostic(kind) {
    var required = kind === 'mtr' ? 'mtr' : 'lg';
    var target = diagTarget.value.trim();
    diagError.hidden = true;
    if (!entry || !entry.online) {
      diagError.textContent = t('diag.offline');
      diagError.hidden = false;
      return;
    }
    if (!P.hasFeature(entry, required)) {
      diagError.textContent = t('feature.disabled').replace('{feature}', t('feature.' + required));
      diagError.hidden = false;
      return;
    }
    if (!target) {
      diagError.textContent = t('diag.targetRequired');
      diagError.hidden = false;
      diagTarget.focus();
      return;
    }
    var body = { agent_id: hostId, target: target };
    var url = '/api/diag/lg';
    if (kind === 'mtr') {
      url = '/api/diag/mtr';
      body.cycles = Math.max(1, Math.min(100, parseInt(diagCycles.value, 10) || 10));
    } else {
      body.kind = kind;
    }
    P.requestJson(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body)
    }).then(function (response) {
      if (!response.request_id) throw new Error(t('diag.missingRequestId'));
      createDiagnosticSession(response.request_id, hostId, kind, target);
    }).catch(function (error) {
      diagError.textContent = t('common.error') + ': ' + error.message;
      diagError.hidden = false;
    });
  }

  /* ---------- streaming service results ---------- */
  function updateStreamingGate() {
    var enabled = !!entry && P.hasFeature(entry, 'streaming');
    if (entry && !enabled) {
      streamingMessage.textContent = t('feature.disabled').replace('{feature}', t('feature.streaming'));
      streamingResults.innerHTML = '';
    }
  }

  function renderServiceResults(results) {
    streamingResults.innerHTML = '';
    (results || []).forEach(function (result) {
      var status = P.serviceStatus(result);
      var cardEl = document.createElement('article');
      cardEl.className = 'service-result ' + P.statusClass(status);
      var head = document.createElement('div');
      head.className = 'service-result-head';
      var name = document.createElement('span');
      name.className = 'service-result-name';
      name.textContent = result.service || result.name || t('streaming.service');
      var statusEl = document.createElement('span');
      statusEl.className = 'service-result-status';
      statusEl.textContent = t('streaming.status.' + status);
      head.appendChild(name);
      head.appendChild(statusEl);
      cardEl.appendChild(head);
      var unlocked = P.statusClass(status) === 'ok';
      var detailText = result.region || result.detail
        || (unlocked && entry.region ? (entry.region.code || entry.region.name) : null);
      if (detailText) {
        var detail = document.createElement('p');
        detail.textContent = detailText;
        cardEl.appendChild(detail);
      }
      streamingResults.appendChild(cardEl);
    });
    if (!results || !results.length) streamingMessage.textContent = t('streaming.noData');
  }

  function loadStreaming() {
    if (!entry || !P.hasFeature(entry, 'streaming')) {
      updateStreamingGate();
      return;
    }
    var seq = ++streamingLoadId;
    streamingMessage.textContent = t('common.loading');
    P.requestJson('/api/agents/' + hostId + '/streaming').then(function (body) {
      if (seq !== streamingLoadId) return;
      var results = Array.isArray(body.results) ? body.results : [];
      entry.unlock = results;
      renderUnlock();
      streamingMessage.textContent = '';
      renderServiceResults(results);
    }).catch(function (error) {
      if (seq !== streamingLoadId) return;
      streamingResults.innerHTML = '';
      streamingMessage.textContent = t('common.error') + ': ' + error.message;
    });
  }

  /* ---------- ws ---------- */
  function handleMessage(msg) {
    if (msg.type === 'snapshot') {
      var found = null;
      msg.agents.forEach(function (a) {
        if (a.agent_id === hostId) {
          found = {
            agent_id: a.agent_id,
            online: a.online,
            data: a.data,
            name: a.name,
            info: a.info,
            billing: a.billing || null,
            traffic: a.traffic || null,
            pings: Array.isArray(a.pings) ? a.pings : [],
            unlock: Array.isArray(a.unlock) ? a.unlock : [],
            region: a.region || null,
            features: Array.isArray(a.features) ? a.features.slice() : null
          };
        }
      });
      if (found) {
        entry = found;
        renderAll();
        loadPingHistory(false);
        loadStreaming();
      } else {
        hostName.textContent = 'Agent #' + hostId;
      }
    } else if (msg.type === 'metrics' && msg.agent_id === hostId) {
      if (!entry) entry = { agent_id: hostId };
      entry.agent_id = hostId;
      entry.online = msg.online;
      entry.data = msg.data;
      renderAll();
    } else if (msg.type === 'status' && msg.agent_id === hostId) {
      if (!entry) entry = { agent_id: hostId };
      entry.agent_id = hostId;
      entry.online = msg.online;
      renderAll();
    } else if (msg.type === 'billing' && msg.agent_id === hostId) {
      if (!entry) entry = { agent_id: hostId };
      entry.billing = msg.billing;
      entry.traffic = msg.traffic;
      renderAll();
    } else if (msg.type === 'pings' && msg.agent_id === hostId) {
      if (!entry) entry = { agent_id: hostId };
      entry.pings = Array.isArray(msg.results) ? msg.results : [];
      renderAll();
    } else if (msg.type === 'unlock' && msg.agent_id === hostId) {
      if (!entry) entry = { agent_id: hostId };
      entry.unlock = Array.isArray(msg.results) ? msg.results : [];
      renderAll();
      if (entry.unlock.length) {
        streamingMessage.textContent = '';
        renderServiceResults(entry.unlock);
      }
    } else if (msg.type === 'diag_result') {
      handleDiagnosticFrame(msg);
    } else if (msg.type === 'features_update' && msg.agent_id === hostId) {
      if (!entry) entry = { agent_id: hostId };
      entry.features = Array.isArray(msg.features) ? msg.features.slice() : [];
      renderAll();
    } else if (msg.type === 'region_update' && msg.agent_id === hostId) {
      if (!entry) entry = { agent_id: hostId };
      entry.region = msg.region || null;
      renderAll();
    }
  }

  function checkToken(tok) {
    return fetch('/api/admin/check', {
      method: 'POST',
      headers: tok ? { Authorization: 'Bearer ' + tok } : {}
    }).then(function (r) { return r.status; }).catch(function () { return 0; });
  }

  function setAdmin(on) {
    adminOn = on;
    editModeBtn.classList.toggle('active', on);
    addtaskPanel.hidden = !on;
    if (card) renderBilling();
  }

  function handleUnauthorized() {
    sessionStorage.removeItem('pharus.admin');
    adminToken = null;
    setAdmin(false);
    tokenErr.textContent = t('admin.unauthorized');
    tokenErr.hidden = false;
    tokenModal.hidden = false;
  }

  function openEdit(id) {
    var b = entry.billing || {};
    editingId = id;
    editTitle.textContent = entry.name || ('Agent #' + id);
    fResetDay.value = b.reset_day != null ? b.reset_day : '';
    fQuotaGb.value = b.quota_bytes != null ? Math.round(b.quota_bytes / 1073741824 * 100) / 100 : '';
    fExpiresOn.value = b.expires_at != null ? P.fmtDate(b.expires_at) : '';
    fPrice.value = b.price != null ? b.price : '';
    fCurrency.value = b.currency || '';
    fCycle.value = b.cycle || '';
    editErr.hidden = true;
    editModal.hidden = false;
  }

  function saveEdit() {
    var body = {
      reset_day: fResetDay.value === '' ? null : parseInt(fResetDay.value, 10),
      quota_gb: fQuotaGb.value === '' ? null : parseFloat(fQuotaGb.value),
      expires_on: fExpiresOn.value === '' ? null : fExpiresOn.value,
      price: fPrice.value === '' ? null : parseFloat(fPrice.value),
      currency: fCurrency.value || null,
      cycle: fCycle.value || null
    };
    fetch('/api/admin/agents/' + editingId + '/billing', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + adminToken },
      body: JSON.stringify(body)
    }).then(function (r) {
      if (r.status === 401) throw 'unauthorized';
      if (!r.ok) return r.json().then(function (j) { throw (j && j.error) || ('HTTP ' + r.status); });
      return r.json();
    }).then(function (res) {
      entry.billing = res.billing;
      entry.traffic = res.traffic;
      renderBilling();
      editModal.hidden = true;
    }).catch(function (e) {
      if (e === 'unauthorized') handleUnauthorized();
      else {
        editErr.textContent = t('admin.error') + ': ' + e;
        editErr.hidden = false;
      }
    });
  }

  function addPingTask() {
    var label = atLabel.value.trim();
    var target = atTarget.value.trim();
    if (!label || !target) {
      atStatus.textContent = t('common.error');
      atStatus.className = 'inline-status error';
      return;
    }
    var body = {
      label: label,
      kind: atKind.value,
      target: target,
      port: atKind.value === 'icmp' ? null : (atPort.value ? Number(atPort.value) : null),
      interval_sec: Number(atInterval.value) || 60,
      probe_count: 4,
      enabled: true,
      agent_ids: [hostId]
    };
    atStatus.textContent = t('common.saving');
    atStatus.className = 'inline-status';
    atSubmit.disabled = true;
    fetch('/api/admin/ping-tasks', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + adminToken },
      body: JSON.stringify(body)
    }).then(function (r) {
      if (r.status === 401) throw 'unauthorized';
      if (!r.ok) return r.json().then(function (j) { throw (j && j.error) || ('HTTP ' + r.status); });
      return r.json();
    }).then(function () {
      atStatus.textContent = t('common.saved');
      atStatus.className = 'inline-status ok';
      atLabel.value = '';
      atTarget.value = '';
    }).catch(function (e) {
      if (e === 'unauthorized') handleUnauthorized();
      else {
        atStatus.textContent = t('common.error') + ': ' + (e && e.message ? e.message : e);
        atStatus.className = 'inline-status error';
      }
    }).then(function () { atSubmit.disabled = false; });
  }

  function probeAdminLink() {
    P.requestJson('/api/meta').then(function (meta) {
      if (!meta.admin_enabled) return;
      adminLink.hidden = false;
      editModeBtn.hidden = false;
      var saved = sessionStorage.getItem('pharus.admin');
      if (!saved) return;
      checkToken(saved).then(function (st) {
        if (st === 200) {
          adminToken = saved;
          setAdmin(true);
        } else {
          sessionStorage.removeItem('pharus.admin');
        }
      });
    }).catch(function () {});
  }

  /* ---------- events ---------- */
  pingTask.addEventListener('change', function () { loadPingHistory(false); });
  pingRange.addEventListener('change', function () { loadPingHistory(false); });
  diagPing.addEventListener('click', function () { runDiagnostic('ping'); });
  diagTraceroute.addEventListener('click', function () { runDiagnostic('traceroute'); });
  diagMtr.addEventListener('click', function () { runDiagnostic('mtr'); });
  editModeBtn.addEventListener('click', function () {
    if (adminOn) {
      setAdmin(false);
    } else if (adminToken) {
      setAdmin(true);
    } else {
      tokenErr.hidden = true;
      tokenInput.value = '';
      tokenModal.hidden = false;
      tokenInput.focus();
    }
  });
  document.getElementById('token-submit').addEventListener('click', function () {
    var tok = tokenInput.value.trim();
    if (!tok) return;
    checkToken(tok).then(function (st) {
      if (st === 200) {
        adminToken = tok;
        sessionStorage.setItem('pharus.admin', tok);
        tokenModal.hidden = true;
        setAdmin(true);
      } else {
        tokenErr.textContent = t('admin.unauthorized');
        tokenErr.hidden = false;
      }
    });
  });
  document.getElementById('token-cancel').addEventListener('click', function () {
    tokenModal.hidden = true;
  });
  document.getElementById('edit-save').addEventListener('click', saveEdit);
  document.getElementById('edit-cancel').addEventListener('click', function () {
    editModal.hidden = true;
  });
  atSubmit.addEventListener('click', addPingTask);
  atKind.addEventListener('change', function () {
    atPortField.hidden = atKind.value === 'icmp';
  });
  [tokenModal, editModal].forEach(function (mask) {
    mask.addEventListener('click', function (ev) {
      if (ev.target === mask || ev.target.hasAttribute('data-close')) mask.hidden = true;
    });
  });
  document.addEventListener('keydown', function (ev) {
    if (ev.key === 'Escape') {
      tokenModal.hidden = true;
      editModal.hidden = true;
    }
  });
  var chartResizePending = false;
  window.addEventListener('resize', function () {
    if (chartResizePending) return;
    chartResizePending = true;
    requestAnimationFrame(function () { chartResizePending = false; drawPingChart(); });
  });

  /* ---------- boot ---------- */
  P.ready().then(function () {
    P.initTheme();
    P.connectStream(handleMessage);
    probeAdminLink();
  });
})();
