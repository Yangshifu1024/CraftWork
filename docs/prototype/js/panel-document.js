/* ==========================================================================
   CraftWork 高保真原型 · 文档面板
   Word 段落与表格、Excel 网格与公式、PDF 只读页面、文本级差异。
   文档写入一律经过审批；应用后显示变更高亮与备份提示。
   ========================================================================== */
(function () {
  const CW = window.CW;
  const U = CW.util;
  const S = CW.State;
  const D = CW.Data;

  S.doc.cell = S.doc.cell || 'B8';
  S.doc.externalNotice = S.doc.externalNotice !== false;

  function currentDocPath() {
    const thread = CW.currentThread();
    if (S.doc.path && D.documents[S.doc.path]) return S.doc.path;
    if (thread && thread.panel && thread.panel.file && D.documents[thread.panel.file]) return thread.panel.file;
    return Object.keys(D.documents)[0];
  }

  function currentDoc() {
    return D.documents[currentDocPath()];
  }

  /* ---------------- 文档外壳 ---------------- */
  function render(host) {
    const path = currentDocPath();
    const doc = D.documents[path];
    if (!doc) {
      host.innerHTML =
        '<div class="panel-empty">' + U.icon('file-text') +
        '<b>没有打开的文档</b><span>在左侧文件树或聊天中的文件引用里选择文档</span></div>';
      return;
    }
    S.doc.path = path;

    const typeLabel = doc.kind === 'docx' ? 'Word 文档' : doc.kind === 'xlsx' ? 'Excel 工作簿' : 'PDF 文档';
    const applied =
      (doc.kind === 'docx' && S.docxApplied) || (doc.kind === 'xlsx' && S.xlsxApplied);

    host.innerHTML =
      '<div class="doc-wrap">' +
      '<div class="doc-toolbar">' +
      '<span class="doc-title">' + U.icon(doc.kind === 'xlsx' ? 'table' : 'file-text') +
      '<span>' + U.esc(doc.name) + '</span>' +
      '<span class="ext">' + typeLabel + ' · ' + doc.size + '</span>' +
      '</span>' +
      '<button type="button" class="tool-btn" data-doc-zoom="out" aria-label="缩小">' + U.icon('minus', 'sm') + '</button>' +
      '<span class="doc-zoom">' + S.doc.zoom + '%</span>' +
      '<button type="button" class="tool-btn" data-doc-zoom="in" aria-label="放大">' + U.icon('plus', 'sm') + '</button>' +
      (doc.kind === 'pdf'
        ? ''
        : '<button type="button" class="tool-btn" id="doc-backup" title="备份与恢复">' + U.icon('hard-drive') + '备份</button>') +
      '<span class="spacer"></span>' +
      '<span class="badge ' + (applied ? 'ok' : 'neutral') + '">' +
      (applied ? U.icon('check') + '已写入' : U.icon('lock') + '只读') + '</span>' +
      '</div>' +
      (applied
        ? '<div class="doc-badge-bar">' + U.icon('shield-check') +
          '<span>写入已通过审批 · 原文件已备份</span>' +
          '<span class="dot-sep">·</span>' +
          '<span>' + U.esc(doc.modified) + '</span>' +
          '<span class="spacer"></span>' +
          '<button type="button" class="btn sm ghost" data-doc-diff>查看文本级差异</button>' +
          '</div>'
        : '') +
      (doc.kind === 'docx' && S.doc.externalNotice && !applied
        ? '<div class="doc-badge-bar" style="background:var(--warn-soft);border-color:rgba(232,176,74,.24)">' +
          U.icon('alert') +
          '<span style="color:var(--warn)">检测到文档在外部被修改（今天 15:18）</span>' +
          '<span class="spacer"></span>' +
          '<button type="button" class="btn sm ghost" data-doc-reload>重新加载</button>' +
          '<button type="button" class="btn sm ghost" data-doc-keep>保留当前</button>' +
          '</div>'
        : '') +
      '<div class="doc-scroll" id="doc-scroll">' +
      (doc.kind === 'docx' ? renderDocx(doc) : doc.kind === 'xlsx' ? renderXlsx(doc) : renderPdf(doc)) +
      '</div>' +
      '</div>';

    bind(host);
  }

  /* ---------------- Word ---------------- */
  function docxTable(table, applied) {
    const header = table.cols
      .map(function (c) {
        return '<th>' + U.esc(c) + '</th>';
      })
      .join('');

    const rows = table.rows
      .map(function (row, ri) {
        return (
          '<tr>' +
          row
            .map(function (cell, ci) {
              const changed = applied && table.changedCells.some(function (c) {
                return c[0] === ri && c[1] === ci;
              });
              return '<td class="' + (changed ? 'changed' : '') + '">' + U.esc(cell) + '</td>';
            })
            .join('') +
          '</tr>'
        );
      })
      .join('');

    return '<table class="doc-table"><thead><tr>' + header + '</tr></thead><tbody>' + rows + '</tbody></table>';
  }

  function summaryTable() {
    return (
      '<table class="doc-table inserted-table">' +
      '<thead><tr><th>事项</th><th>负责人</th><th>截止日期</th></tr></thead><tbody>' +
      '<tr><td>文档协作内测排期</td><td>陈磊</td><td>11-08</td></tr>' +
      '<tr><td>移动端离线缓存方案</td><td>周敏</td><td>11-12</td></tr>' +
      '<tr><td>企业版报价体系</td><td>待定</td><td>11-15</td></tr>' +
      '<tr><td>数据看板改版评审</td><td>待定</td><td>11-20</td></tr>' +
      '<tr><td>客户反馈归档</td><td>林晓</td><td>11-22</td></tr>' +
      '</tbody></table>'
    );
  }

  function renderDocx(doc) {
    const applied = S.docxApplied;

    function blockHtml(block, index) {
      switch (block.t) {
        case 'h1':
          return '<h1>' + U.esc(block.text) + '</h1>';
        case 'sub':
          return '<div class="doc-subtitle">' + U.esc(block.text) + '</div>';
        case 'h2':
          return '<h2>' + U.esc(block.text) + '</h2>';
        case 'p':
          return '<p class="doc-para"><span class="para-handle">¶' + (index + 1) + '</span>' + U.esc(block.text) + '</p>';
        case 'table':
          return docxTable(
            Object.assign({}, block, {
              changedCells: applied ? [[0, 1]] : [],
            }),
            applied
          );
        case 'sign':
          return '<div class="doc-signature">' + U.esc(block.text) + '</div>';
        default:
          return '';
      }
    }

    const page = doc.pages[0];
    let html = '<div class="doc-page">';
    page.blocks.forEach(function (block, i) {
      html += blockHtml(block, i);
      // 汇总表插在原有待办表格之后，避免与原表顺序颠倒
      if (applied && block.id === 'todo-table') {
        html +=
          '<p class="doc-para inserted"><span class="para-handle">¶' + (i + 2) +
          '</span>待办事项汇总（新增）</p>' + summaryTable();
      }
    });
    html += '<span class="page-no">1 / 1</span></div>';
    return html;
  }

  /* ---------------- Excel ---------------- */
  function cellValue(sheet, cellRef, applied) {
    const match = cellRef.match(/^([A-Z]+)(\d+)$/);
    if (!match) return '';
    const col = sheet.cols.indexOf(match[1]);
    const row = Number(match[2]) - 1;
    const raw = (sheet.rows[row] || [])[col] || '';
    if (raw.indexOf('=') === 0) {
      return sheet.computed[cellRef] || '…';
    }
    return raw;
  }

  function displayFormula(sheet, cellRef) {
    const match = cellRef.match(/^([A-Z]+)(\d+)$/);
    if (!match) return '';
    const col = sheet.cols.indexOf(match[1]);
    const row = Number(match[2]) - 1;
    const raw = (sheet.rows[row] || [])[col] || '';
    return raw;
  }

  function renderXlsx(doc) {
    const sheet = doc.sheets[S.doc.sheet] || doc.sheets[0];
    const applied = S.xlsxApplied;
    const selected = S.doc.cell || sheet.selected || 'A1';

    // 固定列宽：行号列窄，科目列宽，数值列等宽，保证四列在面板内可见
    const colWidths = ['36%', '21%', '21%', '22%'];
    const head =
      '<colgroup><col style="width:30px">' +
      sheet.cols
        .map(function (c, i) {
          return '<col style="width:' + (colWidths[i] || 20) + '%">';
        })
        .join('') +
      '</colgroup>' +
      '<thead><tr><th class="row-head"></th>' +
      sheet.cols.map(function (c) { return '<th>' + c + '</th>'; }).join('') +
      '</tr></thead>';

    const body = '<tbody>' + sheet.rows
      .map(function (row, ri) {
        const rowNo = ri + 1;
        const isTitle = rowNo === 1 && row.slice(1).every(function (c) { return !c; });
        if (isTitle) {
          return (
            '<tr><td class="row-head">' + rowNo + '</td>' +
            '<td colspan="' + sheet.cols.length + '" class="sheet-title-cell">' + U.esc(row[0]) + '</td></tr>'
          );
        }
        return (
          '<tr>' +
          '<td class="row-head">' + rowNo + '</td>' +
          sheet.cols
            .map(function (col, ci) {
              const ref = col + rowNo;
              const raw = row[ci] == null ? '' : String(row[ci]);
              const display = raw.indexOf('=') === 0 ? sheet.computed[ref] || '…' : raw;
              const isChanged = applied && doc.changedCells.indexOf(ref) >= 0;
              const numeric = sheet.numericCols.indexOf(ci) >= 0;
              return (
                '<td class="' + (numeric ? 's-num ' : '') + (isChanged ? 'changed ' : '') +
                (ref === selected ? 'is-selected' : '') + '" data-sheet-cell="' + ref + '">' +
                U.esc(display) + '</td>'
              );
            })
            .join('') +
          '</tr>'
        );
      })
      .join('') + '</tbody>';

    const selectedFormula = displayFormula(sheet, selected);
    const selectedDisplay = selectedFormula.indexOf('=') === 0 ? selectedFormula : selectedFormula || '';

    return (
      '<div class="sheet-wrap">' +
      '<div class="sheet-formula-bar">' +
      '<span class="sheet-cell-ref">' + U.esc(selected) + '</span>' +
      '<span class="sheet-formula">' + U.esc(selectedDisplay) + '</span>' +
      '</div>' +
      '<div class="sheet-scroll" id="sheet-scroll">' +
      '<table class="sheet-grid">' + head + body + '</table>' +
      '</div>' +
      '<div class="sheet-tabs">' +
      doc.sheets
        .map(function (s, i) {
          return '<button type="button" class="sheet-tab' + (i === S.doc.sheet ? ' is-active' : '') + '" data-sheet-tab="' + i + '">' + U.esc(s.name) + '</button>';
        })
        .join('') +
      '<span style="flex:1"></span>' +
      '<span style="font-size:9.5px;color:var(--text-4)">' +
      (applied ? '已写入 2 个公式' : '公式可读取 · 不执行宏') +
      '</span>' +
      '</div>' +
      '</div>'
    );
  }

  /* ---------------- PDF ---------------- */
  function renderPdf(doc) {
    return doc.pagesData
      .map(function (page, i) {
        return (
          '<div class="pdf-page">' +
          '<h1>' + U.esc(page.title) + '</h1>' +
          '<div class="pdf-meta">' + U.esc(page.meta) + '</div>' +
          page.sections
            .map(function (sec) {
              return (
                (sec.h ? '<h3>' + U.esc(sec.h) + '</h3>' : '') +
                sec.ps.map(function (p) { return '<p>' + U.esc(p) + '</p>'; }).join('')
              );
            })
            .join('') +
          (i === doc.pagesData.length - 1
            ? '<div class="pdf-select-note">' + U.icon('info') + '第 12 页为扫描图片，不支持 OCR，仅能读取文本层。</div>'
            : '') +
          '<span class="page-no">' + (i + 1) + ' / ' + doc.pages + '</span>' +
          '</div>'
        );
      })
      .join('');
  }

  /* ---------------- 文档差异 ---------------- */
  function diffItemsFor(path) {
    if (path.indexOf('.docx') >= 0) {
      return [
        {
          kind: '表格单元格',
          loc: '表格 1 · 第 2 行 · 负责人',
          before: '陈雷',
          after: '陈磊',
          note: '与参会名单不一致的笔误',
        },
        {
          kind: '插入表格',
          loc: '“三、待办事项”后 · 第 18 段',
          before: '',
          after: '待办事项汇总（5 行 × 3 列）',
          note: '补齐正文中散落的待办，缺失负责人标记“待定”',
        },
      ];
    }
    if (path.indexOf('.xlsx') >= 0) {
      return [
        { kind: '公式', loc: '预算总表!B8', before: '=SUM(B3:B6) → 456,000', after: '=SUM(B3:B7) → 520,000', note: '纳入第 7 行“差旅与招待”' },
        { kind: '公式', loc: '预算总表!C8', before: '=SUM(C3:C6) → 443,900', after: '=SUM(C3:C7) → 505,200', note: '实际支出同步修正' },
        { kind: '公式结果', loc: '预算总表!D8', before: '-12,100', after: '-14,800', note: '差异列随合计自动重算' },
      ];
    }
    return [];
  }

  function renderDiff(host) {
    const path = currentDocPath();
    const doc = D.documents[path];
    const items = diffItemsFor(path);
    if (!doc || items.length === 0) {
      host.innerHTML =
        '<div class="panel-empty">' + U.icon('diff') +
        '<b>没有可显示的文本级差异</b><span>PDF 为只读，代码差异请切换到 Code 模式查看</span></div>';
      return;
    }

    const applied = path.indexOf('.docx') >= 0 ? S.docxApplied : S.xlsxApplied;

    host.innerHTML =
      '<div class="doc-diff">' +
      '<div class="doc-diff-summary">' +
      U.icon(doc.kind === 'xlsx' ? 'table' : 'file-text') +
      '<span>' + U.esc(doc.name) + '</span>' +
      '<span class="spacer"></span>' +
      '<span class="badge ' + (applied ? 'ok' : 'warn') + '">' + (applied ? '已应用' : '待审批') + '</span>' +
      '</div>' +
      '<div class="doc-diff-list">' +
      items
        .map(function (item) {
          return (
            '<div class="doc-diff-item">' +
            '<div class="doc-diff-item-head">' + U.icon('file-text') +
            '<span>' + U.esc(item.kind) + '</span>' +
            '<span class="spacer"></span>' +
            '<span class="loc">' + U.esc(item.loc) + '</span>' +
            '</div>' +
            '<div class="doc-diff-pairs">' +
            (item.before
              ? '<div class="dp-row before"><span class="dp-sign">−</span><span class="dp-text">' + U.esc(item.before) + '</span></div>'
              : '<div class="dp-row before"><span class="dp-sign">−</span><span class="dp-text" style="opacity:.6">（无）</span></div>') +
            '<div class="dp-row after"><span class="dp-sign">+</span><span class="dp-text">' + U.esc(item.after) + '</span></div>' +
            (item.note ? '<div style="font-size:9.5px;color:var(--text-4);margin-top:3px;padding-left:20px">' + U.esc(item.note) + '</div>' : '') +
            '</div>' +
            '</div>'
          );
        })
        .join('') +
      '</div>' +
      '<div class="doc-diff-foot">' +
      U.icon('hard-drive') +
      '<span>' + (applied ? '已备份 · 可恢复最近一次写入' : '写入前备份 · 文档写入每次审批') + '</span>' +
      '<span class="spacer"></span>' +
      (applied ? '<button type="button" class="btn sm ghost" data-doc-restore>' + U.icon('rotate-ccw') + '恢复上次写入</button>' : '') +
      '<button type="button" class="btn sm ghost" data-doc-open>' + U.icon('eye') + '查看文档</button>' +
      '</div>' +
      '</div>';

    bindDiff(host);
  }

  /* ---------------- 交互 ---------------- */
  function bind(host) {
    host.querySelectorAll('[data-doc-zoom]').forEach(function (b) {
      b.addEventListener('click', function () {
        const dir = b.getAttribute('data-doc-zoom');
        S.doc.zoom = U.clamp(S.doc.zoom + (dir === 'in' ? 10 : -10), 70, 150);
        const scroll = U.q('#doc-scroll');
        const top = scroll ? scroll.scrollTop : 0;
        render(host);
        const next = U.q('#doc-scroll');
        if (next) next.scrollTop = top;
      });
    });

    const backup = U.q('#doc-backup');
    if (backup) {
      backup.addEventListener('click', function () {
        CW.openDataPanel('backup');
      });
    }

    host.querySelectorAll('[data-sheet-tab]').forEach(function (b) {
      b.addEventListener('click', function () {
        S.doc.sheet = Number(b.getAttribute('data-sheet-tab'));
        render(host);
      });
    });

    host.querySelectorAll('[data-sheet-cell]').forEach(function (td) {
      td.addEventListener('click', function () {
        S.doc.cell = td.getAttribute('data-sheet-cell');
        render(host);
      });
    });

    host.querySelectorAll('[data-doc-diff]').forEach(function (b) {
      b.addEventListener('click', function () {
        S.panelTab.work = 'diff';
        CW.ui.refresh();
      });
    });

    host.querySelectorAll('[data-doc-open]').forEach(function (b) {
      b.addEventListener('click', function () {
        S.panelTab.work = 'document';
        CW.ui.refresh();
      });
    });

    const reload = host.querySelector('[data-doc-reload]');
    if (reload) {
      reload.addEventListener('click', function () {
        S.doc.externalNotice = false;
        render(host);
        CW.toast({ kind: 'ok', title: '已重新加载', desc: '以磁盘上的最新版本为准' });
      });
    }
    const keep = host.querySelector('[data-doc-keep]');
    if (keep) {
      keep.addEventListener('click', function () {
        S.doc.externalNotice = false;
        render(host);
        CW.toast({ kind: 'warn', title: '已保留当前内容', desc: '保存时会再次检测外部修改，不静默覆盖' });
      });
    }

    const restore = host.querySelector('[data-doc-restore]');
    if (restore) {
      restore.addEventListener('click', function () {
        CW.confirmDialog({
          title: '恢复上一次写入？',
          desc: '会用备份覆盖当前文档。恢复操作不可撤销，请确认备份时间。',
          detail: '备份时间：今天 15:02 · 38 KB',
          danger: true,
          confirmText: '恢复备份',
          onConfirm: function () {
            S.docxApplied = false;
            S.xlsxApplied = false;
            CW.toast({ kind: 'ok', title: '已恢复备份', desc: '文档内容与备份一致' });
            CW.ui.refresh();
          },
        });
      });
    }
  }

  function bindDiff(host) {
    const open = host.querySelector('[data-doc-open]');
    if (open) {
      open.addEventListener('click', function () {
        S.panelTab.work = 'document';
        CW.ui.refresh();
      });
    }
    const restore = host.querySelector('[data-doc-restore]');
    if (restore) {
      restore.addEventListener('click', function () {
        CW.confirmDialog({
          title: '恢复上一次写入？',
          desc: '会用备份覆盖当前文档。恢复操作不可撤销，请确认备份时间。',
          detail: '备份时间：今天 15:02 · 38 KB',
          danger: true,
          confirmText: '恢复备份',
          onConfirm: function () {
            S.docxApplied = false;
            S.xlsxApplied = false;
            CW.toast({ kind: 'ok', title: '已恢复备份', desc: '文档内容与备份一致' });
            S.panelTab.work = 'document';
            CW.ui.refresh();
          },
        });
      });
    }
  }

  function open(path) {
    if (!D.documents[path]) return;
    S.doc.path = path;
    S.panelTab.work = 'document';
    CW.ui.refresh();
  }

  function scrollToLine() {
    const scroll = U.q('#doc-scroll');
    if (scroll) scroll.scrollTop = 0;
  }

  CW.Doc = {
    render: render,
    renderDiff: renderDiff,
    open: open,
    scrollToLine: scrollToLine,
    currentDocPath: currentDocPath,
  };
})();
