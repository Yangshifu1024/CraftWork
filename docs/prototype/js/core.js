/* ==========================================================================
   CraftWork 高保真原型 · 核心状态与通用能力
   包括：应用状态、工具函数、事件总线、通知、下拉菜单、确认对话框、偏好存储。
   ========================================================================== */
(function () {
  const CW = window.CW;
  const D = CW.Data;

  /* ---------------- 偏好读取 ---------------- */
  const PREF_KEY = 'craftwork.prototype.prefs.v1';

  function loadPrefs() {
    try {
      const raw = localStorage.getItem(PREF_KEY);
      return raw ? JSON.parse(raw) : {};
    } catch (e) {
      return {};
    }
  }

  function savePrefs() {
    try {
      localStorage.setItem(
        PREF_KEY,
        JSON.stringify({
          theme: CW.State.theme,
          mode: CW.State.mode,
          panelWidth: CW.State.panelWidth,
          settings: CW.State.settings,
        })
      );
    } catch (e) {
      /* 本地存储不可用时静默忽略 */
    }
  }

  const prefs = loadPrefs();

  /* ---------------- 应用状态 ---------------- */
  const State = {
    mode: prefs.mode === 'work' ? 'work' : 'code',
    theme: prefs.theme === 'light' ? 'light' : 'dark',
    workspace: D.workspace,
    recentWorkspaces: D.recentWorkspaces,
    models: D.models,
    threads: D.threads,
    active: {
      code: 'c1',
      work: 'w1',
    },
    panelTab: {
      code: 'diff',
      work: 'document',
    },
    panelWidth: prefs.panelWidth || 320,
    openFile: null,
    openFileLine: null,
    diff: { view: 'unified', fileIndex: 0 },
    doc: { path: null, sheet: 0, zoom: 100 },
    terminals: JSON.parse(JSON.stringify(D.terminals)),
    terminalActive: 0,
    threadSearch: '',
    searchOpen: false,
    attachments: [],
    busyThreads: {},
    abortFlags: {},
    approvalResolvers: {},
    approvalsAwaiting: 0,
    codeApplied: false,
    docxApplied: false,
    xlsxApplied: false,
    docDiffDismissed: false,
    settings: Object.assign(
      {
        language: 'zh-CN',
        startup: 'restore',
        notify: true,
        autoUpdate: true,
        updateChannel: 'stable',
        defaultMode: 'recommend',
        codeTheme: 'CraftWork Dark',
        fontSize: 13,
        terminalFontSize: 11,
        shellMode: 'confined',
        approvalMode: 'graded',
        networkPolicy: 'ask',
        telemetry: false,
        dataDir: '~/.craftwork',
      },
      prefs.settings || {}
    ),
    settingsTab: 'general',
    dataView: 'audit',
    usage: D.usage,
    scrollFollow: {},
  };

  /* ---------------- DOM 工具 ---------------- */
  function q(sel, root) {
    return (root || document).querySelector(sel);
  }
  function qa(sel, root) {
    return Array.prototype.slice.call((root || document).querySelectorAll(sel));
  }
  function icon(name, cls) {
    return '<svg class="ico' + (cls ? ' ' + cls : '') + '" aria-hidden="true"><use href="#i-' + name + '"/></svg>';
  }
  function esc(text) {
    return String(text == null ? '' : text)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }
  function uid(prefix) {
    return (prefix || 'id') + '-' + Math.random().toString(36).slice(2, 9);
  }
  function clamp(n, min, max) {
    return Math.max(min, Math.min(max, n));
  }
  function sleep(ms) {
    return new Promise(function (resolve) {
      setTimeout(resolve, ms);
    });
  }
  function fmtNum(n) {
    return new Intl.NumberFormat('zh-CN').format(n);
  }
  function fmtCompact(n) {
    if (n >= 1000000) return (n / 1000000).toFixed(1).replace(/\.0$/, '') + 'M';
    if (n >= 1000) return (n / 1000).toFixed(1).replace(/\.0$/, '') + 'k';
    return String(n);
  }
  function timeNow() {
    const d = new Date();
    return String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
  }
  async function copyText(text) {
    try {
      if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(text);
        return true;
      }
    } catch (e) {
      /* 继续走兜底方案 */
    }
    try {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.setAttribute('readonly', '');
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand('copy');
      ta.remove();
      return ok;
    } catch (e) {
      return false;
    }
  }

  /* ---------------- 当前线程 ---------------- */
  function currentThread() {
    const list = State.threads[State.mode] || [];
    return (
      list.find(function (t) {
        return t.id === State.active[State.mode];
      }) || list[0]
    );
  }

  function findThread(id) {
    const all = State.threads.code.concat(State.threads.work);
    return all.find(function (t) {
      return t.id === id;
    });
  }

  function setThreadStatus(thread, status) {
    if (!thread) return;
    thread.status = status;
  }

  function currentModel() {
    const thread = currentThread();
    const id = thread ? thread.model : 'claude-sonnet-4-5';
    return (
      State.models.find(function (m) {
        return m.id === id;
      }) || State.models[0]
    );
  }

  /* ---------------- 通知 ---------------- */
  function toast(opts) {
    const host = q('#toast-layer');
    if (!host) return;
    const kind = opts.kind || 'info';
    const iconName =
      kind === 'ok' ? 'check-circle' : kind === 'warn' ? 'alert' : kind === 'err' ? 'x-circle' : 'info';
    const el = document.createElement('div');
    el.className = 'toast ' + kind;
    el.innerHTML =
      '<span class="toast-ico">' + icon(iconName) + '</span>' +
      '<div class="toast-main">' +
      '<div class="toast-title">' + esc(opts.title || '') + '</div>' +
      (opts.desc ? '<div class="toast-desc">' + esc(opts.desc) + '</div>' : '') +
      '</div>' +
      '<button type="button" class="toast-close" aria-label="关闭通知">' + icon('x') + '</button>';
    host.appendChild(el);

    const close = function () {
      if (!el.isConnected) return;
      el.classList.add('is-out');
      setTimeout(function () {
        el.remove();
      }, 190);
    };
    el.querySelector('.toast-close').addEventListener('click', close);
    setTimeout(close, opts.duration || 4200);
  }

  /* ---------------- 下拉菜单 ---------------- */
  let activePopover = null;

  function closePopover() {
    const layer = q('#popover-layer');
    if (layer) layer.innerHTML = '';
    if (activePopover && activePopover.onClose) activePopover.onClose();
    activePopover = null;
  }

  function popover(anchor, html, opts) {
    opts = opts || {};
    closePopover();
    const layer = q('#popover-layer');
    const rect = anchor.getBoundingClientRect();
    const el = document.createElement('div');
    el.className = 'popover';
    el.innerHTML = html;
    layer.appendChild(el);

    // 定位：默认右对齐，超出视口时自动修正
    const width = el.offsetWidth;
    let left = opts.align === 'left' ? rect.left : rect.right - width;
    left = clamp(left, 8, window.innerWidth - width - 8);
    let top = rect.bottom + 6;
    const height = el.offsetHeight;
    if (top + height > window.innerHeight - 8) {
      top = Math.max(8, rect.top - height - 6);
    }
    el.style.left = left + 'px';
    el.style.top = top + 'px';

    activePopover = { el: el, anchor: anchor, onClose: opts.onClose };

    function onDocClick(e) {
      if (!el.contains(e.target) && !anchor.contains(e.target)) {
        closePopover();
        document.removeEventListener('mousedown', onDocClick, true);
      }
    }
    setTimeout(function () {
      document.addEventListener('mousedown', onDocClick, true);
    }, 0);

    if (opts.onMount) opts.onMount(el);
    return el;
  }

  /* ---------------- 确认对话框 ---------------- */
  // 使用独立的浮层，避免与设置、命令面板等 overlay 互相清空
  function confirmModalLayer() {
    let layer = document.getElementById('confirm-layer');
    if (!layer) {
      layer = document.createElement('div');
      layer.id = 'confirm-layer';
      layer.className = 'overlay-layer';
      layer.style.zIndex = '360';
      document.body.appendChild(layer);
    }
    return layer;
  }

  function confirmDialog(opts) {
    const layer = confirmModalLayer();
    layer.innerHTML =
      '<div class="overlay-backdrop" data-confirm-cancel></div>' +
      '<div class="confirm-dialog" role="dialog" aria-modal="true">' +
      '<h3>' + icon(opts.icon || 'alert') + esc(opts.title) + '</h3>' +
      '<p>' + opts.desc + '</p>' +
      (opts.detail ? '<div class="confirm-detail">' + esc(opts.detail) + '</div>' : '') +
      '<div class="confirm-actions">' +
      '<span class="spacer"></span>' +
      '<button type="button" class="btn" data-confirm-cancel>取消</button>' +
      '<button type="button" class="btn ' + (opts.danger ? 'danger' : 'primary') + '" data-confirm-ok>' +
      esc(opts.confirmText || '确认') +
      '</button>' +
      '</div>' +
      '</div>';

    function close() {
      layer.innerHTML = '';
    }
    layer.querySelectorAll('[data-confirm-cancel]').forEach(function (b) {
      b.addEventListener('click', function () {
        close();
        if (opts.onCancel) opts.onCancel();
      });
    });
    layer.querySelector('[data-confirm-ok]').addEventListener('click', function () {
      close();
      if (opts.onConfirm) opts.onConfirm();
    });
    const okBtn = layer.querySelector('[data-confirm-ok]');
    if (okBtn) okBtn.focus();
  }

  /* ---------------- 事件总线 ---------------- */
  const listeners = {};
  function on(name, fn) {
    (listeners[name] = listeners[name] || []).push(fn);
  }
  function emit(name, payload) {
    (listeners[name] || []).forEach(function (fn) {
      try {
        fn(payload);
      } catch (e) {
        console.error('[CraftWork] 事件处理失败', name, e);
      }
    });
  }

  /* ---------------- 主题与模式 ---------------- */
  function applyTheme(theme) {
    State.theme = theme === 'light' ? 'light' : 'dark';
    document.documentElement.setAttribute('data-theme', State.theme);
    savePrefs();
    emit('theme', State.theme);
  }

  function applyMode(mode) {
    State.mode = mode === 'work' ? 'work' : 'code';
    document.documentElement.setAttribute('data-mode', State.mode);
    savePrefs();
    emit('mode', State.mode);
  }

  /* ---------------- 输入框自适应高度 ---------------- */
  function autoGrow(textarea) {
    textarea.style.height = 'auto';
    textarea.style.height = Math.min(textarea.scrollHeight, 220) + 'px';
  }

  /* ---------------- 导出 ---------------- */
  CW.State = State;
  CW.util = {
    q: q,
    qa: qa,
    icon: icon,
    esc: esc,
    uid: uid,
    clamp: clamp,
    sleep: sleep,
    fmtNum: fmtNum,
    fmtCompact: fmtCompact,
    timeNow: timeNow,
    copyText: copyText,
    autoGrow: autoGrow,
  };
  CW.currentThread = currentThread;
  CW.findThread = findThread;
  CW.setThreadStatus = setThreadStatus;
  CW.currentModel = currentModel;
  CW.toast = toast;
  CW.popover = popover;
  CW.closePopover = closePopover;
  CW.confirmDialog = confirmDialog;
  CW.bus = { on: on, emit: emit };
  CW.applyTheme = applyTheme;
  CW.applyMode = applyMode;
  CW.savePrefs = savePrefs;
  CW.ui = { refresh: function () {} };
})();
