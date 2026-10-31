/* ==========================================================================
   CraftWork 高保真原型 · 浮层
   命令面板、设置中心（七组）、审计 / 用量 / 备份、输入对话框。
   ========================================================================== */
(function () {
  const CW = window.CW;
  const U = CW.util;
  const S = CW.State;
  const D = CW.Data;

  let focusables = [];
  let paletteIndex = 0;
  let paletteItems = [];

  /* ---------------- 浮层基础 ---------------- */
  function overlayLayer() {
    return U.q('#overlay-layer');
  }

  function confirmLayer() {
    let layer = U.q('#confirm-layer');
    if (!layer) {
      layer = document.createElement('div');
      layer.id = 'confirm-layer';
      layer.className = 'overlay-layer';
      layer.style.zIndex = '360';
      document.body.appendChild(layer);
    }
    return layer;
  }

  function openOverlay(html, mount) {
    const layer = overlayLayer();
    layer.innerHTML = '<div class="overlay-backdrop" data-overlay-close></div>' + html;
    layer.querySelectorAll('[data-overlay-close]').forEach(function (b) {
      b.addEventListener('click', closeOverlay);
    });
    if (mount) mount(layer);
  }

  function closeOverlay() {
    const layer = overlayLayer();
    if (layer) layer.innerHTML = '';
    paletteItems = [];
  }

  function closeConfirm() {
    const layer = U.q('#confirm-layer');
    if (layer) layer.innerHTML = '';
  }

  /* ---------------- 输入对话框 ---------------- */
  function inputDialog(opts) {
    const layer = confirmLayer();
    layer.innerHTML =
      '<div class="overlay-backdrop" data-input-cancel></div>' +
      '<div class="confirm-dialog" role="dialog" aria-modal="true">' +
      '<h3>' + U.icon(opts.icon || 'edit') + U.esc(opts.title) + '</h3>' +
      '<p>' + U.esc(opts.desc || '') + '</p>' +
      '<input class="text-input wide" id="input-dialog-field" style="margin-top:10px" value="' +
      U.esc(opts.value || '') + '" aria-label="' + U.esc(opts.label || '输入') + '">' +
      '<div class="confirm-actions">' +
      '<span class="spacer"></span>' +
      '<button type="button" class="btn" data-input-cancel>取消</button>' +
      '<button type="button" class="btn primary" data-input-ok>' + U.esc(opts.confirmText || '确定') + '</button>' +
      '</div>' +
      '</div>';

    const field = layer.querySelector('#input-dialog-field');
    field.focus();
    field.select();
    function ok() {
      const value = field.value;
      layer.innerHTML = '';
      if (opts.onConfirm) opts.onConfirm(value);
    }
    layer.querySelector('[data-input-ok]').addEventListener('click', ok);
    field.addEventListener('keydown', function (e) {
      if (e.key === 'Enter') ok();
    });
    layer.querySelectorAll('[data-input-cancel]').forEach(function (b) {
      b.addEventListener('click', function () {
        layer.innerHTML = '';
        if (opts.onCancel) opts.onCancel();
      });
    });
  }

  /* ---------------- 命令面板 ---------------- */
  function fuzzyMatch(text, query) {
    if (!query) return true;
    const lower = text.toLowerCase();
    const q = query.toLowerCase();
    if (lower.indexOf(q) >= 0) return true;
    let i = 0;
    for (let j = 0; j < lower.length && i < q.length; j++) {
      if (lower[j] === q[i]) i++;
    }
    return i === q.length;
  }

  function highlightMatch(text, query) {
    if (!query) return U.esc(text);
    const idx = text.toLowerCase().indexOf(query.toLowerCase());
    if (idx < 0) return U.esc(text);
    return (
      U.esc(text.slice(0, idx)) +
      '<mark>' + U.esc(text.slice(idx, idx + query.length)) + '</mark>' +
      U.esc(text.slice(idx + query.length))
    );
  }

  function buildPaletteItems(query) {
    const q = query.trim();
    const items = [];

    // 命令
    D.commands.forEach(function (cmd) {
      if (!fuzzyMatch(cmd.title, q) && !fuzzyMatch(cmd.sub || '', q) && !fuzzyMatch(cmd.id, q)) return;
      items.push({ group: '命令', type: 'command', id: cmd.id, title: cmd.title, sub: cmd.sub, icon: cmd.icon, keys: cmd.keys, action: cmd.action });
    });

    // 线程
    const threads = (S.threads[S.mode] || []).filter(function (t) {
      return fuzzyMatch(t.title, q);
    });
    threads.slice(0, 5).forEach(function (t) {
      items.push({
        group: '线程',
        type: 'thread',
        id: t.id,
        title: t.title,
        sub: (t.mode === 'work' ? 'Work' : 'Code') + ' · ' + t.modelName + ' · ' + t.updated,
        icon: t.mode === 'work' ? 'briefcase' : 'code',
        threadId: t.id,
      });
    });

    // 文件
    if (q) {
      const files = [];
      (function walk(node, base) {
        (node.children || []).forEach(function (child) {
          const p = base ? base + '/' + child.name : child.name;
          if (child.type === 'dir') walk(child, p);
          else if (fuzzyMatch(child.name, q)) files.push(p);
        });
      })(D.fileTree, '');
      files.slice(0, 6).forEach(function (p) {
        items.push({ group: '文件', type: 'file', id: p, title: p.split('/').pop(), sub: p, icon: 'file' });
      });
    }

    return items;
  }

  function renderPaletteList(query) {
    const list = U.q('#cp-list');
    if (!list) return;
    paletteItems = buildPaletteItems(query);
    paletteIndex = 0;

    if (paletteItems.length === 0) {
      list.innerHTML =
        '<div class="cp-empty">没有匹配的命令、线程或文件<br><span style="font-size:10.5px;color:var(--text-4)">试试“设置”“Work”“session.go”</span></div>';
      return;
    }

    let html = '';
    let currentGroup = '';
    paletteItems.forEach(function (item, i) {
      if (item.group !== currentGroup) {
        currentGroup = item.group;
        html += '<div class="cp-group-label">' + currentGroup + '</div>';
      }
      html +=
        '<button type="button" class="cp-item' + (i === 0 ? ' is-active' : '') + '" data-cp-index="' + i + '">' +
        '<span class="cp-icon">' + U.icon(item.icon) + '</span>' +
        '<span class="cp-main"><div class="cp-title">' + highlightMatch(item.title, query) + '</div>' +
        '<div class="cp-sub">' + U.esc(item.sub || '') + '</div></span>' +
        '<span class="cp-trail">' +
        (item.keys ? item.keys.map(function (k) { return '<span class="kbd">' + k + '</span>'; }).join('') : '') +
        '</span></button>';
    });
    list.innerHTML = html;
  }

  function openPalette() {
    openOverlay(
      '<div class="command-palette" role="dialog" aria-modal="true" aria-label="命令面板">' +
        '<div class="cp-input-row">' + U.icon('search') +
        '<input class="cp-input" id="cp-input" type="text" placeholder="搜索命令、线程或文件…" autocomplete="off">' +
        '<span class="cp-scope">' + (S.mode === 'work' ? 'Work' : 'Code') + ' 模式</span>' +
        '<span class="kbd">Esc</span>' +
        '</div>' +
        '<div class="cp-list" id="cp-list"></div>' +
        '<div class="cp-foot">' +
        '<span><span class="kbd">↑</span><span class="kbd">↓</span> 选择</span>' +
        '<span><span class="kbd">↵</span> 执行</span>' +
        '<span><span class="kbd">Esc</span> 关闭</span>' +
        '<span class="spacer"></span>' +
        '<span>输入文件路径可直接打开</span>' +
        '</div>' +
        '</div>',
      function (layer) {
        const input = layer.querySelector('#cp-input');
        renderPaletteList('');
        input.focus();
        input.addEventListener('input', function () {
          renderPaletteList(input.value);
        });
        input.addEventListener('keydown', function (e) {
          if (e.key === 'ArrowDown') {
            e.preventDefault();
            movePalette(1);
          } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            movePalette(-1);
          } else if (e.key === 'Enter') {
            e.preventDefault();
            const item = paletteItems[paletteIndex];
            if (item) runPaletteItem(item, input.value);
          }
        });
        layer.querySelector('#cp-list').addEventListener('click', function (e) {
          const btn = e.target.closest('[data-cp-index]');
          if (!btn) return;
          const item = paletteItems[Number(btn.getAttribute('data-cp-index'))];
          if (item) runPaletteItem(item, input.value);
        });
      }
    );
  }

  function movePalette(delta) {
    if (!paletteItems.length) return;
    paletteIndex = (paletteIndex + delta + paletteItems.length) % paletteItems.length;
    U.qa('.cp-item').forEach(function (el, i) {
      el.classList.toggle('is-active', i === paletteIndex);
    });
    const active = U.q('.cp-item.is-active');
    if (active) active.scrollIntoView({ block: 'nearest' });
  }

  function runPaletteItem(item, query) {
    closeOverlay();
    if (item.type === 'thread') {
      const thread = CW.findThread(item.threadId);
      if (thread) {
        S.mode = thread.mode;
        S.active[thread.mode] = thread.id;
        CW.applyMode(thread.mode);
        CW.ui.refresh();
      }
      return;
    }
    if (item.type === 'file') {
      CW.openFile(item.id);
      return;
    }
    executeCommand(item.action, query);
  }

  function executeCommand(action, query) {
    if (action === 'newThread') CW.Scene.newThread();
    else if (action === 'searchThread') CW.toggleThreadSearch(true);
    else if (action === 'modeWork') CW.switchMode('work');
    else if (action === 'modeCode') CW.switchMode('code');
    else if (action === 'settings') openSettings('general');
    else if (action === 'theme') CW.applyTheme(S.theme === 'dark' ? 'light' : 'dark');
    else if (action === 'panelDiff') {
      S.panelTab[S.mode] = 'diff';
      CW.ui.refresh();
    } else if (action === 'panelTerminal') {
      if (S.mode !== 'code') {
        CW.toast({ kind: 'warn', title: '终端只在 Code 模式提供' });
        return;
      }
      S.panelTab.code = 'terminal';
      CW.ui.refresh();
    } else if (action === 'settingsMcp') openSettings('mcp');
    else if (action === 'usage') openDataPanel('usage');
    else if (action === 'audit') openDataPanel('audit');
    else if (action === 'archiveThread') {
      const t = CW.currentThread();
      if (t) CW.Scene.archiveThread(t);
    } else if (action === 'copyThread') {
      const t = CW.currentThread();
      if (t) CW.Scene.copyThread(t);
    } else if (action === 'regenerate') {
      const t = CW.currentThread();
      if (t) {
        const last = t.messages
          .slice()
          .reverse()
          .find(function (m) {
            return m.role === 'assistant';
          });
        if (last) CW.Scene.regenerate(t, last);
      }
    } else if (action === 'clearInput') {
      const input = U.q('#composer-input');
      if (input) {
        input.value = '';
        U.autoGrow(input);
        CW.Chat.updateSendAvailability();
      }
    }
  }

  /* ---------------- 设置中心 ---------------- */
  const SETTINGS_SECTIONS = [
    { id: 'general', label: '通用', icon: 'settings', group: '应用' },
    { id: 'mode', label: '模式', icon: 'layers', group: '应用' },
    { id: 'models', label: '模型与提供商', icon: 'cpu', group: '能力' },
    { id: 'tools', label: '工具与权限', icon: 'shield', group: '能力' },
    { id: 'mcp', label: 'MCP 服务器', icon: 'database', group: '能力' },
    { id: 'appearance', label: '外观', icon: 'sun', group: '界面' },
    { id: 'advanced', label: '高级', icon: 'sliders', group: '界面' },
  ];

  function setRow(label, desc, control, extra) {
    return (
      '<div class="set-row">' +
      '<div class="set-row-main"><div class="set-row-label">' + label + (extra || '') + '</div>' +
      (desc ? '<div class="set-row-desc">' + desc + '</div>' : '') +
      '</div>' +
      '<div class="set-control">' + control + '</div>' +
      '</div>'
    );
  }

  function selectControl(setting, options, value) {
    return (
      '<select class="select" data-setting="' + setting + '">' +
      options
        .map(function (o) {
          return '<option value="' + U.esc(o[0]) + '"' + (o[0] === value ? ' selected' : '') + '>' + U.esc(o[1]) + '</option>';
        })
        .join('') +
      '</select>'
    );
  }

  function switchControl(setting, on) {
    return (
      '<button type="button" class="switch' + (on ? ' is-on' : '') + '" role="switch" aria-checked="' + !!on +
      '" data-toggle-setting="' + setting + '" aria-label="切换设置"></button>'
    );
  }

  function settingsGeneral() {
    return (
      '<section class="settings-section"><h3>' + U.icon('globe') + '语言与区域</h3>' +
      '<div class="sec-desc">界面文案走消息编号，切换后主要界面即时更新。</div>' +
      setRow('界面语言', '简体中文或英文；日期与数字按区域格式化。', selectControl('language', [['zh-CN', '简体中文'], ['en-US', 'English']], S.settings.language)) +
      '</section>' +
      '<section class="settings-section"><h3>' + U.icon('play') + '启动与更新</h3>' +
      setRow('启动时', '决定应用打开后显示的界面。', selectControl('startup', [['restore', '恢复上次会话'], ['new', '打开新线程'], ['workspace', '显示工作区选择']], S.settings.startup)) +
      setRow('run 完成时通知', '等待审批与错误也会通知；系统权限被拒时静默降级。', switchControl('notify', S.settings.notify)) +
      setRow('自动检查更新', '下载前询问；安装前备份数据库，失败回滚。', switchControl('autoUpdate', S.settings.autoUpdate)) +
      setRow('更新渠道', 'stable 为默认渠道，beta 可提前试用。签名校验失败会拒绝安装。', selectControl('updateChannel', [['stable', 'stable'], ['beta', 'beta']], S.settings.updateChannel)) +
      '<div class="settings-tip">' + U.icon('info') +
      '<span>本产品不收集遥测。崩溃日志只保存在本地，可在“高级”中手动导出。</span></div>' +
      '</section>'
    );
  }

  function settingsMode() {
    return (
      '<section class="settings-section"><h3>' + U.icon('layers') + '模式行为</h3>' +
      '<div class="sec-desc">模式是工作台级切换：只改变界面、可用工具与线程列表，不改变工作区。</div>' +
      setRow('默认模式', '新窗口打开时使用的模式。首次启动仍会按工作区类型推荐。', selectControl('defaultMode', [['recommend', '按工作区类型推荐'], ['code', 'Code'], ['work', 'Work']], S.settings.defaultMode)) +
      setRow('新线程默认模型', '线程内切换模型只影响后续 run。', selectControl('defaultModel', S.models.map(function (m) { return [m.id, m.name]; }), S.settings.defaultModel || 'claude-sonnet-4-5')) +
      '</section>' +
      '<section class="settings-section"><h3>' + U.icon('mail') + 'Work 模式连接</h3>' +
      '<div class="sec-desc">邮件与日历通过 MCP 只读接入；未授权的连接不可用。</div>' +
      '<div class="mcp-card"><div class="mcp-head">' +
      '<span class="mcp-icon">' + U.icon('mail') + '</span>' +
      '<div class="mcp-main"><div class="mcp-name">邮件（Gmail）<span class="badge ok">已连接</span></div>' +
      '<div class="mcp-sub">只读 · OAuth 令牌保存在 keys.json 的 mcp 节点</div></div>' +
      '<div class="provider-actions"><button type="button" class="btn sm ghost" data-open-mcp>管理</button></div>' +
      '</div></div>' +
      '<div class="mcp-card"><div class="mcp-head">' +
      '<span class="mcp-icon">' + U.icon('calendar') + '</span>' +
      '<div class="mcp-main"><div class="mcp-name">日历（Google Calendar）<span class="badge ok">已连接</span></div>' +
      '<div class="mcp-sub">只读 · 不提供创建或修改日程的工具</div></div>' +
      '<div class="provider-actions"><button type="button" class="btn sm ghost" data-open-mcp>管理</button></div>' +
      '</div></div>' +
      '<div class="settings-tip">' + U.icon('shield') +
      '<span>邮件与日历内容视为不可信数据，只发送给当前选择的模型，不写入日志、不导出。</span></div>' +
      '</section>'
    );
  }

  function settingsModels() {
    const providers = D.providers
      .map(function (p) {
        return (
          '<div class="provider-card' + (p.id === 'anthropic' ? ' is-open' : '') + '" data-provider-card="' + p.id + '">' +
          '<div class="provider-head">' +
          '<span class="provider-logo">' + U.esc(p.logo) + '</span>' +
          '<div class="provider-main">' +
          '<div class="provider-name">' + U.esc(p.name) +
          '<span class="badge ' + (p.state === 'connected' ? 'ok' : 'neutral') + '">' + (p.state === 'connected' ? '已连接' : '未配置') + '</span>' +
          (p.keyMode === 'env' ? '<span class="badge warn">环境变量</span>' : '') +
          '</div>' +
          '<div class="provider-sub">' + U.esc(p.kind) + ' · ' + U.esc(p.endpoint) + ' · ' + p.models + ' 个模型</div>' +
          '</div>' +
          '<div class="provider-actions">' +
          '<button type="button" class="btn sm" data-test-provider="' + p.id + '">测试连接</button>' +
          '<button type="button" class="icon-btn sm" data-provider-toggle="' + p.id + '" aria-label="展开提供商">' + U.icon('chevron-down') + '</button>' +
          '</div></div>' +
          '<div class="provider-body">' +
          '<div class="key-line">' + U.icon('key') +
          '<span>密钥来源：<span class="key-source">' + U.esc(p.keySource) + '</span></span>' +
          '<span class="spacer"></span>' +
          '<span class="key-mask">' + U.esc(p.keyMask) + '</span>' +
          '</div>' +
          (p.keyMode === 'env'
            ? '<div class="key-note" style="margin-bottom:7px">环境变量优先级低于密钥文件。迁移到 <span class="mono">~/.craftwork/keys.json</span> 后建议删除环境变量。</div>' +
              '<button type="button" class="btn sm ghost" data-migrate-key="' + p.id + '">迁移到密钥文件</button>'
            : '<div class="key-note">密钥文件目录权限 0700，文件权限 0600；不进入配置、日志、工具输出与导出包。</div>') +
          '<div class="settings-tip" style="margin-top:8px">' + U.icon('info') +
          '<span>同一提供商下的模型共用同一密钥。不同密钥需要配置为不同提供商。</span></div>' +
          '</div></div>'
        );
      })
      .join('');

    const caps = [
      ['流式', 'stream'],
      ['工具调用', 'tools'],
      ['图片输入', 'vision'],
      ['推理内容', 'reasoning'],
      ['结构化输出', 'structured'],
    ];

    const capRows = S.models
      .map(function (m) {
        return (
          '<tr><td>' + U.esc(m.name) + '<div style="font-size:9.5px;color:var(--text-4)">' + U.esc(m.provider) + ' · ' + m.context + 'k</div></td>' +
          caps
            .map(function (c) {
              return '<td><span class="cap-pill ' + (m.caps[c[1]] ? 'y' : 'n') + '">' + (m.caps[c[1]] ? '✓' : '–') + '</span></td>';
            })
            .join('') +
          '</tr>'
        );
      })
      .join('');

    return (
      '<section class="settings-section"><h3>' + U.icon('cpu') + '默认模型</h3>' +
      '<div class="sec-desc">能力不满足时对应控件自动禁用，不报错。</div>' +
      setRow('默认模型', '新建线程时使用；线程内仍可单独切换。', selectControl('defaultModel', S.models.map(function (m) { return [m.id, m.name]; }), S.settings.defaultModel || 'claude-sonnet-4-5')) +
      '</section>' +
      '<section class="settings-section"><h3>' + U.icon('key') + '提供商与密钥</h3>' +
      '<div class="sec-desc">密钥按提供商保存到 <span class="mono">~/.craftwork/keys.json</span>；文件权限宽于 0600 时拒绝读取并提示修复。</div>' +
      providers +
      '</section>' +
      '<section class="settings-section"><h3>' + U.icon('sliders') + '能力矩阵</h3>' +
      '<div class="sec-desc">按能力显示或禁用控件：图片输入、推理内容、结构化输出。</div>' +
      '<div style="border:1px solid var(--border);border-radius:8px;overflow:hidden">' +
      '<table class="cap-table"><thead><tr><th>模型</th>' +
      caps.map(function (c) { return '<th>' + c[0] + '</th>'; }).join('') +
      '</tr></thead><tbody>' + capRows + '</tbody></table>' +
      '</div></section>'
    );
  }

  function settingsTools() {
    const policies = [
      ['R0', '读', '读文件、列目录、git status、读取邮件与日历', 'auto', '自动允许并记录审计'],
      ['R1', '写工作区', '写文件、补丁、git commit、Word/Excel 写入', 'ask', '每次询问；文档写入必须每次审批'],
      ['R2', '执行', '运行测试、构建、格式化', 'ask', '每次询问，或按命令前缀允许'],
      ['R3', '网络或越界', 'curl、npm install、git push、访问工作区外', 'ask', '每次询问，不提供永久允许'],
      ['R4', '破坏性', 'rm -rf、sudo、chmod 777、写系统目录', 'block', '默认阻止；双重确认也不放行'],
    ];

    return (
      '<section class="settings-section"><h3>' + U.icon('shield-check') + '审批策略</h3>' +
      '<div class="sec-desc">审批、审计、路径校验、沙箱、命令解析同时存在，沙箱不是唯一防线。输入框底部也提供同一档位开关。</div>' +
      setRow(
        '审批档位',
        '决定写入与命令执行的询问强度；R3 / R4 在任何档位下都不提供永久允许。',
        selectControl(
          'approvalMode',
          [['graded', '分级审批（默认）'], ['strict', '严格审批'], ['readonly', '只读']],
          S.settings.approvalMode || 'graded'
        )
      ) +
      '<div style="border:1px solid var(--border);border-radius:8px;overflow:hidden">' +
      '<table class="perm-table"><thead><tr><th>等级</th><th>名称</th><th>示例</th><th>默认策略</th></tr></thead><tbody>' +
      policies
        .map(function (p) {
          return (
            '<tr><td><span class="perm-level risk-' + p[0] + '" style="padding:1px 4px;border-radius:3px">' + p[0] + '</span></td>' +
            '<td>' + p[1] + '</td><td>' + p[2] + '</td>' +
            '<td><span class="perm-policy ' + p[3] + '">' + p[4] + '</span></td></tr>'
          );
        })
        .join('') +
      '</tbody></table></div>' +
      '<div class="settings-tip">' + U.icon('alert') +
      '<span>审批弹窗默认焦点在“拒绝”，不得默认选中“允许”；多个审批排队逐个展示。</span></div>' +
      '</section>' +
      '<section class="settings-section"><h3>' + U.icon('lock') + '执行与网络</h3>' +
      setRow('Shell 模式', 'confined 为默认：命令解析后分类，危险命令阻止。', selectControl('shellMode', [['confined', 'confined（推荐）'], ['allowlist', '仅允许列表'], ['ask', '全部询问']], S.settings.shellMode)) +
      setRow('网络访问', 'Agent 发起的网络请求默认需要审批。', selectControl('networkPolicy', [['ask', '每次询问'], ['deny', '默认拒绝']], S.settings.networkPolicy)) +
      setRow('工作区信任', '信任后 R1 仍逐次审批；可随时撤销。', '<span class="badge ok">' + U.icon('shield') + '信任</span>' + '<button type="button" class="btn sm ghost" data-revoke-trust style="margin-left:6px">撤销</button>') +
      '</section>' +
      '<section class="settings-section"><h3>' + U.icon('terminal') + '命令允许列表</h3>' +
      '<div class="sec-desc">按命令前缀匹配，只对 R2 生效；R3 与 R4 永不进入允许列表。</div>' +
      '<div class="mcp-tool-chips">' +
      ['go test', 'go build', 'gofmt', 'go vet', 'git status', 'git diff'].map(function (c) {
        return '<span class="mcp-tool-chip">' + c + '</span>';
      }).join('') +
      '<button type="button" class="mcp-tool-chip" data-add-allowlist style="cursor:pointer">+ 添加</button>' +
      '</div>' +
      '<div class="settings-tip">' + U.icon('info') +
      '<span>沙箱能力：macOS Seatbelt 可用；Windows 使用受限令牌与 AppContainer，能力较弱，界面会标注。</span></div>' +
      '</section>'
    );
  }

  function settingsMcp() {
    return (
      '<section class="settings-section"><h3>' + U.icon('database') + 'MCP 服务器</h3>' +
      '<div class="sec-desc">服务器由用户显式添加；未启用不加载工具。首次连接需要确认来源与权限。</div>' +
      '<div class="audit-filters">' +
      '<button type="button" class="btn sm" data-add-mcp>' + U.icon('plus') + '添加服务器</button>' +
      '<button type="button" class="btn sm ghost" data-add-template>' + U.icon('mail') + '添加推荐模板</button>' +
      '<span class="spacer"></span>' +
      '<span style="font-size:9.5px;color:var(--text-4)">' + D.mcpServers.filter(function (s) { return s.state === 'connected'; }).length + ' 个已连接</span>' +
      '</div>' +
      D.mcpServers
        .map(function (m) {
          return (
            '<div class="mcp-card">' +
            '<div class="mcp-head">' +
            '<span class="mcp-icon">' + U.icon(m.transport === 'stdio' ? 'terminal' : 'globe') + '</span>' +
            '<div class="mcp-main">' +
            '<div class="mcp-name">' + U.esc(m.name) +
            '<span class="badge ' + (m.state === 'connected' ? 'ok' : 'neutral') + '">' + (m.state === 'connected' ? '已连接' : '已停用') + '</span>' +
            (m.oauth ? '<span class="badge purple">OAuth</span>' : '') +
            '</div>' +
            '<div class="mcp-sub">' + U.esc(m.transport) + ' · ' + U.esc(m.command) + '</div>' +
            '</div>' +
            '<div class="provider-actions">' +
            '<button type="button" class="btn sm ghost" data-mcp-tools="' + m.id + '">工具 ' + m.tools.length + '</button>' +
            '<button type="button" class="switch' + (m.state === 'connected' ? ' is-on' : '') + '" role="switch" aria-checked="' + (m.state === 'connected') + '" data-mcp-toggle="' + m.id + '" aria-label="启用或停用服务器"></button>' +
            '</div></div>' +
            '<div class="mcp-tools">' +
            '<div class="mcp-tool-chips">' +
            m.tools.map(function (t) { return '<span class="mcp-tool-chip">mcp__' + m.id + '__' + t + '</span>'; }).join('') +
            '</div>' +
            '<div class="mcp-meta-row">' +
            '<span>可见模式：' + U.esc(m.scope) + '</span><span>·</span><span>风险：' + U.esc(m.risk) + '</span>' +
            '<span class="spacer"></span>' +
            '<span>连接超时 10s · 调用超时 60s</span>' +
            '</div></div></div>'
          );
        })
        .join('') +
      '<div class="settings-tip">' + U.icon('shield') +
      '<span>MCP 工具描述视为不可信内容；模型不能自动安装、启动或修改服务器。远程服务器只允许 HTTPS。</span></div>' +
      '</section>'
    );
  }

  function settingsAppearance() {
    return (
      '<section class="settings-section"><h3>' + U.icon('sun') + '主题</h3>' +
      '<div class="sec-desc">界面主题与代码主题分开设置；切换后所有面板即时重建配色。</div>' +
      '<div class="theme-cards">' +
      '<button type="button" class="theme-card dark' + (S.theme === 'dark' ? ' is-on' : '') + '" data-theme-card="dark">' +
      '<span class="theme-preview"><i class="s" style="width:22%"></i><i class="m" style="width:60%"></i><i class="a"></i></span>' +
      '<span class="theme-label">深色' + (S.theme === 'dark' ? U.icon('check') : '') + '</span></button>' +
      '<button type="button" class="theme-card light' + (S.theme === 'light' ? ' is-on' : '') + '" data-theme-card="light">' +
      '<span class="theme-preview"><i class="s" style="width:22%"></i><i class="m" style="width:60%"></i><i class="a"></i></span>' +
      '<span class="theme-label">浅色' + (S.theme === 'light' ? U.icon('check') : '') + '</span></button>' +
      '</div>' +
      setRow('代码主题', '只影响代码块与编辑器。', selectControl('codeTheme', [['CraftWork Dark', 'CraftWork Dark'], ['CraftWork Light', 'CraftWork Light'], ['Solarized', 'Solarized']], S.settings.codeTheme)) +
      '</section>' +
      '<section class="settings-section"><h3>' + U.icon('type') + '字体与字号</h3>' +
      setRow('界面字号', '调整后立即生效，重启后保留。', selectControl('fontSize', [['12', '12 px'], ['13', '13 px（默认）'], ['14', '14 px']], String(S.settings.fontSize))) +
      setRow('终端字号', '中日韩字体缺失时回退到系统等宽字体。', selectControl('terminalFontSize', [['10', '10 px'], ['11', '11 px（默认）'], ['13', '13 px']], String(S.settings.terminalFontSize))) +
      '<div class="settings-tip">' + U.icon('info') +
      '<span>不打包完整中日韩字体，优先使用系统字体；缺失时会回退并提示。</span></div>' +
      '</section>'
    );
  }

  function settingsAdvanced() {
    return (
      '<section class="settings-section"><h3>' + U.icon('hard-drive') + '数据与备份</h3>' +
      setRow('数据目录', '变更后需要重启；可选择复制现有数据或使用空目录。', '<input class="text-input mono" id="data-dir-input" value="' + U.esc(S.settings.dataDir) + '"><button type="button" class="btn sm ghost" data-change-datadir>更改</button>') +
      setRow('每日备份', '每天首次启动备份，保留最近 7 份；启动时做快速完整性检查。', '<button type="button" class="btn sm ghost" data-open-backup>查看备份（' + D.backups.length + '）</button>') +
      setRow('导出数据', '导出数据库、配置与最近日志；不包含 keys.json。', '<button type="button" class="btn sm ghost" data-export-data>导出</button>') +
      setRow('崩溃日志', '只保存在本地，不自动上报；由用户手动导出。', '<button type="button" class="btn sm ghost" data-export-crash>导出日志</button>') +
      setRow('彻底删除线程', '软删除 7 天后清理事件与本地大文件；彻底删除不可恢复。', '<button type="button" class="btn sm danger" data-purge-thread>清理回收站</button>') +
      '</section>' +
      '<section class="settings-section"><h3>' + U.icon('shield') + '隐私</h3>' +
      setRow('遥测', '固定关闭，不收集任何遥测数据。', '<span class="badge neutral">不收集</span>') +
      setRow('密钥文件', '<span class="mono">~/.craftwork/keys.json</span> · 目录 0700 · 文件 0600 · 导出时排除', '<span class="badge ok">' + U.icon('lock') + '正常</span>') +
      '</section>' +
      '<section class="settings-section"><h3>' + U.icon('info') + '关于</h3>' +
      setRow('CraftWork', '版本 1.0.0（原型）· Apache-2.0 · DCO 贡献流程', '<button type="button" class="btn sm ghost" data-check-update>检查更新</button>') +
      '<div class="settings-tip">' + U.icon('code') +
      '<span>高保真原型仅用于交互与视觉评审。正式实现为 Go 原生界面，不使用任何 WebView。</span></div>' +
      '</section>'
    );
  }

  function sectionContent(id) {
    if (id === 'general') return settingsGeneral();
    if (id === 'mode') return settingsMode();
    if (id === 'models') return settingsModels();
    if (id === 'tools') return settingsTools();
    if (id === 'mcp') return settingsMcp();
    if (id === 'appearance') return settingsAppearance();
    return settingsAdvanced();
  }

  function openSettings(tab) {
    S.settingsTab = tab || S.settingsTab || 'general';
    S.settingsQuery = '';
    openOverlay(
      '<div class="settings-modal" role="dialog" aria-modal="true" aria-label="设置">' +
        '<div class="settings-head">' + U.icon('settings') +
        '<h2>设置</h2>' +
        '<label class="settings-search">' + U.icon('search') +
        '<input id="settings-search" type="text" placeholder="搜索设置…" autocomplete="off"></label>' +
        '<button type="button" class="icon-btn sm" data-overlay-close aria-label="关闭设置">' + U.icon('x') + '</button>' +
        '</div>' +
        '<div class="settings-body">' +
        '<div class="settings-nav" id="settings-nav"></div>' +
        '<div class="settings-content" id="settings-content"></div>' +
        '</div>' +
        '</div>',
      function (layer) {
        renderSettingsNav();
        renderSettingsContent();
        const search = layer.querySelector('#settings-search');
        search.addEventListener('input', function () {
          S.settingsQuery = search.value.trim().toLowerCase();
          renderSettingsContent();
        });
        layer.querySelector('#settings-nav').addEventListener('click', function (e) {
          const item = e.target.closest('[data-settings-tab]');
          if (!item) return;
          S.settingsTab = item.getAttribute('data-settings-tab');
          S.settingsQuery = '';
          const s = layer.querySelector('#settings-search');
          if (s) s.value = '';
          renderSettingsNav();
          renderSettingsContent();
        });
        bindSettingsContent(layer.querySelector('#settings-content'));
      }
    );
  }

  function renderSettingsNav() {
    const nav = U.q('#settings-nav');
    if (!nav) return;
    let group = '';
    nav.innerHTML = SETTINGS_SECTIONS.map(function (sec) {
      let html = '';
      if (sec.group !== group) {
        group = sec.group;
        html += '<div class="settings-nav-group">' + group + '</div>';
      }
      html +=
        '<button type="button" class="settings-nav-item' + (sec.id === S.settingsTab ? ' is-active' : '') + '" data-settings-tab="' + sec.id + '">' +
        U.icon(sec.icon) + sec.label + '</button>';
      return html;
    }).join('');
  }

  function renderSettingsContent() {
    const content = U.q('#settings-content');
    if (!content) return;
    const query = S.settingsQuery || '';
    if (query) {
      const matched = SETTINGS_SECTIONS.filter(function (sec) {
        const html = sectionContent(sec.id).replace(/<[^>]+>/g, ' ');
        return html.toLowerCase().indexOf(query) >= 0 || sec.label.toLowerCase().indexOf(query) >= 0;
      });
      content.innerHTML = matched.length
        ? matched.map(function (sec) { return sectionContent(sec.id); }).join('')
        : '<div class="cp-empty">没有匹配的设置项<br><span style="font-size:10.5px;color:var(--text-4)">试试“密钥”“主题”“MCP”“沙箱”</span></div>';
    } else {
      content.innerHTML = sectionContent(S.settingsTab);
    }
    bindSettingsContent(content);
  }

  function bindSettingsContent(content) {
    if (!content) return;

    content.querySelectorAll('[data-setting]').forEach(function (sel) {
      sel.addEventListener('change', function () {
        const key = sel.getAttribute('data-setting');
        S.settings[key] = sel.value;
        CW.savePrefs();
        if (key === 'fontSize') {
          document.documentElement.style.setProperty('--font-size', sel.value + 'px');
        }
        if (key === 'approvalMode') {
          // 与输入框底部的档位开关保持同步
          CW.Chat.renderComposer();
          CW.ui.refresh();
        }
        CW.toast({ kind: 'ok', title: '设置已保存', desc: '无效值不会写入配置。' });
      });
    });

    content.querySelectorAll('[data-toggle-setting]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        const key = btn.getAttribute('data-toggle-setting');
        S.settings[key] = !S.settings[key];
        btn.classList.toggle('is-on', S.settings[key]);
        btn.setAttribute('aria-checked', String(S.settings[key]));
        CW.savePrefs();
        CW.toast({ kind: 'info', title: '已' + (S.settings[key] ? '开启' : '关闭') + '：' + key });
      });
    });

    content.querySelectorAll('[data-provider-toggle]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        const card = btn.closest('.provider-card');
        card.classList.toggle('is-open');
      });
    });

    content.querySelectorAll('[data-test-provider]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        const id = btn.getAttribute('data-test-provider');
        const original = btn.textContent;
        btn.disabled = true;
        btn.textContent = '测试中…';
        setTimeout(function () {
          btn.disabled = false;
          btn.textContent = original;
          const provider = D.providers.find(function (p) { return p.id === id; });
          CW.toast({
            kind: 'ok',
            title: '连接成功 · ' + provider.name,
            desc: '延迟 ' + provider.latency + ' · 密钥不写入日志',
          });
        }, 1100);
      });
    });

    content.querySelectorAll('[data-migrate-key]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        CW.confirmDialog({
          title: '迁移密钥到本地文件？',
          desc: '密钥会写入 ~/.craftwork/keys.json（0600）。迁移后建议删除对应的环境变量。',
          detail: 'DEEPSEEK_API_KEY → providers.deepseek',
          confirmText: '迁移',
          onConfirm: function () {
            const p = D.providers.find(function (x) { return x.id === 'deepseek'; });
            p.keyMode = 'file';
            p.keySource = '密钥文件';
            S.settingsTab = 'models';
            renderSettingsContent();
            CW.toast({ kind: 'ok', title: '已迁移', desc: '请删除环境变量 DEEPSEEK_API_KEY' });
          },
        });
      });
    });

    content.querySelectorAll('[data-mcp-toggle]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        const id = btn.getAttribute('data-mcp-toggle');
        const server = D.mcpServers.find(function (m) { return m.id === id; });
        const on = server.state !== 'connected';
        server.state = on ? 'connected' : 'disabled';
        btn.classList.toggle('is-on', on);
        btn.setAttribute('aria-checked', String(on));
        CW.toast({
          kind: on ? 'ok' : 'info',
          title: on ? '已启用 ' + server.name : '已停用 ' + server.name,
          desc: on ? '正在启动服务器并发现工具…' : '该服务器的工具已从模型中移除',
        });
      });
    });

    content.querySelectorAll('[data-mcp-tools]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        const server = D.mcpServers.find(function (m) { return m.id === btn.getAttribute('data-mcp-tools'); });
        CW.toast({
          kind: 'info',
          title: server.name + ' · 工具列表',
          desc: server.tools.join('、'),
          duration: 6000,
        });
      });
    });

    content.querySelectorAll('[data-theme-card]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        CW.applyTheme(btn.getAttribute('data-theme-card'));
        S.settingsTab = 'appearance';
        renderSettingsContent();
      });
    });

    content.querySelectorAll('[data-open-mcp]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        S.settingsTab = 'mcp';
        renderSettingsNav();
        renderSettingsContent();
      });
    });

    content.querySelectorAll('[data-open-backup]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        closeOverlay();
        openDataPanel('backup');
      });
    });

    content.querySelectorAll('[data-add-mcp]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        CW.toast({ kind: 'info', title: '添加 MCP 服务器', desc: '原型演示：真实产品会打开来源、命令或地址、权限说明的表单。' });
      });
    });

    content.querySelectorAll('[data-add-template]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        CW.toast({ kind: 'ok', title: '推荐模板已添加', desc: '邮件与日历模板只含地址、权限说明与授权方式，不预置账号与密钥。' });
      });
    });

    content.querySelectorAll('[data-add-allowlist]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        inputDialog({
          title: '添加命令前缀',
          desc: '只对 R2 执行类命令生效；R3 与 R4 永远不会进入允许列表。',
          label: '命令前缀',
          value: 'go test',
          confirmText: '添加',
          onConfirm: function () {
            CW.toast({ kind: 'ok', title: '已添加到允许列表' });
          },
        });
      });
    });

    content.querySelectorAll('[data-revoke-trust]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        CW.confirmDialog({
          title: '撤销工作区信任？',
          desc: '撤销后工作区进入受限模式，写入与执行都必须审批，已发出的审批记录保留。',
          detail: S.workspace.path,
          danger: true,
          confirmText: '撤销信任',
          onConfirm: function () {
            S.workspace.trust = 'restricted';
            S.settingsTab = 'tools';
            renderSettingsContent();
            CW.ui.refresh();
            CW.toast({ kind: 'warn', title: '已切换为受限模式' });
          },
        });
      });
    });

    content.querySelectorAll('[data-change-datadir]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        const input = U.q('#data-dir-input');
        CW.confirmDialog({
          title: '更改数据目录？',
          desc: '更改后需要重启应用。可以选择复制现有数据，或使用一个空目录。',
          detail: input ? input.value : '',
          confirmText: '更改并重启',
          onConfirm: function () {
            CW.toast({ kind: 'ok', title: '已记录新的数据目录', desc: '重启后生效，原数据保留' });
          },
        });
      });
    });

    content.querySelectorAll('[data-export-data]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        CW.toast({ kind: 'ok', title: '导出已开始（原型演示）', desc: '包含数据库、配置与最近日志；不包含 keys.json' });
      });
    });
    content.querySelectorAll('[data-export-crash]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        CW.toast({ kind: 'ok', title: '崩溃日志已导出（原型演示）', desc: '仅本地生成，未上传' });
      });
    });
    content.querySelectorAll('[data-purge-thread]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        CW.confirmDialog({
          title: '清理回收站中的线程？',
          desc: '软删除超过 7 天的线程会被彻底删除，包含事件与本地大文件。此操作不可恢复。',
          danger: true,
          confirmText: '彻底删除',
          onConfirm: function () {
            CW.toast({ kind: 'ok', title: '回收站已清理' });
          },
        });
      });
    });
    content.querySelectorAll('[data-check-update]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        btn.disabled = true;
        btn.textContent = '检查中…';
        setTimeout(function () {
          btn.disabled = false;
          btn.textContent = '检查更新';
          CW.toast({ kind: 'ok', title: '已是最新版本', desc: 'stable 渠道 · v1.0.0' });
        }, 900);
      });
    });
  }

  /* ---------------- 审计 / 用量 / 备份 ---------------- */
  function openDataPanel(tab) {
    S.dataView = tab || 'audit';
    openOverlay(
      '<div class="data-modal" role="dialog" aria-modal="true" aria-label="数据面板">' +
        '<div class="data-head">' + U.icon(tab === 'usage' ? 'zap' : tab === 'backup' ? 'hard-drive' : 'shield-check') +
        '<h2>' + (tab === 'usage' ? '用量统计' : tab === 'backup' ? '备份与恢复' : '审计记录') + '</h2>' +
        '<button type="button" class="icon-btn sm" data-overlay-close aria-label="关闭">' + U.icon('x') + '</button>' +
        '</div>' +
        '<div class="data-tabs" id="data-tabs">' +
        '<button type="button" class="data-tab' + (S.dataView === 'audit' ? ' is-active' : '') + '" data-data-tab="audit">' + U.icon('shield-check') + '审计</button>' +
        '<button type="button" class="data-tab' + (S.dataView === 'usage' ? ' is-active' : '') + '" data-data-tab="usage">' + U.icon('zap') + '用量</button>' +
        '<button type="button" class="data-tab' + (S.dataView === 'backup' ? ' is-active' : '') + '" data-data-tab="backup">' + U.icon('hard-drive') + '备份</button>' +
        '</div>' +
        '<div class="data-body" id="data-body"></div>' +
        '</div>',
      function (layer) {
        renderDataBody();
        layer.querySelector('#data-tabs').addEventListener('click', function (e) {
          const btn = e.target.closest('[data-data-tab]');
          if (!btn) return;
          S.dataView = btn.getAttribute('data-data-tab');
          layer.querySelectorAll('[data-data-tab]').forEach(function (b) {
            b.classList.toggle('is-active', b.getAttribute('data-data-tab') === S.dataView);
          });
          renderDataBody();
        });
      }
    );
  }

  function renderDataBody() {
    const body = U.q('#data-body');
    if (!body) return;
    // 切换标签时同步标题与图标
    const head = U.q('.data-head h2');
    if (head) {
      head.textContent =
        S.dataView === 'usage' ? '用量统计' : S.dataView === 'backup' ? '备份与恢复' : '审计记录';
    }
    const use = U.q('.data-head > svg use');
    if (use) {
      use.setAttribute(
        'href',
        '#i-' + (S.dataView === 'usage' ? 'zap' : S.dataView === 'backup' ? 'hard-drive' : 'shield-check')
      );
    }
    if (S.dataView === 'usage') {
      renderUsage(body);
    } else if (S.dataView === 'backup') {
      renderBackup(body);
    } else {
      renderAudit(body);
    }
  }

  function renderAudit(body) {
    const filter = S.auditFilter || 'all';
    const rows = D.auditLog.filter(function (a) {
      if (filter === 'all') return true;
      if (filter === 'deny') return a.decision === 'deny';
      if (filter === 'pending') return a.decision === 'wait';
      return a.risk === filter;
    });

    const decisionLabel = {
      allow: '允许',
      deny: '拒绝',
      auto: '自动',
      wait: '待决定',
    };
    const decisionClass = { allow: 'allow', deny: 'deny', auto: 'auto', wait: 'auto' };

    body.innerHTML =
      '<div class="audit-filters">' +
      ['all', 'R0', 'R1', 'R2', 'pending', 'deny']
        .map(function (f) {
          const label = f === 'all' ? '全部' : f === 'pending' ? '待决定' : f === 'deny' ? '被拒绝' : f;
          return '<button type="button" class="btn sm ' + (filter === f ? 'primary' : 'ghost') + '" data-audit-filter="' + f + '">' + label + '</button>';
        })
        .join('') +
      '<span class="spacer"></span>' +
      '<span style="font-size:9.5px;color:var(--text-4)">' + rows.length + ' 条记录 · 本地保存</span>' +
      '</div>' +
      '<div class="audit-table-wrap"><table class="audit-table"><thead><tr>' +
      '<th>时间</th><th>线程</th><th>工具</th><th>风险</th><th>决定</th><th>范围</th><th>对象</th>' +
      '</tr></thead><tbody>' +
      rows
        .map(function (a) {
          return (
            '<tr>' +
            '<td class="a-time">' + U.esc(a.time) + '</td>' +
            '<td>' + U.esc(a.thread) + '</td>' +
            '<td class="a-tool">' + U.esc(a.tool) + '</td>' +
            '<td><span class="risk-badge risk-' + a.risk + '">' + a.risk + '</span></td>' +
            '<td><span class="audit-decision ' + (decisionClass[a.decision] || 'auto') + '">' + (decisionLabel[a.decision] || a.decision) + '</span></td>' +
            '<td class="a-scope">' + U.esc(a.scope) + '</td>' +
            '<td style="font-family:var(--font-mono);font-size:9.5px;color:var(--text-3)">' + U.esc(a.detail) + '</td>' +
            '</tr>'
          );
        })
        .join('') +
      '</tbody></table></div>' +
      '<div class="settings-tip" style="margin-top:10px">' + U.icon('info') +
      '<span>审计记录包含请求、决定、范围、时间与操作者；可按线程与时间查询。原型中的数据为演示样例。</span></div>';

    body.querySelectorAll('[data-audit-filter]').forEach(function (b) {
      b.addEventListener('click', function () {
        S.auditFilter = b.getAttribute('data-audit-filter');
        renderAudit(body);
      });
    });
  }

  function renderUsage(body) {
    const u = D.usage;
    const today = u.today;
    const max = Math.max.apply(null, u.byDay.map(function (d) { return d.tokens; }));

    body.innerHTML =
      '<div class="usage-cards">' +
      '<div class="usage-card"><div class="usage-label">今日输入</div><div class="usage-value">' + U.fmtCompact(today.input) + '<small>tokens</small></div><div class="usage-sub">含缓存命中 ' + U.fmtCompact(today.cache) + '</div></div>' +
      '<div class="usage-card"><div class="usage-label">今日输出</div><div class="usage-value">' + U.fmtCompact(today.output) + '<small>tokens</small></div><div class="usage-sub">含推理内容</div></div>' +
      '<div class="usage-card"><div class="usage-label">今日估算成本</div><div class="usage-value">$' + today.cost.toFixed(2) + '</div><div class="usage-sub">按模型单价估算</div></div>' +
      '<div class="usage-card"><div class="usage-label">本月累计</div><div class="usage-value">$' + u.month.cost.toFixed(1) + '</div><div class="usage-sub">' + U.fmtCompact(u.month.input + u.month.output) + ' tokens</div></div>' +
      '</div>' +
      '<div class="settings-section" style="max-width:none"><h3>' + U.icon('cpu') + '按模型聚合</h3>' +
      '<div class="usage-bars">' +
      u.byModel
        .map(function (m) {
          return (
            '<div class="usage-bar-row">' +
            '<span class="ub-name">' + U.esc(m.name) + '</span>' +
            '<span class="ub-track"><span class="ub-fill" style="width:' + m.pct + '%"></span></span>' +
            '<span class="ub-num">' + U.fmtCompact(m.tokens) + ' · $' + m.cost.toFixed(2) + '</span>' +
            '</div>'
          );
        })
        .join('') +
      '</div></div>' +
      '<div class="settings-section" style="max-width:none"><h3>' + U.icon('calendar') + '最近 7 天</h3>' +
      '<div class="usage-bar-row" style="align-items:end;height:90px;grid-template-columns:minmax(0,1fr)">' +
      '<div style="display:flex;align-items:flex-end;gap:6px;height:100%;width:100%">' +
      u.byDay
        .map(function (d) {
          return (
            '<div style="flex:1;display:flex;flex-direction:column;justify-content:flex-end;align-items:center;gap:4px;height:100%">' +
            '<span style="font-size:9px;color:var(--text-4);font-family:var(--font-mono)">' + U.fmtCompact(d.tokens) + '</span>' +
            '<span style="width:100%;max-width:38px;height:' + Math.round((d.tokens / max) * 62) + 'px;border-radius:3px 3px 0 0;background:var(--mode-accent);opacity:.85"></span>' +
            '<span style="font-size:9px;color:var(--text-4)">' + d.day + '</span>' +
            '</div>'
          );
        })
        .join('') +
      '</div></div></div>' +
      '<div class="settings-tip" style="margin-top:12px">' + U.icon('info') +
      '<span>用量按线程、模型与日期聚合；数据保留 12 个月，不包含提示词正文。</span></div>';
  }

  function renderBackup(body) {
    body.innerHTML =
      '<div class="backup-list">' +
      D.backups
        .map(function (b, i) {
          return (
            '<div class="backup-item">' + U.icon(b.kind === '文档' ? 'file-text' : 'file-code') +
            '<div class="backup-main"><div class="backup-name">' + U.esc(b.name) + '</div>' +
            '<div class="backup-meta">' + b.kind + ' · ' + b.size + ' · ' + b.time + '</div></div>' +
            '<span class="spacer"></span>' +
            '<button type="button" class="btn sm ghost" data-restore-backup="' + i + '">恢复</button>' +
            '</div>'
          );
        })
        .join('') +
      '</div>' +
      '<div class="settings-section" style="max-width:none;margin-top:18px"><h3>' + U.icon('hard-drive') + '备份策略</h3>' +
      setRow('每日数据库备份', '每天首次启动备份，保留最近 7 份；启动时快速完整性检查。', '<span class="badge ok">开启</span>') +
      setRow('代码与文档写入备份', '每次写入前备份原文件，统一管理，可按时间恢复。', '<span class="badge ok">开启</span>') +
      setRow('导入导出', '导出不包含 keys.json；导入后需要重新输入密钥。', '<button type="button" class="btn sm ghost" data-export-data>导出</button>') +
      '</div>' +
      '<div class="settings-tip">' + U.icon('alert') +
      '<span>恢复操作会覆盖当前文件，执行前会再次确认，并记录审计。</span></div>';

    body.querySelectorAll('[data-restore-backup]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        const b = D.backups[Number(btn.getAttribute('data-restore-backup'))];
        CW.confirmDialog({
          title: '恢复这个备份？',
          desc: '当前文件会被备份内容覆盖。恢复后内容与备份一致，操作会写入审计。',
          detail: b.name,
          danger: true,
          confirmText: '恢复',
          onConfirm: function () {
            S.docxApplied = b.name.indexOf('.docx') >= 0 ? false : S.docxApplied;
            S.xlsxApplied = b.name.indexOf('.xlsx') >= 0 ? false : S.xlsxApplied;
            CW.ui.refresh();
            CW.toast({ kind: 'ok', title: '已恢复备份', desc: b.name });
          },
        });
      });
    });

    const exportBtn = body.querySelector('[data-export-data]');
    if (exportBtn) {
      exportBtn.addEventListener('click', function () {
        CW.toast({ kind: 'ok', title: '导出已开始（原型演示）', desc: '不包含 keys.json' });
      });
    }
  }

  /* ---------------- 键盘 ---------------- */
  function handlePaletteKeys(e) {
    if (!U.q('.command-palette')) return false;
    if (e.key === 'Escape') {
      closeOverlay();
      return true;
    }
    return false;
  }

  CW.openPalette = openPalette;
  CW.openSettings = openSettings;
  CW.openDataPanel = openDataPanel;
  CW.inputDialog = inputDialog;
  CW.closeOverlay = closeOverlay;
  CW.closeConfirm = closeConfirm;
  CW.handlePaletteKeys = handlePaletteKeys;
  CW.overlayIsOpen = function () {
    const layer = overlayLayer();
    return layer && layer.children.length > 0;
  };
  CW.confirmIsOpen = function () {
    const layer = U.q('#confirm-layer');
    return layer && layer.children.length > 0;
  };
})();
