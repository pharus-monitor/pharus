/* Pharus default theme — shared helpers (i18n, formatting, transport).
   Loaded by index.html / host.html / admin.html before their page scripts. */
(function () {
  'use strict';

  var SUPPORTED = ['en', 'zh-CN', 'ja', 'ru'];
  var i18n = {};
  var metaCache = null;

  function fetchMeta() {
    if (metaCache) return metaCache;
    metaCache = fetch('/api/meta').then(function (r) {
      if (!r.ok) return null;
      return r.json();
    }).catch(function () { return null; });
    return metaCache;
  }

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

  /// Load the best-matching language (site default wins over browser),
  /// apply static labels and resolve. Every page calls this once first.
  function ready() {
    return fetchMeta().then(function (meta) {
      var lang = (meta && meta.default_language) || detectLang();
      return loadLang(lang)
        .catch(function () { return loadLang('en'); })
        .catch(function () { return {}; })
        .then(function (dict) {
          i18n = dict || {};
          applyStatics();
          applySiteMeta(meta);
          return dict;
        });
    });
  }

  function applySiteMeta(meta) {
    if (!meta) return;
    if (meta.site_name) {
      // Only the page title carries the site name; the header brand stays "Pharus".
      var suffix = String(t('doc.title') || '').split('·').pop() || '';
      document.title = meta.site_name + (suffix.trim() ? ' · ' + suffix.trim() : '');
    }
    if (meta.site_url) {
      var links = document.querySelectorAll('.site-footer a');
      for (var i = 0; i < links.length; i++) {
        if (links[i].textContent.trim() === 'Pharus') links[i].href = meta.site_url;
      }
    }
  }

  function reloadLanguage(lang) {
    return loadLang(lang)
      .catch(function () { return loadLang('en'); })
      .catch(function () { return {}; })
      .then(function (dict) {
        i18n = dict || {};
        applyStatics();
        return fetchMeta().then(function (meta) { applySiteMeta(meta); return dict; });
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
    pSelectSyncAll();
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
  /// "12.5 MiB/s · 1.2 GiB" — live rate plus the current billing-cycle total.
  function rateWithTotal(bps, cycleTotal) {
    var rate = fmtRate(bps);
    if (cycleTotal) return rate + ' · ' + fmtBytes(cycleTotal);
    return rate;
  }
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

  function isLinkLocalV6(ip) {
    return /^fe[89ab]/i.test(String(ip || ''));
  }

  function maskIp(ip) {
    var s = String(ip || '');
    if (s.indexOf(':') >= 0) {
      var parts = s.split(':');
      return parts.slice(0, 2).join(':') + ':****';
    }
    var v4 = s.split('.');
    if (v4.length === 4) {
      return v4[0] + '.' + v4[1] + '.***.***';
    }
    return '••••••••';
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

  /* ---------- custom dropdown (replaces native select popup) ----------
     The native <select> is kept (display:none) so existing code that reads
     .value / listens to change keeps working; a styled button + listbox is
     rendered on top. Options are re-rendered whenever the select mutates
     (i18n, dynamic option fills) and on open. */
  var SELECT_STATES = [];
  var docObserver = null;
  var valueDesc = null;
  try { valueDesc = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value'); } catch (e) { valueDesc = null; }

  function pSelectInit(root) {
    if (root && (root.nodeType === 1 || root.nodeType === 9)) {
      if (root.tagName === 'SELECT') pSelectBuild(root);
      else if (root.querySelectorAll) {
        var found = root.querySelectorAll('select:not([data-p-select])');
        for (var i = 0; i < found.length; i++) pSelectBuild(found[i]);
      }
    }
    if (!docObserver) {
      docObserver = new MutationObserver(function (mutations) {
        var pending = [];
        for (var m = 0; m < mutations.length; m++) {
          var nodes = mutations[m].addedNodes;
          for (var i = 0; i < nodes.length; i++) {
            var n = nodes[i];
            if (n.nodeType !== 1) continue;
            if (n.tagName === 'SELECT') { if (!n.getAttribute('data-p-select')) pending.push(n); }
            else if (n.querySelectorAll) {
              var inner = n.querySelectorAll('select:not([data-p-select])');
              for (var j = 0; j < inner.length; j++) pending.push(inner[j]);
            }
          }
        }
        for (var k = 0; k < pending.length; k++) pSelectBuild(pending[k]);
      });
      docObserver.observe(document.body, { childList: true, subtree: true });
    }
  }

  function pSelectSyncAll() {
    for (var i = 0; i < SELECT_STATES.length; i++) { SELECT_STATES[i].sync(); SELECT_STATES[i].fit(); }
  }

  function pSelectBuild(sel) {
    if (sel.getAttribute('data-p-select')) return;
    sel.setAttribute('data-p-select', '1');

    var wrap = document.createElement('div');
    wrap.className = 'p-select';
    if (sel.className) wrap.className += ' ' + sel.className;

    var btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'p-select-btn';
    btn.setAttribute('aria-haspopup', 'listbox');
    btn.setAttribute('aria-expanded', 'false');
    var label = document.createElement('span');
    label.className = 'p-select-label';
    btn.appendChild(label);

    var menu = document.createElement('div');
    menu.className = 'p-select-menu';
    menu.setAttribute('role', 'listbox');
    menu.style.display = 'none';

    var st = {
      sel: sel, wrap: wrap, btn: btn, label: label, menu: menu,
      items: [], active: -1, hover: -1, dirty: false
    };

    function render() {
      st.dirty = false;
      st.menu.innerHTML = '';
      st.items = [];
      var opts = sel.options;
      for (var i = 0; i < opts.length; i++) {
        (function (idx) {
          var o = document.createElement('button');
          o.type = 'button';
          o.className = 'p-select-opt';
          o.setAttribute('role', 'option');
          o.setAttribute('data-index', String(idx));
          o.textContent = opts[idx].textContent || ' ';
          o.addEventListener('mousedown', function (ev) { ev.preventDefault(); });
          o.addEventListener('click', function () { choose(idx); });
          st.menu.appendChild(o);
          st.items.push(o);
        })(i);
      }
      sync();
    }
    function sync() {
      var idx = sel.selectedIndex;
      var opt = idx >= 0 && idx < sel.options.length ? sel.options[idx] : null;
      // A no-break space keeps the button at full height when nothing is
      // selected (native selects keep their box height too).
      st.label.textContent = opt ? (opt.textContent || ' ') : ' ';
      st.active = idx;
      for (var i = 0; i < st.items.length; i++) st.items[i].classList.toggle('active', i === idx);
      setDisabled(sel.disabled);
    }
    // Width the button to fit its longest option (native selects do this too),
    // so it never collapses to just the currently-selected text in flex/grid
    // rows. Only matters when the wrapper is shrink-to-fit; full-width fields
    // already size the button to 100%.
    function fit() {
      var cs = getComputedStyle(btn);
      var probe = document.createElement('span');
      probe.style.cssText = 'position:absolute;visibility:hidden;white-space:nowrap;left:-9999px;top:0;';
      probe.style.fontFamily = cs.fontFamily;
      probe.style.fontSize = cs.fontSize;
      probe.style.fontWeight = cs.fontWeight;
      document.body.appendChild(probe);
      var maxW = 0;
      for (var i = 0; i < sel.options.length; i++) {
        probe.textContent = sel.options[i].textContent || '';
        var w = probe.getBoundingClientRect().width;
        if (w > maxW) maxW = w;
      }
      document.body.removeChild(probe);
      var padL = parseFloat(cs.paddingLeft) || 0;
      var padR = parseFloat(cs.paddingRight) || 0;
      btn.style.minWidth = Math.ceil(maxW + padL + padR + 2) + 'px';
    }
    function setDisabled(d) {
      st.wrap.classList.toggle('disabled', !!d);
      btn.tabIndex = d ? -1 : 0;
    }
    function clearHover() {
      st.hover = -1;
      for (var i = 0; i < st.items.length; i++) st.items[i].classList.remove('hover');
    }
    function setHover(i) {
      clearHover();
      if (i >= 0 && i < st.items.length) {
        st.hover = i;
        st.items[i].classList.add('hover');
        var it = st.items[i];
        if (it.offsetTop < menu.scrollTop) menu.scrollTop = it.offsetTop;
        else if (it.offsetTop + it.offsetHeight > menu.scrollTop + menu.offsetHeight) {
          menu.scrollTop = it.offsetTop + it.offsetHeight - menu.offsetHeight;
        }
      }
    }
    function choose(idx) {
      if (sel.disabled) return;
      if (idx >= 0 && idx < sel.options.length) {
        sel.selectedIndex = idx;
        sync();
        close();
        sel.dispatchEvent(new Event('change', { bubbles: true }));
      }
    }
    function positionMenu() {
      var r = btn.getBoundingClientRect();
      var mw = Math.max(120, Math.min(r.width, 260));
      var mh = Math.min(260, menu.offsetHeight || 160);
      var left = Math.max(8, Math.min(r.left, window.innerWidth - mw - 8));
      var top = r.bottom + 4;
      if (top + mh > window.innerHeight - 8) {
        var up = r.top - 4 - mh;
        top = up >= 8 ? up : Math.max(8, window.innerHeight - 8 - mh);
      }
      menu.style.left = left + 'px';
      menu.style.top = top + 'px';
      menu.style.width = mw + 'px';
    }
    function open() {
      if (sel.disabled) return;
      render();
      menu.style.display = 'block';
      positionMenu();
      btn.setAttribute('aria-expanded', 'true');
      window.addEventListener('scroll', positionMenu, true);
      window.addEventListener('resize', positionMenu);
      setHover(st.active >= 0 ? st.active : 0);
    }
    function close() {
      menu.style.display = 'none';
      btn.setAttribute('aria-expanded', 'false');
      window.removeEventListener('scroll', positionMenu, true);
      window.removeEventListener('resize', positionMenu);
      clearHover();
    }

    st.sync = sync;
    st.fit = fit;
    st.close = close;

    wrap.appendChild(btn);
    wrap.appendChild(menu);
    sel.parentNode.insertBefore(wrap, sel);
    wrap.appendChild(sel);
    // Hide the native select inline so it never shows even if the paired
    // stylesheet is stale (e.g. cached before .p-select rules existed).
    sel.style.display = 'none';

    btn.addEventListener('click', function (ev) {
      ev.stopPropagation();
      if (sel.disabled) return;
      var openNow = menu.style.display !== 'none';
      close();
      if (!openNow) open();
    });
    btn.addEventListener('keydown', function (ev) {
      var openNow = menu.style.display !== 'none';
      var key = ev.key;
      if (key === 'ArrowDown' || key === 'ArrowUp') {
        ev.preventDefault();
        if (!openNow) { open(); return; }
        var dir = key === 'ArrowDown' ? 1 : -1;
        var next = st.hover + dir;
        if (next < 0) next = st.items.length - 1;
        if (next >= st.items.length) next = 0;
        setHover(next);
      } else if (key === 'Enter' || key === ' ') {
        ev.preventDefault();
        if (!openNow) open();
        else if (st.hover >= 0) choose(st.hover);
      } else if (key === 'Escape') {
        if (openNow) { ev.preventDefault(); close(); btn.focus(); }
      }
    });

    var mo = new MutationObserver(function () {
      if (!st.dirty) { st.dirty = true; queueMicrotask(function () { render(); fit(); }); }
    });
    mo.observe(sel, { childList: true, attributes: true, attributeFilter: ['disabled'] });

    if (valueDesc && valueDesc.set) {
      try {
        Object.defineProperty(sel, 'value', {
          configurable: true,
          get: function () { return valueDesc.get.call(sel); },
          set: function (v) { valueDesc.set.call(sel, v); sync(); }
        });
      } catch (e) { /* instance already overridden */ }
    }

    SELECT_STATES.push(st);
    render();
    fit();
  }

  document.addEventListener('click', function (ev) {
    var wrap = ev.target && ev.target.closest ? ev.target.closest('.p-select') : null;
    for (var i = 0; i < SELECT_STATES.length; i++) {
      if (SELECT_STATES[i].menu.style.display === 'none') continue;
      if (wrap === SELECT_STATES[i].wrap) continue;
      SELECT_STATES[i].close();
    }
  });

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
    rateWithTotal: rateWithTotal,
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
    connectStream: connectStream,
    fetchMeta: fetchMeta,
    reloadLanguage: reloadLanguage,
    maskIp: maskIp,
    isLinkLocalV6: isLinkLocalV6,
    enhanceSelects: pSelectInit
  };
})();

// Static <select>s are already parsed (common.js loads at end of body).
window.Pharus.enhanceSelects(document);
