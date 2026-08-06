/* Pharus default theme — live dashboard driven by /api/stream */
(function () {
  'use strict';

  var grid = document.getElementById('grid');
  var empty = document.getElementById('empty');
  var tpl = document.getElementById('card-tpl');
  var connBadge = document.getElementById('conn');
  var statTotal = document.getElementById('stat-total');
  var statOnline = document.getElementById('stat-online');
  var statOffline = document.getElementById('stat-offline');
  var statCpu = document.getElementById('stat-cpu');

  var GAUGE_LEN = 251.33; // 2 * PI * 40

  var cards = new Map();
  var state = new Map();

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
      uptime: field(node, 'uptime')
    };
    cards.set(id, card);
    return card;
  }

  function renderHeader() {
    var online = 0, cpuSum = 0, cpuCount = 0;
    state.forEach(function (a) {
      if (a.online) {
        online++;
        if (a.data) { cpuSum += a.data.cpu_usage; cpuCount++; }
      }
    });
    statTotal.textContent = state.size;
    statOnline.textContent = online;
    statOffline.textContent = state.size - online;
    statCpu.textContent = cpuCount > 0 ? (cpuSum / cpuCount).toFixed(1) + '%' : '—';
    empty.hidden = state.size > 0;
  }

  function setStatus(card, online) {
    card.status.innerHTML = '';
    var dot = document.createElement('span');
    dot.className = 'dot';
    card.status.appendChild(dot);
    card.status.appendChild(document.createTextNode(online ? '在线' : '离线'));
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

  function applySnapshot(a) {
    state.set(a.agent_id, { online: a.online, data: a.data, name: a.name, info: a.info });
    var card = ensureCard(a.agent_id);
    card.name.textContent = a.name || 'Agent #' + a.agent_id;
    card.os.textContent = a.info ? a.info.os + ' · ' + a.info.cpu_cores + 'C' : '—';
    setStatus(card, a.online);
    if (a.data) renderMetrics(card, a.data);
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
    setConn(false, '连接中…');

    ws.onopen = function () { setConn(true, '实时推送'); };
    ws.onclose = function () {
      setConn(false, '已断开，重连中…');
      setTimeout(connect, 2000);
    };
    ws.onerror = function () { ws.close(); };
    ws.onmessage = function (ev) {
      var msg;
      try { msg = JSON.parse(ev.data); } catch (e) { return; }
      if (msg.type === 'snapshot') {
        msg.agents.forEach(applySnapshot);
      } else if (msg.type === 'metrics') {
        var a = state.get(msg.agent_id) || {};
        a.online = msg.online;
        a.data = msg.data;
        state.set(msg.agent_id, a);
        var card = ensureCard(msg.agent_id);
        setStatus(card, msg.online);
        renderMetrics(card, msg.data);
        renderHeader();
      } else if (msg.type === 'status') {
        var b = state.get(msg.agent_id) || {};
        b.online = msg.online;
        state.set(msg.agent_id, b);
        setStatus(ensureCard(msg.agent_id), msg.online);
        renderHeader();
      }
    };
  }

  connect();
})();
