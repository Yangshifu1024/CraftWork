/* ==========================================================================
   CraftWork 高保真原型 · 右侧面板（主控）
   负责：面板标签、文件树、文件列表、代码编辑器。差异 / 终端 / 文档由各自模块
   提供渲染函数，本文件只做调度。
   ========================================================================== */
(function () {
  const CW = window.CW;
  const U = CW.util;
  const S = CW.State;
  const D = CW.Data;

  S.treeOpen = S.treeOpen || {
    internal: true,
    'internal/auth': true,
  };
  S.treeFilter = S.treeFilter || '';
  S.editorMode = 'read';
  S.editorSearch = '';

  /* ---------------- 面板标签 ---------------- */
  function tabsForMode(mode) {
    if (mode === 'work') {
      return [
        { id: 'document', label: '文档', icon: 'file-text' },
        { id: 'diff', label: '差异', icon: 'diff' },
        { id: 'files', label: '文件', icon: 'folder' },
      ];
    }
    return [
      { id: 'files', label: '文件', icon: 'folder' },
      { id: 'diff', label: '差异', icon: 'diff' },
      { id: 'terminal', label: '终端', icon: 'terminal' },
    ];
  }

  function pendingDiffCount() {
    if (S.mode === 'work') {
      return S.docxApplied || S.xlsxApplied ? 0 : 1;
    }
    return D.codeDiff.filter(function (f) {
      return f.state !== 'applied';
    }).length;
  }

  function renderTabs() {
    const host = U.q('#panel-tabs');
    if (!host) return;
    const mode = S.mode;
    const active = S.panelTab[mode];
    const tabs = tabsForMode(mode);
    const diffCount = pendingDiffCount();

    host.innerHTML =
      tabs
        .map(function (t) {
          return (
            '<button type="button" role="tab" class="panel-tab' + (t.id === active ? ' is-active' : '') +
            '" data-panel-tab="' + t.id + '" aria-selected="' + (t.id === active) + '">' +
            U.icon(t.icon) + t.label +
            (t.id === 'diff' && diffCount ? '<span class="tab-badge">' + diffCount + '</span>' : '') +
            (t.id === 'terminal' && S.terminals.length > 1 ? '<span class="tab-badge" style="background:var(--bg-active);color:var(--text-3)">' + S.terminals.length + '</span>' : '') +
            '</button>'
          );
        })
        .join('') +
      '<span class="panel-tab-spacer"></span>' +
      '<span class="panel-head-actions">' +
      '<button type="button" class="icon-btn sm" id="panel-maximize" title="全屏面板" aria-label="全屏面板">' + U.icon('maximize') + '</button>' +
      '<button type="button" class="icon-btn sm" id="panel-refresh" title="刷新面板" aria-label="刷新面板">' + U.icon('refresh') + '</button>' +
      '</span>';
  }

  function render() {
    renderTabs();
    const body = U.q('#panel-body');
    if (!body) return;
    const mode = S.mode;
    const tab = S.panelTab[mode];

    if (mode === 'code') {
      if (tab === 'files') renderFilesTab(body);
      else if (tab === 'diff') CW.Diff.render(body);
      else CW.Term.render(body);
    } else {
      if (tab === 'document') CW.Doc.render(body);
      else if (tab === 'diff') CW.Doc.renderDiff(body);
      else renderFilesTab(body);
    }
  }

  function setTab(tab) {
    S.panelTab[S.mode] = tab;
    render();
  }

  /* ---------------- 文件树 ---------------- */
  function fileIcon(name, node) {
    if (node && node.type === 'dir') return { ico: 'folder', cls: 'dir' };
    if (/\.go$/.test(name)) return { ico: 'file-code', cls: 'go' };
    if (/\.md$/.test(name)) return { ico: 'file-text', cls: 'md' };
    if (/\.docx$/.test(name)) return { ico: 'file-text', cls: 'doc' };
    if (/\.xlsx$/.test(name)) return { ico: 'table', cls: 'sheet' };
    if (/\.pdf$/.test(name)) return { ico: 'file-text', cls: 'pdf' };
    return { ico: 'file', cls: '' };
  }

  function nodePath(parentPath, name) {
    return parentPath ? parentPath + '/' + name : name;
  }

  function collectFiles(node, basePath, out) {
    (node.children || []).forEach(function (child) {
      const p = nodePath(basePath, child.name);
      if (child.type === 'dir') collectFiles(child, p, out);
      else out.push({ path: p, name: child.name, node: child });
    });
    return out;
  }

  function renderTreeRows(node, basePath, depth, filter) {
    let html = '';
    (node.children || []).forEach(function (child) {
      const p = nodePath(basePath, child.name);
      const isDir = child.type === 'dir';
      const open = !!S.treeOpen[p];
      const fi = fileIcon(child.name, child);
      const selected = !isDir && S.openFile && S.openFile.path === p;
      const pending = child.pending;
      const git = child.git;

      const matchesSelf = !filter || child.name.toLowerCase().indexOf(filter) >= 0;
      const hasMatchingDescendant =
        isDir &&
        collectFiles(child, p, []).some(function (f) {
          return f.name.toLowerCase().indexOf(filter) >= 0;
        });

      if (filter && !matchesSelf && !hasMatchingDescendant) return;

      html +=
        '<div class="tree-node' + (open ? ' is-open' : '') + '">' +
        '<div class="tree-row ' + (isDir ? 'is-dir' : '') + (open ? ' is-open' : '') +
        (selected ? ' is-selected' : '') + '"' +
        ' style="--depth:' + depth + '"' +
        ' data-tree-path="' + U.esc(p) + '"' +
        ' data-tree-type="' + (isDir ? 'dir' : 'file') + '">' +
        (isDir ? U.icon('chevron-right', 'twist') : '<span class="twist placeholder"></span>') +
        U.icon(fi.ico, 'f-ico ' + fi.cls) +
        '<span class="tree-name">' + U.esc(child.name) + '</span>' +
        (pending ? '<span class="badge accent" style="height:14px;font-size:9px">新增</span>' : '') +
        (S.mode === 'code'
          ? '<span class="tree-git ' + (git || '') + '">' + U.esc(git || '') + '</span>'
          : '') +
        '</div>';

      if (isDir) {
        html += '<div class="tree-children">' + renderTreeRows(child, p, depth + 1, filter) + '</div>';
      }
      html += '</div>';
    });
    return html;
  }

  function renderFilesTab(body) {
    if (S.openFile && !S.openFile.doc) {
      renderEditor(body);
      return;
    }

    const filter = S.treeFilter.toLowerCase();
    body.innerHTML =
      '<div class="tree-toolbar">' +
      '<div class="tree-filter">' + U.icon('search') +
      '<input id="tree-filter-input" type="text" placeholder="过滤文件…" value="' + U.esc(S.treeFilter) + '">' +
      '</div>' +
      '<button type="button" class="icon-btn sm" id="tree-collapse" title="全部折叠" aria-label="全部折叠">' + U.icon('chevron-up') + '</button>' +
      '</div>' +
      '<div class="tree-meta">' +
      '<span>craftwork</span><span>·</span><span>1,284 个文件</span>' +
      (S.mode === 'code' ? '<span>·</span><span>4 处变更</span>' : '') +
      '<span class="spacer"></span>' +
      '<span>' + (S.mode === 'code' ? '已加载 Git 状态' : 'Work 模式不显示 Git') + '</span>' +
      '</div>' +
      '<div class="panel-scroll"><div class="tree" id="file-tree">' +
      renderTreeRows(D.fileTree, '', 0, filter) +
      '</div></div>';

    const input = U.q('#tree-filter-input');
    if (input) {
      input.addEventListener('input', function () {
        S.treeFilter = input.value;
        const scrollHost = U.q('#panel-body .panel-scroll');
        const top = scrollHost ? scrollHost.scrollTop : 0;
        render();
        const nextInput = U.q('#tree-filter-input');
        if (nextInput) {
          nextInput.focus();
          nextInput.setSelectionRange(nextInput.value.length, nextInput.value.length);
        }
        const nextScroll = U.q('#panel-body .panel-scroll');
        if (nextScroll) nextScroll.scrollTop = top;
      });
    }

    const collapse = U.q('#tree-collapse');
    if (collapse) {
      collapse.addEventListener('click', function () {
        S.treeOpen = {};
        render();
        CW.toast({ kind: 'info', title: '已全部折叠' });
      });
    }
  }

  function handleTreeClick(row) {
    const path = row.getAttribute('data-tree-path');
    const type = row.getAttribute('data-tree-type');
    if (type === 'dir') {
      S.treeOpen[path] = !S.treeOpen[path];
      render();
      return;
    }
    openFile(path);
  }

  /* ---------------- 打开文件 ---------------- */
  function openFile(path, line) {
    const doc = D.documents[path];
    if (doc) {
      if (S.mode === 'work') {
        S.doc.path = path;
        S.doc.sheet = 0;
        S.panelTab.work = 'document';
        render();
        if (CW.Doc && CW.Doc.scrollToLine) CW.Doc.scrollToLine(line);
      } else {
        CW.confirmDialog({
          title: '该文件需要 Work 模式',
          desc: 'Office 文档与 PDF 的预览、文本级编辑在 Work 模式下提供。切换到 Work 模式后同一工作区仍然可用。',
          detail: path,
          icon: 'briefcase',
          confirmText: '切换到 Work 模式',
          onConfirm: function () {
            CW.switchMode('work');
            S.doc.path = path;
            S.panelTab.work = 'document';
            CW.ui.refresh();
          },
        });
      }
      return;
    }

    const content = D.files[path];
    if (content == null) {
      CW.toast({ kind: 'err', title: '无法打开文件', desc: path + ' 不存在或不可读（原型未内置该文件）。' });
      return;
    }

    S.openFile = {
      path: path,
      name: path.split('/').pop(),
      content: content,
      line: line || null,
    };
    S.openFileLine = line || null;
    S.editorMode = 'read';
    S.editorSearch = '';
    S.panelTab[S.mode] = 'files';
    render();
    if (line) {
      requestAnimationFrame(function () {
        const target = U.q('.editor-line.hl');
        if (target) target.scrollIntoView({ block: 'center' });
      });
    }
  }

  function closeFile() {
    S.openFile = null;
    S.openFileLine = null;
    render();
  }

  /* ---------------- 编辑器 ---------------- */
  function langFor(path) {
    if (/\.go$/.test(path)) return 'go';
    if (/\.(js|ts)$/.test(path)) return 'js';
    if (/\.json$/.test(path)) return 'json';
    if (/\.md$/.test(path)) return 'md';
    if (/\.(sh|bash|zsh)$/.test(path)) return 'bash';
    return 'text';
  }

  function editorContent() {
    if (!S.openFile) return '';
    if (S.openFile.path === 'internal/auth/session.go' && S.codeApplied) {
      return D.sessionGoAfter;
    }
    return S.openFile.content;
  }

  function renderEditor(body) {
    const file = S.openFile;
    const content = editorContent();
    const lines = content.split('\n');
    const lang = langFor(file.path);
    const parts = file.path.split('/');
    const isEditing = S.editorMode === 'edit';

    const breadcrumb = parts
      .map(function (p, i) {
        return '<span class="crumb' + (i === parts.length - 1 ? ' last' : '') + '">' + U.esc(p) + '</span>' +
          (i < parts.length - 1 ? '<span class="sep">/</span>' : '');
      })
      .join('');

    const matchLines = [];
    if (S.editorSearch) {
      const needle = S.editorSearch.toLowerCase();
      lines.forEach(function (l, i) {
        if (l.toLowerCase().indexOf(needle) >= 0) matchLines.push(i);
      });
    }

    const gutter = lines
      .map(function (_, i) {
        const isHl = file.line === i + 1;
        const isMatch = matchLines.indexOf(i) >= 0;
        return '<span class="gl' + (isHl ? ' hl' : '') + (isMatch ? ' diff-add' : '') + '">' + (i + 1) + '</span>';
      })
      .join('');

    const codeLines = lines
      .map(function (line, i) {
        const isHl = file.line === i + 1;
        return (
          '<span class="editor-line' + (isHl ? ' hl' : '') + '" data-line="' + (i + 1) + '">' +
          (CW.md.highlight(line, lang) || '&nbsp;') +
          '</span>'
        );
      })
      .join('');

    body.innerHTML =
      '<div class="editor">' +
      '<div class="editor-head">' +
      '<div class="editor-breadcrumb">' + breadcrumb + '</div>' +
      '<div class="editor-badges">' +
      '<span class="badge neutral">' + U.esc(lang.toUpperCase()) + '</span>' +
      (isEditing
        ? '<span class="badge warn">编辑中</span>'
        : '<span class="badge neutral">' + U.icon('lock') + '只读</span>') +
      '</div>' +
      '</div>' +
      '<div class="editor-toolbar">' +
      '<button type="button" class="tool-btn" id="editor-back">' + U.icon('chevron-left') + '文件树</button>' +
      '<button type="button" class="tool-btn' + (S.editorSearch ? ' is-on' : '') + '" id="editor-search-btn">' + U.icon('search') + '搜索</button>' +
      '<span class="spacer"></span>' +
      (isEditing
        ? '<button type="button" class="tool-btn" id="editor-cancel">' + U.icon('x') + '取消</button>' +
          '<button type="button" class="tool-btn is-on" id="editor-save">' + U.icon('save') + '保存</button>'
        : '<button type="button" class="tool-btn" id="editor-edit">' + U.icon('edit') + '编辑</button>') +
      '<button type="button" class="tool-btn" id="editor-copy">' + U.icon('copy') + '复制</button>' +
      '</div>' +
      (S.editorSearch
        ? '<div class="editor-search">' + U.icon('search') +
          '<input id="editor-search-input" placeholder="在文件中查找…" value="' + U.esc(S.editorSearch) + '">' +
          '<span class="hit-count">' + matchLines.length + ' 处</span>' +
          '<button type="button" class="icon-btn sm" id="editor-search-close" aria-label="关闭搜索">' + U.icon('x') + '</button>' +
          '</div>'
        : '') +
      '<div class="editor-scroll" id="editor-scroll">' +
      (isEditing
        ? '<textarea class="editor-textarea" id="editor-textarea" spellcheck="false">' + U.esc(content) + '</textarea>'
        : '<div class="editor-code">' +
          '<div class="editor-gutter">' + gutter + '</div>' +
          '<div class="editor-lines">' + codeLines + '</div>' +
          '</div>') +
      '</div>' +
      '<div class="editor-note">' + U.icon('info') +
      '<span>' + (isEditing ? '原型中的编辑不会写入磁盘；真实产品保存前会备份原文件并校验外部修改。' : '默认只读；切换到编辑后保存需要确认。') + '</span>' +
      '<span class="spacer"></span>' +
      '<span>' + lines.length + ' 行 · UTF-8 · LF</span>' +
      '</div>' +
      '</div>';

    bindEditorEvents();
  }

  function bindEditorEvents() {
    const back = U.q('#editor-back');
    if (back) back.addEventListener('click', closeFile);

    const edit = U.q('#editor-edit');
    if (edit) {
      edit.addEventListener('click', function () {
        S.editorMode = 'edit';
        render();
        const ta = U.q('#editor-textarea');
        if (ta) ta.focus();
      });
    }

    const cancel = U.q('#editor-cancel');
    if (cancel) {
      cancel.addEventListener('click', function () {
        S.editorMode = 'read';
        render();
      });
    }

    const save = U.q('#editor-save');
    if (save) {
      save.addEventListener('click', function () {
        CW.confirmDialog({
          title: '保存文件修改？',
          desc: '保存前会先备份原文件，并记录一次 R1 写工作区审计。原型中不会真正写入磁盘。',
          detail: S.openFile.path,
          icon: 'save',
          confirmText: '保存并备份',
          onConfirm: function () {
            S.editorMode = 'read';
            S.openFile.line = null;
            render();
            CW.toast({ kind: 'ok', title: '已保存（原型演示）', desc: '已生成备份，可在设置 · 高级中恢复' });
          },
        });
      });
    }

    const copy = U.q('#editor-copy');
    if (copy) {
      copy.addEventListener('click', function () {
        U.copyText(editorContent()).then(function (ok) {
          CW.toast(ok ? { kind: 'ok', title: '已复制文件内容' } : { kind: 'err', title: '复制失败' });
        });
      });
    }

    const searchBtn = U.q('#editor-search-btn');
    if (searchBtn) {
      searchBtn.addEventListener('click', function () {
        S.editorSearch = S.editorSearch ? '' : 'Refresh';
        render();
        const input = U.q('#editor-search-input');
        if (input) {
          input.focus();
          input.select();
        }
      });
    }

    const searchInput = U.q('#editor-search-input');
    if (searchInput) {
      searchInput.addEventListener('input', function () {
        S.editorSearch = searchInput.value;
        render();
        const next = U.q('#editor-search-input');
        if (next) {
          next.focus();
          next.setSelectionRange(next.value.length, next.value.length);
        }
      });
    }

    const searchClose = U.q('#editor-search-close');
    if (searchClose) {
      searchClose.addEventListener('click', function () {
        S.editorSearch = '';
        render();
      });
    }
  }

  /* ---------------- 绑定 ---------------- */
  function bindPanelEvents() {
    const tabs = U.q('#panel-tabs');
    if (tabs) {
      tabs.addEventListener('click', function (e) {
        const btn = e.target.closest('[data-panel-tab]');
        if (btn) {
          setTab(btn.getAttribute('data-panel-tab'));
          return;
        }
        const maxBtn = e.target.closest('#panel-maximize');
        if (maxBtn) {
          CW.togglePanelMaximize();
          return;
        }
        const refresh = e.target.closest('#panel-refresh');
        if (refresh) {
          render();
          CW.toast({ kind: 'info', title: '面板已刷新' });
        }
      });
    }

    const body = U.q('#panel-body');
    if (body) {
      body.addEventListener('click', function (e) {
        const treeRow = e.target.closest('[data-tree-path]');
        if (treeRow) {
          handleTreeClick(treeRow);
          return;
        }
        const fileItem = e.target.closest('[data-file-open]');
        if (fileItem) {
          openFile(fileItem.getAttribute('data-file-open'));
        }
      });
    }
  }

  /* ---------------- 对外接口 ---------------- */
  CW.Panel = {
    render: render,
    renderTabs: renderTabs,
    setTab: setTab,
    tabsForMode: tabsForMode,
    pendingDiffCount: pendingDiffCount,
  };
  CW.openFile = openFile;
  CW.bindPanelEvents = bindPanelEvents;
})();
