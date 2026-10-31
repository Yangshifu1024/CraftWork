import { create } from "zustand";
import { clamp } from "../lib/utils";
import type { Mode, PanelTab } from "../lib/types";

interface PanelState {
  open: boolean;
  maximized: boolean;
  width: number;
  narrow: boolean;
  tabs: Record<Mode, PanelTab>;
  setOpen(open: boolean): void;
  toggle(): void;
  setMaximized(maximized: boolean): void;
  toggleMaximized(): void;
  setWidth(width: number): void;
  setNarrow(narrow: boolean): void;
  setTab(mode: Mode, tab: PanelTab): void;
}

export const usePanelStore = create<PanelState>((set, get) => ({
  open: true,
  maximized: false,
  width: 440,
  narrow: false,
  tabs: { code: "files", work: "document" },

  setOpen(open) {
    set({ open, maximized: open ? get().maximized : false });
  },
  toggle() {
    const next = !get().open;
    set({ open: next, maximized: next ? get().maximized : false });
  },
  setMaximized(maximized) {
    set({ maximized, open: maximized ? true : get().open });
  },
  toggleMaximized() {
    const current = get();
    if (!current.open) {
      set({ open: true, maximized: true });
      return;
    }
    set({ maximized: !current.maximized });
  },
  setWidth(width) {
    set({ width: clamp(width, 320, 760) });
  },
  setNarrow(narrow) {
    if (narrow === get().narrow) {
      return;
    }
    if (narrow) {
      set({ narrow, open: false, maximized: false });
    } else {
      set({ narrow, open: true });
    }
  },
  setTab(mode, tab) {
    set((state) => ({ tabs: { ...state.tabs, [mode]: tab }, open: true }));
  },
}));
