/* ==========================================================================
   CraftWork 高保真原型 · 聊天区
   负责：线程头部、消息列表、各类内容卡片、输入框、附件、审批交互、滚动跟随。
   场景流程（scenarios.js）通过 CW.Chat 提供的接口增量更新消息。
   ========================================================================== */
(function () {
  const CW = window.CW;
  const U = CW.util;
  const S = CW.State;

  const BRAND_SVG =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round">' +
    '<path d="M12 2.8 20.2 7.4v9.2L12 21.2 3.8 16.6V7.4z"/>' +
    '<path d="M12 7.4l4.6 2.6v4.8L12 17.4l-4.6-2.6V10z" opacity=".5"/></svg>';

  const TOOL_ICONS = {    fs_read: 'file-code',
    fs_list: 'folder',
    fs_glob: 'file',
    fs_grep: 'search',
    fs_write: 'file-plus',
    fs_edit: 'edit',
    apply_patch: 'diff',
    shell_exec: 'terminal',
    git_status: 'git-branch',
    git_diff: 'diff',
    git_log: 'clock',
    git_commit: 'git-branch',
    todo_write: 'list-todo',
    task: 'cpu',
    ask_user: 'message',
    doc_read: 'file-text',
    doc_write: 'file-text',
    pdf_read: 'file-text',
  };

  /* 审批档位：输入框底部可切换，与设置 · 工具与权限里的策略联动 */
  const APPROVAL_MODES = {
    graded: {
      label: '分级',
      name: '分级审批',
      desc: 'R0 自动；R1 / R2 每次询问；R3 每次询问；R4 阻止。',
    },
    strict: {
      label: '严格',
      name: '严格审批',
      desc: '所有写入与执行逐次询问；不提供本会话或本工作区允许。',
    },
    readonly: {
      label: '只读',
      name: '只读',
      desc: '只允许读取；写入与执行在真实产品中会被阻止。',
    },
  };

  function approvalMode() {
    return S.settings.approvalMode || 'graded';
  }

  function approvalModeInfo() {
    return APPROVAL_MODES[approvalMode()] || APPROVAL_MODES.graded;
  }

  function setApprovalMode(key, silent) {
    if (!APPROVAL_MODES[key]) return;
    S.settings.approvalMode = key;
    CW.savePrefs();
    renderComposer();
    CW.ui.refresh();
    if (!silent) {
      const info = APPROVAL_MODES[key];
      CW.toast({
        kind: key === 'graded' ? 'ok' : 'warn',
        title: '审批档位：' + info.name,
        desc: info.desc,
      });
    }
  }

  function openApprovalMenu(anchor) {
    const current = approvalMode();
    const items = Object.keys(APPROVAL_MODES)
      .map(function (key) {
        const m = APPROVAL_MODES[key];
        return (
          '<button type="button" class="popover-item" data-approval-mode="' + key + '">' +
          '<span class="pi-main"><div class="pi-title">' + U.esc(m.name) + '</div>' +
          '<div class="pi-desc">' + U.esc(m.desc) + '</div></span>' +
          (key === current ? '<span class="pi-trail pi-check">' + U.icon('check') + '</span>' : '') +
          '</button>'
        );
      })
      .join('');

    const el = CW.popover(
      anchor,
      '<div class="popover-label">审批档位</div>' + items +
        '<div class="popover-sep"></div>' +
        '<button type="button" class="popover-item" data-approval-settings>' + U.icon('settings') +
        '<span class="pi-main"><div class="pi-title">在设置中查看完整策略…</div></span></button>' +
        '<div class="popover-note">档位只影响本机的新 run。R3 与 R4 在任何档位下都不提供永久允许。</div>',
      {
        align: 'left',
        onMount: function (menu) {
          menu.querySelectorAll('[data-approval-mode]').forEach(function (b) {
            b.addEventListener('click', function () {
              CW.closePopover();
              setApprovalMode(b.getAttribute('data-approval-mode'));
            });
          });
          const toSettings = menu.querySelector('[data-approval-settings]');
          if (toSettings) {
            toSettings.addEventListener('click', function () {
              CW.closePopover();
              CW.openSettings('tools');
            });
          }
        },
      }
    );
    return el;
  }

  function toolIcon(name) {
    if (TOOL_ICONS[name]) return TOOL_ICONS[name];
    if (String(name).startsWith('mcp__')) return 'layers';
    return 'zap';
  }

  /* ---------------- 线程头部 ---------------- */
  function renderHead() {
    const host = U.q('#chat-head');
    const thread = CW.currentThread();
    if (!host) return;
    if (!thread) {
      host.innerHTML = '';
      return;
    }

    const statusBadge =
      thread.status === 'running'
        ? '<span class="badge accent"><span class="pulse-dot" style="width:5px;height:5px;border-radius:50%;background:currentColor"></span>运行中</span>'
        : thread.status === 'awaiting'
        ? '<span class="badge warn">' + U.icon('clock') + '等待审批</span>'
        : thread.status === 'error'
        ? '<span class="badge danger">' + U.icon('alert') + '出错</span>'
        : '';

    host.innerHTML =
      '<div class="chat-head-main">' +
      '<span class="chat-title" id="chat-title">' + U.esc(thread.title) + '</span>' +
      '<span class="badge ' + (thread.mode === 'work' ? 'work' : 'accent') + '">' +
      (thread.mode === 'work' ? 'Work' : 'Code') +
      '</span>' +
      statusBadge +
      '</div>' +
      '<div class="chat-head-actions">' +
      '<button type="button" class="icon-btn sm" data-head-act="panel" id="btn-panel-toggle" title="收起右侧面板" aria-label="收起右侧面板">' + U.icon('panel') + '</button>' +
      '<button type="button" class="icon-btn sm" data-head-act="rename" title="重命名线程" aria-label="重命名线程">' + U.icon('edit') + '</button>' +
      '<button type="button" class="icon-btn sm" data-head-act="copy" title="复制线程为新线程" aria-label="复制线程">' + U.icon('copy') + '</button>' +
      '<button type="button" class="icon-btn sm" data-head-act="archive" title="归档线程" aria-label="归档线程">' + U.icon('archive') + '</button>' +
      '<button type="button" class="icon-btn sm" data-head-act="more" title="更多操作" aria-label="更多操作">' + U.icon('more') + '</button>' +
      '</div>';
  }

  function openModelMenu(anchor, thread) {
    const items = S.models
      .map(function (m) {
        const active = m.id === thread.model;
        const caps = [];
        if (m.caps.reasoning) caps.push('推理');
        if (m.caps.vision) caps.push('图片');
        if (m.caps.tools) caps.push('工具');
        return (
          '<button type="button" class="popover-item" data-model="' + m.id + '">' +
          '<span class="pi-main">' +
          '<div class="pi-title">' + U.esc(m.name) + '</div>' +
          '<div class="pi-desc">' + U.esc(m.provider) + ' · ' + caps.join(' / ') + ' · ' + m.context + 'k</div>' +
          '</span>' +
          (active ? '<span class="pi-trail pi-check">' + U.icon('check') + '</span>' : '') +
          '</button>'
        );
      })
      .join('');

    const el = CW.popover(
      anchor,
      '<div class="popover-label">切换模型</div>' + items +
        '<div class="popover-sep"></div>' +
        '<div class="popover-note">切换只影响后续 run，历史消息保留。不可用能力会自动禁用对应控件。</div>'
    );
    el.querySelectorAll('[data-model]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        const id = btn.getAttribute('data-model');
        const m = S.models.find(function (x) {
          return x.id === id;
        });
        thread.model = id;
        thread.modelName = m ? m.name : id;
        CW.closePopover();
        renderComposer();
        CW.toast({ kind: 'ok', title: '已切换模型', desc: m.name + ' · 仅影响后续 run' });
        CW.ui.refresh();
      });
    });
  }

  /* ---------------- 内容块渲染 ---------------- */
  function renderBlock(block, bi, msg) {
    switch (block.type) {
      case 'text':
        return '<div class="blk md" data-bi="' + bi + '">' + CW.md.render(block.text || '') +
          (msg.streaming && bi === msg.blocks.length - 1 ? '<span class="stream-caret"></span>' : '') + '</div>';

      case 'reasoning':
        return (
          '<div class="blk reasoning' + (block.open ? ' is-open' : '') + '" data-reasoning="' + bi + '">' +
          '<button type="button" class="reasoning-head" data-toggle-reasoning="' + bi + '">' +
          U.icon('chevron-right', 'chev') +
          U.icon('sparkles') +
          '<span>思考过程</span>' +
          '<span class="r-dur">' + U.esc(block.duration || '') + '</span>' +
          '</button>' +
          '<div class="reasoning-body">' + U.esc(block.text || '') + '</div>' +
          '</div>'
        );

      case 'tool':
        return renderTool(block, bi);

      case 'todo':
        return renderTodo(block, bi);

      case 'approval':
        return renderApproval(block, bi);

      case 'error':
        return renderError(block, bi);

      case 'draft':
        return renderDraft(block, bi);

      case 'callout':
        return (
          '<div class="blk callout ' + (block.variant || '') + '">' +
          U.icon(block.variant === 'warn' ? 'alert' : block.variant === 'ok' ? 'check-circle' : 'info') +
          '<div class="callout-body">' +
          (block.title ? '<div class="callout-title">' + U.esc(block.title) + '</div>' : '') +
          '<div>' + CW.md.inline(block.text || '') + '</div>' +
          '</div></div>'
        );

      case 'notice':
        return (
          '<div class="blk sys-note ' + (block.variant || '') + '">' +
          U.icon(block.icon || 'info') + U.esc(block.text) + '</div>'
        );

      default:
        return '';
    }
  }

  function renderTool(block, bi) {
    const statusMap = {
      running: { cls: 'running', label: '运行中', icon: 'loader', spin: true },
      done: { cls: 'done', label: '完成', icon: 'check' },
      error: { cls: 'error', label: '失败', icon: 'x' },
      awaiting: { cls: 'awaiting', label: '等待审批', icon: 'clock' },
    };
    const st = statusMap[block.status] || statusMap.done;
    const args = (block.args || [])
      .map(function (pair) {
        return '<span class="tool-arg"><b>' + U.esc(pair[0]) + '</b> <span class="mono">' + U.esc(pair[1]) + '</span></span>';
      })
      .join('');

    return (
      '<div class="blk tool-card status-' + st.cls + (block.open ? ' is-open' : '') + '" data-tool="' + bi + '">' +
      '<button type="button" class="tool-head" data-tool-toggle="' + bi + '">' +
      '<span class="tool-icon">' + U.icon(toolIcon(block.name)) + '</span>' +
      '<span class="tool-main">' +
      '<span class="tool-name">' + U.esc(block.name) + '</span>' +
      (block.risk ? '<span class="risk-badge risk-' + block.risk + '">' + block.risk + '</span>' : '') +
      '<span class="tool-summary">' + U.esc(block.summary || '') + '</span>' +
      '</span>' +
      '<span class="tool-meta">' +
      (block.duration ? '<span>' + U.esc(block.duration) + '</span>' : '') +
      (block.meta ? '<span>' + U.esc(block.meta) + '</span>' : '') +
      '</span>' +
      '<span class="tool-status ' + st.cls + '">' +
      U.icon(st.icon, st.spin ? 'spin' : '') + st.label +
      '</span>' +
      '<span class="tool-chev">' + U.icon('chevron-right') + '</span>' +
      '</button>' +
      '<div class="tool-body">' +
      (args ? '<div class="tool-args">' + args + '</div>' : '') +
      '<pre class="tool-output">' + U.esc(block.output || '') + '</pre>' +
      '<div class="tool-foot">' +
      '<span>' + U.esc(block.footer || '') + '</span>' +
      '<span class="spacer"></span>' +
      '<button type="button" data-copy-tool="' + bi + '">' + U.icon('copy') + '复制输出</button>' +
      '</div>' +
      '</div>' +
      '</div>'
    );
  }

  function renderTodo(block, bi) {
    const items = block.items || [];
    const done = items.filter(function (i) {
      return i.done;
    }).length;
    const pct = items.length ? Math.round((done / items.length) * 100) : 0;
    return (
      '<div class="blk todo-card">' +
      '<div class="todo-head">' +
      U.icon('list-todo') +
      '<span>任务清单</span>' +
      '<span class="spacer"></span>' +
      '<span class="todo-count">' + done + ' / ' + items.length + '</span>' +
      '<span class="todo-progress"><i style="width:' + pct + '%"></i></span>' +
      '</div>' +
      '<div class="todo-list">' +
      items
        .map(function (it) {
          return (
            '<div class="todo-item ' + (it.done ? 'done' : it.active ? 'active' : '') + '">' +
            '<span class="tbox">' + (it.done ? U.icon('check') : '') + '</span>' +
            '<span class="tlabel">' + U.esc(it.text) + '</span>' +
            (it.active && !it.done ? '<span class="tmeta">进行中</span>' : '') +
            '</div>'
          );
        })
        .join('') +
      '</div></div>'
    );
  }

  function renderDiffLines(lines) {
    return lines
      .map(function (line) {
        const sign = line.t === 'add' ? '+' : line.t === 'del' ? '−' : ' ';
        return (
          '<div class="diff-line ' + line.t + '">' +
          '<span class="sign">' + sign + '</span>' +
          '<span class="ln">' + (line.ln == null ? '' : line.ln) + '</span>' +
          '<span>' + U.esc(line.text) + '</span>' +
          '</div>'
        );
      })
      .join('');
  }

  function renderApproval(block, bi) {
    const waiting = block.status === 'waiting';
    const approved = block.status === 'approved';
    const denied = block.status === 'denied';
    const cls = waiting ? 'is-waiting' : approved ? 'is-approved' : denied ? 'is-denied' : '';
    const facts = (block.facts || [])
      .map(function (f) {
        return '<span class="approval-fact">' + U.icon(f.icon) + U.esc(f.text) + '</span>';
      })
      .join('');

    const mode = approvalMode();
    const modeInfo = approvalModeInfo();
    const permanentAllowed = (block.risk === 'R1' || block.risk === 'R2') && mode === 'graded';
    const isCommand = block.tool === 'shell_exec';
    const previewLabel = block.docDiff ? '文本级差异预览' : isCommand ? '命令预览' : '差异预览';
    const previewName = isCommand ? '' : ' · ' + U.esc(String(block.target).split('/').pop());

    const noteText =
      mode === 'strict'
        ? '严格档位：所有写入与执行逐次询问，不提供永久允许'
        : mode === 'readonly'
        ? '只读档位：真实产品会阻止写入与执行；此处仅用于演示'
        : '默认选中“拒绝”；Esc 拒绝；R3 / R4 不提供永久允许';

    const actions = waiting
      ? '<div class="approval-actions">' +
        '<button type="button" class="btn danger" data-decision="deny">拒绝</button>' +
        '<span class="spacer"></span>' +
        '<button type="button" class="btn" data-decision="once">允许一次</button>' +
        (permanentAllowed
          ? '<button type="button" class="btn ghost" data-decision="session">本会话允许</button>' +
            '<button type="button" class="btn ghost" data-decision="workspace">本工作区始终允许</button>'
          : '') +
        '</div>' +
        '<div class="approval-actions" style="border-top:0;padding-top:0">' +
        '<span class="approval-default-note">' + U.icon('info') + noteText + '</span>' +
        '</div>'
      : '';

    const result = approved
      ? '<div class="approval-result approved">' + U.icon('check-circle') +
        '已允许' + (block.scope === 'once' ? '一次' : block.scope === 'session' ? '（本会话）' : '（本工作区）') +
        '<span class="scope">· ' + U.esc(block.decidedAt || '') + ' · 已写入审计</span></div>'
      : denied
      ? '<div class="approval-result denied">' + U.icon('x-circle') +
        '已拒绝' + '<span class="scope">· ' + U.esc(block.decidedAt || '') + ' · 未执行任何修改</span></div>'
      : '';

    return (
      '<div class="blk approval-card ' + cls + '" data-risk="' + block.risk + '" data-apid="' + block.id + '" data-bid="' + bi + '">' +
      '<div class="approval-strip"></div>' +
      '<div class="approval-head">' +
      '<span class="approval-title">' + U.icon('shield') + '需要你的批准' +
      '<span class="risk-badge risk-' + block.risk + '">' + block.risk + '</span>' +
      '<span class="tool-name">' + U.esc(block.tool) + '</span>' +
      (mode !== 'graded'
        ? '<span class="badge ' + (mode === 'strict' ? 'warn' : 'neutral') + '" style="height:16px">' +
          U.esc(modeInfo.label) + '档位</span>'
        : '') +
      '</span>' +
      '<span class="approval-hint">' + (waiting ? '等待决定' : approved ? '已批准' : '已拒绝') + '</span>' +
      '</div>' +
      '<div class="approval-body">' +
      '<div class="approval-summary">' + CW.md.inline(block.summary || '') + '</div>' +
      '<div class="approval-target">' + U.icon('file') + '<span>' + U.esc(block.target) + '</span></div>' +
      '<div class="approval-facts">' + facts + '</div>' +
      (block.diffPreview
        ? '<div class="approval-diff-preview">' +
          '<div class="approval-diff-head">' +
          U.icon('diff') +
          '<span>' + previewLabel + previewName + '</span>' +
          '<span class="spacer"></span>' +
          (isCommand ? '' : '<button type="button" data-open-diff="' + bi + '">查看完整差异</button>') +
          '</div>' +
          '<div class="approval-diff-body">' + renderDiffLines(block.diffPreview) + '</div>' +
          '</div>'
        : '') +
      '</div>' +
      actions +
      result +
      '</div>'
    );
  }

  function renderError(block, bi) {
    const titles = {
      context_overflow: '上下文超出模型窗口',
      rate_limit: '触发了提供商的限流',
      auth: '认证失败',
      timeout: '请求超时',
    };
    return (
      '<div class="blk error-card">' +
      '<div class="error-head">' + U.icon('alert') +
      '<span>' + U.esc(block.title || titles[block.code] || '执行出错') + '</span>' +
      '<span class="spacer"></span>' +
      '<span class="badge danger">' + U.esc(block.code || 'error') + '</span>' +
      '</div>' +
      '<div class="error-body">' + CW.md.inline(block.desc || '') +
      (block.detail ? '<div class="err-code">' + U.esc(block.detail) + '</div>' : '') +
      '</div>' +
      (block.actions
        ? '<div class="error-actions">' +
          '<button type="button" class="btn sm" data-err-act="retry">' + U.icon('refresh') + '重试</button>' +
          '<button type="button" class="btn sm ghost" data-err-act="model">' + U.icon('cpu') + '切换模型</button>' +
          '<button type="button" class="btn sm ghost" data-err-act="copy">' + U.icon('copy') + '复制错误</button>' +
          '</div>'
        : '') +
      '</div>'
    );
  }

  function renderDraft(block, bi) {
    return (
      '<div class="blk draft-card" data-draft="' + bi + '">' +
      '<div class="draft-head">' + U.icon('mail') +
      '<span class="draft-kind">' + U.esc(block.kind || '草稿') + '</span>' +
      '<span class="spacer"></span>' +
      '<span class="badge work">仅应用内显示</span>' +
      '</div>' +
      '<div class="draft-fields">' +
      '<div class="draft-field"><b>收件人</b><span>' + U.esc(block.to || '') + '</span></div>' +
      '<div class="draft-field"><b>主题</b><span>' + U.esc(block.subject || '') + '</span></div>' +
      '</div>' +
      '<div class="draft-body">' + U.esc(block.body || '') + '</div>' +
      '<div class="draft-foot">' +
      '<span>生成于 ' + U.esc(block.time || '') + ' · 不会自动发送</span>' +
      '<span class="spacer"></span>' +
      '<button type="button" class="btn sm" data-draft-copy="' + bi + '">' + U.icon('copy') + '复制草稿</button>' +
      '</div>' +
      '</div>'
    );
  }

  /* ---------------- 消息渲染 ---------------- */
  function renderMessage(msg) {
    const isUser = msg.role === 'user';
    const blocks = (msg.blocks || [])
      .map(function (b, i) {
        return renderBlock(b, i, msg);
      })
      .join('');

    const actions = isUser
      ? '<div class="msg-actions">' +
        '<button type="button" data-msg-act="copy" data-mid="' + msg.id + '">' + U.icon('copy') + '复制</button>' +
        '<button type="button" data-msg-act="edit" data-mid="' + msg.id + '">' + U.icon('edit') + '编辑重发</button>' +
        '</div>'
      : '<div class="msg-actions">' +
        '<button type="button" data-msg-act="copy-answer" data-mid="' + msg.id + '">' + U.icon('copy') + '复制</button>' +
        '<button type="button" data-msg-act="regenerate" data-mid="' + msg.id + '">' + U.icon('refresh') + '重新生成</button>' +
        '<button type="button" data-msg-act="branch" data-mid="' + msg.id + '">' + U.icon('git-branch') + '分支对话</button>' +
        '</div>';

    return (
      '<div class="msg ' + (isUser ? 'user' : 'assistant') + (msg.superseded ? ' msg-superseded' : '') + '" data-mid="' + msg.id + '">' +
      '<div class="msg-avatar">' + (isUser ? U.icon('user') : BRAND_SVG) + '</div>' +
      '<div class="msg-body">' +
      '<div class="msg-head">' +
      '<span class="msg-author">' + (isUser ? '你' : 'CraftWork') + '</span>' +
      '<span class="msg-time">' + U.esc(msg.time || '') + '</span>' +
      (msg.superseded ? '<span class="msg-tag">已被替代</span>' : '') +
      (msg.streaming ? '<span class="msg-tag">生成中</span>' : '') +
      '</div>' +
      (msg.superseded
        ? '<div class="superseded-bar">' + U.icon('refresh') + '该回复已被替代，可展开查看历史内容</div>'
        : '') +
      blocks +
      actions +
      '</div>' +
      '</div>'
    );
  }

  function renderMessages() {
    const host = U.q('#chat-stream');
    const thread = CW.currentThread();
    if (!host) return;

    if (!thread || !thread.messages || thread.messages.length === 0) {
      host.innerHTML = renderEmptyState();
      return;
    }

    host.innerHTML = thread.messages.map(renderMessage).join('');
    const follow = S.scrollFollow[thread.id] !== false;
    if (follow) {
      requestAnimationFrame(function () {
        scrollToBottom();
      });
    }
  }

  function renderEmptyState() {
    const mode = S.mode;
    const suggestions =
      mode === 'code'
        ? ['梳理这个仓库的结构与测试方式', '修复 `internal/auth/session.go` 的刷新超时', '给 token 刷新补充并发测试']
        : ['总结 `docs/产品方案-v2.pdf` 的要点', '整理会议纪要里的待办事项', '汇总本周邮件并草拟回复'];
    return (
      '<div class="chat-empty">' +
      '<span class="empty-mark">' + U.icon(mode === 'work' ? 'briefcase' : 'code') + '</span>' +
      '<h2>' + (mode === 'work' ? 'Work 模式已就绪' : 'Code 模式已就绪') + '</h2>' +
      '<p>' +
      (mode === 'work'
        ? '读取文档、修改 Word 与 Excel、汇总邮件与日历。所有文档写入都会在审批后执行并自动备份。'
        : '在当前工作区里读代码、改文件、跑命令、看差异。所有写操作与命令执行都会先经过审批。') +
      '</p>' +
      '<div class="empty-suggestions">' +
      suggestions
        .map(function (s) {
          return '<button type="button" data-suggestion="' + U.esc(s) + '">' + U.esc(s) + '</button>';
        })
        .join('') +
      '</div>' +
      '</div>'
    );
  }

  /* ---------------- 消息增量更新 ---------------- */
  function messageEl(msgId) {
    return U.q('.msg[data-mid="' + msgId + '"]');
  }

  function ownerThreadOf(msg) {
    const all = S.threads.code.concat(S.threads.work);
    return (
      all.find(function (t) {
        return (t.messages || []).indexOf(msg) >= 0;
      }) || null
    );
  }

  function patchMessage(msg) {
    if (!msg) return;
    const owner = ownerThreadOf(msg);
    const current = CW.currentThread();
    // 消息不属于当前线程时只更新数据，不重绘，避免串线
    if (!owner || !current || owner.id !== current.id) return;
    const el = messageEl(msg.id);
    if (!el) {
      renderMessages();
      return;
    }
    const follow = S.scrollFollow[owner.id] !== false;
    const scrollHost = U.q('#chat-scroll');
    const nearBottom = scrollHost
      ? scrollHost.scrollHeight - scrollHost.scrollTop - scrollHost.clientHeight < 90
      : false;
    const wrapper = document.createElement('div');
    wrapper.innerHTML = renderMessage(msg);
    const next = wrapper.firstElementChild;
    el.replaceWith(next);
    if (follow && nearBottom) scrollToBottom();
  }

  function streamText(msgId, blockIndex, text) {
    const msgEl = messageEl(msgId);
    if (!msgEl) return;
    const blockEl = msgEl.querySelector('.blk.md[data-bi="' + blockIndex + '"]');
    if (!blockEl) return;
    const thread = CW.currentThread();
    const follow = thread && S.scrollFollow[thread.id] !== false;
    blockEl.innerHTML = CW.md.render(text) + '<span class="stream-caret"></span>';
    if (follow) scrollToBottom();
  }

  function appendBlock(msg, block) {
    if (!msg.blocks) msg.blocks = [];
    msg.blocks.push(block);
    patchMessage(msg);
    const thread = CW.currentThread();
    if (thread) S.scrollFollow[thread.id] = true;
    return block;
  }

  function addMessage(thread, msg) {
    if (!thread.messages) thread.messages = [];
    thread.messages.push(msg);
    if (CW.currentThread() && CW.currentThread().id === thread.id) {
      renderMessages();
    }
    updateThreadMeta(thread);
    return msg;
  }

  function updateThreadMeta(thread) {
    thread.updated = '刚刚';
    const listEl = U.q('#thread-list');
    if (listEl && CW.renderThreads) CW.renderThreads();
  }

  /* ---------------- 输入区 ---------------- */
  function renderComposer() {
    const thread = CW.currentThread();
    const busy = thread && thread.status === 'running';
    const sendBtn = U.q('#btn-send');
    const stopBtn = U.q('#btn-stop');
    const input = U.q('#composer-input');
    const ctxHint = U.q('#ctx-hint');
    const foot = U.q('#composer-foot');
    const modelName = U.q('#composer-model-name');
    const modelBtn = U.q('#composer-model');
    const approvalLabel = U.q('#composer-approval-label');
    const approvalBtn = U.q('#composer-approval');

    if (sendBtn) sendBtn.hidden = !!(busy || (thread && thread.status === 'awaiting'));
    // 等待审批时也提供“停止”，用于取消整个 run（等同于拒绝当前审批）
    if (stopBtn) {
      stopBtn.hidden = !(busy || (thread && thread.status === 'awaiting'));
      stopBtn.title = busy ? '停止生成 Esc' : '取消本次 run Esc';
      stopBtn.setAttribute('aria-label', busy ? '停止生成' : '取消本次 run');
    }
    if (input) {
      input.placeholder =
        S.mode === 'work'
          ? '描述要整理的文档、要汇总的邮件，或粘贴一段内容…'
          : '描述你要修复的问题、要实现的需求，或粘贴一段报错…';
      input.disabled = !!busy;
    }
    if (ctxHint) {
      const used = thread ? thread.tokens || 0 : 0;
      ctxHint.textContent = '≈ ' + U.fmtCompact(Math.round(used / 1000) * 1000 + 2400) + ' / ' + CW.currentModel().context + 'k';
    }
    const activeModel = CW.currentModel();
    if (modelName) modelName.textContent = activeModel.name;
    if (modelBtn) {
      modelBtn.title =
        '切换模型（只影响后续 run）· ' + activeModel.provider + ' · ' + activeModel.context + 'k 上下文';
    }
    const approvalInfo = approvalModeInfo();
    if (approvalLabel) approvalLabel.textContent = approvalInfo.label;
    if (approvalBtn) {
      approvalBtn.classList.toggle('is-strict', approvalMode() === 'strict');
      approvalBtn.classList.toggle('is-readonly', approvalMode() === 'readonly');
      approvalBtn.title = '审批档位：' + approvalInfo.name + '（点击切换）· ' + approvalInfo.desc;
    }
    if (foot) {
      foot.innerHTML =
        '<span class="hints">' +
        '<span><span class="kbd">⌘</span><span class="kbd">↵</span> 发送</span>' +
        '<span><span class="kbd">Esc</span> ' + (busy ? '停止生成' : '关闭浮层') + '</span>' +
        '<span><span class="kbd">⌘</span><span class="kbd">K</span> 命令</span>' +
        '</span>' +
        '<span class="mode-note">' +
        U.icon(S.mode === 'work' ? 'shield' : 'lock') +
        (S.mode === 'work' ? '文档写入每次审批 · 自动备份' : '审批与审计已开启 · 沙箱 confined') +
        '</span>';
    }
    renderAttachments();
    updateSendAvailability();
  }

  function renderAttachments() {
    const host = U.q('#attach-strip');
    if (!host) return;
    if (!S.attachments.length) {
      host.hidden = true;
      host.innerHTML = '';
      return;
    }
    host.hidden = false;
    host.innerHTML = S.attachments
      .map(function (a, i) {
        const thumb = a.kind === 'image' ? '<span class="attach-thumb"></span>' : U.icon(a.kind === 'doc' ? 'file-text' : 'file');
        return (
          '<span class="attach-chip">' + thumb +
          '<span class="fname">' + U.esc(a.name) + '</span>' +
          (a.size ? '<span class="fsize">' + U.esc(a.size) + '</span>' : '') +
          '<button type="button" data-attach-remove="' + i + '" aria-label="移除附件">' + U.icon('x') + '</button>' +
          '</span>'
        );
      })
      .join('');
  }

  function updateSendAvailability() {
    const input = U.q('#composer-input');
    const btn = U.q('#btn-send');
    if (!input || !btn) return;
    const thread = CW.currentThread();
    const busy = thread && (thread.status === 'running' || thread.status === 'awaiting');
    btn.disabled = busy || (!input.value.trim() && S.attachments.length === 0);
  }

  /* ---------------- 发送 ---------------- */
  function sendMessage() {
    const input = U.q('#composer-input');
    const thread = CW.currentThread();
    if (!input || !thread) return;
    const text = input.value.trim();
    if (!text && !S.attachments.length) return;
    if (thread.status === 'running') {
      CW.toast({ kind: 'warn', title: '该线程正在运行', desc: '同一线程同时只允许一个 run，请先停止或等待完成。' });
      return;
    }
    if (thread.status === 'awaiting') {
      CW.toast({ kind: 'warn', title: '等待审批', desc: '请先在审批卡片上做出决定，或点击停止取消本次 run。' });
      return;
    }

    const attachments = S.attachments.slice();
    const msg = {
      id: U.uid('m'),
      role: 'user',
      time: U.timeNow(),
      blocks: [
        {
          type: 'text',
          text: text + (attachments.length
            ? '\n\n' + attachments.map(function (a) { return '`' + a.name + '`'; }).join(' · ')
            : ''),
        },
      ],
    };
    addMessage(thread, msg);
    input.value = '';
    U.autoGrow(input);
    S.attachments = [];
    renderAttachments();
    updateSendAvailability();
    S.scrollFollow[thread.id] = true;
    scrollToBottom();

    CW.Scene.dispatch(thread, text).catch(function (err) {
      console.error('[CraftWork] 场景执行失败', err);
      thread.status = 'idle';
      CW.ui.refresh();
    });
  }

  /* ---------------- 审批 ---------------- */
  function allApprovals() {
    const result = [];
    ['code', 'work'].forEach(function (mode) {
      (S.threads[mode] || []).forEach(function (t) {
        (t.messages || []).forEach(function (m) {
          (m.blocks || []).forEach(function (b) {
            if (b.type === 'approval') result.push({ thread: t, block: b });
          });
        });
      });
    });
    return result;
  }

  function decideApproval(thread, block, decision) {
    if (block.status !== 'waiting') return;
    block.status = decision === 'deny' ? 'denied' : 'approved';
    block.decision = decision;
    block.scope = decision;
    block.decidedAt = U.timeNow();

    const msg = thread.messages.find(function (m) {
      return (m.blocks || []).indexOf(block) >= 0;
    });
    if (msg) patchMessage(msg);

    const resolver = S.approvalResolvers[block.id];
    if (resolver) {
      delete S.approvalResolvers[block.id];
      resolver(decision);
    } else if (block.action && CW.Scene && CW.Scene.resumeAfterApproval) {
      // 预置审批：页面加载时就在等待，决定后由场景继续执行
      CW.Scene.resumeAfterApproval(thread, block, decision);
    }

    if (decision === 'deny') {
      CW.toast({ kind: 'warn', title: '已拒绝该操作', desc: '工具不会执行，审计已记录拒绝决定。' });
    } else {
      CW.toast({
        kind: 'ok',
        title: '已批准',
        desc:
          decision === 'once'
            ? '本次调用已放行'
            : decision === 'session'
            ? '本会话内相同操作自动放行'
            : '本工作区始终允许（R3 / R4 不适用）',
      });
    }
    CW.ui.refresh();
  }

  function requestApproval(block) {
    return new Promise(function (resolve) {
      S.approvalResolvers[block.id] = resolve;
    });
  }

  // 停止 run 时，把等待中的审批一并取消，避免场景永远挂起
  function cancelWaiting(thread) {
    if (!thread) return;
    (thread.messages || []).forEach(function (m) {
      (m.blocks || []).forEach(function (b) {
        if (b.type !== 'approval' || b.status !== 'waiting') return;
        b.status = 'denied';
        b.decision = 'deny';
        b.scope = 'deny';
        b.decidedAt = U.timeNow();
        b.cancelNote = true;
        const resolver = S.approvalResolvers[b.id];
        if (resolver) {
          delete S.approvalResolvers[b.id];
          resolver('deny');
        }
      });
      patchMessage(m);
    });
  }

  /* ---------------- 滚动 ---------------- */
  function scrollToBottom() {
    const host = U.q('#chat-scroll');
    if (!host) return;
    host.scrollTop = host.scrollHeight;
    const jump = U.q('#jump-bottom');
    if (jump) jump.hidden = true;
    const thread = CW.currentThread();
    if (thread) S.scrollFollow[thread.id] = true;
  }

  function bindScroll() {
    const host = U.q('#chat-scroll');
    if (!host) return;
    host.addEventListener('scroll', function () {
      const thread = CW.currentThread();
      const distance = host.scrollHeight - host.scrollTop - host.clientHeight;
      const atBottom = distance < 80;
      if (thread) S.scrollFollow[thread.id] = atBottom;
      const jump = U.q('#jump-bottom');
      if (jump) jump.hidden = atBottom;
    });
  }

  /* ---------------- 复制 / 草稿 ---------------- */
  function messagePlainText(msg) {
    return (msg.blocks || [])
      .filter(function (b) {
        return b.type === 'text';
      })
      .map(function (b) {
        return b.text;
      })
      .join('\n\n');
  }

  /* ---------------- 事件 ---------------- */
  function bindEvents() {
    const stream = U.q('#chat-stream');

    if (stream) {
      stream.addEventListener('click', function (e) {
        const target = e.target;

        // 推理折叠
        const reasoningBtn = target.closest('[data-toggle-reasoning]');
        if (reasoningBtn) {
          const box = reasoningBtn.closest('.reasoning');
          box.classList.toggle('is-open');
          return;
        }

        // 工具卡片展开
        const toolToggle = target.closest('[data-tool-toggle]');
        if (toolToggle) {
          toolToggle.closest('.tool-card').classList.toggle('is-open');
          return;
        }

        // 复制工具输出
        const copyTool = target.closest('[data-copy-tool]');
        if (copyTool) {
          const thread = CW.currentThread();
          const bi = Number(copyTool.getAttribute('data-copy-tool'));
          const msg = findMessageByEl(copyTool);
          const block = msg && msg.blocks[bi];
          if (block) {
            U.copyText(block.output || '').then(function (ok) {
              CW.toast(ok ? { kind: 'ok', title: '已复制工具输出' } : { kind: 'err', title: '复制失败' });
            });
          }
          return;
        }

        // 审批决定：通过消息 id 与块序号直接定位，避免依赖审批 id 的唯一性
        const decide = target.closest('[data-decision]');
        if (decide) {
          const card = decide.closest('.approval-card');
          const msgEl = card.closest('.msg');
          const thread = CW.currentThread();
          const msgId = msgEl ? msgEl.getAttribute('data-mid') : null;
          const bi = Number(card.getAttribute('data-bid'));
          const msg = thread && thread.messages.find(function (m) {
            return m.id === msgId;
          });
          const block = msg && msg.blocks[bi];
          if (block) decideApproval(thread, block, decide.getAttribute('data-decision'));
          return;
        }

        // 打开完整差异
        const openDiff = target.closest('[data-open-diff]');
        if (openDiff) {
          const thread = CW.currentThread();
          if (thread) {
            thread.panel = { tab: 'diff' };
            S.panelTab[thread.mode] = 'diff';
            CW.ui.refresh();
          }
          return;
        }

        // 草稿复制
        const draftCopy = target.closest('[data-draft-copy]');
        if (draftCopy) {
          const msg = findMessageByEl(draftCopy);
          const bi = Number(draftCopy.getAttribute('data-draft-copy'));
          const block = msg && msg.blocks[bi];
          if (block) {
            const text = '收件人：' + block.to + '\n主题：' + block.subject + '\n\n' + block.body;
            U.copyText(text).then(function (ok) {
              CW.toast(ok ? { kind: 'ok', title: '草稿已复制', desc: '请粘贴到邮件客户端后自行发送' } : { kind: 'err', title: '复制失败' });
            });
          }
          return;
        }

        // 消息操作
        const msgAct = target.closest('[data-msg-act]');
        if (msgAct) {
          handleMsgAction(msgAct.getAttribute('data-msg-act'), msgAct.getAttribute('data-mid'));
          return;
        }

        // 错误操作
        const errAct = target.closest('[data-err-act]');
        if (errAct) {
          handleErrAction(errAct.getAttribute('data-err-act'));
          return;
        }

        // 代码块操作
        const codeAct = target.closest('[data-code-act]');
        if (codeAct) {
          handleCodeAction(codeAct);
          return;
        }

        // 文件引用跳转
        const ref = target.closest('[data-ref-path]');
        if (ref) {
          CW.openFile(ref.getAttribute('data-ref-path'), Number(ref.getAttribute('data-ref-line')));
          return;
        }

        // 外链确认
        const link = target.closest('.md-link');
        if (link) {
          const href = link.getAttribute('data-href') || '';
          const domain = href.replace(/^https?:\/\//, '').split('/')[0];
          CW.confirmDialog({
            title: '用系统浏览器打开链接？',
            desc: '外部链接不会在应用内加载。请确认域名后继续。',
            detail: domain + ' · ' + href,
            icon: 'external-link',
            confirmText: '打开链接',
            onConfirm: function () {
              CW.toast({ kind: 'info', title: '已请求系统浏览器打开', desc: domain + '（原型不实际打开页面）' });
            },
          });
          return;
        }

        // 空状态建议
        const suggestion = target.closest('[data-suggestion]');
        if (suggestion) {
          const input = U.q('#composer-input');
          if (input) {
            input.value = suggestion.getAttribute('data-suggestion');
            U.autoGrow(input);
            updateSendAvailability();
            input.focus();
          }
          return;
        }
      });
    }

    // 头部操作
    const head = U.q('#chat-head');
    if (head) {
      head.addEventListener('click', function (e) {
        const btn = e.target.closest('[data-head-act]');
        if (!btn) return;
        handleHeadAction(btn.getAttribute('data-head-act'));
      });
    }

    // 输入框
    const input = U.q('#composer-input');
    if (input) {
      input.addEventListener('input', function () {
        U.autoGrow(input);
        updateSendAvailability();
      });
      input.addEventListener('keydown', function (e) {
        if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
          e.preventDefault();
          sendMessage();
        }
      });
      input.addEventListener('paste', function (e) {
        const items = e.clipboardData && e.clipboardData.items;
        if (!items) return;
        for (let i = 0; i < items.length; i++) {
          if (items[i].type.indexOf('image') === 0) {
            const file = items[i].getAsFile();
            if (file) {
              addAttachment({ kind: 'image', name: '粘贴的截图.png', size: '248 KB' });
              e.preventDefault();
            }
          }
        }
      });
    }

    const sendBtn = U.q('#btn-send');
    if (sendBtn) sendBtn.addEventListener('click', sendMessage);

    const stopBtn = U.q('#btn-stop');
    if (stopBtn) {
      stopBtn.addEventListener('click', function () {
        const thread = CW.currentThread();
        if (!thread) return;
        S.abortFlags[thread.id] = true;
        thread.status = 'idle';
        cancelWaiting(thread);
        CW.toast({ kind: 'info', title: '已请求停止', desc: '底层请求已取消，子进程正在清理。' });
        renderComposer();
        CW.ui.refresh();
      });
    }

    const jump = U.q('#jump-bottom');
    if (jump) jump.addEventListener('click', scrollToBottom);

    // 附件按钮
    const attach = U.q('#btn-attach');
    if (attach) {
      attach.addEventListener('click', function () {
        CW.popover(
          attach,
          '<div class="popover-label">添加附件（演示）</div>' +
            [
              { name: '接口日志.txt', size: '12 KB', kind: 'file' },
              { name: 'session.go', size: '2.1 KB', kind: 'file' },
              { name: '错误截图.png', size: '320 KB', kind: 'image' },
              { name: '2026-Q3-预算表.xlsx', size: '22 KB', kind: 'doc' },
            ]
              .map(function (f, i) {
                return (
                  '<button type="button" class="popover-item" data-attach-demo="' + i + '">' +
                  U.icon(f.kind === 'image' ? 'image' : f.kind === 'doc' ? 'file-text' : 'file') +
                  '<span class="pi-main"><div class="pi-title">' + f.name + '</div>' +
                  '<div class="pi-desc">' + f.size + ' · 单文件上限 50 MB</div></span></button>'
                );
              })
              .join('') +
            '<div class="popover-sep"></div>' +
            '<div class="popover-note">原型演示：选择后会加入附件区，不会真正上传。单条最多 20 个附件。</div>',
          {
            align: 'left',
            onMount: function (el) {
              const files = [
                { name: '接口日志.txt', size: '12 KB', kind: 'file' },
                { name: 'session.go', size: '2.1 KB', kind: 'file' },
                { name: '错误截图.png', size: '320 KB', kind: 'image' },
                { name: '2026-Q3-预算表.xlsx', size: '22 KB', kind: 'doc' },
              ];
              el.querySelectorAll('[data-attach-demo]').forEach(function (b) {
                b.addEventListener('click', function () {
                  addAttachment(files[Number(b.getAttribute('data-attach-demo'))]);
                  CW.closePopover();
                });
              });
            },
          }
        );
      });
    }

    const imageBtn = U.q('#btn-image');
    if (imageBtn) {
      imageBtn.addEventListener('click', function () {
        addAttachment({ kind: 'image', name: '设计稿-登录页.png', size: '1.2 MB' });
      });
    }

    // 审批档位入口：位于模型选择之前
    const composerApproval = U.q('#composer-approval');
    if (composerApproval) {
      composerApproval.addEventListener('click', function (e) {
        e.stopPropagation();
        openApprovalMenu(composerApproval);
      });
    }

    // 模型切换入口：位于输入框底部
    const composerModel = U.q('#composer-model');
    if (composerModel) {
      composerModel.addEventListener('click', function (e) {
        e.stopPropagation();
        const thread = CW.currentThread();
        if (thread) openModelMenu(composerModel, thread);
      });
    }

    // 附件移除
    const strip = U.q('#attach-strip');
    if (strip) {
      strip.addEventListener('click', function (e) {
        const btn = e.target.closest('[data-attach-remove]');
        if (!btn) return;
        S.attachments.splice(Number(btn.getAttribute('data-attach-remove')), 1);
        renderAttachments();
        updateSendAvailability();
      });
    }

    // 拖放
    const pane = U.q('.chat-pane');
    if (pane) {
      const overlay = document.createElement('div');
      overlay.className = 'drop-overlay';
      overlay.innerHTML =
        '<div class="drop-box">' + U.icon('download') +
        '<b>松开以添加附件</b><span>文件会附加到下一条消息；目录可作为工作区添加</span></div>';
      pane.appendChild(overlay);

      let depth = 0;
      pane.addEventListener('dragenter', function (e) {
        e.preventDefault();
        depth++;
        pane.classList.add('is-dragover');
        U.q('#composer').classList.add('is-dragover');
      });
      pane.addEventListener('dragover', function (e) {
        e.preventDefault();
      });
      pane.addEventListener('dragleave', function (e) {
        depth--;
        if (depth <= 0) {
          pane.classList.remove('is-dragover');
          U.q('#composer').classList.remove('is-dragover');
          depth = 0;
        }
      });
      pane.addEventListener('drop', function (e) {
        e.preventDefault();
        depth = 0;
        pane.classList.remove('is-dragover');
        U.q('#composer').classList.remove('is-dragover');
        const files = e.dataTransfer && e.dataTransfer.files;
        if (files && files.length) {
          for (let i = 0; i < Math.min(files.length, 20); i++) {
            const f = files[i];
            addAttachment({
              kind: f.type.indexOf('image') === 0 ? 'image' : 'file',
              name: f.name,
              size: f.size ? Math.round(f.size / 1024) + ' KB' : '',
            });
          }
          CW.toast({ kind: 'ok', title: '已添加 ' + Math.min(files.length, 20) + ' 个附件' });
        }
      });
    }
  }

  function addAttachment(file) {
    if (S.attachments.length >= 20) {
      CW.toast({ kind: 'warn', title: '单条消息最多 20 个附件' });
      return;
    }
    S.attachments.push(file);
    renderAttachments();
    updateSendAvailability();
    U.q('#composer-input').focus();
  }

  function findMessageByEl(el) {
    const msgEl = el.closest('.msg');
    if (!msgEl) return null;
    const id = msgEl.getAttribute('data-mid');
    const thread = CW.currentThread();
    return thread.messages.find(function (m) {
      return m.id === id;
    });
  }

  function handleMsgAction(action, msgId) {
    const thread = CW.currentThread();
    const msg = thread.messages.find(function (m) {
      return m.id === msgId;
    });
    if (!msg) return;

    if (action === 'copy') {
      U.copyText(messagePlainText(msg)).then(function (ok) {
        CW.toast(ok ? { kind: 'ok', title: '已复制消息' } : { kind: 'err', title: '复制失败' });
      });
    } else if (action === 'copy-answer') {
      U.copyText(messagePlainText(msg)).then(function (ok) {
        CW.toast(ok ? { kind: 'ok', title: '已复制回复' } : { kind: 'err', title: '复制失败' });
      });
    } else if (action === 'regenerate') {
      CW.Scene.regenerate(thread, msg);
    } else if (action === 'branch') {
      CW.Scene.branchThread(thread, msg);
    } else if (action === 'edit') {
      const input = U.q('#composer-input');
      input.value = messagePlainText(msg);
      U.autoGrow(input);
      updateSendAvailability();
      input.focus();
      CW.toast({ kind: 'info', title: '已放入输入框', desc: '修改后发送将创建新一轮对话' });
    }
  }

  function handleErrAction(action) {
    if (action === 'retry') {
      CW.toast({ kind: 'info', title: '已重试', desc: '使用相同参数重新发起请求（原型演示）' });
    } else if (action === 'model') {
      const btn = U.q('#composer-model');
      if (btn) btn.click();
    } else if (action === 'copy') {
      U.copyText('context_overflow: 上下文超出模型窗口 (121k / 128k)').then(function () {
        CW.toast({ kind: 'ok', title: '错误信息已复制' });
      });
    }
  }

  function handleCodeAction(btn) {
    const block = btn.closest('.code-block');
    if (!block) return;
    const act = btn.getAttribute('data-code-act');
    if (act === 'copy') {
      U.copyText(block.getAttribute('data-code') || '').then(function (ok) {
        CW.toast(ok ? { kind: 'ok', title: '已复制代码', desc: '复制内容不包含行号' } : { kind: 'err', title: '复制失败' });
      });
    } else if (act === 'lines') {
      btn.classList.toggle('is-on');
      block.classList.toggle('no-lines', !btn.classList.contains('is-on'));
    } else if (act === 'wrap') {
      btn.classList.toggle('is-on');
      block.classList.toggle('wrap');
    } else if (act === 'expand') {
      block.classList.remove('collapsed');
    }
  }

  function handleHeadAction(action) {
    if (action === 'panel') {
      CW.togglePanel();
      return;
    }
    const thread = CW.currentThread();
    if (!thread) return;
    if (action === 'rename') {
      CW.inputDialog({
        title: '重命名线程',
        label: '线程标题',
        value: thread.title,
        confirmText: '保存',
        onConfirm: function (value) {
          if (!value.trim()) return;
          thread.title = value.trim();
          renderHead();
          CW.renderThreads();
          CW.toast({ kind: 'ok', title: '已重命名', desc: '之后的自动标题不会覆盖手动改名' });
        },
      });
    } else if (action === 'copy') {
      CW.Scene.copyThread(thread);
    } else if (action === 'archive') {
      CW.confirmDialog({
        title: '归档这个线程？',
        desc: '归档后从当前列表移除，可在归档中找到并恢复。不会删除磁盘上的任何文件。',
        confirmText: '归档',
        onConfirm: function () {
          CW.Scene.archiveThread(thread);
        },
      });
    } else if (action === 'more') {
      const anchor = U.q('[data-head-act="more"]');
      CW.popover(
        anchor,
        '<div class="popover-label">线程操作</div>' +
          '<button type="button" class="popover-item" data-th-act="rename">' + U.icon('edit') +
          '<span class="pi-main"><div class="pi-title">重命名</div></span></button>' +
          '<button type="button" class="popover-item" data-th-act="copy">' + U.icon('copy') +
          '<span class="pi-main"><div class="pi-title">复制为新线程</div></span></button>' +
          '<button type="button" class="popover-item" data-th-act="archive">' + U.icon('archive') +
          '<span class="pi-main"><div class="pi-title">归档</div></span></button>' +
          '<div class="popover-sep"></div>' +
          '<button type="button" class="popover-item danger" data-th-act="delete">' + U.icon('trash') +
          '<span class="pi-main"><div class="pi-title">软删除</div><div class="pi-desc">7 天内可恢复</div></span></button>' +
          '<div class="popover-note">软删除 7 天后清理事件与本地大文件；彻底删除入口在设置 · 高级。</div>',
        {
          onMount: function (el) {
            el.querySelectorAll('[data-th-act]').forEach(function (b) {
              b.addEventListener('click', function () {
                const act = b.getAttribute('data-th-act');
                CW.closePopover();
                if (act === 'delete') {
                  CW.confirmDialog({
                    title: '软删除线程？',
                    desc: '线程会移入回收站，7 天内可以恢复。审计记录不会删除。',
                    detail: thread.title,
                    danger: true,
                    confirmText: '软删除',
                    onConfirm: function () {
                      CW.Scene.softDeleteThread(thread);
                    },
                  });
                } else {
                  handleHeadAction(act);
                }
              });
            });
          },
        }
      );
    }
  }

  /* ---------------- 对外接口 ---------------- */
  CW.Chat = {
    render: function () {
      renderHead();
      renderMessages();
      renderComposer();
    },
    renderHead: renderHead,
    renderMessages: renderMessages,
    renderComposer: renderComposer,
    patchMessage: patchMessage,
    streamText: streamText,
    appendBlock: appendBlock,
    addMessage: addMessage,
    addAttachment: addAttachment,
    scrollToBottom: scrollToBottom,
    sendMessage: sendMessage,
    requestApproval: requestApproval,
    decideApproval: decideApproval,
    cancelWaiting: cancelWaiting,
    allApprovals: allApprovals,
    bindEvents: bindEvents,
    bindScroll: bindScroll,
    updateThreadMeta: updateThreadMeta,
    updateSendAvailability: updateSendAvailability,
    messagePlainText: messagePlainText,
    findMessageByEl: findMessageByEl,
    setApprovalMode: setApprovalMode,
    approvalMode: approvalMode,
    approvalLabel: function () {
      return approvalModeInfo().label;
    },
  };
})();
