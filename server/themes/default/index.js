/* Pharus default theme — index page (host cards + overview). */
(function () {
  'use strict';

  var P = window.Pharus;
  var t = P.t;

  /* ---------- DOM ---------- */
  var grid = document.getElementById('grid');
  var empty = document.getElementById('empty');
  var tpl = document.getElementById('card-tpl');
  var statTotal = document.getElementById('stat-total');
  var statOnline = document.getElementById('stat-online');
  var statOffline = document.getElementById('stat-offline');
  var statExpiring = document.getElementById('stat-expiring');
  var statCost = document.getElementById('stat-cost');
  var groupToggle = document.getElementById('group-toggle');
  var adminLink = document.getElementById('admin-link');

  var GAUGE_LEN = 251.33; // 2 * PI * 40
  var CURRENCY_SYMBOL = { CNY: '¥', USD: '$', EUR: '€' };
  var CYCLE_DIVISOR = { monthly: 1, quarterly: 3, yearly: 12 };
  var FEATURE_NAMES = ['lg', 'mtr', 'streaming', 'ping', 'tasks'];

  var cards = new Map();
  var state = new Map();
  var groupedView = localStorage.getItem('pharus.regionGrouping') !== 'false';
  // Display order. Site-wide order from /api/meta wins over localStorage;
  // localStorage covers visitors who reorder without an admin session.
  var agentOrder = readOrder('pharus.agentOrder');
  var regionOrder = readOrder('pharus.regionOrder');
  var collapsedRegions = new Set();
  try {
    JSON.parse(localStorage.getItem('pharus.collapsedRegions') || '[]').forEach(function (key) {
      collapsedRegions.add(key);
    });
  } catch (e) { /* ignore invalid view preference */ }

  function openHost(id) {
    window.location.href = 'host.html?id=' + id;
  }

  function ensureCard(id) {
    if (cards.has(id)) return cards.get(id);
    var node = tpl.content.firstElementChild.cloneNode(true);
    grid.appendChild(node);
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
      unlock: P.field(node, 'unlock')
    };
    card.ips4 = P.field(node, 'ips4');
    card.ips6 = P.field(node, 'ips6');
    card.ipRow = node.querySelector('.ip-row');
    node.dataset.agentId = String(id);
    var grip = P.field(node, 'grip');
    if (grip) {
      grip.title = t('sort.drag');
      enableDrag(grip, {
        item: '.card',
        idOf: function (el) { return Number(el.dataset.agentId); },
        onReorder: function (dragId, refId, after) {
          agentOrder = reordered(currentAgentIds(), dragId, refId, after);
          persistOrder();
          renderAgentLayout();
        }
      });
    }
    node.addEventListener('click', function (ev) {
      if (ev.target.closest('button')) return;
      openHost(id);
    });
    cards.set(id, card);
    return card;
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

  function renderPings(card, entry) {
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

  function renderUnlock(card, entry) {
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

  function readOrder(key) {
    try {
      var v = JSON.parse(localStorage.getItem(key) || '[]');
      return Array.isArray(v) ? v : [];
    } catch (e) {
      return [];
    }
  }

  function sortedAgents() {
    var pos = new Map();
    agentOrder.forEach(function (id, i) {
      if (!pos.has(id)) pos.set(id, i);
    });
    return Array.from(state.entries()).sort(function (a, b) {
      var pa = pos.has(a[0]) ? pos.get(a[0]) : agentOrder.length;
      var pb = pos.has(b[0]) ? pos.get(b[0]) : agentOrder.length;
      if (pa !== pb) return pa - pb;
      return P.entryName(a[1], a[0]).localeCompare(P.entryName(b[1], b[0]));
    });
  }

  // Place dragId before/after refId in the current effective order.
  function reordered(order, dragId, refId, after) {
    var ids = order.filter(function (v) { return v !== dragId; });
    var idx = refId == null ? ids.length : ids.indexOf(refId);
    if (idx < 0) idx = ids.length; else if (after) idx += 1;
    ids.splice(idx, 0, dragId);
    return ids;
  }

  function persistOrder() {
    localStorage.setItem('pharus.agentOrder', JSON.stringify(agentOrder));
    localStorage.setItem('pharus.regionOrder', JSON.stringify(regionOrder));
    var tok = localStorage.getItem('pharus.admin');
    if (!tok) return;
    [['agent_order', agentOrder], ['region_order', regionOrder]].forEach(function (kv) {
      fetch('/api/admin/settings', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json', 'Authorization': 'Bearer ' + tok },
        body: JSON.stringify({ key: kv[0], value: JSON.stringify(kv[1]) })
      }).catch(function () {});
    });
  }

  function currentAgentIds() {
    return sortedAgents().map(function (pair) { return pair[0]; });
  }

  function currentRegionCodes() {
    var codes = [];
    sortedAgents().forEach(function (pair) {
      var region = pair[1].region;
      var code = region && region.code ? region.code : null;
      if (code && codes.indexOf(code) < 0) codes.push(code);
    });
    var pos = new Map();
    regionOrder.forEach(function (c, i) { if (!pos.has(c)) pos.set(c, i); });
    return codes.sort(function (a, b) {
      var pa = pos.has(a) ? pos.get(a) : regionOrder.length;
      var pb = pos.has(b) ? pos.get(b) : regionOrder.length;
      if (pa !== pb) return pa - pb;
      return a.localeCompare(b);
    });
  }

  /* Pointer-based drag reorder (works with mouse and touch). */
  function enableDrag(grip, config) {
    grip.addEventListener('click', function (ev) {
      ev.preventDefault();
      ev.stopPropagation();
    });
    grip.addEventListener('pointerdown', function (ev) {
      if (ev.pointerType === 'mouse' && ev.button !== 0) return;
      ev.preventDefault();
      ev.stopPropagation();
      var item = grip.closest(config.item);
      if (!item) return;
      var startX = ev.clientX;
      var startY = ev.clientY;
      var dragging = false;
      var base = null;
      var over = null;

      function onMove(e) {
        if (!dragging) {
          if (Math.abs(e.clientX - startX) + Math.abs(e.clientY - startY) < 6) return;
          dragging = true;
          base = item.getBoundingClientRect();
          item.classList.add('dragging');
          document.body.classList.add('is-dragging');
          try { grip.setPointerCapture(ev.pointerId); } catch (err) {}
        }
        item.style.transform = 'translate(' + (e.clientX - startX) + 'px,' + (e.clientY - startY) + 'px)';
        var el = document.elementFromPoint(e.clientX, e.clientY);
        var target = el ? el.closest(config.item) : null;
        if (target && (target === item || target.parentNode !== item.parentNode)) target = null;
        if (over !== target) {
          if (over) over.classList.remove('drop-target');
          over = target;
          if (over) over.classList.add('drop-target');
        }
      }

      function finish(e, apply) {
        grip.removeEventListener('pointermove', onMove);
        grip.removeEventListener('pointerup', onUp);
        grip.removeEventListener('pointercancel', onCancel);
        item.style.transform = '';
        item.classList.remove('dragging');
        document.body.classList.remove('is-dragging');
        if (over) over.classList.remove('drop-target');
        if (dragging && apply && over) {
          var r = over.getBoundingClientRect();
          var after = config.vertical
            ? e.clientY > r.top + r.height / 2
            : (e.clientY > r.top + r.height * 0.6 ||
               (e.clientY > r.top + r.height * 0.4 && e.clientX > r.left + r.width / 2));
          config.onReorder(config.idOf(item), config.idOf(over), after);
        }
      }

      function onUp(e) { finish(e, true); }
      function onCancel(e) { finish(e, false); }

      grip.addEventListener('pointermove', onMove);
      grip.addEventListener('pointerup', onUp);
      grip.addEventListener('pointercancel', onCancel);
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
    var rpos = new Map();
    regionOrder.forEach(function (code, i) {
      if (!rpos.has(code)) rpos.set(code, i);
    });
    var ordered = Array.from(groups.entries()).sort(function (a, b) {
      if (a[0] === '__ungrouped') return 1;
      if (b[0] === '__ungrouped') return -1;
      var pa = rpos.has(a[0]) ? rpos.get(a[0]) : regionOrder.length;
      var pb = rpos.has(b[0]) ? rpos.get(b[0]) : regionOrder.length;
      if (pa !== pb) return pa - pb;
      var an = (a[1].region && a[1].region.name) || a[0];
      var bn = (b[1].region && b[1].region.name) || b[0];
      return an.localeCompare(bn);
    });
    ordered.forEach(function (groupPair) {
      var key = groupPair[0];
      var group = groupPair[1];
      var section = document.createElement('section');
      section.className = 'region-group' + (collapsedRegions.has(key) ? ' collapsed' : '');
      section.dataset.regionCode = key;
      var head = document.createElement('button');
      head.type = 'button';
      head.className = 'region-head';
      head.setAttribute('aria-expanded', collapsedRegions.has(key) ? 'false' : 'true');
      var grip = document.createElement('span');
      grip.className = 'drag-grip';
      grip.textContent = '⠿';
      grip.title = t('sort.drag');
      if (key === '__ungrouped') {
        grip.hidden = true;
      } else {
        enableDrag(grip, {
          item: '.region-group',
          vertical: true,
          idOf: function (el) { return el.dataset.regionCode; },
          onReorder: function (dragCode, refCode, after) {
            regionOrder = reordered(currentRegionCodes(), dragCode, refCode, after);
            persistOrder();
            renderAgentLayout();
          }
        });
      }
      head.appendChild(grip);
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

  var expiryAlertDays = 3;

  function renderHeader() {
    var online = 0;
    var expiring = 0;
    var costs = {};
    state.forEach(function (a) {
      if (a.online) {
        online++;
      }
      if (a.billing && a.billing.price != null && a.billing.currency) {
        var div = CYCLE_DIVISOR[a.billing.cycle] || 1;
        costs[a.billing.currency] = (costs[a.billing.currency] || 0) + a.billing.price / div;
      }
      if (a.billing && a.billing.expires_at != null) {
        var days = Math.ceil((a.billing.expires_at * 1000 - Date.now()) / 86400000);
        if (days >= 0 && days <= expiryAlertDays) expiring++;
      }
    });
    statTotal.textContent = state.size;
    statOnline.textContent = online;
    statOffline.textContent = state.size - online;
    statExpiring.textContent = expiring;
    statExpiring.title = t('stats.expiringHint').replace('{n}', expiryAlertDays);
    var parts = [];
    ['CNY', 'USD', 'EUR'].forEach(function (c) {
      if (costs[c]) parts.push(CURRENCY_SYMBOL[c] + P.fmtAmount(costs[c]));
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
    card.memFill.style.width = P.pct(d.mem_used, d.mem_total).toFixed(1) + '%';
    card.memVal.textContent = P.fmtBytes(d.mem_used) + ' / ' + P.fmtBytes(d.mem_total);
    card.diskFill.style.width = P.pct(d.disk_used, d.disk_total).toFixed(1) + '%';
    card.diskVal.textContent = P.fmtBytes(d.disk_used) + ' / ' + P.fmtBytes(d.disk_total);
    card.swapFill.style.width = P.pct(d.swap_used, d.swap_total).toFixed(1) + '%';
    card.swapVal.textContent = P.fmtBytes(d.swap_used) + ' / ' + P.fmtBytes(d.swap_total);
    card.rx.textContent = P.fmtRate(d.net_rx_bps);
    card.tx.textContent = P.fmtRate(d.net_tx_bps);
    card.load.textContent = d.load1.toFixed(2);
    card.uptime.textContent = P.fmtUptime(d.uptime);
  }

  function hasBilling(b) {
    return b && (b.reset_day != null || b.quota_bytes != null || b.expires_at != null || b.price != null);
  }

  function renderBilling(card, entry) {
    var b = entry.billing;
    var tr = entry.traffic;
    card.billing.hidden = !hasBilling(b);
    if (card.billing.hidden) return;
    b = b || {};

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
    if (a.data) {
      renderMetrics(card, a.data);
    }
    renderBilling(card, entry);
    renderRegion(card, entry);
    renderPings(card, entry);
    renderUnlock(card, entry);
    renderIps(card, entry);
    renderAgentLayout();
    renderHeader();
  }

  function renderIps(card, entry) {
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
    card.ips4.textContent = v4 ? P.maskIp(v4) : '';
    card.ips6.textContent = v6 ? P.maskIp(v6) : '';
  }

  function handleMessage(msg) {
    if (msg.type === 'snapshot') {
      msg.agents.forEach(applySnapshot);
    } else if (msg.type === 'metrics') {
      var a = state.get(msg.agent_id) || {};
      a.agent_id = msg.agent_id;
      a.online = msg.online;
      a.data = msg.data;
      state.set(msg.agent_id, a);
      var card = ensureCard(msg.agent_id);
      setStatus(card, msg.online);
      renderMetrics(card, msg.data);
      renderHeader();
    } else if (msg.type === 'status') {
      var b = state.get(msg.agent_id) || {};
      b.agent_id = msg.agent_id;
      b.online = msg.online;
      state.set(msg.agent_id, b);
      setStatus(ensureCard(msg.agent_id), msg.online);
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
    } else if (msg.type === 'features_update') {
      var f = state.get(msg.agent_id) || { agent_id: msg.agent_id };
      f.features = Array.isArray(msg.features) ? msg.features.slice() : [];
      state.set(msg.agent_id, f);
      renderPings(ensureCard(msg.agent_id), f);
      renderUnlock(ensureCard(msg.agent_id), f);
    } else if (msg.type === 'region_update') {
      var g = state.get(msg.agent_id) || { agent_id: msg.agent_id };
      g.region = msg.region || null;
      state.set(msg.agent_id, g);
      renderRegion(ensureCard(msg.agent_id), g);
      renderAgentLayout();
    }
  }

  function probeAdminLink() {
    // 404 = admin API disabled on this server; 401/200 = enabled
    P.requestJson('/api/meta').then(function (meta) {
      if (meta.expiry_alert_days) expiryAlertDays = meta.expiry_alert_days;
      if (Array.isArray(meta.agent_order)) agentOrder = meta.agent_order;
      if (Array.isArray(meta.region_order)) regionOrder = meta.region_order;
      renderAgentLayout();
      state.forEach(function (entry, id) {
        renderBilling(ensureCard(id), entry);
      });
      renderHeader();
      if (meta.admin_enabled) adminLink.hidden = false;
    }).catch(function () {});
  }

  groupToggle.addEventListener('click', function () {
    groupedView = !groupedView;
    localStorage.setItem('pharus.regionGrouping', groupedView ? 'true' : 'false');
    renderAgentLayout();
  });

  /* ---------- boot: language first, then live data ---------- */
  P.ready().then(function () {
    P.initTheme();
    P.requestJson('/api/status').then(function (agents) {
      (Array.isArray(agents) ? agents : []).forEach(applySnapshot);
    }).catch(function () {});
    P.connectStream(handleMessage);
    probeAdminLink();
  });
})();
