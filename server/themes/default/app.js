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

  var GAUGE_LEN = 251.33; // 2 * PI * 40
  var CURRENCY_SYMBOL = { CNY: '¥', USD: '$', EUR: '€' };
  var CYCLE_DIVISOR = { monthly: 1, quarterly: 3, yearly: 12 };

  var cards = new Map();
  var state = new Map();

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
      editBtn: field(node, 'editBtn')
    };
    card.editBtn.addEventListener('click', function () { openEdit(id); });
    cards.set(id, card);
    return card;
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

  function applySnapshot(a) {
    state.set(a.agent_id, {
      online: a.online,
      data: a.data,
      name: a.name,
      info: a.info,
      billing: a.billing || null,
      traffic: a.traffic || null
    });
    var card = ensureCard(a.agent_id);
    card.name.textContent = a.name || 'Agent #' + a.agent_id;
    card.os.textContent = a.info ? a.info.os + ' · ' + a.info.cpu_cores + 'C' : '—';
    setStatus(card, a.online);
    if (a.data) renderMetrics(card, a.data);
    renderBilling(card, state.get(a.agent_id));
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
      } else if (msg.type === 'billing') {
        var c = state.get(msg.agent_id) || {};
        c.billing = msg.billing;
        c.traffic = msg.traffic;
        state.set(msg.agent_id, c);
        renderBilling(ensureCard(msg.agent_id), c);
        renderHeader();
      }
    };
  }

  /* ---------- admin mode ---------- */
  var adminToken = null;
  var adminOn = false;
  var editingId = null;

  function setAdmin(on) {
    adminOn = on;
    adminBtn.classList.toggle('active', on);
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

  /* ---------- boot: language first, then live data ---------- */
  var lang = detectLang();
  loadLang(lang)
    .catch(function () { return loadLang('en'); })
    .catch(function () { return {}; })
    .then(function (dict) {
      i18n = dict || {};
      applyStatics();
      connect();
      probeAdmin();
    });
})();
