/* Pharus default theme — host detail page (single machine + diagnostics). */
(function () {
  'use strict';

  var P = window.Pharus;
  var t = P.t;

  var hostId = parseInt(new URLSearchParams(location.search).get('id'), 10);
  if (!Number.isFinite(hostId)) {
    window.location.href = '/';
    return;
  }

  /* ---------- DOM ---------- */
  var hostCard = document.getElementById('host-card');
  var tpl = document.getElementById('card-tpl');
  var adminLink = document.getElementById('admin-link');
  var pingTask = document.getElementById('ping-task');
  var pingRange = document.getElementById('ping-range');
  var pingMessage = document.getElementById('ping-message');
  var pingChart = document.getElementById('ping-chart');
  var pingLegend = document.getElementById('ping-legend');
  var pingLoss = document.getElementById('ping-loss');
  var pingTip = document.getElementById('ping-tip');
  var pingGuide = document.getElementById('ping-guide');
  var metricsRange = document.getElementById('metrics-range');
  var metricsMessage = document.getElementById('metrics-message');
  var metricTiles = document.getElementById('metric-tiles');
  var miniChartsRoot = document.getElementById('mini-charts');
  var diagTarget = document.getElementById('diag-target');
  var diagCycles = document.getElementById('diag-cycles');
  var diagPing = document.getElementById('diag-ping');
  var diagTraceroute = document.getElementById('diag-traceroute');
  var diagMtr = document.getElementById('diag-mtr');
  var diagIperfBox = document.getElementById('diag-iperf3');
  var diagIperfServer = document.getElementById('diag-iperf-server');
  var diagIperfDir = document.getElementById('diag-iperf-dir');
  var diagIperfDuration = document.getElementById('diag-iperf-duration');
  var diagIperfParallel = document.getElementById('diag-iperf-parallel');
  var diagIperfProtocol = document.getElementById('diag-iperf-protocol');
  var diagIperfLength = document.getElementById('diag-iperf-length');
  var diagIperfBtn = document.getElementById('diag-iperf-btn');
  var diagTabLg = document.getElementById('diag-tab-lg');
  var diagTabIperf = document.getElementById('diag-tab-iperf');
  var diagTabSpeedtest = document.getElementById('diag-tab-speedtest');
  var diagLgGroup = document.getElementById('diag-lg-group');
  var diagIperfGroup = document.getElementById('diag-iperf-group');
  var diagSpeedtestGroup = document.getElementById('diag-speedtest-group');
  var diagSpeedtestBtn = document.getElementById('diag-speedtest-btn');
  var diagSpeedtestSize = document.getElementById('diag-speedtest-size');
  var diagSpeedtestDir = document.getElementById('diag-speedtest-dir');
  var diagError = document.getElementById('diag-error');
  var diagSessions = document.getElementById('diag-sessions');
  var diagEmpty = document.getElementById('diag-empty');
  var termBtn = document.getElementById('term-btn');
  var termModal = document.getElementById('term-modal');
  var termOut = document.getElementById('term-out');
  var termInput = document.getElementById('term-input');
  var termErr = document.getElementById('term-err');
  var streamingMessage = document.getElementById('streaming-message');
  var streamingResults = document.getElementById('streaming-results');
  var dockerPanel = document.getElementById('docker-panel');
  var dockerMessage = document.getElementById('docker-message');
  var dockerWrap = document.getElementById('docker-wrap');
  var editModeBtn = document.getElementById('edit-mode-btn');
  var tokenModal = document.getElementById('token-modal');
  var tokenInput = document.getElementById('token-input');
  var tokenUsername = document.getElementById('token-username');
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
  var fBandwidth = document.getElementById('f-bandwidth');
  var fMode = document.getElementById('f-mode');
  var fDir = document.getElementById('f-dir');
  var addTaskBtn = document.getElementById('add-task-btn');
  var renameModal = document.getElementById('rename-modal');
  var renameInput = document.getElementById('rename-input');
  var renameErr = document.getElementById('rename-err');
  var renameSave = document.getElementById('rename-save');
  var renameCancel = document.getElementById('rename-cancel');
  var atModal = document.getElementById('at-modal');
  var atLabel = document.getElementById('at-label');
  var atKind = document.getElementById('at-kind');
  var atTarget = document.getElementById('at-target');
  var atPortField = document.getElementById('at-port-field');
  var atPort = document.getElementById('at-port');
  var atInterval = document.getElementById('at-interval');
  var atSubmit = document.getElementById('at-submit');
  var atCancel = document.getElementById('at-cancel');
  var atErr = document.getElementById('at-err');

  var GAUGE_LEN = 251.33;
  var CURRENCY_SYMBOL = { CNY: '¥', USD: '$', EUR: '€' };
  var CYCLE_DIVISOR = { monthly: 1, quarterly: 3, yearly: 12 };
  var CHART_COLORS = ['#fbbf24', '#38bdf8', '#a78bfa', '#34d399', '#f87171', '#fb7185'];
  var CHART_RANGE = { '1h': 3600, '6h': 21600, '24h': 86400, '7d': 604800 };

  var entry = null;
  var card = null;
  var diagRequests = new Map();
  var pingChartData = { tasks: [], points: [] };
  var pingLoadId = 0;
  var metricsChartData = [];
  var metricsLoadId = 0;
  var streamingLoadId = 0;
  var adminOn = false;
  var editingId = null;
  var lastChart = null;
  var hoverTaskId = null;
  var hoverPoint = null;
  var hoverFramePending = false;

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
      swapFill: P.field(node, 'swapFill'),
      swapVal: P.field(node, 'swapVal'),
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
      bandwidth: P.field(node, 'bandwidth'),
      region: P.field(node, 'region'),
      pingSection: P.field(node, 'pingSection'),
      pings: P.field(node, 'pings'),
      unlockSection: P.field(node, 'unlockSection'),
      unlock: P.field(node, 'unlock'),
      editBtn: P.field(node, 'editBtn')
    };
    card.ips4 = P.field(node, 'ips4');
    card.ips6 = P.field(node, 'ips6');
    card.ipRow = node.querySelector('.ip-row');
    card.ipsToggle = P.field(node, 'ipsToggle');
    card.ipsShown = false;
    card.ipsToggle.addEventListener('click', function () {
      card.ipsShown = !card.ipsShown;
      renderIps();
    });
    card.renameBtn = node.querySelector('#rename-btn');
    card.editBtn.addEventListener('click', function () { openEdit(hostId); });
    card.renameBtn.addEventListener('click', function () {
      renameInput.value = entry.name || ('Agent #' + hostId);
      renameErr.hidden = true;
      renameModal.hidden = false;
    });
    card.renameBtn.hidden = !adminOn;
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
      var cls;
      if (result.cert_days != null) {
        value += ' · ' + t('ping.certDays').replace('{n}', Math.round(result.cert_days));
        if (result.cert_name) value += ' · ' + result.cert_name;
        cls = result.cert_days < 7 ? 'crit' : result.cert_days < 30 ? 'warn' : 'ok';
      } else if (result.status != null) {
        value += ' · HTTP ' + result.status;
        cls = result.status < 400 ? 'ok' : 'crit';
      } else {
        cls = result.rtt_ms == null || loss > 0.2 ? 'crit' : loss > 0 ? 'warn' : 'ok';
      }
      P.chip(card.pings, result.label || ('#' + (result.task_id == null ? '—' : result.task_id)), value, cls);
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

  function renderDocker() {
    dockerPanel.hidden = !entry.info;
    dockerWrap.innerHTML = '';
    if (!entry.info) return;
    var list = entry.containers || [];
    if (!list.length) {
      dockerPanel.hidden = true;
      dockerMessage.textContent = t('docker.empty');
      dockerMessage.hidden = true;
      return;
    }
    var wrap = document.createElement('div');
    wrap.className = 'admin-table-wrap';
    var table = document.createElement('table');
    table.className = 'admin-table';
    var head = document.createElement('thead');
    head.innerHTML = '<tr><th>' + t('docker.name') + '</th><th>' + t('docker.image') + '</th><th>' + t('docker.state') + '</th><th>' + t('docker.cpu') + '</th><th>' + t('docker.memory') + '</th></tr>';
    table.appendChild(head);
    var body = document.createElement('tbody');
    list.forEach(function (c) {
      var tr = document.createElement('tr');
      var mem = c.mem_limit ? (P.fmtBytes(c.mem_used || 0) + ' / ' + P.fmtBytes(c.mem_limit)) : (c.mem_used ? P.fmtBytes(c.mem_used) : '—');
      var cpu = c.cpu_pct != null ? Number(c.cpu_pct).toFixed(1) + '%' : '—';
      [c.name, c.image, c.state].forEach(function (text) {
        var td = document.createElement('td');
        td.textContent = text || '—';
        tr.appendChild(td);
      });
      var tdCpu = document.createElement('td');
      tdCpu.textContent = cpu;
      tr.appendChild(tdCpu);
      var tdMem = document.createElement('td');
      tdMem.textContent = mem;
      tr.appendChild(tdMem);
      body.appendChild(tr);
    });
    table.appendChild(body);
    wrap.appendChild(table);
    dockerWrap.appendChild(wrap);
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
    var d = entry.data;
    if (d) {
      if (d.temperature_c != null) items.push([t('host.temperature'), d.temperature_c.toFixed(1) + ' °C']);
      if (d.gpu_name) {
        var gpu = d.gpu_name;
        if (d.gpu_util != null) gpu += ' · ' + Math.round(d.gpu_util) + '%';
        if (d.gpu_mem_total) gpu += ' · ' + P.fmtBytes(d.gpu_mem_used || 0) + ' / ' + P.fmtBytes(d.gpu_mem_total);
        items.push([t('host.gpu'), gpu]);
      }
      if (d.process_count) items.push([t('host.processes'), String(d.process_count)]);
      if (d.connection_count) items.push([t('host.connections'), String(d.connection_count)]);
    }
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

    var rx = tr ? (tr.rx_bytes || 0) : 0;
    var tx = tr ? (tr.tx_bytes || 0) : 0;
    var bmode = b.traffic_mode || 'bi';
    var bdir = b.traffic_dir || 'down';
    var used = bmode === 'uni'
      ? (bdir === 'up' ? tx : bdir === 'max' ? Math.max(rx, tx) : rx)
      : (rx + tx);
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
    card.bandwidth.textContent = b.bandwidth != null ? b.bandwidth + ' Mbps' : '—';
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
    card.swapFill.style.width = P.pct(d.swap_used, d.swap_total).toFixed(1) + '%';
    card.swapVal.textContent = P.fmtBytes(d.swap_used) + ' / ' + P.fmtBytes(d.swap_total);
    card.rx.textContent = P.rateWithTotal(d.net_rx_bps, entry && entry.traffic ? entry.traffic.rx_bytes : 0);
    card.tx.textContent = P.rateWithTotal(d.net_tx_bps, entry && entry.traffic ? entry.traffic.tx_bytes : 0);
    card.load.textContent = d.load1.toFixed(2);
    card.uptime.textContent = P.fmtUptime(d.uptime);
  }

  function renderIps() {
    var ips = (entry.info && Array.isArray(entry.info.ips)) ? entry.info.ips : [];
    var v4 = null, v6 = null;
    ips.forEach(function (ip) {
      var s = String(ip);
      if (s.indexOf(':') >= 0) {
        if (!v6 && !P.isLinkLocalV6(s)) v6 = s;
      } else if (!v4) {
        v4 = s;
      }
    });
    if (!v4 && !v6) {
      card.ips4.textContent = '';
      card.ips6.textContent = '';
      card.ipRow.hidden = true;
      return;
    }
    card.ipRow.hidden = false;
    card.ipsToggle.hidden = !adminOn;
    var shown = adminOn && card.ipsShown;
    card.ips4.textContent = v4 ? (shown ? v4 : P.maskIp(v4)) : '';
    card.ips6.textContent = v6 ? (shown ? v6 : P.maskIp(v6)) : '';
    card.ipsToggle.textContent = shown ? '🙈' : '👁';
  }

  function renderAll() {
    if (!card) card = buildCard();
    card.name.textContent = entry.name || ('Agent #' + hostId);
    card.os.textContent = entry.info ? entry.info.os + ' · ' + entry.info.kernel + ' · ' + entry.info.cpu_cores + 'C' : '—';
    setStatus(entry.online);
    renderMetrics();
    renderBilling();
    renderHardware();
    renderRegion();
    renderPings();
    renderUnlock();
    renderDocker();
    renderIps();
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
    var nowSec = Math.floor(Date.now() / 1000);
    var minTs = nowSec - (CHART_RANGE[pingRange.value] || 86400);
    var maxTs = nowSec;
    var rtts = points.map(function (point) { return Number(point.rtt_avg); }).filter(Number.isFinite);
    var maxRtt = rtts.length ? Math.max.apply(null, rtts) : 1;
    maxRtt = Math.max(10, Math.ceil(maxRtt / 10) * 10);
    var allTasks = pingTask.value === '';
    var left = 42, right = 12, top = 12, plotBottom = rect.height - (allTasks ? 26 : 52);
    var plotWidth = Math.max(1, rect.width - left - right);
    var plotHeight = Math.max(1, plotBottom - top);
    var xAt = function (ts) { return left + (Number(ts) - minTs) / (maxTs - minTs) * plotWidth; };
    var yAt = function (rtt) { return top + (1 - Number(rtt) / maxRtt) * plotHeight; };
    lastChart = { points: points, tasks: tasks, minTs: minTs, maxTs: maxTs, xAt: xAt, yAt: yAt, left: left, top: top, plotBottom: plotBottom, plotWidth: plotWidth };

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
    if (!allTasks) ctx.fillText(t('ping.lossAxis'), left, rect.height - 38);

    tasks.forEach(function (task, taskIndex) {
      var color = CHART_COLORS[taskIndex % CHART_COLORS.length];
      var series = points.filter(function (point) { return String(point.task_id) === String(task.id); })
        .sort(function (a, b) { return Number(a.ts) - Number(b.ts); });
      if (!series.length) return;
      var isHover = hoverTaskId != null && String(task.id) === String(hoverTaskId);
      ctx.save();
      if (hoverTaskId != null && !isHover) ctx.globalAlpha = 0.22;
      ctx.strokeStyle = color;
      ctx.lineWidth = isHover ? 2.8 : 1.7;
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
      ctx.restore();
      if (!allTasks) series.forEach(function (point) {
        var loss = Math.max(0, Math.min(1, Number(point.loss) || 0));
        if (!loss) return;
        var x = xAt(point.ts);
        ctx.fillStyle = loss >= 0.5 ? 'rgba(248, 113, 113, 0.85)' : 'rgba(251, 191, 36, 0.75)';
        ctx.fillRect(x - 1.5, rect.height - 14 - loss * 22, 3, Math.max(2, loss * 22));
      });
      var legend = document.createElement('span');
      legend.className = 'legend-item' + (isHover ? ' active' : '');
      var swatch = document.createElement('span');
      swatch.className = 'legend-swatch';
      swatch.style.background = color;
      var label = document.createElement('span');
      var last = series[series.length - 1];
      var name = task.label || ('#' + task.id);
      if (last) {
        var curRtt = Number(last.rtt_avg);
        var curLoss = Math.max(0, Math.min(1, Number(last.loss) || 0));
        name += ' · ' + (Number.isFinite(curRtt) ? curRtt.toFixed(1) + ' ms' : '—')
          + ' · ' + Math.round(curLoss * 100) + '% ' + t('ping.lossShort');
      }
      label.textContent = name;
      legend.appendChild(swatch);
      legend.appendChild(label);
      pingLegend.appendChild(legend);
    });

    if (hoverPoint && Number.isFinite(hoverPoint.rtt)) {
      var hx = xAt(hoverPoint.ts);
      var hy = yAt(hoverPoint.rtt);
      ctx.beginPath();
      ctx.arc(hx, hy, 4, 0, Math.PI * 2);
      ctx.fillStyle = hoverPoint.color;
      ctx.fill();
      ctx.lineWidth = 2;
      ctx.strokeStyle = 'rgba(255, 255, 255, 0.9)';
      ctx.stroke();
    }
  }

  function scheduleHoverRedraw() {
    if (hoverFramePending) return;
    hoverFramePending = true;
    requestAnimationFrame(function () {
      hoverFramePending = false;
      drawPingChart();
    });
  }


  /* ---------- metrics history ---------- */
  var BANDWIDTH_DOWN = '#34d399';
  var BANDWIDTH_UP = '#38bdf8';
  var DISK_WRITE_COLOR = '#a78bfa';
  var DISK_READ_COLOR = '#f472b6';

  // 4 tiles in one row: CPU / 内存 / 磁盘读写 / 带宽
  var METRIC_TILES = [
    { label: 'metric.cpu', icon: '▦', color: '#fbbf24',
      value: function (p) { return Number(p.cpu_usage); },
      format: function (v) { return v == null || !Number.isFinite(v) ? '—' : v.toFixed(1) + '%'; } },
    { label: 'metric.mem', icon: '▥', color: '#38bdf8',
      value: function (p) { return p.mem_total > 0 ? Number(p.mem_used) / Number(p.mem_total) * 100 : null; },
      format: function (v) { return v == null || !Number.isFinite(v) ? '—' : v.toFixed(1) + '%'; } },
    { label: 'metric.diskIo', icon: '◫', color: DISK_WRITE_COLOR,
      value: function (p) { return p; },
      format: function (p) { return p == null ? '—' : '读 ' + fmtMbps(Number(p.disk_read_bps)) + ' · 写 ' + fmtMbps(Number(p.disk_write_bps)); } }
  ];

  // 2x2 grid of mini trend charts: CPU / 内存 / 磁盘写入 / 带宽(down+up)
  var MINI_CHARTS = [
    { key: 'cpu', title: 'metric.cpu', rate: false,
      series: [{ key: 'cpu', label: 'metric.cpu', color: '#fbbf24',
        value: function (p) { return Number(p.cpu_usage); } }] },
    { key: 'mem', title: 'metric.mem', rate: false,
      series: [{ key: 'mem', label: 'metric.mem', color: '#38bdf8',
        value: function (p) { return p.mem_total > 0 ? Number(p.mem_used) / Number(p.mem_total) * 100 : null; } }] },
    { key: 'disk', title: 'metric.diskIo', rate: true,
      series: [
        { key: 'read', label: 'metric.diskRead', color: DISK_READ_COLOR,
          value: function (p) { return Number(p.disk_read_bps); } },
        { key: 'write', label: 'metric.diskWrite', color: DISK_WRITE_COLOR,
          value: function (p) { return Number(p.disk_write_bps); } }
      ] },
    { key: 'bandwidth', title: 'history.bandwidth', rate: true,
      series: [
        { key: 'down', label: 'history.down', color: BANDWIDTH_DOWN,
          value: function (p) { return Number(p.net_rx_bps); } },
        { key: 'up', label: 'history.up', color: BANDWIDTH_UP,
          value: function (p) { return Number(p.net_tx_bps); } }
      ] }
  ];

  function fmtMbps(bps) {
    if (!Number.isFinite(bps) || bps <= 0) return '0';
    var mbps = bps / 1e6;
    if (mbps >= 100) return Math.round(mbps) + 'M';
    if (mbps >= 1) return mbps.toFixed(1) + 'M';
    return Math.round(bps / 1e3) + 'K';
  }

  function latestMetricPoint() {
    var best = null;
    metricsChartData.forEach(function (p) {
      if (!best || Number(p.ts) > Number(best.ts)) best = p;
    });
    return best;
  }

  function renderMetricTiles() {
    metricTiles.innerHTML = '';
    var p = latestMetricPoint();
    if (!p) return;
    METRIC_TILES.forEach(function (tile) {
      var v = tile.value(p);
      var el = document.createElement('div');
      el.className = 'metric-tile';
      var icon = document.createElement('span');
      icon.className = 'metric-tile-icon';
      icon.style.color = tile.color;
      icon.style.borderColor = tile.color;
      icon.textContent = tile.icon;
      var info = document.createElement('div');
      info.className = 'metric-tile-info';
      var label = document.createElement('span');
      label.className = 'metric-tile-label';
      label.textContent = t(tile.label);
      var value = document.createElement('span');
      value.className = 'metric-tile-value';
      value.textContent = tile.format(v);
      info.appendChild(label);
      info.appendChild(value);
      el.appendChild(icon);
      el.appendChild(info);
      metricTiles.appendChild(el);
    });
    var bw = document.createElement('div');
    bw.className = 'metric-tile';
    var bwIcon = document.createElement('span');
    bwIcon.className = 'metric-tile-icon';
    bwIcon.style.color = BANDWIDTH_DOWN;
    bwIcon.style.borderColor = BANDWIDTH_DOWN;
    bwIcon.textContent = '⇅';
    var bwInfo = document.createElement('div');
    bwInfo.className = 'metric-tile-info';
    var bwLabel = document.createElement('span');
    bwLabel.className = 'metric-tile-label';
    bwLabel.textContent = t('history.bandwidth');
    var bwVal = document.createElement('span');
    bwVal.className = 'metric-tile-value';
    bwVal.textContent = '↓ ' + fmtMbps(Number(p.net_rx_bps)) + ' · ↑ ' + fmtMbps(Number(p.net_tx_bps));
    bwInfo.appendChild(bwLabel);
    bwInfo.appendChild(bwVal);
    bw.appendChild(bwIcon);
    bw.appendChild(bwInfo);
    metricTiles.appendChild(bw);
  }

  // Build the 2x2 mini-chart grid once; each cell gets a canvas, tooltip, guide.
  function buildMiniCharts() {
    miniChartsRoot.innerHTML = '';
    MINI_CHARTS.forEach(function (cfg) {
      var cell = document.createElement('div');
      cell.className = 'mini-chart';
      var head = document.createElement('div');
      head.className = 'mini-chart-head';
      head.appendChild(node_('span', 'mini-chart-title', t(cfg.title)));
      cell.appendChild(head);
      var wrap = document.createElement('div');
      wrap.className = 'mini-chart-body';
      var canvas = document.createElement('canvas');
      var tip = document.createElement('div');
      tip.className = 'chart-tip';
      tip.hidden = true;
      var guide = document.createElement('div');
      guide.className = 'chart-guide';
      guide.hidden = true;
      var legend = document.createElement('div');
      legend.className = 'mini-chart-legend';
      wrap.appendChild(canvas);
      wrap.appendChild(tip);
      wrap.appendChild(guide);
      cell.appendChild(wrap);
      cell.appendChild(legend);
      miniChartsRoot.appendChild(cell);
      cfg.canvas = canvas;
      cfg.tip = tip;
      cfg.guide = guide;
      cfg.legend = legend;
      cfg.hover = null;
      var bind = cfg;
      wrap.addEventListener('mousemove', function (ev) { miniHoverMove(bind, ev); });
      wrap.addEventListener('mouseleave', function () { miniHoverLeave(bind); });
    });
  }

  function node_(tag, className, text) {
    var el = document.createElement(tag);
    if (className) el.className = className;
    if (text != null) el.textContent = text;
    return el;
  }

  function fmtChartValue(v, rate) {
    if (v == null || !Number.isFinite(v)) return '—';
    return rate ? fmtMbps(v) + '/s' : v.toFixed(1) + '%';
  }

  function drawMiniChart(cfg) {
    var canvas = cfg.canvas;
    var rect = canvas.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    var ratio = window.devicePixelRatio || 1;
    canvas.width = Math.round(rect.width * ratio);
    canvas.height = Math.round(rect.height * ratio);
    var ctx = canvas.getContext('2d');
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    ctx.clearRect(0, 0, rect.width, rect.height);
    cfg.legend.innerHTML = '';

    var points = metricsChartData.filter(function (p) { return Number.isFinite(Number(p.ts)); });
    if (!points.length) return;

    var nowSec = Math.floor(Date.now() / 1000);
    var minTs = nowSec - (CHART_RANGE[metricsRange.value] || 86400);
    var maxTs = nowSec;
    var left = 30, right = 8, top = 10, plotBottom = rect.height - 18;
    var plotWidth = Math.max(1, rect.width - left - right);
    var plotHeight = Math.max(1, plotBottom - top);
    var xAt = function (ts) { return left + (Number(ts) - minTs) / (maxTs - minTs) * plotWidth; };
    var maxV = cfg.rate ? 1 : 100;
    points.forEach(function (p) {
      cfg.series.forEach(function (s) {
        var v = s.value(p);
        if (Number.isFinite(v)) maxV = Math.max(maxV, v / (cfg.rate ? 1e6 : 1));
      });
    });
    if (cfg.rate) maxV = Math.ceil(maxV * 10) / 10;
    var yAt = function (v) { return top + (1 - v / maxV) * plotHeight; };
    cfg.geom = { points: points, minTs: minTs, maxTs: maxTs, left: left, top: top, plotBottom: plotBottom, plotWidth: plotWidth, xAt: xAt, yAt: yAt, maxV: maxV };

    ctx.lineWidth = 1;
    ctx.font = '10px ' + getComputedStyle(document.documentElement).getPropertyValue('--font-mono');
    ctx.textBaseline = 'middle';
    for (var line = 0; line <= 3; line++) {
      var y = top + plotHeight * line / 3;
      ctx.strokeStyle = 'rgba(148, 163, 184, 0.10)';
      ctx.beginPath();
      ctx.moveTo(left, y);
      ctx.lineTo(rect.width - right, y);
      ctx.stroke();
      ctx.fillStyle = '#64748b';
      ctx.textAlign = 'right';
      var label = cfg.rate ? (maxV * (1 - line / 3)).toFixed(1) + 'M' : Math.round(100 * (1 - line / 3)) + '%';
      ctx.fillText(label, left - 5, y);
    }
    ctx.fillStyle = '#64748b';
    ctx.textBaseline = 'alphabetic';
    ctx.textAlign = 'left';
    ctx.fillText(formatMetricsTime(minTs), left, rect.height - 4);
    ctx.textAlign = 'right';
    ctx.fillText(formatMetricsTime(maxTs), rect.width - right, rect.height - 4);

    var sorted = points.slice().sort(function (a, b) { return Number(a.ts) - Number(b.ts); });
    cfg.series.forEach(function (s) {
      var line = [];
      for (var i = 0; i < sorted.length; i++) {
        var v = s.value(sorted[i]);
        if (v == null || !Number.isFinite(v)) continue;
        line.push([xAt(sorted[i].ts), yAt(Math.max(0, v / (cfg.rate ? 1e6 : 1)))]);
      }
      if (line.length < 2) return;
      ctx.beginPath();
      ctx.moveTo(line[0][0], line[0][1]);
      for (var j = 1; j < line.length; j++) ctx.lineTo(line[j][0], line[j][1]);
      ctx.strokeStyle = s.color;
      ctx.lineWidth = 1.5;
      ctx.stroke();
    });

    if (cfg.hover) {
      var ht = cfg.hover.ts;
      cfg.series.forEach(function (s) {
        if (s.key !== cfg.hover.key) return;
        var line = [];
        for (var i = 0; i < sorted.length; i++) {
          var v = s.value(sorted[i]);
          if (!Number.isFinite(v)) continue;
          line.push([xAt(sorted[i].ts), yAt(Math.max(0, v / (cfg.rate ? 1e6 : 1)))]);
        }
        if (line.length) {
          ctx.beginPath();
          ctx.moveTo(line[0][0], line[0][1]);
          for (var j = 1; j < line.length; j++) ctx.lineTo(line[j][0], line[j][1]);
          ctx.strokeStyle = s.color;
          ctx.lineWidth = 3;
          ctx.globalAlpha = 0.9;
          ctx.stroke();
          ctx.globalAlpha = 1;
        }
        var nearest = null, bestD = Infinity;
        sorted.forEach(function (p) {
          var v = s.value(p);
          if (!Number.isFinite(v)) return;
          var d = Math.abs(Number(p.ts) - ht);
          if (d < bestD) { bestD = d; nearest = { x: xAt(p.ts), y: yAt(Math.max(0, v / (cfg.rate ? 1e6 : 1))) }; }
        });
        if (nearest) {
          ctx.beginPath();
          ctx.arc(nearest.x, nearest.y, 5, 0, Math.PI * 2);
          ctx.fillStyle = s.color;
          ctx.fill();
          ctx.lineWidth = 2.5;
          ctx.strokeStyle = 'rgba(255, 255, 255, 0.95)';
          ctx.stroke();
        }
      });
    }

    cfg.series.forEach(function (s) {
      var legend = document.createElement('span');
      legend.className = 'chart-legend-item';
      var swatch = document.createElement('span');
      swatch.className = 'chart-swatch';
      swatch.style.background = s.color;
      var label = document.createElement('span');
      label.textContent = t(s.label);
      legend.appendChild(swatch);
      legend.appendChild(label);
      cfg.legend.appendChild(legend);
    });
  }

  function miniHoverMove(cfg, ev) {
    if (!cfg.geom || !cfg.geom.points.length) {
      cfg.tip.hidden = true;
      cfg.guide.hidden = true;
      return;
    }
    var rect = cfg.canvas.getBoundingClientRect();
    var x = ev.clientX - rect.left;
    var y = ev.clientY - rect.top;
    var g = cfg.geom;
    var ts = g.minTs + (x - g.left) / g.plotWidth * (g.maxTs - g.minTs);
    var best = null;
    cfg.series.forEach(function (s) {
      var nearest = null, md = Infinity;
      g.points.forEach(function (p) {
        var v = s.value(p);
        if (!Number.isFinite(v)) return;
        var d = Math.abs(Number(p.ts) - ts);
        if (d < md) { md = d; nearest = p; }
      });
      if (!nearest) return;
      var v = s.value(nearest);
      var dy = Math.abs(g.yAt(Math.max(0, v / (cfg.rate ? 1e6 : 1))) - y);
      if (!best || dy < best.dy) {
        best = { key: s.key, label: s.label, point: nearest, value: v, dy: dy };
      }
    });
    if (!best) {
      cfg.tip.hidden = true;
      cfg.guide.hidden = true;
      return;
    }
    cfg.hover = { ts: Number(best.point.ts), key: best.key };
    cfg.tip.textContent = formatMetricsTime(Number(best.point.ts)) + ' · ' + t(best.label) + ' ' + fmtChartValue(best.value, cfg.rate);
    cfg.tip.hidden = false;
    var tipLeft = x + 12;
    if (tipLeft + cfg.tip.offsetWidth > rect.width - 8) tipLeft = x - cfg.tip.offsetWidth - 12;
    cfg.tip.style.left = Math.max(4, tipLeft) + 'px';
    cfg.tip.style.top = Math.max(4, y - 14) + 'px';
    cfg.guide.style.left = Math.round(x) + 'px';
    cfg.guide.style.top = g.top + 'px';
    cfg.guide.style.height = Math.max(0, g.plotBottom - g.top) + 'px';
    cfg.guide.hidden = false;
    drawMiniChart(cfg);
  }

  function miniHoverLeave(cfg) {
    cfg.hover = null;
    cfg.tip.hidden = true;
    cfg.guide.hidden = true;
    drawMiniChart(cfg);
  }

  function drawAllMiniCharts() {
    MINI_CHARTS.forEach(function (cfg) { drawMiniChart(cfg); });
  }

  function loadMetricsHistory() {
    if (!entry) return;
    if (!miniChartsRoot.firstChild) buildMiniCharts();
    var seq = ++metricsLoadId;
    var url = '/api/agents/' + hostId + '/history?range=' + encodeURIComponent(metricsRange.value);
    metricsMessage.textContent = t('common.loading');
    P.requestJson(url).then(function (body) {
      if (seq !== metricsLoadId) return;
      metricsChartData = Array.isArray(body.points) ? body.points : [];
      metricsMessage.textContent = metricsChartData.length ? '' : t('common.noData');
      drawAllMiniCharts();
      renderMetricTiles();
    }).catch(function (error) {
      if (seq !== metricsLoadId) return;
      metricsChartData = [];
      metricsMessage.textContent = t('common.error') + ': ' + error.message;
      drawAllMiniCharts();
      renderMetricTiles();
    });
  }

  function formatMetricsTime(ts) {
    var date = new Date(ts * 1000);
    if (metricsRange.value === '7d') {
      return (date.getMonth() + 1) + '/' + date.getDate() + ' ' + date.getHours() + ':00';
    }
    var h = date.getHours();
    var m = date.getMinutes();
    return (h < 10 ? '0' : '') + h + ':' + (m < 10 ? '0' : '') + m;
  }

  /* ---------- diagnostics ---------- */
  function showDiagTab(which) {
    diagTabLg.classList.toggle('active', which === 'lg');
    diagTabIperf.classList.toggle('active', which === 'iperf');
    diagTabSpeedtest.classList.toggle('active', which === 'speedtest');
    diagLgGroup.hidden = which !== 'lg';
    diagIperfGroup.hidden = which !== 'iperf';
    diagSpeedtestGroup.hidden = which !== 'speedtest';
  }

  function updateDiagGate() {
    var online = !!entry && !!entry.online;
    var lg = !!entry && P.hasFeature(entry, 'lg');
    var mtr = !!entry && P.hasFeature(entry, 'mtr');
    var iperf3 = !!entry && P.hasFeature(entry, 'iperf3');
    var speedtest = !!entry && P.hasFeature(entry, 'speedtest');
    diagPing.hidden = !lg;
    diagTraceroute.hidden = !lg;
    diagMtr.hidden = !mtr;
    diagPing.disabled = !online;
    diagTraceroute.disabled = !online;
    diagMtr.disabled = !online;
    diagTabIperf.hidden = !iperf3;
    diagTabSpeedtest.hidden = !speedtest;
    if (!iperf3 && !diagIperfGroup.hidden) showDiagTab('lg');
    if (!speedtest && !diagSpeedtestGroup.hidden) showDiagTab('lg');
    diagIperfBtn.disabled = !online;
    diagSpeedtestBtn.disabled = !online;
    document.querySelector('.cycles-field').hidden = !mtr;
    if (entry && !online) {
      diagError.textContent = t('diag.offline');
      diagError.hidden = false;
    } else if (entry && !lg && !mtr && !iperf3 && !speedtest) {
      diagError.textContent = t('feature.disabled').replace('{feature}', t('feature.diagnostics'));
      diagError.hidden = false;
    } else {
      diagError.hidden = true;
    }
  }

  function diagnosticLabel(kind) {
    return kind === 'traceroute' ? t('diag.traceroute')
      : kind === 'mtr' ? t('diag.mtr')
      : kind === 'iperf3' ? t('diag.iperf3')
      : t('diag.ping');
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
      if (message.kind === 'mtr' && Array.isArray(message.result)) {
        renderMtrTable(request, message.result);
      } else if (message.kind === 'iperf3' && typeof message.result === 'object') {
        renderIperf3Result(request, message.result);
      } else if (message.kind === 'speedtest' && typeof message.result === 'object') {
        if (message.stream === 'progress') {
          // Real-time progress update
          handleSpeedtestProgress(message);
        } else {
          // Final result
          renderSpeedtestResult(request, message.result);
          speedtestState.finalResult = message.result;
        }
      } else {
        appendDiagnosticPart(request, JSON.stringify(message.result, null, 2) + '\n', 'structured');
      }
    }
    if (message.done) {
      setDiagnosticState(request, message.exit_code == null || message.exit_code === 0 ? 'finished' : 'failed', message.exit_code);
    }
  }

  function iperfMbps(bps) {
    var mbps = Number(bps) / 1e6;
    return mbps >= 1000 ? (mbps / 1000).toFixed(2) + ' Gbps' : mbps.toFixed(1) + ' Mbps';
  }

  function formatBytes(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1048576) return (bytes / 1024).toFixed(1) + ' KB';
    if (bytes < 1073741824) return (bytes / 1048576).toFixed(1) + ' MB';
    return (bytes / 1073741824).toFixed(2) + ' GB';
  }

  /* Renders the structured iperf3 summary into the diagnostic session. */
  function renderIperf3Result(request, result) {
    var lines = [];
    lines.push(t('diag.iperf3') + ' · ' + (result.direction === 'up' ? t('diag.directionUp') : t('diag.directionDown')));
    if (result.throughput_bps != null) lines.push(t('diag.throughput') + ': ' + iperfMbps(result.throughput_bps));
    if (result.retransmits != null) lines.push(t('diag.retransmits') + ': ' + result.retransmits);
    if (result.duration_s != null) lines.push(t('diag.duration') + ': ' + Number(result.duration_s).toFixed(1) + ' s');
    appendDiagnosticPart(request, lines.join('\n') + '\n', 'structured');
  }

  /* ---------- Speedtest real-time chart ---------- */
  var speedtestState = {
    running: false,
    direction: null,
    requestId: null,
    points: [],
    finalResult: null,
  };

  function showSpeedtestChart(direction) {
    resetSpeedtestState();
    speedtestState.direction = direction;
    speedtestState.running = true;
    var container = document.getElementById('speedtest-chart-container');
    if (container) container.hidden = false;
    var resultEl = document.getElementById('speedtest-result');
    if (resultEl) resultEl.classList.add('speedtest-running');
    // Reset display
    var gaugeValue = document.getElementById('speedtest-gauge-value');
    var gaugeUnit = document.getElementById('speedtest-gauge-unit');
    if (gaugeValue) gaugeValue.textContent = '0';
    if (gaugeUnit) gaugeUnit.textContent = 'Mbps';
    document.getElementById('speedtest-down').textContent = '--';
    document.getElementById('speedtest-up').textContent = '--';
    document.getElementById('speedtest-duration').textContent = '--';
  }

  function resetSpeedtestState() {
    speedtestState = {
      running: false,
      direction: null,
      requestId: null,
      points: [],
      finalResult: null,
    };
    var container = document.getElementById('speedtest-chart-container');
    if (container) container.hidden = true;
    var resultEl = document.getElementById('speedtest-result');
    if (resultEl) resultEl.classList.remove('speedtest-running');
  }

  function handleSpeedtestProgress(message) {
    var result = message.result;
    if (!result) return;

    speedtestState.running = !result.done;
    speedtestState.direction = result.direction;

    if (!result.done) {
      // Add data point
      speedtestState.points.push({
        time: result.elapsed_ms,
        bps: result.throughput_bps,
      });

      // Update gauge
      updateSpeedtestGauge(result.throughput_bps);

      // Update duration
      var durationEl = document.getElementById('speedtest-duration');
      if (durationEl && result.elapsed_ms) {
        durationEl.textContent = (result.elapsed_ms / 1000).toFixed(1) + ' s';
      }
    } else {
      // Final progress update - update stats
      var resultEl = document.getElementById('speedtest-result');
      if (resultEl) resultEl.classList.remove('speedtest-running');

      if (result.direction === 'down') {
        var downEl = document.getElementById('speedtest-down');
        if (downEl && result.throughput_bps) {
          downEl.textContent = iperfMbps(result.throughput_bps);
        }
      } else {
        var upEl = document.getElementById('speedtest-up');
        if (upEl && result.throughput_bps) {
          upEl.textContent = iperfMbps(result.throughput_bps);
        }
      }
    }

    // Redraw chart
    drawSpeedtestChart();
  }

  function updateSpeedtestGauge(bps) {
    var mbps = Number(bps) / 1e6;
    var valueEl = document.getElementById('speedtest-gauge-value');
    var unitEl = document.getElementById('speedtest-gauge-unit');

    if (!valueEl || !unitEl) return;

    if (mbps >= 1000) {
      valueEl.textContent = (mbps / 1000).toFixed(2);
      unitEl.textContent = 'Gbps';
    } else {
      valueEl.textContent = mbps.toFixed(1);
      unitEl.textContent = 'Mbps';
    }
  }

  function drawSpeedtestChart() {
    var canvas = document.getElementById('speedtest-chart');
    if (!canvas) return;

    var rect = canvas.getBoundingClientRect();
    if (!rect.width || !rect.height) return;

    var ratio = window.devicePixelRatio || 1;
    canvas.width = Math.round(rect.width * ratio);
    canvas.height = Math.round(rect.height * ratio);
    var ctx = canvas.getContext('2d');
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    ctx.clearRect(0, 0, rect.width, rect.height);

    var points = speedtestState.points;
    if (points.length < 2) return;

    // Calculate plot area
    var left = 55, right = 25, top = 20, bottom = 35;
    var plotWidth = rect.width - left - right;
    var plotHeight = rect.height - top - bottom;

    // Calculate max values
    var maxBps = 0;
    points.forEach(function (p) {
      if (p.bps > maxBps) maxBps = p.bps;
    });
    // Round up to nice number
    var maxMbps = maxBps / 1e6;
    if (maxMbps <= 10) maxMbps = 10;
    else if (maxMbps <= 50) maxMbps = Math.ceil(maxMbps / 10) * 10;
    else if (maxMbps <= 100) maxMbps = Math.ceil(maxMbps / 20) * 20;
    else if (maxMbps <= 500) maxMbps = Math.ceil(maxMbps / 50) * 50;
    else maxMbps = Math.ceil(maxMbps / 100) * 100;
    maxBps = maxMbps * 1e6;

    var maxTime = points[points.length - 1].time;
    if (maxTime < 1000) maxTime = 1000; // At least 1 second

    // Coordinate transforms
    function xAt(t) { return left + (t / maxTime) * plotWidth; }
    function yAt(bps) { return top + (1 - bps / maxBps) * plotHeight; }

    // Draw grid lines
    ctx.strokeStyle = 'rgba(255, 255, 255, 0.08)';
    ctx.lineWidth = 1;
    for (var i = 0; i <= 4; i++) {
      var y = top + plotHeight * i / 4;
      ctx.beginPath();
      ctx.moveTo(left, y);
      ctx.lineTo(left + plotWidth, y);
      ctx.stroke();

      // Y axis labels
      ctx.fillStyle = 'rgba(255, 255, 255, 0.4)';
      ctx.font = '10px monospace';
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      var labelMbps = maxMbps * (1 - i / 4);
      ctx.fillText(labelMbps >= 1000 ? (labelMbps / 1000).toFixed(1) + 'G' : labelMbps.toFixed(0) + 'M', left - 8, y);
    }

    // Draw time axis labels
    ctx.fillStyle = 'rgba(255, 255, 255, 0.4)';
    ctx.font = '10px monospace';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';
    var timeSteps = Math.min(5, Math.ceil(maxTime / 1000));
    for (var i = 0; i <= timeSteps; i++) {
      var t = (maxTime / timeSteps) * i;
      var x = xAt(t);
      ctx.fillText((t / 1000).toFixed(1) + 's', x, top + plotHeight + 8);

      // Vertical grid line
      ctx.strokeStyle = 'rgba(255, 255, 255, 0.04)';
      ctx.beginPath();
      ctx.moveTo(x, top);
      ctx.lineTo(x, top + plotHeight);
      ctx.stroke();
    }

    // Create gradient for fill
    var gradient = ctx.createLinearGradient(0, top, 0, top + plotHeight);
    gradient.addColorStop(0, 'rgba(0, 200, 255, 0.3)');
    gradient.addColorStop(1, 'rgba(0, 200, 255, 0.02)');

    // Draw filled area
    ctx.beginPath();
    ctx.moveTo(xAt(points[0].time), yAt(points[0].bps));
    for (var i = 1; i < points.length; i++) {
      // Smooth curve using quadratic bezier
      var x0 = xAt(points[i - 1].time);
      var y0 = yAt(points[i - 1].bps);
      var x1 = xAt(points[i].time);
      var y1 = yAt(points[i].bps);
      var cpx = (x0 + x1) / 2;
      ctx.quadraticCurveTo(cpx, y0, x1, y1);
    }
    ctx.lineTo(xAt(points[points.length - 1].time), top + plotHeight);
    ctx.lineTo(xAt(points[0].time), top + plotHeight);
    ctx.closePath();
    ctx.fillStyle = gradient;
    ctx.fill();

    // Draw line
    ctx.beginPath();
    ctx.moveTo(xAt(points[0].time), yAt(points[0].bps));
    for (var i = 1; i < points.length; i++) {
      var x0 = xAt(points[i - 1].time);
      var y0 = yAt(points[i - 1].bps);
      var x1 = xAt(points[i].time);
      var y1 = yAt(points[i].bps);
      var cpx = (x0 + x1) / 2;
      ctx.quadraticCurveTo(cpx, y0, x1, y1);
    }
    ctx.strokeStyle = '#00c8ff';
    ctx.lineWidth = 2.5;
    ctx.shadowColor = 'rgba(0, 200, 255, 0.5)';
    ctx.shadowBlur = 10;
    ctx.stroke();
    ctx.shadowBlur = 0;

    // Draw current point
    if (points.length > 0) {
      var lastPoint = points[points.length - 1];
      var lx = xAt(lastPoint.time);
      var ly = yAt(lastPoint.bps);

      // Glow effect
      ctx.beginPath();
      ctx.arc(lx, ly, 8, 0, Math.PI * 2);
      ctx.fillStyle = 'rgba(0, 200, 255, 0.3)';
      ctx.fill();

      // Inner circle
      ctx.beginPath();
      ctx.arc(lx, ly, 4, 0, Math.PI * 2);
      ctx.fillStyle = '#00c8ff';
      ctx.fill();
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = 2;
      ctx.stroke();
    }
  }

  /* Renders the structured speedtest summary into the diagnostic session. */
  function renderSpeedtestResult(request, result) {
    var lines = [];
    lines.push(t('diag.speedtest') + ' · ' + (result.direction === 'up' ? t('diag.directionUp') : t('diag.directionDown')));
    if (result.throughput_bps != null) lines.push(t('diag.speed') + ': ' + iperfMbps(result.throughput_bps));
    if (result.elapsed_ms != null) lines.push(t('diag.duration') + ': ' + (result.elapsed_ms / 1000).toFixed(1) + ' s');
    if (result.bytes_transferred != null) lines.push(t('diag.transferred') + ': ' + formatBytes(result.bytes_transferred));
    appendDiagnosticPart(request, lines.join('\n') + '\n', 'structured');
  }

  function mtrPad(value, width) {
    var s = String(value);
    while (s.length < width) s += ' ';
    return s;
  }

  function mtrPadL(value, width) {
    var s = String(value);
    while (s.length < width) s = ' ' + s;
    return s;
  }

  var mtrCharW = 0;
  function mtrCharWidth(el) {
    if (mtrCharW) return mtrCharW;
    var probe = document.createElement('span');
    probe.style.visibility = 'hidden';
    probe.textContent = '00000000000000000000000000000000000000000000000000';
    el.appendChild(probe);
    mtrCharW = probe.getBoundingClientRect().width / 50 || 7.2;
    el.removeChild(probe);
    return mtrCharW;
  }

  function mtrNum(value) {
    var n = Number(value);
    return Number.isFinite(n) ? n.toFixed(1) : '0.0';
  }

  /* Renders hops like the live `mtr` curses table, rebuilt on each snapshot.
     The stats columns spread right-aligned across the terminal's real width;
     hosts wider than the column wrap onto indented continuation lines. */
  function renderMtrTable(request, hops) {
    var charW = mtrCharWidth(request.output);
    // -8 chars guards the box padding and the vertical scrollbar; 92 caps the
    // total so wide panels never blow past the visible area.
    var total = Math.max(62, Math.min(92, Math.floor(request.output.clientWidth / charW) - 8));
    var COLS = Math.max(26, Math.min(36, total - 56));
    var rows = [mtrPad('Host', COLS)
      + mtrPadL('Loss%', 8) + mtrPadL('Snt', 8) + mtrPadL('Last', 8)
      + mtrPadL('Avg', 8) + mtrPadL('Best', 8) + mtrPadL('Wrst', 8) + mtrPadL('StDev', 8)];
    hops.forEach(function (h) {
      var prefix = h.hop + '.|-- ';
      var host = h.host || '???';
      var stats = mtrPadL(((Number(h.loss) || 0) * 100).toFixed(1) + '%', 8)
        + mtrPadL(h.sent || 0, 8)
        + mtrPadL(mtrNum(h.last), 8)
        + mtrPadL(mtrNum(h.avg), 8)
        + mtrPadL(mtrNum(h.best), 8)
        + mtrPadL(mtrNum(h.worst), 8)
        + mtrPadL(mtrNum(h.stdev), 8);
      var parts = [host.slice(0, COLS - prefix.length)];
      var rest = host.slice(COLS - prefix.length);
      while (rest.length) {
        parts.push(rest.slice(0, COLS - 6));
        rest = rest.slice(COLS - 6);
      }
      for (var i = 0; i < parts.length; i++) {
        var line = mtrPad((i === 0 ? prefix : '      ') + parts[i], COLS);
        rows.push(i === parts.length - 1 ? line + stats : line.trimEnd());
      }
    });
    request.output.textContent = rows.join('\n') + '\n';
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

  function runIperf3() {
    diagError.hidden = true;
    if (!entry || !entry.online) {
      diagError.textContent = t('diag.offline');
      diagError.hidden = false;
      return;
    }
    if (!P.hasFeature(entry, 'iperf3')) {
      diagError.textContent = t('feature.disabled').replace('{feature}', t('feature.iperf3'));
      diagError.hidden = false;
      return;
    }
    var duration = Math.max(1, Math.min(15, parseInt(diagIperfDuration.value, 10) || 10));
    var direction = diagIperfDir.value;
    var server = diagIperfServer.value.trim();
    if (!server) {
      diagError.textContent = t('diag.targetRequired');
      diagError.hidden = false;
      diagIperfServer.focus();
      return;
    }
    var parallel = parseInt(diagIperfParallel.value, 10);
    var length = parseInt(diagIperfLength.value, 10);
    var body = { agent_id: hostId, server: server, port: 5201, direction: direction, duration: duration, protocol: diagIperfProtocol.value };
    if (parallel > 0) body.parallel = parallel;
    if (length > 0) body.length = length;
    P.requestJson('/api/diag/iperf3', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body)
    }).then(function (response) {
      if (!response.request_id) throw new Error(t('diag.missingRequestId'));
      createDiagnosticSession(response.request_id, hostId, 'iperf3', server + ' · ' + (direction === 'up' ? t('diag.directionUp') : t('diag.directionDown')));
    }).catch(function (error) {
      diagError.textContent = t('common.error') + ': ' + (error && error.message ? error.message : error);
      diagError.hidden = false;
    });
  }

  function runSpeedtest() {
    diagError.hidden = true;
    if (!entry || !entry.online) {
      diagError.textContent = t('diag.offline');
      diagError.hidden = false;
      return;
    }
    if (!P.hasFeature(entry, 'speedtest')) {
      diagError.textContent = t('feature.disabled').replace('{feature}', t('diag.speedtest'));
      diagError.hidden = false;
      return;
    }
    var size = parseInt(diagSpeedtestSize.value, 10) || 10485760;
    var direction = diagSpeedtestDir.value;
    var body = { agent_id: hostId, size: size, direction: direction };

    // Show chart container and reset state
    showSpeedtestChart(direction);

    P.requestJson('/api/diag/speedtest', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body)
    }).then(function (response) {
      if (!response.request_id) throw new Error(t('diag.missingRequestId'));
      speedtestState.requestId = response.request_id;
      createDiagnosticSession(response.request_id, hostId, 'speedtest', t('diag.speedtest') + ' · ' + (direction === 'up' ? t('diag.directionUp') : t('diag.directionDown')));
    }).catch(function (error) {
      diagError.textContent = t('common.error') + ': ' + (error && error.message ? error.message : error);
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
  function applySnapshot(a) {
    return {
      agent_id: a.agent_id,
      online: a.online,
      data: a.data,
      containers: Array.isArray(a.containers) ? a.containers : [],
      name: a.name,
      info: a.info,
      billing: a.billing || null,
      traffic: a.traffic || null,
      pings: Array.isArray(a.pings) ? a.pings : [],
      unlock: Array.isArray(a.unlock) ? a.unlock : [],
      region: a.region || null,
      features: Array.isArray(a.features) ? a.features.slice() : null,
      app_version: a.app_version || null
    };
  }

  function applyHost(agent) {
    entry = agent;
    renderAll();
    loadPingHistory(false);
    loadStreaming();
    loadMetricsHistory();
  }

  function handleMessage(msg) {
    if (msg.type === 'snapshot') {
      var found = null;
      msg.agents.forEach(function (a) {
        if (a.agent_id === hostId) found = applySnapshot(a);
      });
      if (found) {
        applyHost(found);
      } else {
        if (card) card.name.textContent = 'Agent #' + hostId;
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
    } else if (msg.type === 'containers' && msg.agent_id === hostId) {
      if (!entry) entry = { agent_id: hostId };
      entry.containers = Array.isArray(msg.containers) ? msg.containers : [];
      renderAll();
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

  function checkSession() {
    // Admin session is an HttpOnly cookie; no token lives in JS.
    return fetch('/api/admin/check', { method: 'POST' })
      .then(function (r) {
        return r.json().then(function (b) { return b.role || null; }).catch(function () { return null; });
      }).catch(function () { return null; });
  }

  function doLogin(username, password) {
    return fetch('/api/admin/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username: username, password: password })
    }).then(function (r) {
      return r.json().then(function (body) { return { status: r.status, body: body }; });
    }).catch(function () { return { status: 0, body: {} }; });
  }

  function setAdmin(on) {
    adminOn = on;
    editModeBtn.classList.toggle('active', on);
    addTaskBtn.hidden = !on;
    termBtn.hidden = !on;
    if (card) card.renameBtn.hidden = !on;
    if (card) renderBilling();
    if (card) renderIps();
  }

  function handleUnauthorized() {
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
    fBandwidth.value = b.bandwidth != null ? b.bandwidth : '';
    fMode.value = b.traffic_mode || 'bi';
    fDir.value = b.traffic_dir || 'down';
    fDir.parentElement.hidden = fMode.value === 'bi';
    editErr.hidden = true;
    editModal.hidden = false;
  }

  function parseAdmin(res) {
    if (res.status === 401) throw 'unauthorized';
    return res.text().then(function (text) {
      var body = null;
      if (text) {
        try { body = JSON.parse(text); } catch (e) { body = null; }
      }
      if (!res.ok) throw (body && body.error) || ('HTTP ' + res.status);
      return body;
    });
  }

  function saveEdit() {
    var body = {
      reset_day: fResetDay.value === '' ? null : parseInt(fResetDay.value, 10),
      quota_gb: fQuotaGb.value === '' ? null : parseFloat(fQuotaGb.value),
      expires_on: fExpiresOn.value === '' ? null : fExpiresOn.value,
      price: fPrice.value === '' ? null : parseFloat(fPrice.value),
      currency: fCurrency.value || null,
      cycle: fCycle.value || null,
      bandwidth: fBandwidth.value === '' ? null : parseFloat(fBandwidth.value),
      traffic_mode: fMode.value,
      traffic_dir: fDir.value
    };
    fetch('/api/admin/agents/' + editingId + '/billing', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body)
    }).then(parseAdmin).then(function (res) {
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
      atErr.textContent = t('common.error');
      atErr.hidden = false;
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
    atErr.hidden = true;
    atSubmit.disabled = true;
    fetch('/api/admin/ping-tasks', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body)
    }).then(parseAdmin).then(function () {
      atLabel.value = '';
      atTarget.value = '';
      atModal.hidden = true;
    }).catch(function (e) {
      if (e === 'unauthorized') handleUnauthorized();
      else {
        atErr.textContent = t('common.error') + ': ' + (e && e.message ? e.message : e);
        atErr.hidden = false;
      }
    }).then(function () { atSubmit.disabled = false; });
  }

  function renameAgent() {
    var name = renameInput.value.trim();
    if (!name) {
      renameErr.textContent = t('common.error');
      renameErr.hidden = false;
      return;
    }
    renameErr.hidden = true;
    renameSave.disabled = true;
    fetch('/api/admin/agents/' + hostId + '/name', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: name })
    }).then(parseAdmin).then(function () {
      entry.name = name;
      card.name.textContent = name;
      renameModal.hidden = true;
    }).catch(function (e) {
      if (e === 'unauthorized') handleUnauthorized();
      else {
        renameErr.textContent = t('common.error') + ': ' + e;
        renameErr.hidden = false;
      }
    }).then(function () { renameSave.disabled = false; });
  }

  function probeAdminLink() {
    // Sessions are HttpOnly cookies now; purge tokens stored by older versions.
    localStorage.removeItem('pharus.admin');
    sessionStorage.removeItem('pharus.admin');
    P.requestJson('/api/meta').then(function (meta) {
      if (card && entry) renderBilling();
      if (!meta.admin_enabled) return;
      adminLink.hidden = false;
      editModeBtn.hidden = false;
      checkSession().then(function (st) {
        if (st === 200) setAdmin(true);
      });
    }).catch(function () {});
  }

  /* ---------- events ---------- */
  pingTask.addEventListener('change', function () { loadPingHistory(false); });
  pingRange.addEventListener('change', function () { loadPingHistory(false); });
  metricsRange.addEventListener('change', loadMetricsHistory);
  diagPing.addEventListener('click', function () { runDiagnostic('ping'); });
  diagTraceroute.addEventListener('click', function () { runDiagnostic('traceroute'); });
  diagMtr.addEventListener('click', function () { runDiagnostic('mtr'); });
  diagIperfBtn.addEventListener('click', runIperf3);
  diagSpeedtestBtn.addEventListener('click', runSpeedtest);
  diagTabLg.addEventListener('click', function () { showDiagTab('lg'); });
  diagTabIperf.addEventListener('click', function () { showDiagTab('iperf'); });
  diagTabSpeedtest.addEventListener('click', function () { showDiagTab('speedtest'); });
  editModeBtn.addEventListener('click', function () {
    if (adminOn) {
      setAdmin(false);
      return;
    }
    checkSession().then(function (role) {
      if (role === 'admin') {
        setAdmin(true);
      } else {
        tokenErr.hidden = true;
        tokenUsername.value = '';
        tokenInput.value = '';
        tokenModal.hidden = false;
        tokenUsername.focus();
      }
    });
  });
  document.getElementById('token-submit').addEventListener('click', function () {
    var u = tokenUsername.value.trim();
    var p = tokenInput.value;
    if (!u || !p) return;
    doLogin(u, p).then(function (res) {
      if (res.status === 200) {
        tokenModal.hidden = true;
        checkSession().then(function (role) {
          if (role === 'admin') {
            setAdmin(true);
          } else {
            tokenErr.textContent = t('admin.viewer');
            tokenErr.hidden = false;
            tokenModal.hidden = false;
          }
        });
      } else {
        tokenErr.textContent = t('admin.unauthorized');
        tokenErr.hidden = false;
      }
    });
  });
  function tokenKey(ev) {
    if (ev.key === 'Enter') document.getElementById('token-submit').click();
  }
  tokenUsername.addEventListener('keydown', tokenKey);
  tokenInput.addEventListener('keydown', tokenKey);
  document.getElementById('token-cancel').addEventListener('click', function () {
    tokenModal.hidden = true;
  });
  document.getElementById('edit-save').addEventListener('click', saveEdit);
  document.getElementById('edit-cancel').addEventListener('click', function () {
    editModal.hidden = true;
  });
  fMode.addEventListener('change', function () {
    fDir.parentElement.hidden = fMode.value === 'bi';
  });
  addTaskBtn.addEventListener('click', function () {
    if (!adminOn) {
      checkSession().then(function (role) {
        if (role === 'admin') {
          setAdmin(true);
          openAddTask();
        } else {
          tokenErr.hidden = true;
          tokenUsername.value = '';
          tokenInput.value = '';
          tokenModal.hidden = false;
          tokenUsername.focus();
        }
      });
      return;
    }
    openAddTask();
  });
  function openAddTask() {
    atErr.hidden = true;
    atLabel.value = '';
    atTarget.value = '';
    atPortField.hidden = atKind.value === 'icmp';
    atModal.hidden = false;
  }
  atSubmit.addEventListener('click', addPingTask);
  atCancel.addEventListener('click', function () { atModal.hidden = true; });
  renameSave.addEventListener('click', renameAgent);
  renameCancel.addEventListener('click', function () { renameModal.hidden = true; });
  atKind.addEventListener('change', function () {
    atPortField.hidden = atKind.value === 'icmp';
  });
  [tokenModal, editModal, atModal, renameModal].forEach(function (mask) {
    mask.addEventListener('click', function (ev) {
      // only explicit close controls dismiss a modal; clicking the backdrop
      // outside the dialog keeps it open
      if (ev.target.hasAttribute('data-close')) mask.hidden = true;
    });
  });
  document.addEventListener('keydown', function (ev) {
    if (ev.key === 'Escape') {
      tokenModal.hidden = true;
      editModal.hidden = true;
      atModal.hidden = true;
      renameModal.hidden = true;
    }
  });
  var chartResizePending = false;
  window.addEventListener('resize', function () {
    if (chartResizePending) return;
    chartResizePending = true;
    requestAnimationFrame(function () {
      chartResizePending = false;
      drawPingChart();
      drawAllMiniCharts();
    });
  });

  // hover tooltip on the latency chart: the curve closest to the pointer
  // (vertically, at the pointer's timestamp) gets highlighted and read out
  pingChart.addEventListener('mousemove', function (ev) {
    if (!lastChart || !lastChart.points.length) {
      pingTip.hidden = true;
      pingGuide.hidden = true;
      return;
    }
    var rect = pingChart.getBoundingClientRect();
    var x = ev.clientX - rect.left;
    var y = ev.clientY - rect.top;
    var ts = lastChart.minTs + (x - lastChart.left) / lastChart.plotWidth * (lastChart.maxTs - lastChart.minTs);
    var best = null;
    lastChart.tasks.forEach(function (task, taskIndex) {
      var nearest = null, minDist = Infinity;
      lastChart.points.forEach(function (p) {
        if (String(p.task_id) !== String(task.id)) return;
        if (!Number.isFinite(Number(p.rtt_avg))) return;
        var d = Math.abs(Number(p.ts) - ts);
        if (d < minDist) { minDist = d; nearest = p; }
      });
      if (!nearest) return;
      var dy = Math.abs(lastChart.yAt(nearest.rtt_avg) - y);
      if (!best || dy < best.dy) {
        best = { task: task, point: nearest, dy: dy, color: CHART_COLORS[taskIndex % CHART_COLORS.length] };
      }
    });
    if (!best) {
      pingTip.hidden = true;
      pingGuide.hidden = true;
      return;
    }
    hoverTaskId = best.task.id;
    hoverPoint = { ts: Number(best.point.ts), rtt: Number(best.point.rtt_avg), color: best.color };
    var name = best.task.label || ('#' + best.task.id);
    var rtt = Number(best.point.rtt_avg);
    var loss = Math.max(0, Math.min(1, Number(best.point.loss) || 0));
    pingTip.textContent = name + ' · ' + formatChartTime(Number(best.point.ts))
      + ' · ' + rtt.toFixed(1) + ' ms'
      + ' · ' + Math.round(loss * 100) + '% ' + t('ping.lossShort');
    pingTip.hidden = false;
    var tipLeft = x + 12;
    if (tipLeft + pingTip.offsetWidth > rect.width - 8) tipLeft = x - pingTip.offsetWidth - 12;
    pingTip.style.left = Math.max(4, tipLeft) + 'px';
    pingTip.style.top = Math.max(4, y - 14) + 'px';
    pingGuide.style.left = Math.round(x) + 'px';
    pingGuide.style.top = lastChart.top + 'px';
    pingGuide.style.height = Math.max(0, lastChart.plotBottom - lastChart.top) + 'px';
    pingGuide.hidden = false;
    scheduleHoverRedraw();
  });
  pingChart.addEventListener('mouseleave', function () {
    hoverTaskId = null;
    hoverPoint = null;
    pingTip.hidden = true;
    pingGuide.hidden = true;
    drawPingChart();
  });

  /* ---------- boot ---------- */
  /* ---------- terminal ---------- */
  var termWs = null;

  function termAppend(text) {
    termOut.textContent += text;
    termOut.scrollTop = termOut.scrollHeight;
  }

  function closeTerminal() {
    if (termWs) {
      try { termWs.send(JSON.stringify({ type: 'close' })); } catch (e) {}
      try { termWs.close(); } catch (e) {}
    }
    termWs = null;
    termModal.hidden = true;
    termErr.hidden = true;
  }

  function openTerminal() {
    termOut.textContent = '';
    termInput.value = '';
    termInput.disabled = true;
    termErr.hidden = true;
    termModal.hidden = false;
    var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    var ws = new WebSocket(proto + '//' + location.host + '/ws/term');
    termWs = ws;
    ws.onopen = function () {
      ws.send(JSON.stringify({ type: 'open', agent_id: hostId, cols: 80, rows: 24 }));
      termInput.disabled = false;
      termInput.focus();
    };
    ws.onmessage = function (ev) {
      termAppend(String(ev.data));
    };
    ws.onclose = function () {
      termAppend('\r\n[连接已关闭]\r\n');
      termInput.disabled = true;
      if (termWs === ws) termWs = null;
    };
    ws.onerror = function () {
      termErr.textContent = t('host.terminalError');
      termErr.hidden = false;
    };
  }

  termBtn.addEventListener('click', function () {
    if (!adminOn) return;
    openTerminal();
  });
  termInput.addEventListener('keydown', function (ev) {
    if (ev.key === 'Enter') {
      ev.preventDefault();
      if (!termWs || termInput.disabled) return;
      termWs.send(JSON.stringify({ type: 'input', data: termInput.value + '\n' }));
      termInput.value = '';
    }
  });
  document.getElementById('term-x').addEventListener('click', closeTerminal);
  document.addEventListener('keydown', function (ev) {
    if (ev.key === 'Escape' && !termModal.hidden) closeTerminal();
  });

  P.ready().then(function () {
    P.initTheme();
    P.requestJson('/api/status').then(function (agents) {
      var a = (Array.isArray(agents) ? agents : []).find(function (x) { return x.agent_id === hostId; });
      if (a) applyHost(applySnapshot(a));
    }).catch(function () {});
    P.connectStream(handleMessage);
    probeAdminLink();
  });
})();
