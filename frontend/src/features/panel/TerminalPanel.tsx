import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import "@xterm/xterm/css/xterm.css";
import { Icon } from "../../components/Icon";
import { cn } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { useTerminalStore } from "../../stores/terminal";

const searchDecorations = {
  matchBackground: "#39452a",
  matchBorder: "#6f8a3f",
  matchOverviewRuler: "#6f8a3f",
  activeMatchBackground: "#5b9dff",
  activeMatchBorder: "#ffffff",
  activeMatchColorOverviewRuler: "#5b9dff",
};

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
  const [searchResult, setSearchResult] = useState({ index: -1, count: 0 });

  useEffect(() => {
    activeRef.current = activeID;
  }, [activeID]);

  useEffect(() => {
    if (sessions.length === 0 && !createdRef.current) {
      createdRef.current = true;
      void create();
    }
  }, [sessions.length, create]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) {
      return;
    }
    const term = new Terminal({
      fontSize: settings.terminalFontSize,
      fontFamily: "var(--font-mono)",
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
      setSearchResult({ index: resultIndex, count: resultCount });
    });
    term.open(host);
    fit.fit();
    term.onData((data) => {
      const id = activeRef.current;
      if (id) {
        void useTerminalStore.getState().write(data);
      }
    });
    term.onResize(({ cols, rows }) => {
      const id = activeRef.current;
      if (id) {
        void useTerminalStore.getState().resize(cols, rows);
      }
    });
    termRef.current = term;
    fitRef.current = fit;
    searchRef.current = search;
    counterRef.current = { id: null, length: 0 };

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
      observer.disconnect();
      searchSubscription.dispose();
      search.dispose();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      searchRef.current = null;
    };
  }, [theme, settings.terminalFontSize]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) {
      return;
    }
    const full = activeID ? outputs[activeID] ?? "" : "";
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

  function openSearch() {
    setSearchOpen(true);
    requestAnimationFrame(() => searchInputRef.current?.focus());
  }

  function closeSearch() {
    setSearchOpen(false);
    searchRef.current?.clearDecorations();
    setSearchResult({ index: -1, count: 0 });
  }

  const resultLabel =
    searchResult.count > 0
      ? `${Math.max(searchResult.index + 1, 1)} / ${searchResult.count}`
      : query.trim()
        ? "无匹配"
        : "";

  return (
    <div className="terminal-wrap">
      <div className="term-tabs">
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
        <button type="button" className="icon-btn sm" title="新建终端" aria-label="新建终端" onClick={() => void create()}>
          <Icon name="plus" className="ico" />
        </button>
      </div>

      <div className="term-head">
        <span>{active?.cwd ?? "工作区"}</span>
        <span className={cn("badge", active?.running ? "ok" : "neutral")}>
          {active?.running ? "运行中" : "未运行"}
        </span>
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
          <button type="button" className="icon-btn sm" title="上一个匹配" aria-label="上一个匹配" onClick={() => runFind(true)}>
            <Icon name="chevron-up" className="ico" />
          </button>
          <button type="button" className="icon-btn sm" title="下一个匹配" aria-label="下一个匹配" onClick={() => runFind(false)}>
            <Icon name="chevron-down" className="ico" />
          </button>
          <button type="button" className="icon-btn sm" title="关闭搜索" aria-label="关闭终端搜索" onClick={closeSearch}>
            <Icon name="x" className="ico" />
          </button>
        </div>
      ) : null}

      <div className="term-host" ref={hostRef} />

      <div className="term-foot">
        <span>用户终端不经过 Agent 审批；Agent 的 shell_exec 走策略引擎。</span>
      </div>
    </div>
  );
}
