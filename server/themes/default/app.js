/* Pharus default theme — live dashboard driven by /api/stream */
(function () {
  'use strict';

  /* ---------- i18n ---------- */
  var SUPPORTED = ['en', 'zh-CN', 'ja', 'ru'];
  var i18n = {};

  function detectLang() {
    var candidates = navigator.languages && navigator.languages.length
      ? navigator.languages
      : [navigator.language || 'en'];
    for (var i = 0; i < candidates.length; i++) {
      var l = String(candidates[i]).toLowerCase();
      if (l.indexOf('zh') === 0) return 'zh-CN';
      if (l.indexOf('ja') === 0) return 'ja';
      if (l.indexOf('ru') === 0) return 'ru';
      if (l.indexOf('en') === 0) return 'en';
    }
    return 'en';
  }

  function loadLang(lang) {
    if (window.PHARUS_DEMO_I18N) return Promise.resolve(window.PHARUS_DEMO_I18N);
    return fetch('i18n/' + lang + '.json').then(function (r) {
      if (!r.ok) throw new Error('lang ' + r.status);
      return r.json();
    });
  }

  function t(key) {
    return Object.prototype.hasOwnProperty.call(i18n, key) ? i18n[key] : key;
  }

  function applyStatics() {
    document.title = t('doc.title');
    var nodes = document.querySelectorAll('[data-i18n]');
    for (var i = 0; i < nodes.length; i++) {
      nodes[i].textContent = t(nodes[i].getAttribute('data-i18n'));
    }
    var aria = document.querySelectorAll('[data-i18n-aria]');
    for (var j = 0; j < aria.length; j++) {
      aria[j].setAttribute('aria-label', t(aria[j].getAttribute('data-i18n-aria')));
    }
    var placeholders = document.querySelectorAll('[data-i18n-placeholder]');
    for (var p = 0; p < placeholders.length; p++) {
      placeholders[p].setAttribute('placeholder', t(placeholders[p].getAttribute('data-i18n-placeholder')));
    }
    // template content is not matched by document.querySelectorAll
    var tplNodes = tpl.content.querySelectorAll('[data-i18n]');
    for (var k = 0; k < tplNodes.length; k++) {
      tplNodes[k].textContent = t(tplNodes[k].getAttribute('data-i18n'));
    }
    var tplAria = tpl.content.querySelectorAll('[data-i18n-aria]');
    for (var m = 0; m < tplAria.length; m++) {
      tplAria[m].setAttribute('aria-label', t(tplAria[m].getAttribute('data-i18n-aria')));
    }
  }

  /* ---------- DOM ---------- */
  var grid = document.getElementById('grid');
  var empty = document.getElementById('empty');
  var tpl = document.getElementById('card-tpl');
  var connBadge = document.getElementById('conn');
  var statTotal = document.getElementById('stat-total');
  var statOnline = document.getElementById('stat-online');
  var statOffline = document.getElementById('stat-offline');
  var statCpu = document.getElementById('stat-cpu');
  var statCost = document.getElementById('stat-cost');
  var adminBtn = document.getElementById('admin-btn');
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
  var groupToggle = document.getElementById('group-toggle');
  var pingPanel = document.getElementById('ping-panel');
  var pingAgent = document.getElementById('ping-agent');
  var pingTask = document.getElementById('ping-task');
  var pingRange = document.getElementById('ping-range');
  var pingMessage = document.getElementById('ping-message');
  var pingChart = document.getElementById('ping-chart');
  var pingLegend = document.getElementById('ping-legend');
  var pingLoss = document.getElementById('ping-loss');
  var diagPanel = document.getElementById('diag-panel');
  var diagAgent = document.getElementById('diag-agent');
  var diagTarget = document.getElementById('diag-target');
  var diagCycles = document.getElementById('diag-cycles');
  var diagPing = document.getElementById('diag-ping');
  var diagTraceroute = document.getElementById('diag-traceroute');
  var diagMtr = document.getElementById('diag-mtr');
  var diagError = document.getElementById('diag-error');
  var diagSessions = document.getElementById('diag-sessions');
  var diagEmpty = document.getElementById('diag-empty');
  var streamingPanel = document.getElementById('streaming-panel');
  var streamingAgent = document.getElementById('streaming-agent');
  var streamingMessage = document.getElementById('streaming-message');
  var streamingResults = document.getElementById('streaming-results');
  var adminPanel = document.getElementById('admin-panel');
  var entityModal = document.getElementById('entity-modal');

  var GAUGE_LEN = 251.33; // 2 * PI * 40
  var CURRENCY_SYMBOL = { CNY: '¥', USD: '$', EUR: '€' };
  var CYCLE_DIVISOR = { monthly: 1, quarterly: 3, yearly: 12 };
  var FEATURE_NAMES = ['lg', 'mtr', 'streaming', 'ping', 'tasks'];
  var CHART_COLORS = ['#fbbf24', '#38bdf8', '#a78bfa', '#34d399', '#f87171', '#fb7185'];

  var cards = new Map();
  var state = new Map();
  var diagRequests = new Map();
  var pingChartData = { tasks: [], points: [] };
  var pingLoadId = 0;
  var streamingLoadId = 0;
  var demoSeeded = false;
  var groupedView = localStorage.getItem('pharus.regionGrouping') !== 'false';
  var collapsedRegions = new Set();
  var adminController = null;
  try {
    JSON.parse(localStorage.getItem('pharus.collapsedRegions') || '[]').forEach(function (key) {
      collapsedRegions.add(key);
    });
  } catch (e) { /* ignore invalid view preference */ }

  /* ---------- formatting ---------- */
  function fmtBytes(b) {
    if (b == null) return '—';
    var units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
    var v = b, i = 0;
    while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
    return v.toFixed(v >= 100 || i === 0 ? 0 : 1) + ' ' + units[i];
  }
  function fmtRate(bps) { return bps == null ? '—' : fmtBytes(bps) + '/s'; }
  function fmtUptime(s) {
    if (s == null) return '—';
    var d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
    return d > 0 ? d + 'd ' + h + 'h' : h > 0 ? h + 'h ' + m + 'm' : m + 'm';
  }
  function fmtAmount(v) { return v >= 100 ? v.toFixed(0) : v.toFixed(2); }
  function fmtDate(ts) {
    var d = new Date(ts * 1000);
    var p = function (n) { return n < 10 ? '0' + n : '' + n; };
    return d.getFullYear() + '-' + p(d.getMonth() + 1) + '-' + p(d.getDate());
  }
  function pct(used, total) { return total > 0 ? Math.min(100, (used / total) * 100) : 0; }

  function field(node, name) { return node.querySelector('[data-f="' + name + '"]'); }

  function ensureCard(id) {
    if (cards.has(id)) return cards.get(id);
    var node = tpl.content.firstElementChild.cloneNode(true);
    grid.appendChild(node);
    var card = {
      el: node,
      name: field(node, 'name'),
      os: field(node, 'os'),
      status: field(node, 'status'),
      cpuArc: field(node, 'cpuArc'),
      cpuVal: field(node, 'cpuVal'),
      memFill: field(node, 'memFill'),
      memVal: field(node, 'memVal'),
      diskFill: field(node, 'diskFill'),
      diskVal: field(node, 'diskVal'),
      rx: field(node, 'rx'),
      tx: field(node, 'tx'),
      load: field(node, 'load'),
      uptime: field(node, 'uptime'),
      billing: field(node, 'billing'),
      trafficVal: field(node, 'trafficVal'),
      trafficFill: field(node, 'trafficFill'),
      expires: field(node, 'expires'),
      price: field(node, 'price'),
      resetDay: field(node, 'resetDay'),
      editBtn: field(node, 'editBtn'),
      region: field(node, 'region'),
      pingSection: field(node, 'pingSection'),
      pings: field(node, 'pings'),
      unlockSection: field(node, 'unlockSection'),
      unlock: field(node, 'unlock'),
      pingBtn: field(node, 'pingBtn'),
      diagBtn: field(node, 'diagBtn'),
      streamBtn: field(node, 'streamBtn')
    };
    card.editBtn.addEventListener('click', function () { openEdit(id); });
    card.pingBtn.addEventListener('click', function () { openFeaturePanel(pingPanel, pingAgent, id); });
    card.diagBtn.addEventListener('click', function () { openFeaturePanel(diagPanel, diagAgent, id); });
    card.streamBtn.addEventListener('click', function () { openFeaturePanel(streamingPanel, streamingAgent, id); });
    cards.set(id, card);
    return card;
  }

  function hasFeature(entry, name) {
    // Older snapshots did not include `features`; keep their existing UI available.
    return !entry || !Array.isArray(entry.features) || entry.features.indexOf(name) !== -1;
  }

  function entryName(entry, id) {
    return (entry && entry.name) || ('Agent #' + id);
  }

  function openFeaturePanel(panel, select, id) {
    select.value = String(id);
    select.dispatchEvent(new Event('change'));
    panel.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  function requestJson(url, options) {
    return fetch(url, options || {}).then(function (response) {
      return response.text().then(function (text) {
        var body = null;
        if (text) {
          try { body = JSON.parse(text); } catch (e) { body = null; }
        }
        if (!response.ok) {
          throw new Error((body && body.error) || ('HTTP ' + response.status));
        }
        return body || {};
      });
    });
  }

  function renderRegion(card, entry) {
    if (!entry.region) {
      card.region.hidden = true;
      card.region.textContent = '';
      return;
    }
    card.region.hidden = false;
    card.region.textContent = entry.region.code || entry.region.name || '—';
    card.region.title = entry.region.name || entry.region.code || '';
  }

  function chip(container, label, value, className) {
    var el = document.createElement('span');
    el.className = 'result-chip' + (className ? ' ' + className : '');
    var name = document.createElement('b');
    name.textContent = label;
    var val = document.createElement('span');
    val.className = 'num';
    val.textContent = value;
    el.appendChild(name);
    el.appendChild(val);
    container.appendChild(el);
  }

  function renderPings(card, entry) {
    var allowed = hasFeature(entry, 'ping');
    card.pingBtn.hidden = !allowed;
    card.pingSection.hidden = !allowed || !entry.pings || !entry.pings.length;
    card.pings.innerHTML = '';
    if (card.pingSection.hidden) return;
    entry.pings.forEach(function (result) {
      var loss = result.loss == null ? 0 : Math.max(0, Math.min(1, result.loss));
      var value = result.rtt_ms == null
        ? t('ping.unreachable')
        : Number(result.rtt_ms).toFixed(1) + ' ms';
      value += ' · ' + Math.round(loss * 100) + '% ' + t('ping.lossShort');
      chip(card.pings, result.label || ('#' + (result.task_id == null ? '—' : result.task_id)), value,
        result.rtt_ms == null || loss > 0.2 ? 'crit' : loss > 0 ? 'warn' : 'ok');
    });
  }

  function serviceStatus(result) {
    if (typeof result.status === 'string') return result.status;
    if (typeof result.unlocked === 'boolean') return result.unlocked ? 'available' : 'unavailable';
    if (typeof result.available === 'boolean') return result.available ? 'available' : 'unavailable';
    return 'unknown';
  }

  function statusClass(status) {
    status = String(status || '').toLowerCase();
    if (status === 'available' || status === 'ok' || status === 'unlocked' || status === 'true') return 'ok';
    if (status === 'unavailable' || status === 'blocked' || status === 'failed' || status === 'false') return 'crit';
    return '';
  }

  function renderUnlock(card, entry) {
    var allowed = hasFeature(entry, 'streaming');
    card.streamBtn.hidden = !allowed;
    card.unlockSection.hidden = !allowed || !entry.unlock || !entry.unlock.length;
    card.unlock.innerHTML = '';
    if (card.unlockSection.hidden) return;
    entry.unlock.forEach(function (result) {
      var status = serviceStatus(result);
      var label = result.service || result.name || t('streaming.service');
      var detail = result.region || result.detail || t('streaming.status.' + status);
      chip(card.unlock, label, detail, statusClass(status));
    });
  }

  function renderFeatures(card, entry) {
    card.diagBtn.hidden = !(hasFeature(entry, 'lg') || hasFeature(entry, 'mtr'));
    renderPings(card, entry);
    renderUnlock(card, entry);
  }

  function sortedAgents() {
    return Array.from(state.entries()).sort(function (a, b) {
      return entryName(a[1], a[0]).localeCompare(entryName(b[1], b[0]));
    });
  }

  function saveCollapsedRegions() {
    localStorage.setItem('pharus.collapsedRegions', JSON.stringify(Array.from(collapsedRegions)));
  }

  function renderAgentLayout() {
    var agents = sortedAgents();
    grid.innerHTML = '';
    grid.classList.toggle('is-grouped', groupedView);
    groupToggle.setAttribute('aria-pressed', groupedView ? 'true' : 'false');
    var toggleLabel = groupToggle.querySelector('[data-i18n]');
    toggleLabel.setAttribute('data-i18n', groupedView ? 'region.grouped' : 'region.flat');
    toggleLabel.textContent = t(groupedView ? 'region.grouped' : 'region.flat');

    if (!groupedView) {
      agents.forEach(function (pair) { grid.appendChild(ensureCard(pair[0]).el); });
      return;
    }

    var groups = new Map();
    agents.forEach(function (pair) {
      var region = pair[1].region;
      var key = region && region.code ? region.code : '__ungrouped';
      if (!groups.has(key)) groups.set(key, { region: region, agents: [] });
      groups.get(key).agents.push(pair);
    });
    var ordered = Array.from(groups.entries()).sort(function (a, b) {
      if (a[0] === '__ungrouped') return 1;
      if (b[0] === '__ungrouped') return -1;
      var an = (a[1].region && a[1].region.name) || a[0];
      var bn = (b[1].region && b[1].region.name) || b[0];
      return an.localeCompare(bn);
    });
    ordered.forEach(function (groupPair) {
      var key = groupPair[0];
      var group = groupPair[1];
      var section = document.createElement('section');
      section.className = 'region-group' + (collapsedRegions.has(key) ? ' collapsed' : '');
      var head = document.createElement('button');
      head.type = 'button';
      head.className = 'region-head';
      head.setAttribute('aria-expanded', collapsedRegions.has(key) ? 'false' : 'true');
      var code = document.createElement('span');
      code.className = 'region-code';
      code.textContent = key === '__ungrouped' ? '—' : key;
      var name = document.createElement('span');
      name.className = 'region-name';
      name.textContent = key === '__ungrouped' ? t('region.ungrouped') : ((group.region && group.region.name) || key);
      var count = document.createElement('span');
      count.className = 'region-count';
      count.textContent = t('region.count').replace('{n}', group.agents.length);
      var chevron = document.createElement('span');
      chevron.className = 'region-chevron';
      chevron.textContent = '⌄';
      head.appendChild(code);
      head.appendChild(name);
      head.appendChild(count);
      head.appendChild(chevron);
      var nested = document.createElement('div');
      nested.className = 'grid region-grid';
      nested.hidden = collapsedRegions.has(key);
      group.agents.forEach(function (pair) { nested.appendChild(ensureCard(pair[0]).el); });
      head.addEventListener('click', function () {
        var collapsed = collapsedRegions.has(key);
        if (collapsed) collapsedRegions.delete(key); else collapsedRegions.add(key);
        section.classList.toggle('collapsed', !collapsed);
        nested.hidden = !collapsed;
        head.setAttribute('aria-expanded', collapsed ? 'true' : 'false');
        saveCollapsedRegions();
      });
      section.appendChild(head);
      section.appendChild(nested);
      grid.appendChild(section);
    });
  }

  function refillAgentSelect(select) {
    var selected = select.value;
    select.innerHTML = '';
    sortedAgents().forEach(function (pair) {
      var option = document.createElement('option');
      option.value = pair[0];
      option.textContent = entryName(pair[1], pair[0]);
      select.appendChild(option);
    });
    if (selected && state.has(Number(selected))) select.value = selected;
  }

  function anyAgentFeature(features) {
    var found = false;
    state.forEach(function (entry) {
      if (features.some(function (feature) { return hasFeature(entry, feature); })) found = true;
    });
    return found;
  }

  function refreshAgentSelectors() {
    [pingAgent, diagAgent, streamingAgent].forEach(refillAgentSelect);
    pingPanel.hidden = state.size > 0 && !anyAgentFeature(['ping']);
    diagPanel.hidden = state.size > 0 && !anyAgentFeature(['lg', 'mtr']);
    streamingPanel.hidden = state.size > 0 && !anyAgentFeature(['streaming']);
    updatePingGate();
    updateDiagGate();
    updateStreamingGate();
  }

  function renderHeader() {
    var online = 0, cpuSum = 0, cpuCount = 0;
    var costs = {};
    state.forEach(function (a) {
      if (a.online) {
        online++;
        if (a.data) { cpuSum += a.data.cpu_usage; cpuCount++; }
      }
      if (a.billing && a.billing.price != null && a.billing.currency) {
        var div = CYCLE_DIVISOR[a.billing.cycle] || 1;
        costs[a.billing.currency] = (costs[a.billing.currency] || 0) + a.billing.price / div;
      }
    });
    statTotal.textContent = state.size;
    statOnline.textContent = online;
    statOffline.textContent = state.size - online;
    statCpu.textContent = cpuCount > 0 ? (cpuSum / cpuCount).toFixed(1) + '%' : '—';
    var parts = [];
    ['CNY', 'USD', 'EUR'].forEach(function (c) {
      if (costs[c]) parts.push(CURRENCY_SYMBOL[c] + fmtAmount(costs[c]));
    });
    statCost.textContent = parts.length ? parts.join(' + ') : '—';
    empty.hidden = state.size > 0;
  }

  function setStatus(card, online) {
    card.status.innerHTML = '';
    var dot = document.createElement('span');
    dot.className = 'dot';
    card.status.appendChild(dot);
    card.status.appendChild(document.createTextNode(online ? t('status.online') : t('status.offline')));
    card.status.classList.toggle('online', online);
    card.el.classList.toggle('is-online', online);
    card.el.classList.toggle('is-offline', !online);
  }

  function renderMetrics(card, d) {
    var cpu = Math.max(0, Math.min(100, d.cpu_usage));
    card.cpuArc.style.strokeDashoffset = (GAUGE_LEN * (1 - cpu / 100)).toFixed(2);
    card.cpuVal.textContent = cpu.toFixed(1) + '%';
    card.memFill.style.width = pct(d.mem_used, d.mem_total).toFixed(1) + '%';
    card.memVal.textContent = fmtBytes(d.mem_used) + ' / ' + fmtBytes(d.mem_total);
    card.diskFill.style.width = pct(d.disk_used, d.disk_total).toFixed(1) + '%';
    card.diskVal.textContent = fmtBytes(d.disk_used) + ' / ' + fmtBytes(d.disk_total);
    card.rx.textContent = fmtRate(d.net_rx_bps);
    card.tx.textContent = fmtRate(d.net_tx_bps);
    card.load.textContent = d.load1.toFixed(2);
    card.uptime.textContent = fmtUptime(d.uptime);
  }

  function hasBilling(b) {
    return b && (b.reset_day != null || b.quota_bytes != null || b.expires_at != null || b.price != null);
  }

  function renderBilling(card, entry) {
    var b = entry.billing;
    var tr = entry.traffic;
    card.billing.hidden = !(hasBilling(b) || adminOn);
    card.editBtn.hidden = !adminOn;
    if (card.billing.hidden) return;
    b = b || {};

    var used = tr ? (tr.rx_bytes || 0) + (tr.tx_bytes || 0) : 0;
    if (b.quota_bytes != null) {
      card.trafficVal.textContent = fmtBytes(used) + ' / ' + fmtBytes(b.quota_bytes);
      var p = pct(used, b.quota_bytes);
      card.trafficFill.style.width = p.toFixed(1) + '%';
      card.trafficFill.classList.toggle('warn', p > 80 && p <= 95);
      card.trafficFill.classList.toggle('crit', p > 95);
    } else {
      card.trafficVal.textContent = fmtBytes(used);
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
        card.expires.textContent = fmtDate(b.expires_at) + ' · ' + t('billing.daysLeft').replace('{n}', days);
        card.expires.className = 'v' + (days <= 7 ? ' warn' : '');
      }
    } else {
      card.expires.textContent = '—';
      card.expires.className = 'v';
    }

    if (b.price != null && b.currency) {
      var sym = CURRENCY_SYMBOL[b.currency] || (b.currency + ' ');
      card.price.textContent = sym + fmtAmount(b.price) + (b.cycle ? ' · ' + t('billing.cycle.' + b.cycle) : '');
    } else {
      card.price.textContent = '—';
    }

    card.resetDay.textContent = b.reset_day != null ? String(b.reset_day) : '—';
  }

  /* ---------- ping history ---------- */
  function updatePingGate() {
    var id = Number(pingAgent.value);
    var entry = state.get(id);
    var enabled = !!entry && hasFeature(entry, 'ping');
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
    var id = Number(pingAgent.value);
    var entry = state.get(id);
    if (!entry || !hasFeature(entry, 'ping')) {
      updatePingGate();
      return;
    }
    if (resetTask) pingTask.value = '';
    var selectedTask = pingTask.value;
    var seq = ++pingLoadId;
    var url = '/api/agents/' + id + '/ping?range=' + encodeURIComponent(pingRange.value);
    if (selectedTask) url += '&task_id=' + encodeURIComponent(selectedTask);
    pingMessage.textContent = t('common.loading');
    requestJson(url).then(function (body) {
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
    var entry = state.get(Number(diagAgent.value));
    var online = !!entry && !!entry.online;
    var lg = !!entry && hasFeature(entry, 'lg');
    var mtr = !!entry && hasFeature(entry, 'mtr');
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
    title.textContent = entryName(state.get(agentId), agentId) + ' · ' + diagnosticLabel(kind) + ' ' + target;
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
    var id = Number(diagAgent.value);
    var entry = state.get(id);
    var required = kind === 'mtr' ? 'mtr' : 'lg';
    var target = diagTarget.value.trim();
    diagError.hidden = true;
    if (!entry || !entry.online) {
      diagError.textContent = t('diag.offline');
      diagError.hidden = false;
      return;
    }
    if (!hasFeature(entry, required)) {
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
    var body = { agent_id: id, target: target };
    var url = '/api/diag/lg';
    if (kind === 'mtr') {
      url = '/api/diag/mtr';
      body.cycles = Math.max(1, Math.min(100, parseInt(diagCycles.value, 10) || 10));
    } else {
      body.kind = kind;
    }
    requestJson(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body)
    }).then(function (response) {
      if (!response.request_id) throw new Error(t('diag.missingRequestId'));
      createDiagnosticSession(response.request_id, id, kind, target);
    }).catch(function (error) {
      diagError.textContent = t('common.error') + ': ' + error.message;
      diagError.hidden = false;
    });
  }

  /* ---------- streaming service results ---------- */
  function updateStreamingGate() {
    var entry = state.get(Number(streamingAgent.value));
    var enabled = !!entry && hasFeature(entry, 'streaming');
    if (entry && !enabled) {
      streamingMessage.textContent = t('feature.disabled').replace('{feature}', t('feature.streaming'));
      streamingResults.innerHTML = '';
    }
  }

  function renderServiceResults(results) {
    streamingResults.innerHTML = '';
    (results || []).forEach(function (result) {
      var status = serviceStatus(result);
      var card = document.createElement('article');
      card.className = 'service-result ' + statusClass(status);
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
      card.appendChild(head);
      if (result.detail || result.region) {
        var detail = document.createElement('p');
        detail.textContent = result.detail || result.region;
        card.appendChild(detail);
      }
      streamingResults.appendChild(card);
    });
    if (!results || !results.length) streamingMessage.textContent = t('streaming.noData');
  }

  function loadStreaming() {
    var id = Number(streamingAgent.value);
    var entry = state.get(id);
    if (!entry || !hasFeature(entry, 'streaming')) {
      updateStreamingGate();
      return;
    }
    var seq = ++streamingLoadId;
    streamingMessage.textContent = t('common.loading');
    requestJson('/api/agents/' + id + '/streaming').then(function (body) {
      if (seq !== streamingLoadId) return;
      var results = Array.isArray(body.results) ? body.results : [];
      entry.unlock = results;
      renderUnlock(ensureCard(id), entry);
      streamingMessage.textContent = '';
      renderServiceResults(results);
    }).catch(function (error) {
      if (seq !== streamingLoadId) return;
      streamingResults.innerHTML = '';
      streamingMessage.textContent = t('common.error') + ': ' + error.message;
    });
  }

  function applySnapshot(a) {
    var entry = {
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
    state.set(a.agent_id, entry);
    var card = ensureCard(a.agent_id);
    card.name.textContent = a.name || 'Agent #' + a.agent_id;
    card.os.textContent = a.info ? a.info.os + ' · ' + a.info.cpu_cores + 'C' : '—';
    setStatus(card, a.online);
    if (a.data) renderMetrics(card, a.data);
    renderBilling(card, entry);
    renderRegion(card, entry);
    renderFeatures(card, entry);
    renderAgentLayout();
    refreshAgentSelectors();
    renderHeader();
  }

  function setConn(up, label) {
    connBadge.innerHTML = '';
    var dot = document.createElement('span');
    dot.className = 'conn-dot';
    connBadge.appendChild(dot);
    connBadge.appendChild(document.createTextNode(label));
    connBadge.classList.toggle('conn-up', up);
    connBadge.classList.toggle('conn-down', !up);
  }

  function connect() {
    var proto = location.protocol === 'https:' ? 'wss' : 'ws';
    var ws = new WebSocket(proto + '://' + location.host + '/api/stream');
    setConn(false, t('conn.connecting'));

    ws.onopen = function () { setConn(true, t('conn.live')); };
    ws.onclose = function () {
      setConn(false, t('conn.reconnecting'));
      setTimeout(connect, 2000);
    };
    ws.onerror = function () { ws.close(); };
    ws.onmessage = function (ev) {
      var msg;
      try { msg = JSON.parse(ev.data); } catch (e) { return; }
      if (msg.type === 'snapshot') {
        msg.agents.forEach(applySnapshot);
        loadPingHistory(false);
        loadStreaming();
        seedDemoFixtures();
      } else if (msg.type === 'metrics') {
        var a = state.get(msg.agent_id) || {};
        a.agent_id = msg.agent_id;
        a.online = msg.online;
        a.data = msg.data;
        state.set(msg.agent_id, a);
        var card = ensureCard(msg.agent_id);
        setStatus(card, msg.online);
        renderMetrics(card, msg.data);
        updateDiagGate();
        renderHeader();
      } else if (msg.type === 'status') {
        var b = state.get(msg.agent_id) || {};
        b.agent_id = msg.agent_id;
        b.online = msg.online;
        state.set(msg.agent_id, b);
        setStatus(ensureCard(msg.agent_id), msg.online);
        updateDiagGate();
        renderHeader();
      } else if (msg.type === 'billing') {
        var c = state.get(msg.agent_id) || {};
        c.agent_id = msg.agent_id;
        c.billing = msg.billing;
        c.traffic = msg.traffic;
        state.set(msg.agent_id, c);
        renderBilling(ensureCard(msg.agent_id), c);
        renderHeader();
      } else if (msg.type === 'pings') {
        var d = state.get(msg.agent_id) || { agent_id: msg.agent_id };
        d.pings = Array.isArray(msg.results) ? msg.results : [];
        state.set(msg.agent_id, d);
        renderPings(ensureCard(msg.agent_id), d);
      } else if (msg.type === 'unlock') {
        var e = state.get(msg.agent_id) || { agent_id: msg.agent_id };
        e.unlock = Array.isArray(msg.results) ? msg.results : [];
        state.set(msg.agent_id, e);
        renderUnlock(ensureCard(msg.agent_id), e);
        if (Number(streamingAgent.value) === msg.agent_id) {
          streamingMessage.textContent = '';
          renderServiceResults(e.unlock);
        }
      } else if (msg.type === 'diag_result') {
        handleDiagnosticFrame(msg);
      } else if (msg.type === 'features_update') {
        var f = state.get(msg.agent_id) || { agent_id: msg.agent_id };
        f.features = Array.isArray(msg.features) ? msg.features.slice() : [];
        state.set(msg.agent_id, f);
        renderFeatures(ensureCard(msg.agent_id), f);
        refreshAgentSelectors();
        if (adminController) adminController.notifyAgentUpdate();
      } else if (msg.type === 'region_update') {
        var g = state.get(msg.agent_id) || { agent_id: msg.agent_id };
        g.region = msg.region || null;
        state.set(msg.agent_id, g);
        renderRegion(ensureCard(msg.agent_id), g);
        renderAgentLayout();
        if (adminController) adminController.notifyAgentUpdate();
      }
    };
  }

  /* ---------- admin mode ---------- */
  var adminToken = null;
  var adminOn = false;
  var editingId = null;

  function handleUnauthorized() {
    sessionStorage.removeItem('pharus.admin');
    adminToken = null;
    setAdmin(false);
    tokenErr.textContent = t('admin.unauthorized');
    tokenErr.hidden = false;
    tokenModal.hidden = false;
  }

  function adminRequest(url, options) {
    options = options || {};
    var headers = {};
    Object.keys(options.headers || {}).forEach(function (key) { headers[key] = options.headers[key]; });
    headers.Authorization = 'Bearer ' + adminToken;
    return fetch(url, {
      method: options.method || 'GET',
      headers: headers,
      body: options.body
    }).then(function (response) {
      return response.text().then(function (text) {
        var body = null;
        if (text) {
          try { body = JSON.parse(text); } catch (e) { body = null; }
        }
        if (response.status === 401) {
          handleUnauthorized();
          throw new Error(t('admin.unauthorized'));
        }
        if (!response.ok) throw new Error((body && body.error) || ('HTTP ' + response.status));
        return body || {};
      });
    });
  }

  function initAdminController() {
    if (adminController || !window.PharusAdmin) return;
    adminController = window.PharusAdmin.create({
      root: adminPanel,
      content: document.getElementById('admin-content'),
      error: document.getElementById('admin-error'),
      modal: entityModal,
      form: document.getElementById('entity-form'),
      fields: document.getElementById('entity-fields'),
      title: document.getElementById('entity-title'),
      formError: document.getElementById('entity-err'),
      t: t,
      request: adminRequest,
      getAgents: function () { return sortedAgents(); },
      hasFeature: hasFeature
    });
  }

  function setAdmin(on) {
    adminOn = on;
    adminBtn.classList.toggle('active', on);
    adminPanel.hidden = !on;
    if (adminController) adminController.setVisible(on);
    state.forEach(function (entry, id) {
      renderBilling(ensureCard(id), entry);
    });
  }

  function checkToken(tok) {
    return fetch('/api/admin/check', {
      method: 'POST',
      headers: tok ? { Authorization: 'Bearer ' + tok } : {}
    }).then(function (r) { return r.status; }).catch(function () { return 0; });
  }

  function probeAdmin() {
    // 404 = admin api disabled on this server; 401 = enabled
    checkToken(null).then(function (status) {
      if (status === 404 || status === 0) return;
      adminBtn.hidden = false;
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
    });
  }

  function openEdit(id) {
    var entry = state.get(id) || {};
    var b = entry.billing || {};
    editingId = id;
    editTitle.textContent = entry.name || ('Agent #' + id);
    fResetDay.value = b.reset_day != null ? b.reset_day : '';
    fQuotaGb.value = b.quota_bytes != null ? Math.round(b.quota_bytes / 1073741824 * 100) / 100 : '';
    fExpiresOn.value = b.expires_at != null ? fmtDate(b.expires_at) : '';
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
      var c = state.get(editingId) || {};
      c.billing = res.billing;
      c.traffic = res.traffic;
      state.set(editingId, c);
      renderBilling(ensureCard(editingId), c);
      renderHeader();
      editModal.hidden = true;
    }).catch(function (e) {
      if (e === 'unauthorized') {
        sessionStorage.removeItem('pharus.admin');
        adminToken = null;
        setAdmin(false);
        editModal.hidden = true;
        tokenErr.textContent = t('admin.unauthorized');
        tokenErr.hidden = false;
        tokenModal.hidden = false;
      } else {
        editErr.textContent = t('admin.error') + ': ' + e;
        editErr.hidden = false;
      }
    });
  }

  adminBtn.addEventListener('click', function () {
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
  groupToggle.addEventListener('click', function () {
    groupedView = !groupedView;
    localStorage.setItem('pharus.regionGrouping', groupedView ? 'true' : 'false');
    renderAgentLayout();
  });
  pingAgent.addEventListener('change', function () { updatePingGate(); loadPingHistory(true); });
  pingTask.addEventListener('change', function () { loadPingHistory(false); });
  pingRange.addEventListener('change', function () { loadPingHistory(false); });
  diagAgent.addEventListener('change', updateDiagGate);
  diagPing.addEventListener('click', function () { runDiagnostic('ping'); });
  diagTraceroute.addEventListener('click', function () { runDiagnostic('traceroute'); });
  diagMtr.addEventListener('click', function () { runDiagnostic('mtr'); });
  streamingAgent.addEventListener('change', function () { updateStreamingGate(); loadStreaming(); });
  var chartResizePending = false;
  window.addEventListener('resize', function () {
    if (chartResizePending) return;
    chartResizePending = true;
    requestAnimationFrame(function () { chartResizePending = false; drawPingChart(); });
  });
  [tokenModal, editModal, entityModal].forEach(function (mask) {
    mask.addEventListener('click', function (ev) {
      if (ev.target === mask || ev.target.hasAttribute('data-close')) mask.hidden = true;
    });
  });
  document.addEventListener('keydown', function (ev) {
    if (ev.key === 'Escape') {
      tokenModal.hidden = true;
      editModal.hidden = true;
      entityModal.hidden = true;
    }
  });

  function seedDemoFixtures() {
    if (demoSeeded || !window.PHARUS_DEMO_FIXTURES) return;
    demoSeeded = true;
    (window.PHARUS_DEMO_FIXTURES.diagnostics || []).forEach(function (example) {
      createDiagnosticSession(example.request_id, example.agent_id, example.kind, example.target);
      (example.frames || []).forEach(function (frame) {
        var message = Object.assign({
          type: 'diag_result', request_id: example.request_id, agent_id: example.agent_id,
          kind: example.kind, stream: null, data: null, result: null, done: false, exit_code: null
        }, frame);
        handleDiagnosticFrame(message);
      });
    });
  }

  /* ---------- boot: language first, then live data ---------- */
  var lang = detectLang();
  loadLang(lang)
    .catch(function () { return loadLang('en'); })
    .catch(function () { return {}; })
    .then(function (dict) {
      i18n = dict || {};
      applyStatics();
      initAdminController();
      connect();
      probeAdmin();
    });
})();
