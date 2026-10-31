/* ==========================================================================
   CraftWork 高保真原型 · Markdown 与语法高亮
   目标：无第三方依赖，支持需求文档要求的常见语法，并对未闭合的流式内容容错。
   支持：标题、段落、无序/有序列表、任务列表、引用、分隔线、表格、代码块、
        行内代码、粗体、斜体、删除线、链接、path:line 引用。
   ========================================================================== */
(function () {
  const CW = window.CW;

  function escapeHtml(text) {
    return String(text)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  /* ---------------- 语法高亮 ---------------- */
  const LANG_RULES = {
    go: {
      keywords: /\b(package|import|func|return|type|struct|interface|var|const|if|else|for|range|switch|case|default|defer|go|chan|map|select|break|continue|fallthrough|goto|nil|true|false|error|string|int|int64|bool|byte|rune|any)\b/,
      types: /\b(context|sync|time|errors|fmt|http|json|bytes|strings|os|filepath|sql|testing|Store|Session|Token|TokenClient|Run|Registry|Tool|Risk|Policy|Options|App|State|DB|Message)\b/,
    },
    js: {
      keywords: /\b(const|let|var|function|return|if|else|for|while|new|class|extends|import|export|from|default|async|await|try|catch|finally|throw|typeof|instanceof|of|in|this|null|undefined|true|false)\b/,
      types: /\b(Array|Object|String|Number|Boolean|Promise|Map|Set|Date|JSON|console|window|document)\b/,
    },
    json: {
      keywords: /\b(true|false|null)\b/,
      types: /\b(\d+)\b/,
    },
    bash: {
      keywords: /\b(go|git|npm|npx|cd|ls|cat|grep|rg|make|docker|curl|echo|export|if|then|fi|for|do|done|test|run|build)\b/,
      types: /\b(\d+)\b/,
    },
    text: { keywords: /$^/, types: /$^/ },
  };

  function highlight(code, lang) {
    lang = (lang || 'text').toLowerCase();
    if (lang === 'md' || lang === 'markdown') return escapeHtml(code);
    const rules = LANG_RULES[lang] || LANG_RULES.text;

    // 一次性扫描：注释、字符串、数字、关键字、类型，避免替换后互相污染。
    const pattern = new RegExp(
      [
        '(\\/\\/[^\\n]*|#[^\\n]*|--[^\\n]*)', // 1 注释
        '("(?:[^"\\\\\\n]|\\\\.)*"|\'(?:[^\'\\\\\\n]|\\\\.)*\'|`(?:[^`\\\\]|\\\\.)*`)', // 2 字符串
        '(\\b\\d+(?:\\.\\d+)?\\b)', // 3 数字
        rules.keywords.source, // 4 关键字
        rules.types.source, // 5 类型/标识
      ].join('|'),
      'g'
    );

    let out = '';
    let last = 0;
    let match;
    while ((match = pattern.exec(code)) !== null) {
      out += escapeHtml(code.slice(last, match.index));
      const text = escapeHtml(match[0]);
      if (match[1]) out += '<span class="tok-com">' + text + '</span>';
      else if (match[2]) out += '<span class="tok-str">' + text + '</span>';
      else if (match[3]) out += '<span class="tok-num">' + text + '</span>';
      else if (match[4]) out += '<span class="tok-kw">' + text + '</span>';
      else if (match[5]) out += '<span class="tok-type">' + text + '</span>';
      else out += text;
      last = match.index + match[0].length;
    }
    out += escapeHtml(code.slice(last));
    return out;
  }

  /* ---------------- 行内格式 ---------------- */
  function renderInline(raw, opts) {
    opts = opts || {};
    const tokens = [];
    let text = String(raw);

    // 先摘出行内代码，防止内部内容被再次格式化
    text = text.replace(/`([^`\n]+)`/g, function (_, code) {
      const i = tokens.length;
      tokens.push('<code class="inline">' + escapeHtml(code) + '</code>');
      return '\u0000' + i + '\u0000';
    });

    text = escapeHtml(text);

    // 链接只允许明确协议，其余按文本显示
    text = text.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, function (whole, label, href) {
      if (!/^(https?:|mailto:)/i.test(href)) return label;
      return '<a class="md-link" data-href="' + href.replace(/"/g, '&quot;') + '" role="link" tabindex="0">' + label + '</a>';
    });

    text = text.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    text = text.replace(/(^|[^*])\*([^*\n]+)\*(?!\*)/g, '$1<em>$2</em>');
    text = text.replace(/~~([^~]+)~~/g, '<del>$1</del>');

    // path:line 引用：仅识别带扩展名的路径，避免误伤时间与比例
    text = text.replace(
      /(^|[\s(（「“])((?:[\w.-]+\/)*[\w.-]+\.[A-Za-z0-9]{1,8}):(\d{1,5})(?=\b)/g,
      function (whole, prefix, path, line) {
        return (
          prefix +
          '<button type="button" class="ref-link" data-ref-path="' + path +
          '" data-ref-line="' + line + '" title="打开 ' + path + ' 第 ' + line + ' 行">' +
          path + '<span class="ref-line">:' + line + '</span></button>'
        );
      }
    );

    // 还原行内代码
    text = text.replace(/\u0000(\d+)\u0000/g, function (_, i) {
      return tokens[Number(i)] || '';
    });

    return text;
  }

  /* ---------------- 代码块 ---------------- */
  function renderCodeBlock(code, lang, opts) {
    opts = opts || {};
    const id = 'cb-' + Math.random().toString(36).slice(2, 9);
    const lines = code.replace(/\n$/, '').split('\n');
    const showLines = opts.lines !== false;
    const collapsed = lines.length > 26;
    const gutter = lines
      .map(function (_, i) {
        return '<span>' + (i + 1) + '</span>';
      })
      .join('');
    const body = highlight(code.replace(/\n$/, ''), lang);
    const langLabel = lang || 'text';

    return (
      '<div class="code-block' + (showLines ? '' : ' no-lines') + (collapsed ? ' collapsed' : '') +
      '" id="' + id + '" data-code="' + escapeHtml(code) + '">' +
      '<div class="code-head">' +
      '<span class="code-lang"><svg class="ico"><use href="#i-code"/></svg>' + escapeHtml(langLabel) + '</span>' +
      '<span class="code-actions">' +
      '<button type="button" data-code-act="lines" class="is-on" title="显示或隐藏行号">行号</button>' +
      '<button type="button" data-code-act="wrap" title="自动换行">换行</button>' +
      '<button type="button" data-code-act="copy" title="复制代码"><svg class="ico"><use href="#i-copy"/></svg>复制</button>' +
      '</span>' +
      '</div>' +
      '<div class="code-body">' +
      (showLines ? '<div class="code-gutter">' + gutter + '</div>' : '') +
      '<pre>' + body + '</pre>' +
      '</div>' +
      '<button type="button" class="code-more" data-code-act="expand">展开全部 ' + lines.length + ' 行</button>' +
      '</div>'
    );
  }

  /* ---------------- 表格 ---------------- */
  function isTableSeparator(line) {
    return /^\s*\|?[\s:|-]+\|[\s:|-]*\|?\s*$/.test(line) && line.indexOf('-') >= 0;
  }

  function splitRow(line) {
    let s = line.trim();
    if (s.startsWith('|')) s = s.slice(1);
    if (s.endsWith('|')) s = s.slice(0, -1);
    return s.split('|').map(function (c) {
      return c.trim();
    });
  }

  /* ---------------- 主渲染 ---------------- */
  function render(src) {
    const lines = String(src).replace(/\r\n/g, '\n').split('\n');
    const out = [];
    let i = 0;

    while (i < lines.length) {
      const line = lines[i];

      // 代码块（未闭合时也按代码块处理，保证流式期间不崩）
      const fence = line.match(/^\s*```(\S*)\s*$/);
      if (fence) {
        const lang = fence[1] || 'text';
        const buf = [];
        i++;
        while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) {
          buf.push(lines[i]);
          i++;
        }
        if (i < lines.length) i++; // 跳过结束 fence
        out.push(renderCodeBlock(buf.join('\n'), lang));
        continue;
      }

      if (!line.trim()) {
        i++;
        continue;
      }

      // 标题
      const heading = line.match(/^(#{1,4})\s+(.*)$/);
      if (heading) {
        const level = heading[1].length;
        out.push('<h' + level + '>' + renderInline(heading[2]) + '</h' + level + '>');
        i++;
        continue;
      }

      // 分隔线
      if (/^\s*([-*_])\1{2,}\s*$/.test(line)) {
        out.push('<hr>');
        i++;
        continue;
      }

      // 引用
      if (/^\s*>\s?/.test(line)) {
        const buf = [];
        while (i < lines.length && /^\s*>\s?/.test(lines[i])) {
          buf.push(lines[i].replace(/^\s*>\s?/, ''));
          i++;
        }
        out.push('<blockquote>' + render(buf.join('\n')) + '</blockquote>');
        continue;
      }

      // 表格
      if (line.indexOf('|') >= 0 && i + 1 < lines.length && isTableSeparator(lines[i + 1])) {
        const head = splitRow(line);
        i += 2;
        const rows = [];
        while (i < lines.length && lines[i].indexOf('|') >= 0 && lines[i].trim()) {
          rows.push(splitRow(lines[i]));
          i++;
        }
        let html = '<table><thead><tr>';
        head.forEach(function (h) {
          html += '<th>' + renderInline(h) + '</th>';
        });
        html += '</tr></thead><tbody>';
        rows.forEach(function (r) {
          html += '<tr>';
          head.forEach(function (_, idx) {
            html += '<td>' + renderInline(r[idx] || '') + '</td>';
          });
          html += '</tr>';
        });
        html += '</tbody></table>';
        out.push(html);
        continue;
      }

      // 列表（无序、有序、任务）
      if (/^\s*([-*+]|\d+\.)\s+/.test(line)) {
        const ordered = /^\s*\d+\.\s+/.test(line);
        const items = [];
        let hasTask = false;
        while (i < lines.length && /^\s*([-*+]|\d+\.)\s+/.test(lines[i])) {
          let item = lines[i].replace(/^\s*([-*+]|\d+\.)\s+/, '');
          let task = null;
          const taskMatch = item.match(/^\[([ xX])\]\s+(.*)$/);
          if (taskMatch) {
            hasTask = true;
            task = taskMatch[1].toLowerCase() === 'x';
            item = taskMatch[2];
          }
          items.push({ text: item, task: task });
          i++;
        }
        const tag = ordered ? 'ol' : 'ul';
        const cls = hasTask ? ' class="task-list"' : '';
        let html = '<' + tag + cls + '>';
        items.forEach(function (it) {
          if (it.task === null) {
            html += '<li>' + renderInline(it.text) + '</li>';
          } else {
            html +=
              '<li>' +
              '<span class="task-box' + (it.task ? ' done' : '') + '">' +
              (it.task ? '<svg class="ico"><use href="#i-check"/></svg>' : '') +
              '</span><span>' + renderInline(it.text) + '</span></li>';
          }
        });
        html += '</' + tag + '>';
        out.push(html);
        continue;
      }

      // 段落：收集连续普通行
      const buf = [];
      while (
        i < lines.length &&
        lines[i].trim() &&
        !/^\s*```/.test(lines[i]) &&
        !/^(#{1,4})\s+/.test(lines[i]) &&
        !/^\s*([-*+]|\d+\.)\s+/.test(lines[i]) &&
        !/^\s*>\s?/.test(lines[i]) &&
        !/^\s*([-*_])\1{2,}\s*$/.test(lines[i]) &&
        !(lines[i].indexOf('|') >= 0 && i + 1 < lines.length && isTableSeparator(lines[i + 1]))
      ) {
        buf.push(lines[i]);
        i++;
      }
      if (buf.length === 0) {
        // 防御：避免死循环
        buf.push(lines[i]);
        i++;
      }
      out.push('<p>' + renderInline(buf.join('\n')).replace(/\n/g, '<br>') + '</p>');
    }

    return out.join('\n');
  }

  CW.md = { render: render, inline: renderInline, highlight: highlight, escape: escapeHtml };
})();
