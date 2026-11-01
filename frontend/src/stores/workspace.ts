import { create } from "zustand";
import { api } from "../lib/api";
import type { DiffResult, DocumentPreview, FileContent, FileNode } from "../lib/types";
import { useAppStore } from "./app";
import { usePanelStore } from "./panel";
import { useUiStore } from "./ui";

interface WorkspaceState {
  tree: Record<string, FileNode[]>;
  expanded: Record<string, boolean>;
  filter: string;
  git: Record<string, string>;
  selected: string | null;
  content: FileContent | null;
  editorMode: "read" | "edit";
  draft: string;
  targetLine: number | null;
  diff: { title: string; oldText: string; newText: string; result: DiffResult | null; view: "unified" | "side" } | null;
  doc: DocumentPreview | null;
  docLoading: boolean;
  loadRoot(): Promise<void>;
  loadDir(path: string): Promise<void>;
  toggleDir(path: string): Promise<void>;
  setFilter(value: string): void;
  openFile(path: string): Promise<void>;
  openFileAt(path: string, line: number | null): Promise<void>;
  closeFile(): void;
  setEditorMode(mode: "read" | "edit"): void;
  setDraft(value: string): void;
  save(): Promise<void>;
  openDiff(title: string, oldText: string, newText: string): Promise<void>;
  applyDiff(): Promise<void>;
  setDiffView(view: "unified" | "side"): void;
  openDocument(path: string): Promise<void>;
  reset(): void;
}

function workspaceID(): string {
  return useAppStore.getState().activeWorkspace?.id ?? "";
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  tree: {},
  expanded: {},
  filter: "",
  git: {},
  selected: null,
  content: null,
  editorMode: "read",
  draft: "",
  targetLine: null,
  diff: null,
  doc: null,
  docLoading: false,

  async loadRoot() {
    const id = workspaceID();
    if (!id) {
      return;
    }
    const [nodes, git] = await Promise.all([
      api.workspace.listDir(id, "."),
      api.workspace.gitStatuses(id).catch(() => ({})),
    ]);
    set((state) => ({ tree: { ...state.tree, ".": nodes }, git }));
  },

  async loadDir(path) {
    const id = workspaceID();
    if (!id) {
      return;
    }
    const nodes = await api.workspace.listDir(id, path);
    set((state) => ({ tree: { ...state.tree, [path]: nodes } }));
  },

  async toggleDir(path) {
    const expanded = { ...get().expanded, [path]: !get().expanded[path] };
    set({ expanded });
    if (expanded[path] && !get().tree[path]) {
      await get().loadDir(path);
    }
  },

  setFilter(value) {
    set({ filter: value });
  },

  async openFile(path) {
    const id = workspaceID();
    if (!id) {
      return;
    }
    const lower = path.toLowerCase();
    if (lower.endsWith(".docx") || lower.endsWith(".xlsx") || lower.endsWith(".pdf")) {
      const mode = useAppStore.getState().mode;
      if (mode === "work") {
        await get().openDocument(path);
        usePanelStore.getState().setTab("work", "document");
      } else {
        useUiStore.getState().toast({
          title: "Office / PDF 预览在 Work 模式",
          description: "切换到 Work 模式后可以在右侧面板查看文档。",
          tone: "info",
        });
      }
      return;
    }
    try {
      const content = await api.workspace.readFile(id, path);
      set({
        selected: path,
        content,
        draft: content.content,
        editorMode: "read",
        targetLine: null,
      });
    } catch (error) {
      useUiStore.getState().toast({ title: "读取文件失败", description: String(error), tone: "danger" });
    }
  },

  async openFileAt(path, line) {
    set({ targetLine: line });
    await get().openFile(path);
  },

  closeFile() {
    set({ selected: null, content: null, draft: "", editorMode: "read", targetLine: null });
  },

  setEditorMode(mode) {
    const content = get().content;
    if (mode === "edit" && (content?.binary || content?.truncated)) {
      return;
    }
    set({ editorMode: mode, draft: content?.content ?? get().draft });
  },

  setDraft(value) {
    set({ draft: value });
  },

  async save() {
    const id = workspaceID();
    const content = get().content;
    if (!id || !content) {
      return;
    }
    try {
      await api.workspace.writeFile(id, content.path, get().draft, content.hash);
      const refreshed = await api.workspace.readFile(id, content.path);
      set({ content: refreshed, draft: refreshed.content, editorMode: "read" });
      useUiStore.getState().toast({ title: "已保存", description: content.path, tone: "ok" });
    } catch (error) {
      const message = String(error);
      if (message.includes("外部修改")) {
        const reload = await useUiStore.getState().askConfirm({
          title: "文件已被外部修改",
          description: "重新加载会丢弃当前未保存的修改。",
          detail: content.path,
          confirmText: "重新加载",
          danger: true,
        });
        if (reload) {
          await get().openFile(content.path);
        }
        return;
      }
      useUiStore.getState().toast({ title: "保存失败", description: message, tone: "danger" });
    }
  },

  async openDiff(title, oldText, newText) {
    const result = await api.workspace.computeDiff(oldText, newText);
    set({ diff: { title, oldText, newText, result, view: get().diff?.view ?? "unified" } });
    usePanelStore.getState().setTab(useAppStore.getState().mode, "diff");
  },

  async applyDiff() {
    const diff = get().diff;
    const id = workspaceID();
    if (!diff || !id) {
      return;
    }
    const confirmed = await useUiStore.getState().askConfirm({
      title: "应用全部修改？",
      description: "将把右侧的新内容写入文件。写入前会备份原文件。",
      detail: diff.title,
      confirmText: "应用修改",
      danger: true,
    });
    if (!confirmed) {
      return;
    }
    try {
      await api.workspace.writeFile(id, diff.title, diff.newText, "");
      useUiStore.getState().toast({ title: "已应用修改", description: diff.title, tone: "ok" });
      await get().openFile(diff.title);
    } catch (error) {
      useUiStore.getState().toast({ title: "应用失败", description: String(error), tone: "danger" });
    }
  },

  setDiffView(view) {
    const diff = get().diff;
    if (diff) {
      set({ diff: { ...diff, view } });
    }
  },

  async openDocument(path) {
    const id = workspaceID();
    if (!id) {
      return;
    }
    set({ docLoading: true });
    try {
      const doc = await api.documents.preview(id, path);
      set({ doc, docLoading: false });
    } catch (error) {
      set({ docLoading: false });
      useUiStore.getState().toast({ title: "读取文档失败", description: String(error), tone: "danger" });
    }
  },

  reset() {
    set({ tree: {}, expanded: {}, git: {}, selected: null, content: null, draft: "", diff: null, doc: null, targetLine: null });
  },
}));
