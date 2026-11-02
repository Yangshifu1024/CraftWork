/**
 * 终端面板。
 *
 * 负责：管理多个用户终端会话的标签页，把后端推送的输出写入 xterm.js，
 * 在面板尺寸变化时同步行列数，并提供终端输出搜索。
 *
 * 用户终端直接与本地 shell 交互，不经过 Agent 审批；
 * Agent 自己执行的 shell_exec 仍然要走策略引擎与审批。
 */

import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef, useState } from "react";
import "@xterm/xterm/css/xterm.css";
import { Icon } from "../../components/Icon";
import { cn } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { useTerminalStore } from "../../stores/terminal";

/** 搜索命中与当前命中在终端里的高亮配色。 */
const searchDecorations = {
  matchBackground: "#39452a",
  matchBorder: "#6f8a3f",
  matchOverviewRuler: "#6f8a3f",
  activeMatchBackground: "#5b9dff",
  activeMatchBorder: "#ffffff",
  activeMatchColorOverviewRuler: "#5b9dff",
};

/** 根据界面主题返回 xterm.js 的主题配色（浅色 / 深色各一套）。 */
function terminalTheme(theme: string) {
  if (theme === "light") {
    return {
      background: "#f7f9fc",
      foreground: "#10141b",
      cursor: "#2563eb",
      selectionBackground: "rgba(37, 99, 235, 0.22)",
      black: "#10141b",
      red: "#d64545",
      green: "#0f9d6e",
      yellow: "#b7791f",
      blue: "#2563eb",
      magenta: "#7c5cf0",
      cyan: "#0d8f9c",
      white: "#e0e5ee",
      brightBlack: "#7d8798",
      brightRed: "#c53c3c",
      brightGreen: "#0f9d6e",
      brightYellow: "#b7791f",
      brightBlue: "#1d4ed8",
      brightMagenta: "#7c5cf0",
      brightCyan: "#0d8f9c",
      brightWhite: "#ffffff",
    };
  }
  return {
    background: "#0c0f14",
    foreground: "#e8ebf1",
    cursor: "#5b9dff",
    selectionBackground: "rgba(91, 157, 255, 0.28)",
    black: "#0b0d11",
    red: "#ef7070",
    green: "#43c08d",
    yellow: "#e8b04a",
    blue: "#5b9dff",
    magenta: "#a78bfa",
    cyan: "#4ec9d4",
    white: "#e8ebf1",
    brightBlack: "#4d5563",
    brightRed: "#f08a8a",
    brightGreen: "#5ad3a2",
    brightYellow: "#f0c46a",
    brightBlue: "#7cb2ff",
    brightMagenta: "#bfa8ff",
    brightCyan: "#6fdbe4",
    brightWhite: "#ffffff",
  };
}

/**
 * xterm 的字体栈。
 *
 * xterm 通过 canvas 渲染文字，不能解析 CSS 变量；这里必须给出具体字体名。
 * 前半段是常见等宽字体，后半段是 Nerd Font，保证 shell 提示符里的分支、
 * 目录等图标有回退字体可用，而不是显示成方框。
 */
const TERMINAL_FONT_FAMILY = [
  '"SF Mono"',
  '"Menlo"',
  '"Monaco"',
  '"Cascadia Code"',
  '"Consolas"',
  '"DejaVu Sans Mono"',
  '"Lilex Nerd Font Mono"',
  '"MesloLGS NF"',
  '"MesloLGS Nerd Font"',
  '"JetBrainsMono Nerd Font"',
  '"Hack Nerd Font"',
  '"FiraCode Nerd Font"',
  '"CaskaydiaCove Nerd Font"',
  '"Symbols Nerd Font Mono"',
  '"Symbols Nerd Font"',
  "monospace",
].join(", ");

/**
 * 终端面板组件。
 *
 * props：无。关键状态：
 * - sessions / activeID / outputs：会话列表、当前会话与各会话的累积输出，来自 terminal store；
 * - searchOpen / query / searchResult：搜索栏开关、关键字与当前命中位置；
 * - theme 与 settings.terminalFontSize：决定 xterm 的主题与字号。
 *
 * xterm 是命令式实例，通过 ref 持有；主题或字号变化时整体重建。
 */
