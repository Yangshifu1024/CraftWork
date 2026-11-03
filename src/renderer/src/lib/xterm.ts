// xterm 实例工厂：主题取自 CSS 变量（跟随深浅主题），fit 插件随容器。
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

function cssVar(name: string, fallback: string): string {
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

export interface XtermHandle {
  term: Terminal;
  fit(): void;
}

export function createTerminal(container: HTMLElement, fontSize: number): XtermHandle {
  const term = new Terminal({
    fontSize,
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace',
    cursorBlink: true,
    allowProposedApi: true,
    theme: {
      background: cssVar("--color-terminal-bg", "#161616"),
      foreground: cssVar("--color-terminal-fg", "#f8f8f8"),
      cursor: cssVar("--color-terminal-cursor", "#f8f8f8"),
      selectionBackground: cssVar("--color-terminal-selection", "rgba(64,153,255,0.28)"),
      black: cssVar("--color-terminal-black", "#363636"),
      red: cssVar("--color-terminal-red", "#ff5c5c"),
      green: cssVar("--color-terminal-green", "#46bf72"),
      yellow: cssVar("--color-terminal-yellow", "#ff8a30"),
      blue: cssVar("--color-terminal-blue", "#4099ff"),
      magenta: cssVar("--color-terminal-magenta", "#7b5ce5"),
      cyan: cssVar("--color-terminal-cyan", "#42c8c8"),
      white: cssVar("--color-terminal-white", "#adadad"),
      brightBlack: cssVar("--color-terminal-bright-black", "#747474"),
      brightRed: cssVar("--color-terminal-bright-red", "#ff9999"),
      brightGreen: cssVar("--color-terminal-bright-green", "#87d9a4"),
      brightYellow: cssVar("--color-terminal-bright-yellow", "#ffb26b"),
      brightBlue: cssVar("--color-terminal-bright-blue", "#80beff"),
      brightMagenta: cssVar("--color-terminal-bright-magenta", "#c2a2ff"),
      brightCyan: cssVar("--color-terminal-bright-cyan", "#8ee5e5"),
      brightWhite: cssVar("--color-terminal-bright-white", "#f8f8f8"),
    },
  });
  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);
  term.open(container);
  return {
    term,
    fit(): void {
      try {
        fitAddon.fit();
      } catch {
        // 容器尚未布局
      }
    },
  };
}
