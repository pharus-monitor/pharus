/* Pharus default theme — shared helpers (i18n, formatting, transport).
   Loaded by index.html / host.html / admin.html before their page scripts. */
(function () {
  'use strict';

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

  /// Load the best-matching language, apply static labels and resolve.
  /// Every page calls this once before doing anything else.
  function ready() {
    return loadLang(detectLang())
      .catch(function () { return loadLang('en'); })
      .catch(function () { return {}; })
      .then(function (dict) {
        i18n = dict || {};
        applyStatics();
        return dict;
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
    var tpl = document.getElementById('card-tpl');
    if (tpl) {
      var tplNodes = tpl.content.querySelectorAll('[data-i18n]');
      for (var k = 0; k < tplNodes.length; k++) {
        tplNodes[k].textContent = t(tplNodes[k].getAttribute('data-i18n'));
      }
      var tplAria = tpl.content.querySelectorAll('[data-i18n-aria]');
      for (var m = 0; m < tplAria.length; m++) {
        tplAria[m].setAttribute('aria-label', t(tplAria[m].getAttribute('data-i18n-aria')));
      }
    }
  }

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

  function hasFeature(entry, name) {
    // Older snapshots did not include `features`; keep their existing UI available.
    return !entry || !Array.isArray(entry.features) || entry.features.indexOf(name) !== -1;
  }

  function entryName(entry, id) {
    return (entry && entry.name) || ('Agent #' + id);
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

  function serviceStatus(result) {
    if (typeof result.status === 'string') return result.status;
    if (typeof result.unlocked === 'boolean') return result.unlocked ? 'available' : 'unavailable';
    if (typeof result.available === 'boolean') return result.available ? 'available' : 'unavailable';
    return 'unknown';
  }

  function statusClass(status) {
    status = String(status || '').toLowerCase();
    if (status === 'available' || status === 'ok' || status === 'unlocked' || status === 'true' || status === 'yes') return 'ok';
    if (status === 'unavailable' || status === 'blocked' || status === 'failed' || status === 'false' || status === 'no') return 'crit';
    return '';
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

  /* ---------- live stream ---------- */
  function connectStream(onMessage) {
    var connBadge = document.getElementById('conn');
    function setConn(up, label) {
      if (!connBadge) return;
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
        onMessage(msg);
      };
    }
    connect();
  }

  /* ---------- theme (dark / light / follow system) ---------- */
  function applyTheme(mode) {
    var root = document.documentElement;
    var systemLight = window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches;
    if (mode === 'light' || (mode === 'auto' && systemLight)) {
      root.setAttribute('data-theme', 'light');
    } else {
      root.removeAttribute('data-theme'); // dark default
    }
  }

  function initTheme() {
    var mode = localStorage.getItem('pharus.theme') || 'auto';
    applyTheme(mode);
    var media = window.matchMedia && window.matchMedia('(prefers-color-scheme: light)');
    if (media && media.addEventListener) {
      media.addEventListener('change', function () {
        if ((localStorage.getItem('pharus.theme') || 'auto') === 'auto') applyTheme('auto');
      });
    }
    var btn = document.getElementById('theme-btn');
    if (!btn) return;

    // dropdown menu: dark / light / auto
    var ICONS = { dark: '●', light: '○', auto: '◐' };
    var menu = document.createElement('div');
    menu.className = 'theme-menu';
    menu.setAttribute('role', 'menu');
    menu.hidden = true;
    var items = {};
    ['light', 'dark', 'auto'].forEach(function (m) {
      var item = document.createElement('button');
      item.type = 'button';
      item.className = 'theme-menu-item';
      item.setAttribute('role', 'menuitem');
      item.setAttribute('data-theme-mode', m);
      var icon = document.createElement('span');
      icon.className = 'theme-menu-icon';
      icon.textContent = ICONS[m];
      var label = document.createElement('span');
      label.textContent = t('theme.' + m);
      item.appendChild(icon);
      item.appendChild(label);
      item.addEventListener('click', function (ev) {
        ev.stopPropagation();
        mode = m;
        localStorage.setItem('pharus.theme', mode);
        applyTheme(mode);
        updateItems();
        menu.hidden = true;
      });
      items[m] = item;
      menu.appendChild(item);
    });
    document.body.appendChild(menu);

    function positionMenu() {
      var r = btn.getBoundingClientRect();
      var w = menu.offsetWidth || 150;
      var left = Math.max(8, Math.min(r.left, window.innerWidth - w - 8));
      menu.style.top = (r.bottom + 6) + 'px';
      menu.style.left = left + 'px';
      menu.style.right = 'auto';
    }
    function updateItems() {
      Object.keys(items).forEach(function (m) {
        items[m].classList.toggle('active', m === mode);
      });
      btn.textContent = t('theme.' + mode);
    }
    btn.addEventListener('click', function (ev) {
      ev.stopPropagation();
      positionMenu();
      menu.hidden = !menu.hidden;
    });
    menu.addEventListener('click', function (ev) { ev.stopPropagation(); });
    document.addEventListener('click', function () { menu.hidden = true; });
    updateItems();
  }

  window.Pharus = {
    t: t,
    ready: ready,
    applyStatics: applyStatics,
    detectLang: detectLang,
    loadLang: loadLang,
    applyTheme: applyTheme,
    initTheme: initTheme,
    fmtBytes: fmtBytes,
    fmtRate: fmtRate,
    fmtUptime: fmtUptime,
    fmtAmount: fmtAmount,
    fmtDate: fmtDate,
    pct: pct,
    field: field,
    hasFeature: hasFeature,
    entryName: entryName,
    chip: chip,
    serviceStatus: serviceStatus,
    statusClass: statusClass,
    requestJson: requestJson,
    connectStream: connectStream
  };
})();