export function TerminalPanel() {
  const sessions = useTerminalStore((state) => state.sessions);
  const activeID = useTerminalStore((state) => state.activeID);
  const outputs = useTerminalStore((state) => state.outputs);
  const create = useTerminalStore((state) => state.create);
  const closeSession = useTerminalStore((state) => state.close);
  const setActive = useTerminalStore((state) => state.setActive);
  const theme = useAppStore((state) => state.theme);
  const settings = useAppStore((state) => state.settings);

  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const activeRef = useRef<string | null>(activeID);
  const counterRef = useRef<{ id: string | null; length: number }>({ id: null, length: 0 });
  const createdRef = useRef(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  // 搜索结果的当前位置与总数，由 SearchAddon 的回调更新。
  const [searchResult, setSearchResult] = useState({ index: -1, count: 0 });

  // 把当前会话 ID 同步到 ref，供 xterm 的输入、尺寸回调读取，
  // 避免回调闭包拿到过期的值。
  useEffect(() => {
    activeRef.current = activeID;
  }, [activeID]);

  // 首次进入且没有任何会话时自动创建一个，保证界面不是空白；
  // createdRef 防止重复创建。
  useEffect(() => {
    if (sessions.length === 0 && !createdRef.current) {
      createdRef.current = true;
      void create();
    }
  }, [sessions.length, create]);

  // 创建 xterm 实例：加载自适应尺寸与搜索插件，绑定用户输入、尺寸变化与搜索结果回调。
  // 主题或字号变化时重建实例，保证设置立即生效。
  useEffect(() => {
    const host = hostRef.current;
    if (!host) {
      return;
    }
    const term = new Terminal({
      fontSize: settings.terminalFontSize,
      fontFamily: TERMINAL_FONT_FAMILY,
      cursorBlink: true,
      convertEol: false,
      scrollback: 10000,
      theme: terminalTheme(theme),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    const search = new SearchAddon();
    term.loadAddon(search);
    const searchSubscription = search.onDidChangeResults(({ resultIndex, resultCount }) => {
      // 搜索插件每次命中变化都会回调，用它更新「第几个 / 共几个」。
      setSearchResult({ index: resultIndex, count: resultCount });
    });
    term.open(host);
    fit.fit();
    term.onData((data) => {
      // 用户键盘输入直接写入当前会话。
      const id = activeRef.current;
      if (id) {
        void useTerminalStore.getState().write(data);
      }
    });
    term.onResize(({ cols, rows }) => {
      // 行列数变化时通知后端调整 PTY 尺寸，保证换行与光标位置正确。
      const id = activeRef.current;
      if (id) {
        void useTerminalStore.getState().resize(cols, rows);
      }
    });
    termRef.current = term;
    fitRef.current = fit;
    searchRef.current = search;
    counterRef.current = { id: null, length: 0 };

    // 监听宿主尺寸变化并重新适配行列数；面板隐藏时尺寸为 0，fit 可能抛错，忽略即可。
    const observer = new ResizeObserver(() => {
      try {
        fit.fit();
      } catch {
        // 面板隐藏时尺寸为 0，忽略即可。
      }
    });
    observer.observe(host);
    term.focus();

    return () => {
      // 重建或卸载时释放观察器、插件与终端实例。
      observer.disconnect();
      searchSubscription.dispose();
      search.dispose();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      searchRef.current = null;
    };
  }, [theme, settings.terminalFontSize]);

  // 增量写入输出：
  // 切换会话或输出长度变短（会话被重置）时先清空终端，再从头写入；
  // 否则只写入上次已写入位置之后的部分，避免重复渲染全部历史。
  useEffect(() => {
    const term = termRef.current;
    if (!term) {
      return;
    }
    const full = activeID ? (outputs[activeID] ?? "") : "";
    if (counterRef.current.id !== activeID || full.length < counterRef.current.length) {
      term.reset();
      counterRef.current = { id: activeID, length: 0 };
    }
    const next = full.slice(counterRef.current.length);
    if (next) {
      term.write(next);
    }
    counterRef.current.length = full.length;
  }, [outputs, activeID]);

  const active = sessions.find((session) => session.id === activeID);

  /**
   * 在终端输出中查找关键字。
   *
   * @param backward 为 true 时查找上一个匹配，否则查找下一个；
   * @param value 要查找的内容，默认使用搜索框里的关键字。
   * 关键字为空时清除高亮并重置计数。
   */
  function runFind(backward = false, value = query) {
    const search = searchRef.current;
    if (!search || !value.trim()) {
      search?.clearDecorations();
      setSearchResult({ index: -1, count: 0 });
      return;
    }
    const options = { incremental: !backward, decorations: searchDecorations };
    if (backward) {
      search.findPrevious(value, options);
    } else {
      search.findNext(value, options);
    }
  }

  /** 打开搜索栏，并在下一帧把焦点放到输入框（等元素渲染完成后再聚焦）。 */
  function openSearch() {
    setSearchOpen(true);
    requestAnimationFrame(() => searchInputRef.current?.focus());
  }

  /** 关闭搜索栏并清除终端里的高亮。 */
  function closeSearch() {
    setSearchOpen(false);
    searchRef.current?.clearDecorations();
    setSearchResult({ index: -1, count: 0 });
  }

  // 命中计数文案：有结果时显示「当前 / 总数」，有输入但无结果时显示「无匹配」。
  const resultLabel =
    searchResult.count > 0
      ? `${Math.max(searchResult.index + 1, 1)} / ${searchResult.count}`
      : query.trim()
        ? "无匹配"
        : "";

  return (
    <div className="terminal-wrap">
      <div className="term-tabs">
        {/* 每个会话一个标签；关闭按钮阻止冒泡，避免同时切换会话 */}
        {sessions.map((session) => (
          <button
            key={session.id}
            type="button"
            className={cn("term-tab", session.id === activeID && "is-active")}
            onClick={() => setActive(session.id)}
          >
            <Icon name="terminal" className="ico" />
            <span>{session.name}</span>
            <span
              className="term-tab-close"
              role="button"
              tabIndex={0}
              aria-label={`关闭 ${session.name}`}
              onClick={(event) => {
                event.stopPropagation();
                void closeSession(session.id);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  void closeSession(session.id);
                }
              }}
            >
              <Icon name="x" />
            </span>
          </button>
        ))}
        <button
          type="button"
          className="icon-btn sm"
          title="新建终端"
          aria-label="新建终端"
          onClick={() => void create()}
        >
          <Icon name="plus" className="ico" />
        </button>
      </div>

      <div className="term-head">
        {/* 当前会话的工作目录与运行状态，右侧是搜索开关 */}
        <span>{active?.cwd ?? "工作区"}</span>
        <span className={cn("badge", active?.running ? "ok" : "neutral")}>{active?.running ? "运行中" : "未运行"}</span>
        <span className="spacer" />
        <button
          type="button"
          className={cn("icon-btn sm", searchOpen && "is-on")}
          title="搜索终端输出 ⌘F"
          aria-label="搜索终端输出"
          onClick={() => (searchOpen ? closeSearch() : openSearch())}
        >
          <Icon name="search" className="ico" />
        </button>
        <span className="term-hint">点击终端后直接输入 · Ctrl+L 清屏</span>
      </div>

      {searchOpen ? (
        /* 搜索栏：输入即查找，回车下一个 / Shift+回车上一个，Esc 关闭 */
        <div className="term-search">
          <Icon name="search" className="ico" />
          <input
            ref={searchInputRef}
            value={query}
            placeholder="搜索终端输出…"
            aria-label="搜索终端输出"
            onChange={(event) => {
              setQuery(event.target.value);
              runFind(false, event.target.value);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                runFind(event.shiftKey);
              } else if (event.key === "Escape") {
                event.preventDefault();
                closeSearch();
              }
            }}
          />
          <span className="term-search-count">{resultLabel}</span>
          <button
            type="button"
            className="icon-btn sm"
            title="上一个匹配"
            aria-label="上一个匹配"
            onClick={() => runFind(true)}
          >
            <Icon name="chevron-up" className="ico" />
          </button>
          <button
            type="button"
            className="icon-btn sm"
            title="下一个匹配"
            aria-label="下一个匹配"
            onClick={() => runFind(false)}
          >
            <Icon name="chevron-down" className="ico" />
          </button>
          <button
            type="button"
            className="icon-btn sm"
            title="关闭搜索"
            aria-label="关闭终端搜索"
            onClick={closeSearch}
          >
            <Icon name="x" className="ico" />
          </button>
        </div>
      ) : null}

      {/* xterm 的挂载节点，终端实例在 effect 中创建 */}
      <div className="term-host" ref={hostRef} />

      <div className="term-foot">
        {/* 说明用户终端与 Agent 执行命令在审批上的差别 */}
        <span>用户终端不经过 Agent 审批；Agent 的 shell_exec 走策略引擎。</span>
      </div>
    </div>
  );
}
