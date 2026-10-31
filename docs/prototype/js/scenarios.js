/* ==========================================================================
   CraftWork 高保真原型 · 演示场景
   模拟一轮 run：推理 → 文本流式 → 工具调用 → 审批 → 执行 → 总结。
   预置线程的审批卡在页面加载时已存在，决定后由 resumeAfterApproval 继续。
   ========================================================================== */
(function () {
  const CW = window.CW;
  const U = CW.util;
  const S = CW.State;
  const D = CW.Data;

  /* ---------------- 基础工具 ---------------- */
  function sleepAbortable(thread, ms) {
    return new Promise(function (resolve) {
      const started = Date.now();
      (function tick() {
        if (S.abortFlags[thread.id] || Date.now() - started >= ms) {
          resolve();
          return;
        }
        setTimeout(tick, 50);
      })();
    });
  }

  function aborted(thread) {
    return !!S.abortFlags[thread.id];
  }

  function isActive(thread) {
    return CW.currentThread() && CW.currentThread().id === thread.id;
  }

  function refresh(thread) {
    if (isActive(thread)) CW.ui.refresh();
  }

  function newAssistant(thread) {
    const msg = {
      id: U.uid('m'),
      role: 'assistant',
      time: U.timeNow(),
      streaming: false,
      blocks: [],
    };
    CW.Chat.addMessage(thread, msg);
    return msg;
  }

  function assistantOf(thread) {
    const list = (thread.messages || []).filter(function (m) {
      return m.role === 'assistant';
    });
    return list[list.length - 1];
  }

  function patch(thread, msg) {
    if (isActive(thread)) CW.Chat.patchMessage(msg);
  }

  async function say(thread, msg, text) {
    msg.streaming = true;
    patch(thread, msg);
    const block = CW.Chat.appendBlock(msg, { type: 'text', text: '' });
    const index = msg.blocks.length - 1;
    const chars = Array.from(text);
    let current = '';
    if (isActive(thread)) {
      for (let i = 0; i < chars.length; i += 2) {
        if (aborted(thread)) break;
        current += chars.slice(i, i + 2).join('');
        CW.Chat.streamText(msg.id, index, current);
        await sleepAbortable(thread, 14);
      }
    }
    block.text = aborted(thread) ? current || text : text;
    msg.streaming = false;
    patch(thread, msg);
    if (isActive(thread)) CW.Chat.scrollToBottom();
    return block;
  }

  async function reason(thread, msg, text, duration) {
    const block = CW.Chat.appendBlock(msg, {
      type: 'reasoning',
      text: text,
      duration: duration || '4 秒',
      open: false,
    });
    await sleepAbortable(thread, 220);
    return block;
  }

  function toolData(data) {
    return Object.assign(
      {
        type: 'tool',
        id: U.uid('tool'),
        status: 'running',
        duration: '',
        open: false,
      },
      data
    );
  }

  async function runTool(thread, msg, data, ms) {
    const block = CW.Chat.appendBlock(msg, toolData(data));
    await sleepAbortable(thread, ms);
    block.status = 'done';
    block.duration = (ms / 1000).toFixed(2) + 's';
    block.open = true;
    patch(thread, msg);
    return block;
  }

  function appendApproval(thread, msg, data) {
    const block = Object.assign(
      {
        type: 'approval',
        id: U.uid('ap'),
        status: 'waiting',
        decision: null,
        scope: null,
        decidedAt: null,
      },
      data
    );
    CW.Chat.appendBlock(msg, block);
    thread.status = 'awaiting';
    refresh(thread);
    return block;
  }

  async function askApproval(thread, msg, data) {
    const block = appendApproval(thread, msg, data);
    const decision = await CW.Chat.requestApproval(block);
    thread.status = 'running';
    refresh(thread);
    return decision;
  }

  function updateTodos(thread, msg, doneFlags, activeIndex) {
    const block = msg.blocks.find(function (b) {
      return b.type === 'todo';
    });
    if (!block) return;
    block.items.forEach(function (item, i) {
      item.done = !!doneFlags[i];
      item.active = i === activeIndex;
    });
    patch(thread, msg);
  }

  function stopNotice(thread, msg) {
    if (msg) {
      CW.Chat.appendBlock(msg, {
        type: 'notice',
        icon: 'stop',
        text: '已停止生成',
        variant: '',
      });
    }
    thread.status = 'idle';
    S.abortFlags[thread.id] = false;
    refresh(thread);
    CW.toast({ kind: 'info', title: '已停止生成', desc: '请求已取消，没有残留进程。' });
  }

  /* ---------------- 预置线程的审批后续 ---------------- */
  async function resumeAfterApproval(thread, block, decision) {
    if (block.action === 'c1-write') return codeFixAfterWrite(thread, block, decision);
    if (block.action === 'w1-doc') return workDocAfterWrite(thread, block, decision);
    if (block.action === 'w3-xlsx') return workXlsxAfterWrite(thread, block, decision);
  }

  async function codeFixAfterWrite(thread, approval, decision) {
    const msg = assistantOf(thread);
    if (!msg) return;
    if (decision === 'deny') {
      thread.status = 'idle';
      refresh(thread);
      await say(
        thread,
        msg,
        '已取消写入，`internal/auth/session.go` 保持不变。如果你希望先看补丁内容，我可以把完整 diff 输出到聊天里，由你手动应用。'
      );
      return;
    }

    thread.status = 'running';
    refresh(thread);
    await sleepAbortable(thread, 520);
    if (aborted(thread)) return stopNotice(thread, msg);

    await runTool(
      thread,
      msg,
      {
        name: 'fs_edit',
        risk: 'R1',
        summary: 'internal/auth/session.go',
        meta: '+18 −7',
        status: 'done',
        open: true,
        args: [
          ['path', 'internal/auth/session.go'],
          ['strategy', 'replace'],
          ['backup', 'session.go.bak-20261031-1412'],
        ],
        output:
          '已替换 1 处代码块\n备份：~/.craftwork/backups/session.go.bak-20261031-141208\n备份大小：2.1 KB',
        footer: 'R1 写工作区 · 已备份',
      },
      420
    );

    await runTool(
      thread,
      msg,
      {
        name: 'fs_write',
        risk: 'R1',
        summary: 'internal/auth/session_test.go',
        meta: '新建 · 31 行',
        status: 'done',
        open: false,
        args: [['path', 'internal/auth/session_test.go'], ['mode', 'create']],
        output: '已创建 internal/auth/session_test.go（31 行）',
        footer: '同一次审批范围内',
      },
      320
    );

    updateTodos(thread, msg, [true, true, true, 'active'], 3);
    await say(thread, msg, '代码和测试都已写入。现在运行认证包的测试，确认刷新期间读路径不会被阻塞。');

    const testDecision = await askApproval(thread, msg, {
      action: 'c1-test',
      risk: 'R2',
      tool: 'shell_exec',
      target: 'go test ./internal/auth/... -race -count=1',
      summary: '运行认证包测试，带竞态检测。命令在工作区根目录执行，默认无网络访问。',
      facts: [
        { icon: 'terminal', text: 'R2 执行' },
        { icon: 'clock', text: '超时 120 秒' },
        { icon: 'lock', text: '仅工作区 · 无网络' },
      ],
      diffPreview: [
        { t: 'ctx', ln: 1, text: '$ go test ./internal/auth/... -race -count=1' },
        { t: 'ctx', ln: 2, text: '# 工作目录：/Users/yangzhenbiao/Code/craftwork' },
        { t: 'ctx', ln: 3, text: '# 网络：拒绝 · 超时：120s' },
      ],
    });

    if (testDecision === 'deny') {
      thread.status = 'idle';
      refresh(thread);
      CW.Chat.appendBlock(msg, {
        type: 'callout',
        variant: 'warn',
        title: '测试未执行',
        text: '代码修改已写入并备份。你可以稍后手动运行 `go test ./internal/auth/... -race`。',
      });
      return;
    }

    const testTool = await runTool(
      thread,
      msg,
      {
        name: 'shell_exec',
        risk: 'R2',
        summary: 'go test ./internal/auth/... -race -count=1',
        meta: '退出码 0',
        status: 'running',
        open: true,
        args: [['command', 'go test ./internal/auth/... -race -count=1'], ['cwd', '.'], ['network', 'deny']],
        output: '正在编译并运行测试…',
        footer: 'R2 执行 · 输出上限 32 KB',
      },
      900
    );
    testTool.output =
      '=== RUN   TestRefreshSessionConcurrent\n' +
      '--- PASS: TestRefreshSessionConcurrent (0.42s)\n' +
      '=== RUN   TestRefreshSessionDoesNotBlockReads\n' +
      '--- PASS: TestRefreshSessionDoesNotBlockReads (0.21s)\n' +
      'PASS\n' +
      'ok  \tcraftwork/internal/auth\t1.284s';
    testTool.meta = '退出码 0 · 1.28s';
    patch(thread, msg);

    updateTodos(thread, msg, [true, true, true, true], -1);
    await say(
      thread,
      msg,
      '测试全部通过，读路径在刷新期间不再阻塞。\n\n**修改摘要**\n\n- `internal/auth/session.go`：网络调用移出写锁，回写前二次检查\n- `internal/auth/session_test.go`：新增刷新期间读路径不被阻塞的测试\n\n差异已在右侧面板生成，你可以逐块应用或跳过。'
    );

    thread.status = 'idle';
    thread.updated = '刚刚';
    S.codeApplied = false;
    refresh(thread);
    CW.renderThreads && CW.renderThreads();
    CW.toast({ kind: 'ok', title: 'run 完成', desc: '测试通过 · 差异待你应用' });
  }

  async function workDocAfterWrite(thread, approval, decision) {
    const msg = assistantOf(thread);
    if (!msg) return;
    if (decision === 'deny') {
      thread.status = 'idle';
      refresh(thread);
      await say(thread, msg, '已取消写入，文档保持原样。如需先查看完整差异，可以点审批卡片上的“查看完整差异”。');
      return;
    }

    thread.status = 'running';
    refresh(thread);
    await sleepAbortable(thread, 650);
    if (aborted(thread)) return stopNotice(thread, msg);

    const doc = D.documents['docs/2026-Q3-产品评审纪要.docx'];
    const table = doc.pages[0].blocks.find(function (b) {
      return b.id === 'todo-table';
    });
    if (table) {
      table.rows[0][1] = '陈磊';
      table.changedCells = [[0, 1]];
    }
    S.docxApplied = true;
    S.doc.externalNotice = false;
    S.panelTab.work = 'document';
    S.doc.path = 'docs/2026-Q3-产品评审纪要.docx';

    await runTool(
      thread,
      msg,
      {
        name: 'doc_write',
        risk: 'R1',
        summary: 'docs/2026-Q3-产品评审纪要.docx',
        meta: '2 处修改 · 保留样式',
        status: 'done',
        open: true,
        args: [
          ['path', 'docs/2026-Q3-产品评审纪要.docx'],
          ['strategy', 'text-level'],
          ['backup', 'docx.bak-20261031-1502'],
        ],
        output:
          '已修正表格单元格：表格 1 / 第 2 行 / 负责人\n已插入表格：待办事项汇总（5 行 × 3 列）\n原有样式与结构保持不变\n备份：2026-Q3-产品评审纪要.docx.bak-20261031-1502',
        footer: '文档写入 · 已备份 · 未执行宏',
      },
      620
    );

    await say(
      thread,
      msg,
      '文档已更新：修正了 1 处姓名笔误，并新增 5 行待办汇总表。原文件已备份，右侧可以看到修改后的文档与文本级差异。\n\n如果需要调整表格列或补充负责人，告诉我即可。'
    );
    CW.Chat.appendBlock(msg, {
      type: 'callout',
      variant: 'ok',
      title: '写入完成',
      text: '备份保留在本地，可在文档工具栏的“备份”中按时间恢复。',
    });

    thread.status = 'idle';
    refresh(thread);
    CW.toast({ kind: 'ok', title: '文档已保存', desc: '原文件已备份 · 可恢复' });
  }

  async function workXlsxAfterWrite(thread, approval, decision) {
    const msg = assistantOf(thread);
    if (!msg) return;
    if (decision === 'deny') {
      thread.status = 'idle';
      refresh(thread);
      await say(thread, msg, '已取消写入，公式保持原样。');
      return;
    }

    thread.status = 'running';
    refresh(thread);
    await sleepAbortable(thread, 520);

    const xlsx = D.documents['docs/2026-Q3-预算表.xlsx'];
    const sheet = xlsx.sheets[0];
    sheet.rows[7][1] = '=SUM(B3:B7)';
    sheet.rows[7][2] = '=SUM(C3:C7)';
    sheet.computed.B8 = '520,000';
    sheet.computed.C8 = '505,200';
    sheet.computed.D8 = '-14,800';
    xlsx.changedCells = ['B8', 'C8', 'D8'];
    S.xlsxApplied = true;
    S.panelTab.work = 'document';
    S.doc.path = xlsx.path;
    S.doc.cell = 'B8';

    await runTool(
      thread,
      msg,
      {
        name: 'doc_write',
        risk: 'R1',
        summary: 'docs/2026-Q3-预算表.xlsx',
        meta: '2 个公式',
        status: 'done',
        open: true,
        args: [['path', 'docs/2026-Q3-预算表.xlsx'], ['strategy', 'cell-level'], ['backup', 'xlsx.bak-20261030-1822']],
        output: '预算总表!B8 = SUM(B3:B7) → 520,000\n预算总表!C8 = SUM(C3:C7) → 505,200\n预算总表!D8 = C8-B8 → -14,800\n样式与结构保持不变',
        footer: '文档写入 · 已备份',
      },
      520
    );

    await say(
      thread,
      msg,
      '公式已修正。`预算总表` 的合计现在包含第 7 行“差旅与招待”，差异列随之重算为 **−14,800**。右侧可以直接看到高亮的单元格。'
    );

    thread.status = 'idle';
    refresh(thread);
    CW.toast({ kind: 'ok', title: '预算表已更新', desc: '2 个公式已写入并备份' });
  }

  /* ---------------- 通用场景 ---------------- */
  function introForCode(text) {
    if (/测试|test/i.test(text)) return '明白，我先确认测试范围与相关包，再执行最小必要的测试。';
    if (/修|改|bug|超时|问题/i.test(text)) return '我先读相关文件与最近的改动，确认根因后再动手修改。';
    if (/结构|梳理|了解|接手/i.test(text)) return '我先把工作区列一遍，再读关键文件，最后给出结构和测试方式。';
    return '我先看一下当前工作区，确认相关文件后再继续。';
  }

  async function genericCode(thread, text) {
    const msg = newAssistant(thread);
    thread.status = 'running';
    refresh(thread);

    try {
      await reason(thread, msg, '先判断任务属于读取、修改还是执行；读取可以直接做，修改与命令需要审批。', '3 秒');
      if (aborted(thread)) return stopNotice(thread, msg);
      await say(thread, msg, introForCode(text));
      if (aborted(thread)) return stopNotice(thread, msg);

      await runTool(
        thread,
        msg,
        {
          name: 'fs_list',
          risk: 'R0',
          summary: '. · 深度 1',
          meta: '9 项',
          status: 'running',
          open: false,
          args: [['path', '.'], ['depth', '1']],
          output: 'cmd/  internal/  docs/  go.mod  go.sum  README.md  .gitignore',
          footer: '只读 · 遵守忽略规则',
        },
        480
      );
      if (aborted(thread)) return stopNotice(thread, msg);

      if (/测试|test/i.test(text)) {
        await say(thread, msg, '工作区已加载。执行完整测试会运行多个包，预计 40 秒，需要你批准这一次 R2 执行。');
        const decision = await askApproval(thread, msg, {
          action: 'generic-test',
          risk: 'R2',
          tool: 'shell_exec',
          target: 'go test ./... -race -count=1',
          summary: '在工作区根目录运行完整测试，带竞态检测；默认无网络访问。',
          facts: [
            { icon: 'terminal', text: 'R2 执行' },
            { icon: 'clock', text: '预计 40 秒' },
            { icon: 'lock', text: '无网络 · 仅工作区' },
          ],
          diffPreview: [
            { t: 'ctx', ln: 1, text: '$ go test ./... -race -count=1' },
            { t: 'ctx', ln: 2, text: '# 超时 120s · 输出上限 32 KB' },
          ],
        });
        if (decision === 'deny') {
          await say(thread, msg, '已取消测试执行。你可以在终端里手动运行，或者告诉我只跑某一个包。');
          thread.status = 'idle';
          refresh(thread);
          return;
        }
        await runTool(
          thread,
          msg,
          {
            name: 'shell_exec',
            risk: 'R2',
            summary: 'go test ./... -race -count=1',
            meta: '退出码 0',
            status: 'running',
            open: true,
            args: [['command', 'go test ./... -race -count=1'], ['cwd', '.'], ['network', 'deny']],
            output: 'ok  \tcraftwork/internal/agent\t2.104s\nok  \tcraftwork/internal/auth\t1.284s\nok  \tcraftwork/internal/store\t0.913s\nok  \tcraftwork/internal/ui\t1.640s',
            footer: '输出已截断至 32 KB',
          },
          900
        );
        await say(thread, msg, '全部测试通过。当前工作区没有失败用例，也没有竞态告警。');
      } else if (/修|改|bug|超时|问题/i.test(text)) {
        await say(
          thread,
          msg,
          '我已经定位到相关文件。接下来会修改 `internal/auth/session.go`，把网络调用移出写锁，并在回写前做二次检查。这个修改属于 R1，需要你批准。'
        );
        const decision = await askApproval(thread, msg, {
          risk: 'R1',
          tool: 'fs_edit',
          target: 'internal/auth/session.go',
          summary: '调整刷新逻辑的锁范围，避免网络抖动时阻塞整个会话表。',
          facts: [
            { icon: 'file-code', text: '1 个文件' },
            { icon: 'hard-drive', text: '写入前备份' },
            { icon: 'shield', text: 'R1 写工作区' },
          ],
          diffPreview: [
            { t: 'del', ln: 38, text: '  s.mu.Lock()' },
            { t: 'add', ln: 38, text: '  s.mu.Lock()' },
            { t: 'add', ln: 39, text: '  sess, ok := s.sessions[sid]' },
            { t: 'add', ln: 40, text: '  s.mu.Unlock()' },
            { t: 'ctx', ln: 48, text: '  tok, err := s.client.Exchange(ctx, sess.Token)' },
          ],
        });
        if (decision === 'deny') {
          await say(thread, msg, '已取消修改，文件保持不变。需要我改为输出补丁吗？');
          thread.status = 'idle';
          refresh(thread);
          return;
        }
        await runTool(
          thread,
          msg,
          {
            name: 'fs_edit',
            risk: 'R1',
            summary: 'internal/auth/session.go',
            meta: '+18 −7',
            status: 'running',
            open: true,
            args: [['path', 'internal/auth/session.go'], ['strategy', 'replace']],
            output: '已写入 internal/auth/session.go\n备份：session.go.bak-' + Date.now(),
            footer: 'R1 写工作区 · 已备份',
          },
          620
        );
        await say(thread, msg, '修改已完成并备份。差异已同步到右侧面板，你可以逐块应用或跳过。');
      } else {
        await say(
          thread,
          msg,
          '工作区结构如下：`cmd/` 是入口，`internal/agent` 负责模型循环与工具，`internal/auth` 是会话与令牌，`internal/store` 是本地数据库，`internal/ui` 是原生界面。\n\n测试入口是 `go test ./... -race`。需要我深入某一块吗？'
        );
      }

      if (aborted(thread)) return stopNotice(thread, msg);
      thread.status = 'idle';
      refresh(thread);
      CW.renderThreads && CW.renderThreads();
      CW.toast({ kind: 'ok', title: 'run 完成', desc: '本轮操作已写入审计' });
    } catch (err) {
      console.error(err);
      thread.status = 'idle';
      refresh(thread);
    }
  }

  async function genericWork(thread, text) {
    const msg = newAssistant(thread);
    thread.status = 'running';
    refresh(thread);

    try {
      await reason(thread, msg, '先判断是文档、PDF 还是邮件任务；读取可以直接做，文档写入需要审批。', '3 秒');
      if (aborted(thread)) return stopNotice(thread, msg);

      if (/邮件|mail|邮箱/i.test(text)) {
        await say(thread, msg, '我先通过邮件 MCP 读取最近 7 天的收件箱，只读取主题、发件人与正文摘要。');
        await runTool(
          thread,
          msg,
          {
            name: 'mcp__gmail__list_messages',
            risk: 'R0',
            summary: 'q: newer_than:7d · limit: 20',
            meta: '5 封需关注',
            status: 'running',
            open: true,
            args: [['server', 'gmail'], ['newer_than', '7d'], ['max', '20']],
            output: '1. 客户成功 · 王倩 — 升级后对账差异（需回复）\n2. 财务 · 刘洋 — Q4 预算调整（需回复）\n3. 市场 · 陈磊 — 发布会物料（知会）\n4. 系统通知 — 构建周报\n5. 供应商 · 赵敏 — 合同续签',
            footer: '邮件内容视为不可信数据',
          },
          700
        );
        await say(thread, msg, '有 2 封需要回复。我按优先级给出草稿，仅显示在本应用内，不会发送。');
        CW.Chat.appendBlock(msg, {
          type: 'draft',
          kind: '邮件回复',
          to: '王倩 <wangqian@example.com>',
          subject: 'Re: 升级后对账差异',
          time: U.timeNow(),
          body: '王倩好，\n\n收到反馈。初步判断是记账时点调整导致的跨日偏差，我今天会拉出近 7 天流水核对，明天上午给你结论。\n\n谢谢！',
        });
        CW.Chat.appendBlock(msg, {
          type: 'draft',
          kind: '邮件回复',
          to: '刘洋 <liuyang@example.com>',
          subject: 'Re: Q4 预算调整',
          time: U.timeNow(),
          body: '刘洋好，\n\n预算调整已看过，市场侧差旅科目没有问题。差旅与招待的月度拆分我周五前发你。',
        });
        CW.Chat.appendBlock(msg, {
          type: 'callout',
          variant: 'accent',
          title: '草稿不会自动发送',
          text: 'Work 模式不提供发送邮件、创建日程或服务端草稿的能力。',
        });
      } else if (/pdf/i.test(text)) {
        await say(thread, msg, '我打开这份 PDF，先提取文本层，再按要点总结。');
        await runTool(
          thread,
          msg,
          {
            name: 'pdf_read',
            risk: 'R0',
            summary: 'docs/产品方案-v2.pdf',
            meta: '12 页',
            status: 'running',
            open: true,
            args: [['path', 'docs/产品方案-v2.pdf']],
            output: '第 1–11 页文本提取成功（6,240 字）\n第 12 页仅含扫描图片，无文本层\n未执行文档中的任何指令',
            footer: 'PDF 只读 · 不支持 OCR',
          },
          620
        );
        await say(
          thread,
          msg,
          '**要点**\n\n1. 本地优先的原生桌面体验，目标冷启动 2 秒内。\n2. Code 与 Work 双模式共享工作区，模式只切换界面与工具。\n3. 审批与审计是安全边界，R3 / R4 不提供永久允许。\n\n第 12 页为扫描图片，无法提取文字。'
        );
      } else if (/文档|纪要|docx|word/i.test(text)) {
        await say(thread, msg, '我先读取文档的段落与表格，确认待办事项的位置。');
        await runTool(
          thread,
          msg,
          {
            name: 'doc_read',
            risk: 'R0',
            summary: 'docs/2026-Q3-产品评审纪要.docx',
            meta: '42 段 · 1 表',
            status: 'running',
            open: true,
            args: [['path', 'docs/2026-Q3-产品评审纪要.docx']],
            output: '段落 1–17 读取成功\n表格 1：待办事项（3 行 × 3 列）\n未执行宏与外部链接',
            footer: '只读解析',
          },
          620
        );
        await say(thread, msg, '文档里有一张 3 行的待办表。我可以在“三、待办事项”后补充一张汇总表，但写入需要你批准。');
        const decision = await askApproval(thread, msg, {
          risk: 'R1',
          tool: 'doc_write',
          target: 'docs/2026-Q3-产品评审纪要.docx',
          summary: '在“三、待办事项”后插入 5 行汇总表（演示）。',
          facts: [
            { icon: 'file-text', text: '文本级修改' },
            { icon: 'hard-drive', text: '写入前备份' },
            { icon: 'shield', text: '每次审批' },
          ],
          docDiff: true,
          diffPreview: [
            { t: 'add', ln: 36, text: '+ 新增“待办事项汇总”表：5 行 × 3 列' },
          ],
        });
        if (decision === 'deny') {
          await say(thread, msg, '已取消写入，文档保持不变。');
          thread.status = 'idle';
          refresh(thread);
          return;
        }
        S.docxApplied = true;
        S.panelTab.work = 'document';
        S.doc.path = 'docs/2026-Q3-产品评审纪要.docx';
        await runTool(
          thread,
          msg,
          {
            name: 'doc_write',
            risk: 'R1',
            summary: 'docs/2026-Q3-产品评审纪要.docx',
            meta: '1 处插入',
            status: 'running',
            open: true,
            args: [['path', 'docs/2026-Q3-产品评审纪要.docx']],
            output: '已插入表格：待办事项汇总（5 行 × 3 列）\n原样式保持不变',
            footer: '已备份',
          },
          560
        );
        await say(thread, msg, '表格已插入，原文件已备份，可以在右侧查看修改后的文档。');
      } else {
        await say(
          thread,
          msg,
          '当前工作区里可以读取文档、PDF 与文件夹清单。告诉我要处理哪个文件，或者直接说“汇总本周邮件”，我会调用对应的只读工具。'
        );
        CW.Chat.appendBlock(msg, {
          type: 'callout',
          variant: 'accent',
          title: 'Work 模式边界',
          text: '不提供 Shell、Git 与终端；文档写入每次审批并自动备份；邮件与日历只读。',
        });
      }

      if (aborted(thread)) return stopNotice(thread, msg);
      thread.status = 'idle';
      refresh(thread);
      CW.renderThreads && CW.renderThreads();
      CW.toast({ kind: 'ok', title: 'run 完成', desc: '可在审计中查看本轮工具调用' });
    } catch (err) {
      console.error(err);
      thread.status = 'idle';
      refresh(thread);
    }
  }

  async function dispatch(thread, text) {
    if (!thread) return;
    if (thread.title === '新线程' && text) {
      thread.title = text.length > 22 ? text.slice(0, 22) + '…' : text;
    }
    thread.updated = '刚刚';
    CW.renderThreads && CW.renderThreads();
    if (thread.mode === 'code') return genericCode(thread, text);
    return genericWork(thread, text);
  }

  /* ---------------- 线程管理 ---------------- */
  function newThread() {
    const mode = S.mode;
    const id = U.uid('th');
    const thread = {
      id: id,
      mode: mode,
      title: '新线程',
      model: S.settings.defaultModel || (mode === 'code' ? 'claude-sonnet-4-5' : 'claude-sonnet-4-5'),
      modelName: '',
      status: 'idle',
      updated: '刚刚',
      tokens: 0,
      contextPct: 0,
      panel: { tab: mode === 'work' ? 'document' : 'files' },
      messages: [],
    };
    thread.modelName = (S.models.find(function (m) { return m.id === thread.model; }) || S.models[0]).name;
    S.threads[mode].unshift(thread);
    S.active[mode] = id;
    S.panelTab[mode] = mode === 'work' ? 'document' : 'files';
    S.openFile = null;
    S.doc.path = mode === 'work' ? 'docs/2026-Q3-产品评审纪要.docx' : null;
    CW.ui.refresh();
    const input = U.q('#composer-input');
    if (input) input.focus();
    CW.toast({ kind: 'ok', title: '已新建 ' + (mode === 'work' ? 'Work' : 'Code') + ' 线程', desc: '线程按模式隔离，不会混入另一种模式的历史' });
    return thread;
  }

  function copyThread(thread) {
    const id = U.uid('th');
    const copy = JSON.parse(JSON.stringify(thread));
    copy.id = id;
    copy.title = thread.title + '（副本）';
    copy.updated = '刚刚';
    copy.status = 'idle';
    copy.messages.forEach(function (m, i) {
      m.id = id + '-m' + i;
    });
    const list = S.threads[thread.mode];
    list.unshift(copy);
    S.active[thread.mode] = id;
    CW.ui.refresh();
    CW.toast({ kind: 'ok', title: '已复制线程', desc: '原线程不受影响' });
  }

  function branchThread(thread, msg) {
    const idx = thread.messages.findIndex(function (m) { return m.id === msg.id; });
    if (idx < 0) return;
    const id = U.uid('th');
    const copy = {
      id: id,
      mode: thread.mode,
      title: thread.title + '（分支）',
      model: thread.model,
      modelName: thread.modelName,
      status: 'idle',
      updated: '刚刚',
      tokens: 0,
      contextPct: 0,
      panel: thread.panel,
      messages: JSON.parse(JSON.stringify(thread.messages.slice(0, idx + 1))),
    };
    copy.messages.forEach(function (m, i) {
      m.id = id + '-m' + i;
    });
    S.threads[thread.mode].unshift(copy);
    S.active[thread.mode] = id;
    CW.ui.refresh();
    CW.toast({ kind: 'ok', title: '已创建分支线程', desc: '从该条消息开始新的对话' });
  }

  function archiveThread(thread) {
    const list = S.threads[thread.mode];
    const idx = list.findIndex(function (t) { return t.id === thread.id; });
    if (idx < 0) return;
    list.splice(idx, 1);
    if (S.active[thread.mode] === thread.id) {
      S.active[thread.mode] = list[0] ? list[0].id : null;
    }
    CW.ui.refresh();
    CW.toast({ kind: 'ok', title: '线程已归档', desc: '可在归档中恢复，磁盘文件未受影响' });
  }

  function softDeleteThread(thread) {
    const list = S.threads[thread.mode];
    const idx = list.findIndex(function (t) { return t.id === thread.id; });
    if (idx < 0) return;
    list.splice(idx, 1);
    if (S.active[thread.mode] === thread.id) {
      S.active[thread.mode] = list[0] ? list[0].id : null;
    }
    CW.ui.refresh();
    CW.toast({ kind: 'warn', title: '已软删除', desc: '7 天内可以在设置 · 高级中恢复' });
  }

  async function regenerate(thread, msg) {
    if (thread.status === 'running') {
      CW.toast({ kind: 'warn', title: '当前线程正在运行' });
      return;
    }
    const lastUser = thread.messages
      .slice(0, thread.messages.indexOf(msg))
      .reverse()
      .find(function (m) { return m.role === 'user'; });
    msg.superseded = true;
    CW.Chat.patchMessage(msg);
    if (!lastUser) {
      CW.toast({ kind: 'warn', title: '找不到对应的用户消息' });
      return;
    }
    const text = lastUser.blocks
      .filter(function (b) { return b.type === 'text'; })
      .map(function (b) { return b.text; })
      .join('\n');
    CW.toast({ kind: 'info', title: '已重新生成', desc: '旧回复标记为“已被替代”，仍可追溯' });
    if (thread.mode === 'code') await genericCode(thread, text);
    else await genericWork(thread, text);
  }

  CW.Scene = {
    dispatch: dispatch,
    resumeAfterApproval: resumeAfterApproval,
    newThread: newThread,
    copyThread: copyThread,
    branchThread: branchThread,
    archiveThread: archiveThread,
    softDeleteThread: softDeleteThread,
    regenerate: regenerate,
  };
})();
