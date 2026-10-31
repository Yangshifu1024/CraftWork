/* ==========================================================================
   CraftWork 高保真原型 · 终端面板
   多标签、命令历史、基础命令模拟。用户终端不经过 Agent 审批，与 Shell 工具
   是两条独立路径，这一点在界面中始终提示。
   ========================================================================== */
(function () {
  const CW = window.CW;
  const U = CW.util;
  const S = CW.State;
  const D = CW.Data;

  S.termHistory = S.termHistory || [];
  S.termHistoryIndex = -1;

  const PROMPTS = {
    '~/Code/craftwork': 'craftwork',
  };

  function active() {
    return S.terminals[S.terminalActive] || S.terminals[0];
  }

  function promptHtml(term) {
    return (
      '<span class="prompt">➜ <span class="path">' + U.esc(PROMPTS[term.cwd] || term.cwd) + '</span>' +
      ' <span style="color:var(--text-3)">git:(main)</span> </span>'
    );
  }

  function lineHtml(line) {
    const cls = line.k === 'cmd' ? 'cmd' : line.k === 'out' ? 'out' : line.k;
    if (line.k === 'cmd') {
      return '<div class="term-line cmd">' + promptHtml({ cwd: '~/Code/craftwork' }) + U.esc(line.text) + '</div>';
    }
    return '<div class="term-line ' + cls + '">' + U.esc(line.text || '\u00a0') + '</div>';
  }

  function render(host) {
    const term = active();

    host.innerHTML =
      '<div class="terminal-wrap">' +
      '<div class="term-tabs">' +
      S.terminals
        .map(function (t, i) {
          return (
            '<button type="button" class="term-tab' + (i === S.terminalActive ? ' is-active' : '') + '" data-term-tab="' + i + '">' +
            U.icon('terminal') + U.esc(t.name) +
            (S.terminals.length > 1
              ? '<span class="term-close" data-term-close="' + i + '" role="button" aria-label="关闭终端">' + U.icon('x') + '</span>'
              : '') +
            '</button>'
          );
        })
        .join('') +
      '<button type="button" class="icon-btn sm" id="term-add" title="新建终端标签" aria-label="新建终端标签">' + U.icon('plus') + '</button>' +
      '</div>' +
      '<div class="term-head">' +
      U.icon('terminal') +
      '<span class="mono">' + U.esc(term.cwd) + '</span>' +
      '<span>·</span><span>zsh</span>' +
      '<span class="spacer"></span>' +
      '<span>' + U.icon('shield') + ' 用户终端不经过 Agent 审批</span>' +
      '</div>' +
      '<div class="term-scroll" id="term-scroll">' +
      '<div class="term-rows" id="term-rows">' +
      term.lines.map(lineHtml).join('') +
      '</div>' +
      '<div class="term-input-row" id="term-input-row">' +
      promptHtml(term) +
      '<input class="term-input" id="term-input" type="text" autocomplete="off" spellcheck="false" aria-label="终端输入">' +
      '</div>' +
      '<div class="term-hint">Enter 执行 · ↑ ↓ 浏览历史 · 输入 help 查看可用命令（原型演示环境）</div>' +
      '</div>' +
      '</div>';

    bind(host);

    const scroll = U.q('#term-scroll');
    if (scroll) scroll.scrollTop = scroll.scrollHeight;
    const input = U.q('#term-input');
    if (input) input.focus();
  }

  function bind(host) {
    host.querySelectorAll('[data-term-tab]').forEach(function (b) {
      b.addEventListener('click', function (e) {
        if (e.target.closest('[data-term-close]')) return;
        S.terminalActive = Number(b.getAttribute('data-term-tab'));
        render(host);
      });
    });

    host.querySelectorAll('[data-term-close]').forEach(function (b) {
      b.addEventListener('click', function (e) {
        e.stopPropagation();
        const idx = Number(b.getAttribute('data-term-close'));
        closeTerminal(idx);
      });
    });

    const add = U.q('#term-add');
    if (add) {
      add.addEventListener('click', function () {
        S.terminals.push({
          id: U.uid('t'),
          name: 'zsh ' + (S.terminals.length + 1),
          cwd: '~/Code/craftwork',
          lines: [{ k: 'dim', text: '新终端会话 · 退出后不跨进程恢复' }],
        });
        S.terminalActive = S.terminals.length - 1;
        render(host);
        CW.toast({ kind: 'info', title: '已新建终端标签', desc: '每个标签有独立的伪终端与仿真器' });
      });
    }

    const scroll = U.q('#term-scroll');
    if (scroll) {
      scroll.addEventListener('click', function (e) {
        if (e.target.closest('input')) return;
        const input = U.q('#term-input');
        if (input) input.focus();
      });
    }

    const input = U.q('#term-input');
    if (input) {
      input.addEventListener('keydown', function (e) {
        if (e.key === 'Enter') {
          e.preventDefault();
          execute(input.value, host);
        } else if (e.key === 'ArrowUp') {
          e.preventDefault();
          if (!S.termHistory.length) return;
          S.termHistoryIndex = Math.min(S.termHistoryIndex + 1, S.termHistory.length - 1);
          input.value = S.termHistory[S.termHistory.length - 1 - S.termHistoryIndex] || '';
          input.setSelectionRange(input.value.length, input.value.length);
        } else if (e.key === 'ArrowDown') {
          e.preventDefault();
          S.termHistoryIndex = Math.max(S.termHistoryIndex - 1, -1);
          input.value =
            S.termHistoryIndex < 0 ? '' : S.termHistory[S.termHistory.length - 1 - S.termHistoryIndex] || '';
        } else if (e.key === 'l' && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          active().lines = [];
          render(host);
        }
      });
    }
  }

  function closeTerminal(idx) {
    if (S.terminals.length <= 1) {
      CW.toast({ kind: 'warn', title: '至少保留一个终端标签' });
      return;
    }
    const term = S.terminals[idx];
    S.terminals.splice(idx, 1);
    S.terminalActive = Math.max(0, Math.min(S.terminalActive, S.terminals.length - 1));
    CW.toast({ kind: 'info', title: '终端已关闭', desc: term.name + ' · 进程组已清理' });
    CW.ui.refresh();
  }

  function out(lines) {
    const term = active();
    lines.forEach(function (l) {
      if (typeof l === 'string') term.lines.push({ k: 'out', text: l });
      else term.lines.push(l);
    });
  }

  function execute(raw, host) {
    const term = active();
    const cmd = raw.trim();
    const input = U.q('#term-input');
    if (input) input.value = '';
    if (!cmd) {
      render(host);
      return;
    }

    term.lines.push({ k: 'cmd', text: cmd });
    S.termHistory.push(cmd);
    S.termHistoryIndex = -1;

    const lower = cmd.toLowerCase();

    if (cmd === 'clear' || lower === 'clear') {
      term.lines = [];
      render(host);
      return;
    }

    if (cmd === 'help') {
      out([
        { k: 'info', text: '原型支持的演示命令：' },
        '  ls / ls -la          列出目录',
        '  pwd                  当前目录',
        '  cat <file>           查看文件（README.md、session.go…）',
        '  git status           工作区状态',
        '  git log              最近提交',
        '  go test ./...        运行测试',
        '  npm install          演示网络类命令',
        '  echo <text>          回显',
        '  whoami / date        当前用户与时间',
        '  clear / Ctrl+L       清屏',
        '  exit                 结束当前终端',
      ]);
    } else if (lower === 'pwd') {
      out(['/Users/yangzhenbiao/Code/craftwork']);
    } else if (lower === 'whoami') {
      out(['yangzhenbiao']);
    } else if (lower === 'date') {
      out([new Date().toString()]);
    } else if (lower.indexOf('ls') === 0) {
      out([
        { k: 'info', text: 'cmd/       internal/   docs/       go.mod   go.sum' },
        { k: 'out', text: 'README.md  .gitignore  dist/       .craftwork' },
      ]);
    } else if (lower.startsWith('cd ')) {
      out([{ k: 'dim', text: '（原型演示：目录切换已模拟）' }]);
    } else if (lower.startsWith('cat ')) {
      const target = cmd.slice(4).trim();
      if (target === 'README.md') {
        D.files['README.md'].split('\n').slice(0, 18).forEach(function (l) {
          term.lines.push({ k: 'out', text: l });
        });
      } else if (target.indexOf('session.go') >= 0) {
        D.files['internal/auth/session.go'].split('\n').slice(0, 32).forEach(function (l) {
          term.lines.push({ k: 'out', text: l });
        });
        term.lines.push({ k: 'dim', text: '… 共 83 行' });
      } else {
        out([{ k: 'err', text: 'cat: ' + target + ': No such file or directory' }]);
      }
    } else if (lower === 'git status' || lower === 'git status --short') {
      out([
        { k: 'warn', text: 'On branch main' },
        { k: 'dim', text: 'Your branch is ahead of \'origin/main\' by 2 commits.' },
        'Changes not staged for commit:',
        { k: 'warn', text: '  modified:   internal/agent/loop.go' },
        { k: 'warn', text: '  modified:   internal/auth/session.go' },
        { k: 'warn', text: '  modified:   internal/store/db.go' },
        { k: 'warn', text: '  modified:   internal/ui/chat.go' },
        { k: 'dim', text: 'no changes added to commit (use "git add" ...)' },
      ]);
    } else if (lower === 'git log' || lower.startsWith('git log')) {
      out([
        { k: 'warn', text: 'a3f91c2 (HEAD -> main) 修正文档备份路径' },
        { k: 'out', text: '7bd0e14 增加 MCP 服务器信任确认' },
        { k: 'out', text: '2c8a5f0 终端中文宽字符对齐' },
        { k: 'out', text: '91e3b7d 会话刷新锁范围调整' },
      ]);
    } else if (lower.startsWith('go test')) {
      out([
        { k: 'dim', text: '=== RUN   TestRefreshSessionConcurrent' },
        { k: 'ok', text: '--- PASS: TestRefreshSessionConcurrent (0.42s)' },
        { k: 'dim', text: '=== RUN   TestRefreshSessionDoesNotBlockReads' },
        { k: 'ok', text: '--- PASS: TestRefreshSessionDoesNotBlockReads (0.21s)' },
        { k: 'ok', text: 'PASS' },
        { k: 'ok', text: 'ok  \tcraftwork/internal/auth\t1.284s' },
      ]);
    } else if (lower.startsWith('npm install') || lower.startsWith('npm i')) {
      out([
        { k: 'warn', text: 'npm warn idealTree 检测到网络访问' },
        { k: 'dim', text: 'added 128 packages in 6s' },
        { k: 'dim', text: '（用户终端直接执行，不经过 Agent 审批；Agent 的 shell_exec 会走 R3 审批）' },
      ]);
    } else if (lower.startsWith('echo ')) {
      out([cmd.slice(5)]);
    } else if (lower === 'exit') {
      out([{ k: 'dim', text: '终端会话结束 · 进程已退出（原型中保留标签）' }]);
    } else if (lower.startsWith('git commit')) {
      out([
        { k: 'err', text: 'error: 原型环境不实际提交代码' },
        { k: 'dim', text: '真实产品中 git_commit 属于 R1，需要审批且默认不推送。' },
      ]);
    } else if (lower.startsWith('sudo') || lower.startsWith('rm -rf') || lower.startsWith('chmod 777')) {
      out([
        { k: 'err', text: 'zsh: 原型环境禁止执行该命令' },
        { k: 'warn', text: 'R4 破坏性操作在 Agent 中默认阻止，用户终端也受系统权限约束。' },
      ]);
    } else {
      out([{ k: 'err', text: 'zsh: command not found: ' + cmd.split(' ')[0] }]);
    }

    out([{ k: 'dim', text: '' }]);
    render(host);
  }

  CW.Term = { render: render, execute: execute };
})();
