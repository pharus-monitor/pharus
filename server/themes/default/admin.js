/* Pharus default theme — additive admin workspace */
(function () {
  'use strict';

  /* Keep every channel-specific field in this single table. */
  var CHANNEL_FIELD_DESCRIPTORS = {
    telegram: [
      { name: 'bot_token', label: 'channel.field.botToken', secret: true },
      { name: 'chat_id', label: 'channel.field.chatId' }
    ],
    webhook: [
      { name: 'url', label: 'channel.field.url', type: 'url' }
    ],
    email: [
      { name: 'smtp_host', label: 'channel.field.smtpHost' },
      { name: 'smtp_port', label: 'channel.field.smtpPort', type: 'number', min: 1, max: 65535 },
      { name: 'username', label: 'channel.field.username' },
      { name: 'password', label: 'channel.field.password', secret: true },
      { name: 'from', label: 'channel.field.from', type: 'email' },
      { name: 'to', label: 'channel.field.to', type: 'email' }
    ],
    bark: [
      { name: 'url', label: 'channel.field.url', type: 'url' },
      { name: 'device_key', label: 'channel.field.deviceKey', secret: true }
    ],
    feishu: [
      { name: 'url', label: 'channel.field.url', type: 'url' }
    ],
    dingtalk: [
      { name: 'url', label: 'channel.field.url', type: 'url' }
    ],
    wecom: [
      { name: 'url', label: 'channel.field.url', type: 'url' }
    ],
    discord: [
      { name: 'url', label: 'channel.field.url', type: 'url' }
    ]
  };

  var FEATURE_NAMES = ['lg', 'mtr', 'iperf3', 'streaming', 'ping', 'tasks'];

  function node(tag, className, text) {
    var result = document.createElement(tag);
    if (className) result.className = className;
    if (text != null) result.textContent = text;
    return result;
  }

  function append(parent) {
    for (var i = 1; i < arguments.length; i++) {
      if (arguments[i]) parent.appendChild(arguments[i]);
    }
    return parent;
  }

  function actionButton(text, handler, className) {
    var result = node('button', className || 'edit-btn', text);
    result.type = 'button';
    result.addEventListener('click', handler);
    return result;
  }

  function create(options) {
    var t = options.t;
    var active = false;
    var currentView = 'alerts';
    var modalSubmit = null;
    var adminAgents = [];

    function showError(error) {
      options.error.textContent = t('common.error') + ': ' + (error && error.message ? error.message : error);
      options.error.hidden = false;
    }

    function clearError() {
      options.error.hidden = true;
      options.error.textContent = '';
    }

    function setLoading() {
      options.content.innerHTML = '';
      options.content.appendChild(node('p', 'admin-empty', t('common.loading')));
    }

    function formatAgent(id) {
      if (id == null) return t('common.allAgents');
      for (var i = 0; i < adminAgents.length; i++) {
        if (Number(adminAgents[i].agent_id) === Number(id)) return adminAgents[i].name || ('Agent #' + id);
      }
      return 'Agent #' + id;
    }

    function liveEntry(id) {
      var pairs = options.getAgents();
      for (var i = 0; i < pairs.length; i++) {
        if (Number(pairs[i][0]) === Number(id)) return pairs[i][1];
      }
      return null;
    }

    function loadAgents() {
      return options.request('/api/admin/agents').then(function (agents) {
        adminAgents = Array.isArray(agents) ? agents : [];
        return adminAgents;
      });
    }

    function toolbar(title, addLabel, handler) {
      var bar = node('div', 'admin-toolbar');
      bar.appendChild(node('h3', '', title));
      if (handler) {
        var actions = node('div', 'admin-toolbar-actions');
        actions.appendChild(actionButton(addLabel, handler, 'btn primary'));
        bar.appendChild(actions);
      }
      return bar;
    }

    function makeTable(headers, rows) {
      if (!rows.length) return node('p', 'admin-empty', t('common.noData'));
      var wrap = node('div', 'admin-table-wrap');
      var table = node('table', 'admin-table');
      var thead = node('thead');
      var headerRow = node('tr');
      headers.forEach(function (header) { headerRow.appendChild(node('th', '', header)); });
      thead.appendChild(headerRow);
      table.appendChild(thead);
      var tbody = node('tbody');
      rows.forEach(function (cells) {
        var row = node('tr');
        cells.forEach(function (value) {
          var cell = node('td');
          if (value instanceof Node) cell.appendChild(value); else cell.textContent = value == null ? '—' : String(value);
          row.appendChild(cell);
        });
        tbody.appendChild(row);
      });
      table.appendChild(tbody);
      wrap.appendChild(table);
      return wrap;
    }

    function actionsCell() {
      return node('div', 'table-actions');
    }

    function enabledLabel(enabled) {
      return enabled ? t('common.enabled') : t('common.disabled');
    }

    function fieldLabel(key) {
      var label = node('label', 'field');
      label.appendChild(node('span', '', t(key)));
      return label;
    }

    function inputField(key, value, settings) {
      settings = settings || {};
      var label = fieldLabel(key);
      var input = node(settings.textarea ? 'textarea' : 'input');
      // textarea.type is read-only; assigning it throws under strict mode.
      if (!settings.textarea) input.type = settings.type || 'text';
      input.value = value == null ? '' : value;
      if (settings.required) input.required = true;
      if (settings.min != null) input.min = settings.min;
      if (settings.max != null) input.max = settings.max;
      if (settings.step != null) input.step = settings.step;
      if (settings.placeholder) input.placeholder = settings.placeholder;
      if (settings.autocomplete) input.autocomplete = settings.autocomplete;
      label.appendChild(input);
      return { el: label, input: input, label: label.firstElementChild };
    }

    function selectField(key, choices, value) {
      var label = fieldLabel(key);
      var select = node('select');
      choices.forEach(function (choice) {
        var option = node('option', '', choice.label);
        option.value = choice.value;
        if (choice.disabled) option.disabled = true;
        select.appendChild(option);
      });
      select.value = value == null ? '' : String(value);
      label.appendChild(select);
      return { el: label, input: select, label: label.firstElementChild };
    }

    function checkboxField(key, checked) {
      var label = node('label', 'field-check');
      var input = node('input');
      input.type = 'checkbox';
      input.checked = !!checked;
      label.appendChild(input);
      label.appendChild(document.createTextNode(t(key)));
      return { el: label, input: input };
    }

    function agentChoices(includeAll, requiredFeature) {
      var choices = [];
      if (includeAll) choices.push({ value: '', label: t('common.allAgents') });
      adminAgents.forEach(function (agent) {
        var entry = liveEntry(agent.agent_id) || agent;
        var unavailable = requiredFeature && !options.hasFeature(entry, requiredFeature);
        choices.push({
          value: agent.agent_id,
          label: (agent.name || ('Agent #' + agent.agent_id)) + (unavailable ? ' · ' + t('feature.notAvailable') : ''),
          disabled: unavailable
        });
      });
      return choices;
    }

    function nullableNumber(input) {
      return input.value === '' ? null : Number(input.value);
    }

    /// Checkbox list for choosing which hosts a task applies to. Empty result
    /// means "all hosts" (the backend's default). Checking "All hosts" selects
    /// every host visually.
    function agentMultiField(selectedIds) {
      var wrap = node('div', 'agent-multi');
      var allChecked = !selectedIds || !selectedIds.length;
      function checkbox(labelText, checked, extraClass) {
        var label = node('label', 'field-check' + (extraClass ? ' ' + extraClass : ''));
        var input = node('input');
        input.type = 'checkbox';
        input.checked = !!checked;
        label.appendChild(input);
        label.appendChild(document.createTextNode(labelText));
        return { el: label, input: input };
      }
      var all = checkbox(t('common.allAgents'), allChecked, 'agent-multi-all');
      wrap.appendChild(all.el);
      var agentCbs = {};
      adminAgents.forEach(function (agent) {
        var cb = checkbox(agent.name || ('Agent #' + agent.agent_id),
          selectedIds && selectedIds.indexOf(Number(agent.agent_id)) !== -1, 'agent-multi-item');
        agentCbs[agent.agent_id] = cb;
        wrap.appendChild(cb.el);
      });
      function syncAll() {
        var ids = Object.keys(agentCbs);
        all.input.checked = ids.length > 0 && ids.every(function (id) { return agentCbs[id].input.checked; });
      }
      all.input.addEventListener('change', function () {
        var on = all.input.checked;
        Object.keys(agentCbs).forEach(function (id) { agentCbs[id].input.checked = on; });
      });
      Object.keys(agentCbs).forEach(function (id) {
        agentCbs[id].input.addEventListener('change', syncAll);
      });
      function collect() {
        if (all.input.checked) return [];
        var ids = [];
        Object.keys(agentCbs).forEach(function (id) {
          if (agentCbs[id].input.checked) ids.push(Number(id));
        });
        return ids;
      }
      return { el: wrap, collect: collect };
    }

    function formatScopes(ids) {
      if (!ids || !ids.length) return t('common.allAgents');
      return ids.map(function (id) { return formatAgent(id); }).join(', ');
    }

    function openModal(title, build, submit) {
      options.title.textContent = title;
      options.fields.innerHTML = '';
      options.formError.hidden = true;
      options.formError.textContent = '';
      modalSubmit = submit;
      build(options.fields);
      options.modal.hidden = false;
      var first = options.fields.querySelector('input, select, textarea');
      if (first) first.focus();
    }

    function closeModal() {
      options.modal.hidden = true;
      modalSubmit = null;
    }

    function saveEntity(path, id, body) {
      return options.request(path + (id == null ? '' : '/' + id), {
        method: id == null ? 'POST' : 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body)
      });
    }

    function deleteEntity(path, id, reload) {
      if (!window.confirm(t('admin.deleteConfirm'))) return;
      clearError();
      options.request(path + '/' + id, { method: 'DELETE' }).then(reload).catch(showError);
    }

    function editActions(edit, remove) {
      var actions = actionsCell();
      actions.appendChild(actionButton(t('admin.edit'), edit));
      actions.appendChild(actionButton(t('admin.delete'), remove, 'edit-btn danger'));
      return actions;
    }

    /* ---------- Alert rules ---------- */
    function openAlertForm(rule, channels, tasks) {
      rule = rule || {};
      tasks = tasks || [];
      var refs = {};
      openModal(rule.id == null ? t('alert.create') : t('alert.edit'), function (root) {
        var explain = node('p', 'rule-explain', t('alert.semantics'));
        root.appendChild(explain);
        refs.name = inputField('common.name', rule.name, { required: true });
        refs.kind = selectField('alert.kind', ['metric', 'offline', 'task'].map(function (kind) {
          return { value: kind, label: t('alert.kind.' + kind) };
        }), rule.kind || 'metric');
        refs.agent = selectField('common.agent', agentChoices(true), rule.agent_id);
        refs.metric = selectField('alert.metric', [
          { value: '', label: '—' }, { value: 'cpu', label: t('metric.cpu') },
          { value: 'mem', label: t('metric.mem') }, { value: 'disk', label: t('metric.disk') },
          { value: 'load', label: t('metric.load') }, { value: 'traffic', label: t('metric.traffic') }
        ], rule.metric || 'cpu');
        refs.op = selectField('alert.op', [{ value: '>', label: '>' }, { value: '<', label: '<' }], rule.op || '>');
        refs.threshold = inputField('alert.threshold', rule.threshold == null ? 80 : rule.threshold, { type: 'number', required: true, step: 'any' });
        refs.duration = inputField('alert.duration', rule.duration == null ? 300 : rule.duration, { type: 'number', required: true, min: 1 });
        refs.ratio = inputField('alert.ratio', rule.ratio == null ? 1 : rule.ratio, { type: 'number', required: true, min: 0, max: 1, step: 0.01 });
        refs.cooldown = inputField('alert.cooldown', rule.cooldown == null ? 1800 : rule.cooldown, { type: 'number', min: 0 });
        refs.taskId = selectField('alert.task', [{ value: '', label: '—' }].concat(tasks.map(function (task) {
          return { value: String(task.id), label: task.name };
        })), rule.task_id == null ? '' : String(rule.task_id));
        refs.enabled = checkboxField('common.enabled', rule.enabled !== false);
        [refs.name, refs.kind, refs.agent, refs.metric, refs.op, refs.threshold, refs.duration, refs.ratio, refs.cooldown, refs.taskId].forEach(function (ref) { root.appendChild(ref.el); });
        var channelBox = fieldLabel('alert.channels');
        var selectedChannels = Array.isArray(rule.channels) ? rule.channels.map(Number) : [];
        refs.channels = [];
        channels.forEach(function (channel) {
          var check = checkboxField('', selectedChannels.indexOf(Number(channel.id)) !== -1);
          check.el.lastChild.textContent = channel.name;
          check.input.value = channel.id;
          channelBox.appendChild(check.el);
          refs.channels.push(check.input);
        });
        root.appendChild(channelBox);
        root.appendChild(refs.enabled.el);
        function updateKind() {
          var kind = refs.kind.input.value;
          refs.metric.el.hidden = kind !== 'metric';
          refs.op.el.hidden = kind === 'offline';
          refs.taskId.el.hidden = kind !== 'task';
          refs.threshold.label.textContent = t(kind === 'offline' ? 'alert.gracePeriod' : 'alert.threshold');
        }
        refs.kind.input.addEventListener('change', updateKind);
        updateKind();
      }, function () {
        var kind = refs.kind.input.value;
        return saveEntity('/api/admin/alert-rules', rule.id, {
          name: refs.name.input.value.trim(),
          kind: kind,
          agent_id: refs.agent.input.value === '' ? null : Number(refs.agent.input.value),
          metric: kind === 'metric' ? (refs.metric.input.value || null) : null,
          op: refs.op.input.value,
          threshold: Number(refs.threshold.input.value),
          duration: Number(refs.duration.input.value),
          ratio: Number(refs.ratio.input.value),
          cooldown: Number(refs.cooldown.input.value),
          task_id: kind === 'task' && refs.taskId.input.value ? Number(refs.taskId.input.value) : null,
          channels: refs.channels.filter(function (input) { return input.checked; }).map(function (input) { return Number(input.value); }),
          enabled: refs.enabled.input.checked
        }).then(function () { closeModal(); renderAlerts(); });
      });
    }

    function renderAlerts() {
      setLoading(); clearError();
      Promise.all([
        options.request('/api/admin/alert-rules'),
        options.request('/api/admin/channels'),
        loadAgents(),
        options.request('/api/admin/tasks')
      ]).then(function (values) {
        if (!active || currentView !== 'alerts') return;
        var rules = Array.isArray(values[0]) ? values[0] : [];
        var channels = Array.isArray(values[1]) ? values[1] : [];
        var tasks = Array.isArray(values[3]) ? values[3] : [];
        var channelNames = {};
        channels.forEach(function (channel) { channelNames[channel.id] = channel.name; });
        options.content.innerHTML = '';
        options.content.appendChild(toolbar(t('admin.alerts'), t('alert.create'), function () { openAlertForm(null, channels, tasks); }));
        options.content.appendChild(node('p', 'rule-explain', t('alert.semantics')));
        var rows = rules.map(function (rule) {
          var condition = rule.kind === 'offline'
            ? t('alert.graceSummary').replace('{n}', rule.threshold)
            : ((rule.metric ? t('metric.' + rule.metric) : t('alert.kind.' + rule.kind)) + ' ' + rule.op + ' ' + rule.threshold);
          var windowText = t('alert.windowSummary').replace('{duration}', rule.duration).replace('{ratio}', Math.round(rule.ratio * 100));
          return [
            rule.name, t('alert.kind.' + rule.kind), formatAgent(rule.agent_id), condition, windowText,
            (rule.channels || []).map(function (id) { return channelNames[id] || ('#' + id); }).join(', ') || '—',
            enabledLabel(rule.enabled),
            editActions(function () { openAlertForm(rule, channels, tasks); }, function () { deleteEntity('/api/admin/alert-rules', rule.id, renderAlerts); })
          ];
        });
        options.content.appendChild(makeTable([
          t('common.name'), t('alert.kind'), t('common.scope'), t('alert.condition'),
          t('alert.window'), t('alert.channels'), t('common.status'), t('common.actions')
        ], rows));
      }).catch(showError);
    }

    /* ---------- Notification channels ---------- */
    function openChannelForm(channel) {
      channel = channel || {};
      var refs = {};
      var currentConfig = channel.config || {};
      openModal(channel.id == null ? t('channel.create') : t('channel.edit'), function (root) {
        refs.name = inputField('common.name', channel.name, { required: true });
        refs.kind = selectField('channel.kind', Object.keys(CHANNEL_FIELD_DESCRIPTORS).map(function (kind) {
          return { value: kind, label: t('channel.kind.' + kind) };
        }), channel.kind || 'telegram');
        refs.enabled = checkboxField('common.enabled', channel.enabled !== false);
        refs.configRoot = node('div');
        root.appendChild(refs.name.el);
        root.appendChild(refs.kind.el);
        root.appendChild(refs.configRoot);
        root.appendChild(refs.enabled.el);
        function renderConfigFields() {
          refs.configRoot.innerHTML = '';
          refs.configInputs = [];
          CHANNEL_FIELD_DESCRIPTORS[refs.kind.input.value].forEach(function (descriptor) {
            var ref = inputField(descriptor.label, currentConfig[descriptor.name], {
              type: descriptor.secret ? 'password' : (descriptor.type || 'text'),
              min: descriptor.min,
              max: descriptor.max,
              autocomplete: descriptor.secret ? 'new-password' : null
            });
            ref.input.dataset.configName = descriptor.name;
            refs.configRoot.appendChild(ref.el);
            refs.configInputs.push({ descriptor: descriptor, input: ref.input });
          });
          if (channel.id != null) refs.configRoot.appendChild(node('p', 'field-hint', t('channel.secretHint')));
        }
        refs.kind.input.addEventListener('change', function () { currentConfig = {}; renderConfigFields(); });
        renderConfigFields();
      }, function () {
        var config = {};
        refs.configInputs.forEach(function (item) {
          var value = item.input.value;
          if (item.descriptor.secret && value === '***') return;
          if (channel.id == null && value === '') return;
          config[item.descriptor.name] = item.descriptor.type === 'number' && value !== '' ? Number(value) : value;
        });
        return saveEntity('/api/admin/channels', channel.id, {
          name: refs.name.input.value.trim(),
          kind: refs.kind.input.value,
          config: config,
          enabled: refs.enabled.input.checked
        }).then(function () { closeModal(); renderChannels(); });
      });
    }

    function testChannel(channel, actions) {
      var status = actions.querySelector('.inline-status');
      status.textContent = t('channel.testing');
      status.className = 'inline-status';
      options.request('/api/admin/channels/' + channel.id + '/test', { method: 'POST' }).then(function () {
        status.textContent = t('channel.testSuccess');
        status.className = 'inline-status ok';
      }).catch(function (error) {
        status.textContent = t('channel.testFailed') + ': ' + error.message;
        status.className = 'inline-status error';
      });
    }

    function renderChannels() {
      setLoading(); clearError();
      options.request('/api/admin/channels').then(function (channels) {
        if (!active || currentView !== 'channels') return;
        channels = Array.isArray(channels) ? channels : [];
        options.content.innerHTML = '';
        options.content.appendChild(toolbar(t('admin.channels'), t('channel.create'), function () { openChannelForm(null); }));
        var rows = channels.map(function (channel) {
          var actions = actionsCell();
          actions.appendChild(actionButton(t('channel.test'), function () { testChannel(channel, actions); }));
          actions.appendChild(actionButton(t('admin.edit'), function () { openChannelForm(channel); }));
          actions.appendChild(actionButton(t('admin.delete'), function () { deleteEntity('/api/admin/channels', channel.id, renderChannels); }, 'edit-btn danger'));
          actions.appendChild(node('span', 'inline-status'));
          return [channel.name, t('channel.kind.' + channel.kind), enabledLabel(channel.enabled), actions];
        });
        options.content.appendChild(makeTable([t('common.name'), t('channel.kind'), t('common.status'), t('common.actions')], rows));
      }).catch(showError);
    }

    /* ---------- Ping tasks ---------- */
    function openPingTaskForm(task) {
      task = task || {};
      var refs = {};
      openModal(task.id == null ? t('pingTask.create') : t('pingTask.edit'), function (root) {
        refs.label = inputField('pingTask.label', task.label, { required: true });
        refs.agent = agentMultiField(task.agent_ids);
        refs.kind = selectField('pingTask.kind', ['icmp', 'tcp', 'http'].map(function (kind) {
          return { value: kind, label: kind.toUpperCase() };
        }), task.kind || 'icmp');
        refs.target = inputField('pingTask.target', task.target, { required: true });
        refs.port = inputField('pingTask.port', task.port, { type: 'number', min: 1, max: 65535 });
        refs.interval = inputField('pingTask.interval', task.interval_sec == null ? 60 : task.interval_sec, { type: 'number', required: true, min: 1 });
        refs.count = inputField('pingTask.probeCount', task.probe_count == null ? 3 : task.probe_count, { type: 'number', required: true, min: 1 });
        refs.enabled = checkboxField('common.enabled', task.enabled !== false);
        [refs.label, refs.agent, refs.kind, refs.target, refs.port, refs.interval, refs.count].forEach(function (ref) { root.appendChild(ref.el); });
        root.appendChild(refs.enabled.el);
        function updateKind() { refs.port.el.hidden = refs.kind.input.value === 'icmp'; }
        refs.kind.input.addEventListener('change', updateKind);
        updateKind();
      }, function () {
        return saveEntity('/api/admin/ping-tasks', task.id, {
          agent_ids: refs.agent.collect(),
          label: refs.label.input.value.trim(),
          kind: refs.kind.input.value,
          target: refs.target.input.value.trim(),
          port: refs.kind.input.value === 'icmp' ? null : nullableNumber(refs.port.input),
          interval_sec: Number(refs.interval.input.value),
          probe_count: Number(refs.count.input.value),
          enabled: refs.enabled.input.checked
        }).then(function () { closeModal(); renderPingTasks(); });
      });
    }

    function renderPingTasks() {
      setLoading(); clearError();
      Promise.all([options.request('/api/admin/ping-tasks'), loadAgents()]).then(function (values) {
        if (!active || currentView !== 'pingTasks') return;
        var tasks = Array.isArray(values[0]) ? values[0] : [];
        options.content.innerHTML = '';
        options.content.appendChild(toolbar(t('admin.pingTasks'), t('pingTask.create'), function () { openPingTaskForm(null); }));
        var rows = tasks.map(function (task) {
          var target = task.target + (task.port == null ? '' : ':' + task.port);
          return [task.label, formatScopes(task.agent_ids), task.kind.toUpperCase(), target,
            task.interval_sec + 's', String(task.probe_count), enabledLabel(task.enabled),
            editActions(function () { openPingTaskForm(task); }, function () { deleteEntity('/api/admin/ping-tasks', task.id, renderPingTasks); })];
        });
        options.content.appendChild(makeTable([
          t('pingTask.label'), t('common.scope'), t('pingTask.kind'), t('pingTask.target'),
          t('pingTask.interval'), t('pingTask.probeCount'), t('common.status'), t('common.actions')
        ], rows));
      }).catch(showError);
    }

    /* ---------- Custom tasks and history ---------- */
    function openTaskForm(task) {
      task = task || {};
      var refs = {};
      openModal(task.id == null ? t('task.create') : t('task.edit'), function (root) {
        refs.name = inputField('common.name', task.name, { required: true });
        refs.command = inputField('task.command', task.command, { required: true, textarea: true });
        refs.agent = agentMultiField(task.agent_ids);
        refs.interval = inputField('task.interval', task.interval_sec == null ? 0 : task.interval_sec, { type: 'number', required: true, min: 0 });
        refs.timeout = inputField('task.timeout', task.timeout_sec == null ? 30 : task.timeout_sec, { type: 'number', required: true, min: 1 });
        refs.enabled = checkboxField('common.enabled', task.enabled !== false);
        [refs.name, refs.command, refs.agent, refs.interval, refs.timeout].forEach(function (ref) { root.appendChild(ref.el); });
        root.appendChild(node('p', 'field-hint', t('task.manualHint')));
        root.appendChild(refs.enabled.el);
      }, function () {
        return saveEntity('/api/admin/tasks', task.id, {
          name: refs.name.input.value.trim(),
          command: refs.command.input.value,
          agent_ids: refs.agent.collect(),
          interval_sec: Number(refs.interval.input.value),
          timeout_sec: Number(refs.timeout.input.value),
          enabled: refs.enabled.input.checked
        }).then(function () { closeModal(); renderTasks(); });
      });
    }

    function historyTable(results) {
      var rows = results.map(function (result) {
        var output = node('pre', 'run-output', result.output || '');
        return [new Date(Number(result.ts) * 1000).toLocaleString(), '#' + result.task_id,
          formatAgent(result.agent_id), String(result.exit_code), output];
      });
      return makeTable([t('common.time'), t('task.task'), t('common.agent'), t('diag.exitCodeLabel'), t('task.output')], rows);
    }

    function renderTasks() {
      setLoading(); clearError();
      Promise.all([
        options.request('/api/admin/tasks'),
        options.request('/api/admin/task-results?limit=20'),
        loadAgents()
      ]).then(function (values) {
        if (!active || currentView !== 'tasks') return;
        var tasks = Array.isArray(values[0]) ? values[0] : [];
        var results = Array.isArray(values[1]) ? values[1] : [];
        options.content.innerHTML = '';
        options.content.appendChild(toolbar(t('admin.tasks'), t('task.create'), function () { openTaskForm(null); }));
        var rows = tasks.map(function (task) {
          return [task.name, formatScopes(task.agent_ids), task.interval_sec === 0 ? t('task.manualOnly') : task.interval_sec + 's',
            task.timeout_sec + 's', enabledLabel(task.enabled),
            editActions(function () { openTaskForm(task); }, function () { deleteEntity('/api/admin/tasks', task.id, renderTasks); })];
        });
        options.content.appendChild(makeTable([
          t('common.name'), t('common.scope'), t('task.interval'), t('task.timeout'), t('common.status'), t('common.actions')
        ], rows));

        options.content.appendChild(toolbar(t('task.runNow')));
        var runControls = node('div', 'history-filter');
        var taskSelect = selectField('task.task', tasks.map(function (task) { return { value: task.id, label: task.name }; }), tasks.length ? tasks[0].id : '');
        var agentSelect = selectField('common.agent', agentChoices(false, 'tasks'), '');
        var runButton = actionButton(t('task.run'), function () {
          runStatus.textContent = '';
          runOutput.textContent = '';
          if (!taskSelect.input.value || !agentSelect.input.value) {
            runStatus.textContent = t('task.selectRunTarget');
            runStatus.className = 'inline-status error';
            return;
          }
          runButton.disabled = true;
          runStatus.textContent = t('task.running');
          runStatus.className = 'inline-status';
          options.request('/api/admin/tasks/' + taskSelect.input.value + '/run', {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ agent_id: Number(agentSelect.input.value) })
          }).then(function (response) {
            runStatus.textContent = t('task.finished').replace('{code}', response.exit_code);
            runStatus.className = response.exit_code === 0 ? 'inline-status ok' : 'inline-status error';
            runOutput.textContent = response.output || '';
          }).catch(function (error) {
            runStatus.textContent = t('common.error') + ': ' + error.message;
            runStatus.className = 'inline-status error';
          }).then(function () { runButton.disabled = false; });
        }, 'btn primary');
        var runStatus = node('p', 'inline-status');
        var runOutput = node('pre', 'run-output');
        runControls.appendChild(taskSelect.el);
        runControls.appendChild(agentSelect.el);
        runControls.appendChild(runButton);
        options.content.appendChild(runControls);
        options.content.appendChild(runStatus);
        options.content.appendChild(runOutput);

        options.content.appendChild(toolbar(t('task.history')));
        var historyControls = node('div', 'history-filter');
        var historyChoices = [{ value: '', label: t('task.allTasks') }].concat(tasks.map(function (task) { return { value: task.id, label: task.name }; }));
        var historySelect = selectField('task.task', historyChoices, '');
        var historyRoot = node('div');
        var loadHistory = actionButton(t('common.refresh'), function () {
          var url = '/api/admin/task-results?limit=20';
          if (historySelect.input.value) url += '&task_id=' + encodeURIComponent(historySelect.input.value);
          loadHistory.disabled = true;
          options.request(url).then(function (items) {
            historyRoot.innerHTML = '';
            historyRoot.appendChild(historyTable(Array.isArray(items) ? items : []));
          }).catch(showError).then(function () { loadHistory.disabled = false; });
        });
        historyControls.appendChild(historySelect.el);
        historyControls.appendChild(loadHistory);
        historyRoot.appendChild(historyTable(results));
        options.content.appendChild(historyControls);
        options.content.appendChild(historyRoot);
      }).catch(showError);
    }

    /* ---------- Region overrides ---------- */
    function renderRegions() {
      setLoading(); clearError();
      Promise.all([loadAgents(), options.request('/api/regions')]).then(function (values) {
        if (!active || currentView !== 'regions') return;
        var regions = Array.isArray(values[1]) ? values[1] : [];
        options.content.innerHTML = '';
        options.content.appendChild(toolbar(t('admin.regions')));
        var rows = adminAgents.map(function (agent) {
          var region = agent.region || null;
          var select = node('select', 'feature-select');
          select.appendChild(new Option(t('region.autoDetect'), ''));
          regions.forEach(function (item) { select.appendChild(new Option(item.code + ' · ' + item.name, item.code)); });
          select.value = region && region.source === 'manual' ? region.code : '';
          var actions = actionsCell();
          var status = node('span', 'inline-status');
          var save = actionButton(t('admin.save'), function () {
            save.disabled = true;
            status.textContent = t('common.saving');
            options.request('/api/admin/agents/' + agent.agent_id + '/region', {
              method: 'PUT', headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ code: select.value || null })
            }).then(function () {
              status.textContent = t('common.saved');
              status.className = 'inline-status ok';
            }).catch(function (error) {
              status.textContent = t('common.error') + ': ' + error.message;
              status.className = 'inline-status error';
            }).then(function () { save.disabled = false; });
          });
          actions.appendChild(save);
          actions.appendChild(status);
          return [agent.name || ('Agent #' + agent.agent_id), region ? region.code + ' · ' + region.name : t('region.ungrouped'),
            region ? t('region.source.' + region.source) : '—', select, actions];
        });
        options.content.appendChild(makeTable([
          t('common.agent'), t('region.current'), t('region.source'), t('region.override'), t('common.actions')
        ], rows));
      }).catch(showError);
    }

    /* ---------- Feature defaults and overrides ---------- */
    function overrideValue(overrides, feature) {
      if (!Object.prototype.hasOwnProperty.call(overrides || {}, feature)) return '';
      if (overrides[feature] == null) return '';
      return overrides[feature] ? 'true' : 'false';
    }

    function renderFeatures() {
      setLoading(); clearError();
      Promise.all([options.request('/api/admin/features'), loadAgents()]).then(function (values) {
        var defaults = values[0] || {};
        return Promise.all(adminAgents.map(function (agent) {
          return options.request('/api/admin/agents/' + agent.agent_id + '/features').then(function (detail) {
            return { agent: agent, detail: detail || {} };
          });
        })).then(function (details) { return { defaults: defaults, details: details }; });
      }).then(function (data) {
        if (!active || currentView !== 'features') return;
        options.content.innerHTML = '';
        options.content.appendChild(toolbar(t('feature.globalDefaults')));
        var defaultsRoot = node('div', 'feature-defaults');
        var defaultInputs = {};
        FEATURE_NAMES.forEach(function (feature) {
          var check = checkboxField('feature.' + feature, !!data.defaults[feature]);
          check.el.className = 'toggle-field';
          defaultsRoot.appendChild(check.el);
          defaultInputs[feature] = check.input;
        });
        var saveDefaults = actionButton(t('admin.save'), function () {
          var body = {};
          FEATURE_NAMES.forEach(function (feature) { body[feature] = defaultInputs[feature].checked; });
          saveDefaults.disabled = true;
          defaultStatus.textContent = t('common.saving');
          options.request('/api/admin/features', {
            method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body)
          }).then(function () {
            defaultStatus.textContent = t('common.saved'); defaultStatus.className = 'inline-status ok';
          }).catch(function (error) {
            defaultStatus.textContent = t('common.error') + ': ' + error.message; defaultStatus.className = 'inline-status error';
          }).then(function () { saveDefaults.disabled = false; });
        }, 'btn primary');
        var defaultStatus = node('span', 'inline-status');
        defaultsRoot.appendChild(saveDefaults);
        defaultsRoot.appendChild(defaultStatus);
        options.content.appendChild(defaultsRoot);
        options.content.appendChild(toolbar(t('feature.agentOverrides')));

        var rows = data.details.map(function (item) {
          var overrides = item.detail.overrides || {};
          var effective = Array.isArray(item.detail.effective) ? item.detail.effective : [];
          var selects = {};
          var cells = [item.agent.name || ('Agent #' + item.agent.agent_id)];
          FEATURE_NAMES.forEach(function (feature) {
            var select = node('select', 'feature-select');
            select.appendChild(new Option(t('feature.inherit'), ''));
            select.appendChild(new Option(t('feature.forceOn'), 'true'));
            select.appendChild(new Option(t('feature.forceOff'), 'false'));
            select.value = overrideValue(overrides, feature);
            selects[feature] = select;
            var wrap = node('div');
            wrap.appendChild(select);
            wrap.appendChild(node('p', 'inline-status', effective.indexOf(feature) !== -1 ? t('common.effectiveOn') : t('common.effectiveOff')));
            cells.push(wrap);
          });
          var actions = actionsCell();
          var status = node('span', 'inline-status');
          var save = actionButton(t('admin.save'), function () {
            var body = { overrides: {} };
            FEATURE_NAMES.forEach(function (feature) {
              body.overrides[feature] = selects[feature].value === '' ? null : selects[feature].value === 'true';
            });
            save.disabled = true;
            status.textContent = t('common.saving');
            options.request('/api/admin/agents/' + item.agent.agent_id + '/features', {
              method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body)
            }).then(function () {
              status.textContent = t('common.saved'); status.className = 'inline-status ok';
            }).catch(function (error) {
              status.textContent = t('common.error') + ': ' + error.message; status.className = 'inline-status error';
            }).then(function () { save.disabled = false; });
          });
          actions.appendChild(save);
          actions.appendChild(status);
          cells.push(actions);
          return cells;
        });
        options.content.appendChild(makeTable([t('common.agent')].concat(FEATURE_NAMES.map(function (feature) {
          return t('feature.' + feature);
        })).concat([t('common.actions')]), rows));
      }).catch(showError);
    }

    /* ---------- Host billing (per-agent cycle traffic / expiry / price) ---------- */
    var BILLING_CURRENCY_SYMBOL = { CNY: '¥', USD: '$', EUR: '€' };

    function billingPriceLabel(b) {
      if (b.price == null || !b.currency) return '—';
      var sym = BILLING_CURRENCY_SYMBOL[b.currency] || (b.currency + ' ');
      return sym + b.price + (b.cycle ? ' · ' + t('billing.cycle.' + b.cycle) : '');
    }

    function openBillingForm(agent) {
      var b = agent.billing || {};
      var curMode = b.traffic_mode || 'bi';
      var curDir = b.traffic_dir || 'down';
      openModal((agent.name || ('Agent #' + agent.agent_id)) + ' · ' + t('billing.manage'), function (root) {
        var f = {};
        f.resetDay = inputField('admin.resetDay', b.reset_day != null ? b.reset_day : '', { type: 'number', min: 1, max: 31 });
        f.quotaGb = inputField('admin.quotaGb', b.quota_bytes != null ? Math.round(b.quota_bytes / 1073741824 * 100) / 100 : '', { type: 'number', min: 0 });
        f.expiresOn = inputField('admin.expiresOn', b.expires_at ? window.Pharus.fmtDate(b.expires_at) : '', { type: 'date' });
        f.price = inputField('admin.price', b.price != null ? b.price : '', { type: 'number', min: 0, step: '0.01' });
        f.currency = selectField('admin.currency', [
          { value: '', label: '—' }, { value: 'CNY', label: 'CNY' },
          { value: 'USD', label: 'USD' }, { value: 'EUR', label: 'EUR' }
        ], b.currency);
        f.cycle = selectField('admin.cycle', [
          { value: '', label: '—' },
          { value: 'monthly', label: t('billing.cycle.monthly') },
          { value: 'quarterly', label: t('billing.cycle.quarterly') },
          { value: 'yearly', label: t('billing.cycle.yearly') }
        ], b.cycle);
        f.bandwidth = inputField('admin.bandwidth', b.bandwidth, { type: 'number', min: 0, step: '0.1' });
        f.mode = selectField('settings.trafficMode', [
          { value: 'bi', label: t('settings.modeBi') },
          { value: 'uni', label: t('settings.modeUni') }
        ], curMode);
        f.dir = selectField('settings.trafficDir', [
          { value: 'down', label: t('settings.dirDown') },
          { value: 'up', label: t('settings.dirUp') },
          { value: 'max', label: t('settings.dirMax') }
        ], curDir);
        ['resetDay', 'quotaGb', 'expiresOn', 'price', 'currency', 'cycle', 'bandwidth'].forEach(function (k) { root.appendChild(f[k].el); });
        root.appendChild(f.mode.el);
        root.appendChild(f.dir.el);
        f.dir.el.hidden = f.mode.input.value === 'bi';
        f.mode.input.addEventListener('change', function () {
          f.dir.el.hidden = f.mode.input.value === 'bi';
        });
        modalSubmit = function () {
          var body = {
            reset_day: f.resetDay.input.value === '' ? null : parseInt(f.resetDay.input.value, 10),
            quota_gb: f.quotaGb.input.value === '' ? null : parseFloat(f.quotaGb.input.value),
            expires_on: f.expiresOn.input.value === '' ? null : f.expiresOn.input.value,
            price: f.price.input.value === '' ? null : parseFloat(f.price.input.value),
            currency: f.currency.input.value || null,
            cycle: f.cycle.input.value || null,
            bandwidth: f.bandwidth.input.value === '' ? null : parseFloat(f.bandwidth.input.value),
            traffic_mode: f.mode.input.value,
            traffic_dir: f.dir.input.value
          };
          return options.request('/api/admin/agents/' + agent.agent_id + '/billing', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body)
          }).then(function () { closeModal(); renderHostBilling(); });
        };
      });
    }

    function renderHostBilling() {
      setLoading(); clearError();
      loadAgents().then(function () {
        if (!active || currentView !== 'hostBilling') return;
        return options.request('/api/meta');
      }).then(function (meta) {
        if (!active || currentView !== 'hostBilling') return;
        options.content.innerHTML = '';
        options.content.appendChild(toolbar(t('billing.manage'), null));
        var daysBox = node('div', 'feature-defaults');
        var days = inputField('settings.expiryDays', (meta && meta.expiry_alert_days) || 3, { type: 'number', min: 1, max: 365 });
        daysBox.appendChild(days.el);
        var daysStatus = node('span', 'inline-status');
        var daysSave = actionButton(t('admin.save'), function () {
          var v = parseInt(days.input.value, 10);
          if (!Number.isFinite(v) || v < 1 || v > 365) {
            daysStatus.textContent = t('settings.expiryRange');
            daysStatus.className = 'inline-status error';
            return;
          }
          daysSave.disabled = true;
          daysStatus.textContent = t('common.saving');
          options.request('/api/admin/settings', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ key: 'expiry_alert_days', value: String(v) })
          }).then(function () {
            daysStatus.textContent = t('common.saved'); daysStatus.className = 'inline-status ok';
          }).catch(function (error) {
            daysStatus.textContent = t('common.error') + ': ' + error.message; daysStatus.className = 'inline-status error';
          }).then(function () { daysSave.disabled = false; });
        }, 'btn primary');
        daysBox.appendChild(daysSave);
        daysBox.appendChild(daysStatus);
        options.content.appendChild(daysBox);
        var rows = adminAgents.map(function (agent) {
          var b = agent.billing || {};
          var expires = b.expires_at ? window.Pharus.fmtDate(b.expires_at) : '—';
          var actions = editActions(function () { openBillingForm(agent); }, null);
          return [
            agent.name || ('Agent #' + agent.agent_id),
            b.reset_day != null ? String(b.reset_day) : '—',
            b.quota_bytes != null ? window.Pharus.fmtBytes(b.quota_bytes) : '—',
            expires,
            billingPriceLabel(b),
            actions
          ];
        });
        options.content.appendChild(makeTable(
          [t('common.agent'), t('admin.resetDay'), t('billing.quota'), t('billing.expires'), t('billing.price'), t('common.actions')],
          rows));
      }).catch(showError);
    }

    /* ---------- Site settings (site identity, agent secrets, language) ---------- */
    function renderSettings() {
      setLoading(); clearError();
      options.request('/api/meta').then(function (meta) {
        if (!active || currentView !== 'settings') return;
        options.content.innerHTML = '';
        options.content.appendChild(toolbar(t('admin.settings'), null));

        var box = node('div', 'settings-stack');
        var name = inputField('settings.siteName', meta.site_name || '', { type: 'text' });
        var lang = selectField('settings.defaultLanguage', [
          { value: 'en', label: 'English' },
          { value: 'zh-CN', label: '中文' },
          { value: 'ja', label: '日本語' },
          { value: 'ru', label: 'Русский' }
        ], meta.default_language || 'en');
        box.appendChild(name.el);
        box.appendChild(lang.el);
        var status = node('span', 'inline-status');
        var save = actionButton(t('admin.save'), function () {
          function put(key, value) {
            return options.request('/api/admin/settings', {
              method: 'PUT',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ key: key, value: value })
            });
          }
          var puts = [
            put('site_name', name.input.value.trim()),
            put('default_language', lang.input.value)
          ];
          save.disabled = true;
          status.textContent = t('common.saving');
          Promise.all(puts).then(function () {
            status.textContent = t('common.saved'); status.className = 'inline-status ok';
            return window.Pharus.reloadLanguage(lang.input.value).then(loadView);
          }).catch(function (error) {
            status.textContent = t('common.error') + ': ' + error.message; status.className = 'inline-status error';
          }).then(function () { save.disabled = false; });
        }, 'btn primary');
        var actions = node('div', 'form-actions');
        actions.appendChild(save);
        actions.appendChild(status);
        box.appendChild(actions);
        options.content.appendChild(box);

        // agent communication keys: multiple secrets, each with note/copy/delete
        var secretsBox = node('div', 'settings-stack');
        var secrets = [];
        var secretsStatus = node('span', 'inline-status');
        var secretsList = node('div', 'secret-list');
        secretsBox.appendChild(secretsList);
        function maskSecret(s) {
          return s.length > 4 ? s.slice(0, 4) + '••••••' : '••••••••';
        }
        function renderSecrets() {
          secretsList.innerHTML = '';
          if (!secrets.length) {
            secretsList.appendChild(node('p', 'admin-empty', t('settings.noSecrets')));
            return;
          }
          secrets.forEach(function (entry, idx) {
            var item = node('div', 'secret-item');
            item.appendChild(node('span', 'secret-mask', maskSecret(entry.secret)));
            item.appendChild(node('span', 'secret-note', entry.note || ''));
            var actions = node('div', 'secret-actions');
            actions.appendChild(actionButton(t('common.copy'), function () {
              if (navigator.clipboard) navigator.clipboard.writeText(entry.secret);
            }, 'icon-btn'));
            actions.appendChild(actionButton(t('admin.delete'), function () {
              secrets.splice(idx, 1);
              saveSecrets().then(renderSecrets);
            }, 'icon-btn danger'));
            item.appendChild(actions);
            secretsList.appendChild(item);
          });
        }
        function saveSecrets() {
          secretsStatus.textContent = t('common.saving');
          return options.request('/api/admin/agent-secrets', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(secrets)
          }).then(function () {
            secretsStatus.textContent = t('common.saved'); secretsStatus.className = 'inline-status ok';
          }).catch(function (error) {
            secretsStatus.textContent = t('common.error') + ': ' + error.message; secretsStatus.className = 'inline-status error';
          });
        }
        var secretActions = node('div', 'form-actions');
        secretActions.appendChild(actionButton(t('settings.addSecret'), function () {
          openModal(t('settings.addSecret'), function (root) {
            var f = {};
            f.secret = inputField('settings.agentSecret', '', { type: 'password', autocomplete: 'new-password' });
            f.note = inputField('settings.secretNote', '', { type: 'text' });
            var gen = actionButton(t('settings.generate'), function () {
              var chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
              var out = '';
              var arr = new Uint32Array(16);
              if (window.crypto && crypto.getRandomValues) {
                crypto.getRandomValues(arr);
                for (var i = 0; i < 16; i++) out += chars[arr[i] % chars.length];
              } else {
                for (var i = 0; i < 16; i++) out += chars[Math.floor(Math.random() * chars.length)];
              }
              f.secret.input.value = out;
            }, 'btn ghost');
            var secretRow = node('div', 'form-row');
            secretRow.appendChild(f.secret.el);
            secretRow.appendChild(gen);
            root.appendChild(secretRow);
            root.appendChild(f.note.el);
            modalSubmit = function () {
              var s = f.secret.input.value.trim();
              if (s.length < 6) throw new Error(t('settings.secretTooShort'));
              if (secrets.some(function (e) { return e.secret === s; })) throw new Error(t('settings.secretDuplicate'));
              secrets.push({ secret: s, note: f.note.input.value.trim() || null });
              return saveSecrets().then(function () { closeModal(); renderSecrets(); });
            };
          });
        }, 'btn ghost'));
        secretActions.appendChild(secretsStatus);
        secretsBox.appendChild(secretActions);
        options.content.appendChild(secretsBox);
        options.request('/api/admin/agent-secrets').then(function (list) {
          if (!active || currentView !== 'settings') return;
          secrets = Array.isArray(list) ? list : [];
          renderSecrets();
        }).catch(function () { renderSecrets(); });

        // change password: current + new on the same row
        var pwBox = node('div', 'settings-stack');
        pwBox.appendChild(node('h3', 'settings-heading', t('admin.changePasswordHint')));
        var pwRow = node('div', 'form-row');
        var oldPw = inputField('admin.oldPassword', '', { type: 'password', autocomplete: 'current-password' });
        var newPw = inputField('admin.newPassword', '', { type: 'password', autocomplete: 'new-password' });
        pwRow.appendChild(oldPw.el);
        pwRow.appendChild(newPw.el);
        pwBox.appendChild(pwRow);
        var pwStatus = node('span', 'inline-status');
        var pwSave = actionButton(t('admin.changePassword'), function () {
          if (!oldPw.input.value || !newPw.input.value) {
            pwStatus.textContent = t('common.error');
            pwStatus.className = 'inline-status error';
            return;
          }
          pwSave.disabled = true;
          pwStatus.textContent = t('common.saving');
          options.request('/api/admin/password', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ old_password: oldPw.input.value, new_password: newPw.input.value })
          }).then(function () {
            pwStatus.textContent = t('common.saved'); pwStatus.className = 'inline-status ok';
            oldPw.input.value = '';
            newPw.input.value = '';
          }).catch(function (error) {
            pwStatus.textContent = t('common.error') + ': ' + error.message; pwStatus.className = 'inline-status error';
          }).then(function () { pwSave.disabled = false; });
        }, 'btn primary');
        var pwActions = node('div', 'form-actions');
        pwActions.appendChild(pwSave);
        pwActions.appendChild(pwStatus);
        pwBox.appendChild(pwActions);
        options.content.appendChild(pwBox);
      }).catch(showError);
    }

    function loadView() {
      if (!active) return;
      if (currentView === 'alerts') renderAlerts();
      else if (currentView === 'channels') renderChannels();
      else if (currentView === 'pingTasks') renderPingTasks();
      else if (currentView === 'tasks') renderTasks();
      else if (currentView === 'regions') renderRegions();
      else if (currentView === 'features') renderFeatures();
      else if (currentView === 'hostBilling') renderHostBilling();
      else if (currentView === 'settings') renderSettings();
    }

    options.root.querySelectorAll('[data-admin-view]').forEach(function (tab) {
      tab.addEventListener('click', function () {
        currentView = tab.getAttribute('data-admin-view');
        options.root.querySelectorAll('[data-admin-view]').forEach(function (item) {
          item.classList.toggle('active', item === tab);
        });
        loadView();
      });
    });
    document.getElementById('entity-cancel').addEventListener('click', closeModal);
    options.modal.querySelectorAll('[data-close]').forEach(function (el) {
      el.addEventListener('click', closeModal);
    });
    options.form.addEventListener('submit', function (event) {
      event.preventDefault();
      if (!modalSubmit) return;
      options.formError.hidden = true;
      document.getElementById('entity-save').disabled = true;
      Promise.resolve().then(modalSubmit).catch(function (error) {
        options.formError.textContent = t('common.error') + ': ' + error.message;
        options.formError.hidden = false;
      }).then(function () { document.getElementById('entity-save').disabled = false; });
    });

    return {
      setVisible: function (visible) {
        active = visible;
        if (visible) loadView();
      },
      notifyAgentUpdate: function () {
        if (active && (currentView === 'regions' || currentView === 'features')) loadView();
      }
    };
  }

  window.PharusAdmin = { create: create };
})();
