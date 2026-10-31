/* ==========================================================================
   CraftWork 高保真原型 · 代码差异面板
   统一视图 / 并排视图、文件列表、统计、逐块应用或跳过、全部应用。
   ========================================================================== */
(function () {
  const CW = window.CW;
  const U = CW.util;
  const S = CW.State;
  const D = CW.Data;

  function totalStats() {
    let add = 0;
    let del = 0;
    D.codeDiff.forEach(function (f) {
      add += f.added;
      del += f.removed;
    });
    return { add: add, del: del };
  }

  function activeFile() {
    return D.codeDiff[S.diff.fileIndex] || D.codeDiff[0];
  }

  function renderHunkTools(file, hunk) {
    if (hunk.applied) {
      return '<span style="color:var(--ok);display:inline-flex;align-items:center;gap:3px">' + U.icon('check-circle') + '已应用</span>';
    }
    if (hunk.skipped) {
      return '<span style="color:var(--text-4);display:inline-flex;align-items:center;gap:3px">' + U.icon('x-circle') + '已跳过</span>' +
        '<button type="button" data-hunk-apply="' + hunk.id + '">重新应用</button>';
    }
    return (
      '<button type="button" class="primary" data-hunk-apply="' + hunk.id + '">' + U.icon('check') + '应用此块</button>' +
      '<button type="button" data-hunk-skip="' + hunk.id + '">' + U.icon('x') + '跳过</button>'
    );
  }

  function renderUnified(file) {
    return file.hunks
      .map(function (hunk) {
        return (
          '<div class="diff-hunk' + (hunk.applied ? ' applied' : '') + '">' +
          '<div class="diff-hunk-head">' +
          '<span class="hunk-title">' + U.esc(hunk.header) + '</span>' +
          '<span class="spacer"></span>' +
          renderHunkTools(file, hunk) +
          '</div>' +
          '<div class="diff-table">' +
          hunk.lines
            .map(function (line) {
              const sign = line.t === 'add' ? '+' : line.t === 'del' ? '−' : ' ';
              return (
                '<div class="dline ' + line.t + '">' +
                '<span class="dn">' + (line.old == null ? '' : line.old) + '</span>' +
                '<span class="dn">' + (line.nw == null ? '' : line.nw) + '</span>' +
                '<span class="ds">' + sign + '</span>' +
                '<span class="dt">' + (CW.md.highlight(line.text, 'go') || '&nbsp;') + '</span>' +
                '</div>'
              );
            })
            .join('') +
          '</div>' +
          '</div>'
        );
      })
      .join('');
  }

  function pairLines(lines) {
    const rows = [];
    let dels = [];
    let adds = [];
    function flush() {
      const max = Math.max(dels.length, adds.length);
      for (let i = 0; i < max; i++) {
        rows.push({ left: dels[i] || null, right: adds[i] || null });
      }
      dels = [];
      adds = [];
    }
    lines.forEach(function (l) {
      if (l.t === 'del') dels.push(l);
      else if (l.t === 'add') adds.push(l);
      else {
        flush();
        rows.push({ left: l, right: l });
      }
    });
    flush();
    return rows;
  }

  function renderSide(file) {
    return file.hunks
      .map(function (hunk) {
        const rows = pairLines(hunk.lines);
        const left = rows
          .map(function (r) {
            if (!r.left) return '<div class="sline empty"><span class="dn"></span><span class="dt"></span></div>';
            const cls = r.left.t === 'del' ? 'del' : r.left.t === 'add' ? 'add' : '';
            return (
              '<div class="sline ' + cls + '">' +
              '<span class="dn">' + (r.left.old == null ? '' : r.left.old) + '</span>' +
              '<span class="dt">' + (CW.md.highlight(r.left.text, 'go') || '&nbsp;') + '</span>' +
              '</div>'
            );
          })
          .join('');
        const right = rows
          .map(function (r) {
            if (!r.right) return '<div class="sline empty"><span class="dn"></span><span class="dt"></span></div>';
            const cls = r.right.t === 'add' ? 'add' : '';
            return (
              '<div class="sline ' + cls + '">' +
              '<span class="dn">' + (r.right.nw == null ? '' : r.right.nw) + '</span>' +
              '<span class="dt">' + (CW.md.highlight(r.right.text, 'go') || '&nbsp;') + '</span>' +
              '</div>'
            );
          })
          .join('');

        return (
          '<div class="diff-hunk' + (hunk.applied ? ' applied' : '') + '">' +
          '<div class="diff-hunk-head">' +
          '<span class="hunk-title">' + U.esc(hunk.header) + '</span>' +
          '<span class="spacer"></span>' +
          renderHunkTools(file, hunk) +
          '</div>' +
          '<div class="diff-side">' +
          '<div class="diff-side-col">' + left + '</div>' +
          '<div class="diff-side-col">' + right + '</div>' +
          '</div>' +
          '</div>'
        );
      })
      .join('');
  }

  function render(host) {
    const file = activeFile();
    const stats = totalStats();
    const appliedCount = D.codeDiff.filter(function (f) {
      return f.state === 'applied';
    }).length;
    const pendingHunks = file.hunks.filter(function (h) {
      return !h.applied;
    }).length;

    host.innerHTML =
      '<div class="diff-wrap">' +
      '<div class="diff-summary">' +
      '<span>' + D.codeDiff.length + ' 个文件</span>' +
      '<span class="stat add">+' + stats.add + '</span>' +
      '<span class="stat del">−' + stats.del + '</span>' +
      '<span class="spacer"></span>' +
      '<span class="segmented">' +
      '<button type="button" data-diff-view="unified" class="' + (S.diff.view === 'unified' ? 'is-on' : '') + '">统一</button>' +
      '<button type="button" data-diff-view="side" class="' + (S.diff.view === 'side' ? 'is-on' : '') + '">并排</button>' +
      '</span>' +
      '</div>' +

      '<div class="diff-files">' +
      D.codeDiff
        .map(function (f, i) {
          const fIcon = /\.go$/.test(f.path) ? 'file-code' : 'file';
          return (
            '<button type="button" class="diff-file' + (i === S.diff.fileIndex ? ' is-active' : '') + '" data-diff-file="' + i + '">' +
            U.icon(fIcon, 'f-ico') +
            '<span class="df-name"><span class="df-dir">' + U.esc(f.path.split('/').slice(0, -1).join('/')) + '/</span>' +
            U.esc(f.path.split('/').pop()) + '</span>' +
            '<span class="file-stat"><span class="add">+' + f.added + '</span><span class="del">−' + f.removed + '</span></span>' +
            (f.state === 'applied' ? '<span class="badge ok" style="height:15px">已应用</span>' : '<span class="badge neutral" style="height:15px">待应用</span>') +
            '</button>'
          );
        })
        .join('') +
      '</div>' +

      '<div class="diff-scroll">' +
      (S.diff.view === 'unified' ? renderUnified(file) : renderSide(file)) +
      '</div>' +

      '<div class="diff-foot">' +
      U.icon('hard-drive') +
      '<span>写入前自动备份 · ' + appliedCount + ' / ' + D.codeDiff.length + ' 个文件已应用</span>' +
      '<span class="spacer"></span>' +
      '<button type="button" data-diff-apply-all>' + (pendingHunks ? '应用当前文件全部块' : '已全部应用') + '</button>' +
      '</div>' +
      '</div>';

    bindEvents(host);
  }

  function bindEvents(host) {
    host.querySelectorAll('[data-diff-view]').forEach(function (b) {
      b.addEventListener('click', function () {
        S.diff.view = b.getAttribute('data-diff-view');
        render(host);
      });
    });

    host.querySelectorAll('[data-diff-file]').forEach(function (b) {
      b.addEventListener('click', function () {
        S.diff.fileIndex = Number(b.getAttribute('data-diff-file'));
        render(host);
      });
    });

    host.querySelectorAll('[data-hunk-apply]').forEach(function (b) {
      b.addEventListener('click', function () {
        const file = activeFile();
        const hunk = file.hunks.find(function (h) {
          return h.id === b.getAttribute('data-hunk-apply');
        });
        if (!hunk) return;
        hunk.applied = true;
        hunk.skipped = false;
        if (file.hunks.every(function (h) { return h.applied || h.skipped; })) {
          file.state = 'applied';
        }
        if (file.path === 'internal/auth/session.go') {
          S.codeApplied = true;
          if (S.openFile && S.openFile.path === 'internal/auth/session.go') {
            S.openFile.content = D.sessionGoAfter;
          }
        }
        CW.toast({ kind: 'ok', title: '已应用补丁块', desc: file.path + ' · 写入前已备份' });
        CW.ui.refresh();
      });
    });

    host.querySelectorAll('[data-hunk-skip]').forEach(function (b) {
      b.addEventListener('click', function () {
        const file = activeFile();
        const hunk = file.hunks.find(function (h) {
          return h.id === b.getAttribute('data-hunk-skip');
        });
        if (!hunk) return;
        hunk.skipped = true;
        hunk.applied = false;
        CW.toast({ kind: 'info', title: '已跳过该块', desc: '文件内容保持不变' });
        CW.ui.refresh();
      });
    });

    const applyAll = host.querySelector('[data-diff-apply-all]');
    if (applyAll) {
      applyAll.addEventListener('click', function () {
        const file = activeFile();
        file.hunks.forEach(function (h) {
          h.applied = true;
          h.skipped = false;
        });
        file.state = 'applied';
        if (file.path === 'internal/auth/session.go') {
          S.codeApplied = true;
          if (S.openFile && S.openFile.path === 'internal/auth/session.go') {
            S.openFile.content = D.sessionGoAfter;
          }
        }
        CW.toast({ kind: 'ok', title: '已应用全部补丁块', desc: file.path });
        CW.ui.refresh();
      });
    }
  }

  CW.Diff = { render: render };
})();
