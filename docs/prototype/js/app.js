/* ==========================================================================
   CraftWork 高保真原型 · 启动与全局交互
   负责：线程列表、工作区卡片、顶栏、状态栏、模式切换、面板拖拽、快捷键。
   ========================================================================== */
(function () {
  const CW = window.CW;
  const U = CW.util;
  const S = CW.State;
  const D = CW.Data;

  /* ---------------- 线程列表 ---------------- */
  function highlight(text, query) {
    if (!query) return U.esc(text);
    const idx = text.toLowerCase().indexOf(query.toLowerCase());
    if (idx < 0) return U.esc(text);
    return (
      U.esc(text.slice(0, idx)) +
      '<mark>' + U.esc(text.slice(idx, idx + query.length)) + '</mark>' +
      U.esc(text.slice(idx + query.length))
    );
  }

  function threadMatches(thread, query) {
    if (!query) return true;
    const q = query.toLowerCase();
    if (thread.title.toLowerCase().indexOf(q) >= 0) return true;
    return (thread.messages || []).some(function (m) {
      return (m.blocks || []).some(function (b) {
        return (b.text || b.summary || '').toLowerCase().indexOf(q) >= 0;
      });
    });
  }

  function contentHit(thread, query) {
    if (!query) return false;
    const q = query.toLowerCase();
    if (thread.title.toLowerCase().indexOf(q) >= 0) return false;
    return (thread.messages || []).some(function (m) {
      return (m.blocks || []).some(function (b) {
        return (b.text || b.summary || '').toLowerCase().indexOf(q) >= 0;
      });
    });
  }

  function renderThreads() {
    const host = U.q('#thread-list');
    if (!host) return;
    const mode = S.mode;
    const query = S.threadSearch || '';
    const list = (S.threads[mode] || []).filter(function (t) {
      return threadMatches(t, query);
    });
    const activeId = S.active[mode];

    U.q('#thread-section-label').textContent = mode === 'work' ? 'Work 线程' : 'Code 线程';
    U.q('#thread-count').textContent = String((S.threads[mode] || []).length);

    if (list.length === 0) {
      host.innerHTML =
        '<div class="thread-empty">' +
        (query ? '没有匹配的线程<br><span style="font-size:10px;color:var(--text-4)">试试更短的关键词</span>' : '还没有线程<br>点击右上角 + 新建一个') +
        '</div>';
      return;
    }

    host.innerHTML = list
      .map(function (t) {
        const status =
          t.status === 'running'
            ? '<span class="thread-status running"><span class="st-dot"></span>运行中</span>'
            : t.status === 'awaiting'
            ? '<span class="thread-status awaiting"><span class="st-dot"></span>等待审批</span>'
            : t.status === 'error'
            ? '<span class="thread-status error"><span class="st-dot"></span>错误</span>'
            : '<span class="thread-status idle">已完成</span>';
        const hit = contentHit(t, query);
        return (
          '<button type="button" class="thread-item' + (t.id === activeId ? ' is-active' : '') + '" data-thread-id="' + t.id + '">' +
          '<span class="thread-title">' + highlight(t.title, query) + '</span>' +
          '<span class="thread-time">' + U.esc(t.updated) + '</span>' +
          '<span class="thread-sub">' +
          '<span class="model-name">' + U.esc(t.modelName || t.model) + '</span>' +
          '<span class="dot-sep">·</span>' +
          (hit
            ? '<span style="color:var(--warn)">内容命中</span>'
            : '<span>' + (t.tokens ? U.fmtCompact(t.tokens) : '0') + ' tokens</span>') +
          '</span>' +
          status +
          '</button>'
        );
      })
      .join('');
  }

  /* ---------------- 工作区卡片 ---------------- */
  function renderWorkspaceCard() {
    const host = U.q('#ws-card');
    if (!host) return;
    const ws = S.workspace;
    const trustLabel =
      ws.trust === 'trusted' ? '信任' : ws.trust === 'restricted' ? '受限模式' : '未选择';
    const trustIcon = ws.trust === 'trusted' ? 'shield-check' : ws.trust === 'restricted' ? 'lock' : 'alert';

    host.innerHTML =
      '<div class="ws-card-top">' +
      '<span class="ws-avatar">' + U.icon(S.mode === 'work' ? 'briefcase' : 'folder-open') + '</span>' +
      '<div class="ws-meta">' +
      '<div class="ws-title">' + U.esc(ws.name) +
      '<span class="badge ' + (S.mode === 'work' ? 'work' : 'accent') + '" style="height:15px;font-size:9px">' +
      (S.mode === 'work' ? 'Work' : 'Code') + '</span>' +
      '</div>' +
      '<div class="ws-path" title="' + U.esc(ws.path) + '">' + U.esc(ws.path) + '</div>' +
      '</div>' +
      '</div>' +
      '<div class="ws-card-bottom">' +
      '<button type="button" class="trust-chip ' + ws.trust + '" id="trust-chip" title="点击调整工作区信任">' +
      U.icon(trustIcon) + trustLabel + '</button>' +
      (S.mode === 'code'
        ? '<span class="ws-mini-btn">' + U.icon('git-branch') + 'main' +
          (ws.ahead ? ' ↑' + ws.ahead : '') + '</span>'
        : '<span class="ws-mini-btn">' + U.icon('file-text') + '4 个文档</span>') +
      '<span class="spacer"></span>' +
      '<button type="button" class="ws-mini-btn" id="ws-switch" title="切换工作区">' + U.icon('chevron-down') + '</button>' +
      '</div>';

    const chip = U.q('#trust-chip');
    if (chip) {
      chip.addEventListener('click', function () {
        CW.popover(
          chip,
          '<div class="popover-label">工作区信任</div>' +
            '<button type="button" class="popover-item" data-trust="trusted">' + U.icon('shield-check') +
            '<span class="pi-main"><div class="pi-title">信任</div><div class="pi-desc">R1 仍逐次审批，R3 / R4 每次确认</div></span>' +
            (ws.trust === 'trusted' ? '<span class="pi-trail pi-check">' + U.icon('check') + '</span>' : '') +
            '</button>' +
            '<button type="button" class="popover-item" data-trust="restricted">' + U.icon('lock') +
            '<span class="pi-main"><div class="pi-title">受限模式</div><div class="pi-desc">写入与执行必须审批，网络默认拒绝</div></span>' +
            (ws.trust === 'restricted' ? '<span class="pi-trail pi-check">' + U.icon('check') + '</span>' : '') +
            '</button>' +
            '<div class="popover-note">信任状态在重启后保留；可随时撤销。</div>',
          {
            align: 'left',
            onMount: function (el) {
              el.querySelectorAll('[data-trust]').forEach(function (b) {
                b.addEventListener('click', function () {
                  const value = b.getAttribute('data-trust');
                  if (value === 'restricted') {
                    CW.confirmDialog({
                      title: '切换为受限模式？',
                      desc: '所有写入与执行都需要审批，网络访问默认拒绝。已发出的审计记录保留。',
                      detail: ws.path,
                      icon: 'lock',
                      confirmText: '切换为受限',
                      onConfirm: function () {
                        ws.trust = 'restricted';
                        CW.closePopover();
                        CW.ui.refresh();
                        CW.toast({ kind: 'warn', title: '已切换为受限模式' });
                      },
                    });
                    return;
                  }
                  ws.trust = value;
                  CW.closePopover();
                  CW.ui.refresh();
                  CW.toast({ kind: 'ok', title: '已设为信任工作区', desc: '审批策略仍然生效' });
                });
              });
            },
          }
        );
      });
    }

    const switcher = U.q('#ws-switch');
    if (switcher) {
      switcher.addEventListener('click', function () {
        CW.popover(
          switcher,
          '<div class="popover-label">最近工作区</div>' +
            D.recentWorkspaces
              .map(function (w) {
                return (
                  '<button type="button" class="popover-item" data-ws="' + w.id + '">' + U.icon('folder') +
                  '<span class="pi-main"><div class="pi-title">' + U.esc(w.name) + '</div>' +
                  '<div class="pi-desc">' + U.esc(w.kind) + ' · ' + U.esc(w.path) + '</div></span>' +
                  (w.id === S.workspace.id ? '<span class="pi-trail pi-check">' + U.icon('check') + '</span>' : '<span class="pi-trail">' + U.icon('external-link') + '</span>') +
                  '</button>'
                );
              })
              .join('') +
            '<div class="popover-sep"></div>' +
            '<button type="button" class="popover-item" data-ws-add>' + U.icon('plus') +
            '<span class="pi-main"><div class="pi-title">添加工作区…</div><div class="pi-desc">使用系统原生文件夹对话框</div></span></button>',
          {
            align: 'left',
            onMount: function (el) {
              el.querySelectorAll('[data-ws]').forEach(function (b) {
                b.addEventListener('click', function () {
                  const id = b.getAttribute('data-ws');
                  const w = D.recentWorkspaces.find(function (x) { return x.id === id; });
                  if (!w) return;
                  S.workspace = Object.assign({}, S.workspace, {
                    id: w.id,
                    name: w.name,
                    path: w.path,
                  });
                  CW.closePopover();
                  CW.ui.refresh();
                  CW.toast({ kind: 'ok', title: '已切换工作区', desc: w.name + ' · 线程仍然按模式隔离' });
                });
              });
              const add = el.querySelector('[data-ws-add]');
              if (add) {
                add.addEventListener('click', function () {
                  CW.closePopover();
                  CW.toast({
                    kind: 'info',
                    title: '打开系统文件夹对话框（原型演示）',
                    desc: '真实产品使用系统原生对话框；路径必须存在且可读。',
                  });
                });
              }
            },
          }
        );
      });
    }
  }

  /* ---------------- 顶栏 ---------------- */
  // 顶栏只保留品牌与模式切换；工作区、模型、命令面板、设置与运行状态都不再出现在右上角。
  // 运行状态仍可在聊天头部、线程列表与状态栏看到。

  /* ---------------- 状态栏 ---------------- */
  function renderStatusbar() {
    const host = U.q('#statusbar');
    if (!host) return;
    const thread = CW.currentThread();
    const model = CW.currentModel();
    const waiting = CW.Chat.allApprovals().filter(function (a) {
      return a.block.status === 'waiting' && thread && a.thread.id === thread.id;
    }).length;
    const used = thread ? thread.tokens : 0;
    const pct = Math.min(100, Math.round((used / 1000 / model.context) * 100));
    const barClass = pct > 85 ? 'danger' : pct > 70 ? 'warn' : '';

    const left =
      (S.mode === 'code'
        ? '<span class="status-item">' + U.icon('git-branch') + 'main ↑' + S.workspace.ahead + '</span>' +
          '<span class="status-sep"></span>' +
          '<span class="status-item">' + U.icon('folder') + U.esc(S.workspace.name) + '</span>'
        : '<span class="status-item">' + U.icon('folder') + U.esc(S.workspace.name) + '</span>' +
          '<span class="status-item">' + U.icon('mail') + '邮件 / 日历只读</span>') +
      '<button type="button" class="status-item" data-status="usage">' + U.icon('zap') +
      '<span>上下文</span><span class="mini-bar"><i class="' + barClass + '" style="width:' + Math.max(2, pct) + '%"></i></span>' +
      '<span class="mono">' + U.fmtCompact(used) + ' / ' + model.context + 'k</span></button>' +
      (waiting
        ? '<button type="button" class="status-item warn" data-status="approvals">' + U.icon('clock') +
          '<span>待审批 ' + waiting + '</span></button>'
        : '<span class="status-item ok">' + U.icon('check-circle') + '无待审批</span>') +
      (S.mode === 'code'
        ? '<span class="status-item">' + U.icon('terminal') + '后台 1</span>'
        : '<span class="status-item">' + U.icon('file-text') + '草稿 2</span>') +
      (thread && thread.status === 'error'
        ? '<button type="button" class="status-item danger" data-status="error">' + U.icon('alert') + '错误 1</button>'
        : '');

    const approvalLabel = CW.Chat && CW.Chat.approvalLabel ? CW.Chat.approvalLabel() : '分级';
    // 工具范围从输入框移到底部状态栏：Code 显示内置工具数量，Work 显示文档 / 文件 / MCP
    const toolScope =
      S.mode === 'work'
        ? '<span class="status-item" title="Work 模式不提供 Shell、Git 与终端">' +
          U.icon('file-text') + '文档 · 文件 · MCP</span>'
        : '<span class="status-item" title="Code 模式启用 Shell、Git、终端与差异面板">' +
          U.icon('terminal') + '17 个内置工具 · Shell 已启用</span>';
    const right =
      toolScope +
      '<span class="status-sep"></span>' +
      '<button type="button" class="status-item" data-status="approval">' + U.icon('shield') +
      '审批 ' + U.esc(approvalLabel) + '</button>' +
      '<span class="status-item">' + U.icon('lock') + '沙箱 ' + U.esc(S.settings.shellMode) + '</span>' +
      '<span class="status-item ok">' + U.icon('shield-check') + '审计已开启</span>' +
      '<span class="status-item">' + U.icon('lock') + '本地 · 无遥测</span>';

    host.innerHTML = left + '<span class="status-spacer"></span>' + right;
  }

  /* ---------------- 右侧面板：自动折叠、浮层与全屏 ---------------- */
  const PANEL_NARROW_WIDTH = 1180;

  function updatePanelLayout() {
    const panel = U.q('#panel');
    if (!panel) return;
    const narrow = window.innerWidth < PANEL_NARROW_WIDTH;
    S.panelNarrow = narrow;

    panel.hidden = !S.panelOpen;
    panel.classList.toggle('is-maximized', !!S.panelOpen && !!S.panelMaximized);
    panel.classList.toggle('is-overlay', !!S.panelOpen && narrow && !S.panelMaximized);

    // 面板右上角的全屏按钮：图标与提示随状态变化
    const maxBtn = U.q('#panel-maximize');
    if (maxBtn) {
      const use = maxBtn.querySelector('use');
      if (use) use.setAttribute('href', S.panelMaximized ? '#i-minimize' : '#i-maximize');
      const label = S.panelMaximized ? '退出全屏面板' : '全屏面板';
      maxBtn.title = label;
      maxBtn.setAttribute('aria-label', label);
    }

    // 聊天头部的面板开关
    const toggle = U.q('#btn-panel-toggle');
    if (toggle) {
      toggle.classList.toggle('is-on', !S.panelOpen);
      const label = S.panelOpen ? '收起右侧面板' : '展开右侧面板';
      toggle.title = label;
      toggle.setAttribute('aria-label', label);
    }
  }

  function togglePanel() {
    S.panelOpen = !S.panelOpen;
    if (!S.panelOpen) S.panelMaximized = false;
    updatePanelLayout();
  }

  function togglePanelMaximize() {
    if (!S.panelOpen) S.panelOpen = true;
    S.panelMaximized = !S.panelMaximized;
    updatePanelLayout();
    CW.toast({
      kind: 'info',
      title: S.panelMaximized ? '面板已全屏' : '已退出全屏',
      desc: S.panelMaximized ? '面板覆盖中间区域，点击右上角按钮恢复分栏' : '恢复三栏布局',
    });
  }

  CW.togglePanel = togglePanel;
  CW.togglePanelMaximize = togglePanelMaximize;
  CW.updatePanelLayout = updatePanelLayout;

  /* ---------------- 渲染总入口 ---------------- */
  function renderAll() {
    renderThreads();
    renderWorkspaceCard();
    CW.Chat.render();
    CW.Panel.render();
    renderStatusbar();
    updatePanelLayout();
    document.documentElement.classList.toggle('is-work', S.mode === 'work');
  }

  CW.ui.refresh = renderAll;
  CW.renderThreads = renderThreads;

  /* ---------------- 线程切换 ---------------- */
  function selectThread(id) {
    const thread = CW.findThread(id);
    if (!thread) return;
    S.mode = thread.mode;
    S.active[thread.mode] = id;
    CW.applyMode(thread.mode);

    if (thread.panel) {
      S.panelTab[thread.mode] = thread.panel.tab || (thread.mode === 'work' ? 'document' : 'files');
      if (thread.panel.file) {
        if (D.documents[thread.panel.file]) {
          S.doc.path = thread.panel.file;
          S.openFile = null;
        } else if (D.files[thread.panel.file]) {
          S.openFile = {
            path: thread.panel.file,
            name: thread.panel.file.split('/').pop(),
            content: D.files[thread.panel.file],
            line: null,
          };
        }
      } else if (S.panelTab[thread.mode] === 'files') {
        S.openFile = null;
      }
    } else {
      S.panelTab[thread.mode] = thread.mode === 'work' ? 'document' : 'files';
      S.openFile = null;
    }

    renderAll();
    CW.Chat.scrollToBottom();
    const input = U.q('#composer-input');
    if (input) input.focus();
  }

  /* ---------------- 模式切换 ---------------- */
  function switchMode(mode) {
    if (mode === S.mode) return;
    CW.applyMode(mode);
    const list = S.threads[mode];
    if (list && list.length) {
      const activeId = S.active[mode];
      const thread = list.find(function (t) { return t.id === activeId; }) || list[0];
      S.active[mode] = thread.id;
      selectThreadForMode(thread);
    } else {
      S.openFile = null;
      renderAll();
    }
    CW.toast({
      kind: 'info',
      title: '已切换到 ' + (mode === 'work' ? 'Work' : 'Code') + ' 模式',
      desc:
        mode === 'work'
          ? '隐藏 Git、代码差异、终端与 Shell；保留文件树、文档预览与文本级差异。'
          : '可用 Shell、Git、差异面板与终端；工作区保持不变。',
    });
  }

  function selectThreadForMode(thread) {
    if (thread.panel) {
      S.panelTab[thread.mode] = thread.panel.tab || (thread.mode === 'work' ? 'document' : 'files');
      if (thread.panel.file) {
        if (D.documents[thread.panel.file]) {
          S.doc.path = thread.panel.file;
          S.openFile = null;
        } else if (D.files[thread.panel.file]) {
          S.openFile = {
            path: thread.panel.file,
            name: thread.panel.file.split('/').pop(),
            content: D.files[thread.panel.file],
            line: null,
          };
        }
      }
    }
    renderAll();
    CW.Chat.scrollToBottom();
  }

  CW.switchMode = switchMode;

  /* ---------------- 面板拖拽 ---------------- */
  function bindResize() {
    const handle = U.q('#panel-resize');
    if (!handle) return;
    handle.addEventListener('mousedown', function (e) {
      e.preventDefault();
      const startX = e.clientX;
      const startWidth = S.panelWidth;
      handle.classList.add('is-dragging');
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';

      function move(ev) {
        const max = Math.min(760, window.innerWidth * 0.56);
        S.panelWidth = U.clamp(startWidth + (startX - ev.clientX), 320, max);
        document.documentElement.style.setProperty('--panel-w', S.panelWidth + 'px');
      }
      function up() {
        handle.classList.remove('is-dragging');
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
        document.removeEventListener('mousemove', move);
        document.removeEventListener('mouseup', up);
        CW.savePrefs();
      }
      document.addEventListener('mousemove', move);
      document.addEventListener('mouseup', up);
    });
  }

  /* ---------------- 全局事件 ---------------- */
  function bindGlobal() {
    // 线程列表
    const list = U.q('#thread-list');
    if (list) {
      list.addEventListener('click', function (e) {
        const item = e.target.closest('[data-thread-id]');
        if (item) selectThread(item.getAttribute('data-thread-id'));
      });
    }

    // 模式切换
    const modeSwitch = U.q('#mode-switch');
    if (modeSwitch) {
      modeSwitch.addEventListener('click', function (e) {
        const btn = e.target.closest('[data-mode]');
        if (btn) switchMode(btn.getAttribute('data-mode'));
      });
    }

    // 搜索线程
    const searchBtn = U.q('#btn-thread-search');
    if (searchBtn) {
      searchBtn.addEventListener('click', function () {
        const open = U.q('#thread-search').hidden;
        toggleThreadSearch(open);
      });
    }
    const searchClose = U.q('#btn-thread-search-close');
    if (searchClose) {
      searchClose.addEventListener('click', function () {
        toggleThreadSearch(false);
      });
    }
    const searchInput = U.q('#thread-search-input');
    if (searchInput) {
      searchInput.addEventListener('input', function () {
        S.threadSearch = searchInput.value.trim();
        renderThreads();
      });
    }

    // 新建线程
    const newThread = U.q('#btn-new-thread');
    if (newThread) newThread.addEventListener('click', function () { CW.Scene.newThread(); });

    // 侧栏底部
    const usage = U.q('#btn-usage');
    if (usage) usage.addEventListener('click', function () { CW.openDataPanel('usage'); });
    const audit = U.q('#btn-audit');
    if (audit) audit.addEventListener('click', function () { CW.openDataPanel('audit'); });

    // 顶栏
    const palette = U.q('#btn-palette');
    if (palette) palette.addEventListener('click', CW.openPalette);
    const settings = U.q('#btn-settings');
    if (settings) settings.addEventListener('click', function () { CW.openSettings('general'); });

    // 状态栏
    const statusbar = U.q('#statusbar');
    if (statusbar) {
      statusbar.addEventListener('click', function (e) {
        const item = e.target.closest('[data-status]');
        if (!item) return;
        const type = item.getAttribute('data-status');
        if (type === 'usage') CW.openDataPanel('usage');
        else if (type === 'approvals') CW.openDataPanel('audit');
        else if (type === 'approval') CW.openSettings('tools');
        else if (type === 'error') {
          CW.toast({ kind: 'err', title: '当前线程存在错误', desc: '错误卡片中可重试或切换模型' });
        }
      });
    }

    // 收藏夹/侧栏底部数值
    U.q('#foot-usage').textContent = U.fmtCompact(D.usage.today.input + D.usage.today.output);
    U.q('#foot-audit').textContent = String(D.auditLog.length);

    // 文件树右键
    const panelBody = U.q('#panel-body');
    if (panelBody) {
      panelBody.addEventListener('contextmenu', function (e) {
        const row = e.target.closest('[data-tree-path]');
        if (!row) return;
        e.preventDefault();
        const path = row.getAttribute('data-tree-path');
        const isDir = row.getAttribute('data-tree-type') === 'dir';
        CW.popover(
          row,
          '<div class="popover-label">' + U.esc(isDir ? '目录' : '文件') + '</div>' +
            (isDir
              ? '<button type="button" class="popover-item" data-ctx="expand">' + U.icon('chevron-right') +
                '<span class="pi-main"><div class="pi-title">展开 / 折叠</div></span></button>'
              : '<button type="button" class="popover-item" data-ctx="open">' + U.icon('file-code') +
                '<span class="pi-main"><div class="pi-title">打开</div></span></button>') +
            '<button type="button" class="popover-item" data-ctx="copy">' + U.icon('copy') +
            '<span class="pi-main"><div class="pi-title">复制相对路径</div></span></button>' +
            '<button type="button" class="popover-item" data-ctx="reveal">' + U.icon('external-link') +
            '<span class="pi-main"><div class="pi-title">在文件管理器中显示</div></span></button>' +
            '<div class="popover-sep"></div>' +
            '<button type="button" class="popover-item danger" data-ctx="trash">' + U.icon('trash') +
            '<span class="pi-main"><div class="pi-title">移到废纸篓</div><div class="pi-desc">需要确认，记录审计</div></span></button>',
          {
            align: 'left',
            onMount: function (el) {
              el.querySelectorAll('[data-ctx]').forEach(function (b) {
                b.addEventListener('click', function () {
                  const act = b.getAttribute('data-ctx');
                  CW.closePopover();
                  if (act === 'copy') {
                    U.copyText(path).then(function (ok) {
                      CW.toast(ok ? { kind: 'ok', title: '已复制相对路径', desc: path } : { kind: 'err', title: '复制失败' });
                    });
                  } else if (act === 'open') {
                    CW.openFile(path);
                  } else if (act === 'reveal') {
                    CW.toast({ kind: 'info', title: '已在文件管理器中显示（原型演示）', desc: path });
                  } else if (act === 'trash') {
                    CW.confirmDialog({
                      title: '移到废纸篓？',
                      desc: '文件会被移入系统废纸篓，而不是直接删除。此操作属于 R1，会写入审计。',
                      detail: path,
                      danger: true,
                      confirmText: '移到废纸篓',
                      onConfirm: function () {
                        CW.toast({ kind: 'ok', title: '已移到废纸篓（原型演示）' });
                      },
                    });
                  }
                });
              });
            },
          }
        );
      });
    }

    // 全局快捷键
    document.addEventListener('keydown', function (e) {
      const mod = e.metaKey || e.ctrlKey;

      if (e.key === 'Escape') {
        if (CW.confirmIsOpen()) {
          CW.closeConfirm();
          e.preventDefault();
          return;
        }
        if (CW.overlayIsOpen()) {
          CW.closeOverlay();
          e.preventDefault();
          return;
        }
        // 全屏面板优先退出，避免误触停止
        if (S.panelMaximized) {
          S.panelMaximized = false;
          updatePanelLayout();
          e.preventDefault();
          return;
        }
        const thread = CW.currentThread();
        if (thread && thread.status === 'running') {
          U.q('#btn-stop').click();
          e.preventDefault();
          return;
        }
        if (thread && thread.status === 'awaiting') {
          const cards = CW.Chat.allApprovals().filter(function (a) {
            return a.thread.id === thread.id && a.block.status === 'waiting';
          });
          if (cards.length) {
            CW.Chat.decideApproval(cards[cards.length - 1].thread, cards[cards.length - 1].block, 'deny');
            e.preventDefault();
          }
        }
        return;
      }

      if (mod && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        if (CW.overlayIsOpen()) CW.closeOverlay();
        CW.openPalette();
        return;
      }
      if (mod && e.key.toLowerCase() === 'n') {
        e.preventDefault();
        CW.Scene.newThread();
        return;
      }
      if (mod && e.key === ',') {
        e.preventDefault();
        CW.openSettings('general');
        return;
      }
      if (mod && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        toggleThreadSearch(true);
        return;
      }
    });

    // 窗口尺寸变化时限制面板宽度
    window.addEventListener('resize', function () {
      const max = Math.min(760, window.innerWidth * 0.56);
      if (S.panelWidth > max) {
        S.panelWidth = max;
        document.documentElement.style.setProperty('--panel-w', S.panelWidth + 'px');
      }
      // 跨过窄窗口阈值时重置面板状态：窄窗口自动折叠，宽窗口恢复分栏
      const narrow = window.innerWidth < PANEL_NARROW_WIDTH;
      if (narrow !== S.panelNarrow) {
        S.panelNarrow = narrow;
        S.panelOpen = !narrow;
        S.panelMaximized = false;
        updatePanelLayout();
      }
    });
  }

  function toggleThreadSearch(open) {
    const box = U.q('#thread-search');
    if (!box) return;
    box.hidden = !open;
    S.searchOpen = open;
    if (open) {
      const input = U.q('#thread-search-input');
      input.focus();
      input.select();
    } else {
      S.threadSearch = '';
      const input = U.q('#thread-search-input');
      if (input) input.value = '';
      renderThreads();
    }
  }

  CW.toggleThreadSearch = toggleThreadSearch;

  /* ---------------- 初始化 ---------------- */
  function init() {
    document.documentElement.style.setProperty('--panel-w', S.panelWidth + 'px');
    // 窄窗口默认折叠右侧面板，保证聊天区可读；宽窗口恢复三栏
    S.panelNarrow = window.innerWidth < PANEL_NARROW_WIDTH;
    S.panelOpen = !S.panelNarrow;
    S.panelMaximized = false;
    CW.applyTheme(S.theme);
    CW.applyMode(S.mode);

    // 恢复当前模式的线程面板
    const thread = CW.currentThread();
    if (thread && thread.panel) {
      S.panelTab[thread.mode] = thread.panel.tab || (thread.mode === 'work' ? 'document' : 'files');
      if (thread.panel.file) {
        if (D.documents[thread.panel.file]) S.doc.path = thread.panel.file;
        else if (D.files[thread.panel.file]) {
          S.openFile = {
            path: thread.panel.file,
            name: thread.panel.file.split('/').pop(),
            content: D.files[thread.panel.file],
            line: null,
          };
        }
      }
    }

    CW.Chat.bindEvents();
    CW.Chat.bindScroll();
    CW.bindPanelEvents();
    bindResize();
    bindGlobal();
    renderAll();

    // 首次进入提示
    setTimeout(function () {
      const thread = CW.currentThread();
      const awaiting = CW.Chat.allApprovals().filter(function (a) {
        return a.block.status === 'waiting' && thread && a.thread.id === thread.id;
      }).length;
      CW.toast({
        kind: 'info',
        title: '这是可交互的高保真原型',
        desc: awaiting
          ? '当前线程有 ' + awaiting + ' 个待审批操作。批准或拒绝后，Agent 会继续执行并生成差异。'
          : '试试 ⌘K 命令面板、Code / Work 模式切换，或在输入框发送一条消息。',
        duration: 6500,
      });
    }, 900);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
