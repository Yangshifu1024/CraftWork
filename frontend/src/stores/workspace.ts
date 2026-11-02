/**
 * 工作区 store：文件树的加载与展开、文件预览与编辑草稿、外部修改冲突处理、
 * 差异预览与应用、Office / PDF 文档预览。
 *
 * 所有数据都按「当前激活工作区」请求，工作区来自 app store；模式切换时由 App.tsx 调用 reset() 清空。
 *
 * 多文件夹：一个工作区可以挂载多个源文件夹（第一个是主要目录）。文件树
 * 按文件夹分段渲染，内部缓存的键是「文件夹 ID + 相对路径」的组合键，
 * 避免不同根之间的同名相对路径互相覆盖；读写接口显式携带 folderId
 * （空值表示主要目录），定位永远准确。
 */
import { create } from "zustand";
import { api } from "../lib/api";
import { fileTabId } from "../lib/sidePane";
import type { DiffResult, DocumentPreview, FileContent, FileNode, WorkspaceView } from "../lib/types";
import { basename } from "../lib/utils";
import { useAppStore } from "./app";
import { usePanelStore } from "./panel";
import { useUiStore } from "./ui";

/**
 * 源文件夹的组合键：树缓存与展开状态都以它为键。
 * folderId 为空表示主要目录（兼容尚未挂载文件夹列表的工作区）。
 */
export function folderKey(folderID: string, path: string): string {
  return `${folderID}|${path}`;
}

/**
 * 从组合键拆出文件夹 ID 与相对路径。
 * 组合键中的路径可能包含「|」吗？相对路径来自文件系统，理论上可以包含
 * 任意字符；这里约定以第一个「|」为分隔，因为 folderID（wfo_xxx）不含该字符。
 */
export function splitFolderKey(key: string): { folderID: string; path: string } {
  const index = key.indexOf("|");
  if (index < 0) {
    return { folderID: "", path: key };
  }
  return { folderID: key.slice(0, index), path: key.slice(index + 1) };
}

/** 取出当前激活工作区；没有时返回 null。 */
function activeWorkspace(): WorkspaceView | null {
  return useAppStore.getState().activeWorkspace;
}

/** 取出当前激活工作区的 ID；没有激活工作区时返回空字符串。 */
function workspaceID(): string {
  return activeWorkspace()?.id ?? "";
}

/**
 * 一个 file tab 的编辑器数据槽。
 *
 * 多标签页之下每个 file tab 拥有独立的一份（内容、草稿、编辑模式、
 * 定位行），key 是面板 tab 的结构化 id（lib/sidePane 的 fileTabId），
 * 与 tab 同生共死：关闭 tab 即清槽。
 */
interface FileSlot {
  /** 所属源文件夹；空字符串表示主要目录。 */
  folderId: string;
  /** 相对所属源文件夹的路径。 */
  path: string;
  /** 文件内容与元信息（hash / language / binary 等）。 */
  content: FileContent | null;
  /** 编辑器模式：只读或编辑。 */
  editorMode: "read" | "edit";
  /** 编辑草稿；保存成功前不写回 `content`。 */
  draft: string;
  /** 从搜索或差异跳转时要定位到的行号。 */
  targetLine: number | null;
}

function newFileSlot(folderId: string, path: string): FileSlot {
  return { folderId, path, content: null, editorMode: "read", draft: "", targetLine: null };
}

/**
 * 工作区状态与操作。
 *
 * 关键状态说明：
 * - `tree`：按「文件夹 ID + 目录路径」缓存的子节点列表，每段根目录是 `"."`；
 * - `expanded`：按组合键记录目录是否展开；
 * - `filter`：文件树过滤关键字；
 * - `git`：主要目录内文件路径到 Git 状态字母的映射（Git 语义只覆盖主要目录）；
 * - `files`：file tab 的编辑器数据槽，key 是 tab id（见 FileSlot）；
 * - `diff`：当前差异预览的数据与视图方式（统一 / 并排）；
 * - `doc` / `docLoading`：文档预览内容与加载中标记。
 */
interface WorkspaceState {
  /** 「文件夹 ID + 目录路径」到子节点列表的缓存，目录路径 "." 表示该段根目录。 */
  tree: Record<string, FileNode[]>;
  /** 组合键到是否展开的映射。 */
  expanded: Record<string, boolean>;
  /** 文件树过滤关键字。 */
  filter: string;
  /** 主要目录内文件路径到 Git 状态字母的映射。 */
  git: Record<string, string>;
  /** file tab 的编辑器数据槽，key 是面板 tab id。 */
  files: Record<string, FileSlot>;
  /** 差异预览：标题、原文、新文、计算结果、视图方式与所属工作区。 */
  diff: {
    title: string;
    oldText: string;
    newText: string;
    result: DiffResult | null;
    view: "unified" | "side";
    workspaceId: string;
  } | null;
  /** 文档（Word / Excel / PDF / 文本）预览内容。 */
  doc: DocumentPreview | null;
  /** 文档是否正在加载。 */
  docLoading: boolean;
  /** 加载全部源文件夹的根目录文件树与主要目录的 Git 状态。 */
  loadRoot(): Promise<void>;
  /** 按需加载某个源文件夹内目录的子节点。 */
  loadDir(folderID: string, path: string): Promise<void>;
  /** 展开或收起目录；首次展开时才请求子节点。 */
  toggleDir(folderID: string, path: string): Promise<void>;
  /** 设置文件树过滤关键字。 */
  setFilter(value: string): void;
  /** 打开文件：Office / PDF 交给文档预览，其它文件读取文本内容并开 tab。 */
  openFile(folderID: string, path: string, options?: { quiet?: boolean }): Promise<boolean>;
  /** 读取文件内容进指定 tab 的编辑器数据槽；供 tab 切换时按需重载。 */
  loadFileSlot(tabId: string, folderID: string, path: string, options?: { quiet?: boolean }): Promise<boolean>;
  /** 打开文件并定位到指定行；路径先按主要目录解析，失败时跨源文件夹唯一匹配。 */
  openFileAt(path: string, line: number | null): Promise<void>;
  /** 清空某个 file tab 的数据槽（关闭 tab 时调用）。 */
  closeFileSlot(tabId: string): void;
  /** 切换某个 tab 的只读 / 编辑；二进制或内容被截断的文件不允许编辑。 */
  setEditorMode(tabId: string, mode: "read" | "edit"): void;
  /** 更新某个 tab 的编辑草稿。 */
  setDraft(tabId: string, value: string): void;
  /** 保存某个 tab 的草稿；遇到「文件已被外部修改」时询问用户是否重新加载。 */
  saveSlot(tabId: string): Promise<void>;
  /** 打开差异预览并聚焦差异 tab。 */
  openDiff(title: string, oldText: string, newText: string): Promise<void>;
  /** 二次确认后把差异中的新内容整份写入文件。 */
  applyDiff(): Promise<void>;
  /** 切换差异视图（统一 / 并排）。 */
  setDiffView(view: "unified" | "side"): void;
  /** 加载文档预览。 */
  openDocument(folderID: string, path: string): Promise<void>;
  /** 清空所有工作区数据，模式切换时调用。 */
  reset(): void;
}

/**
 * 工作区数据与文件编辑流程的状态容器。
 *
 * 初始状态：没有缓存目录、没有打开文件、只读模式、草稿为空。
 */
export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  tree: {},
  expanded: {},
  filter: "",
  git: {},
  files: {},
  diff: null,
  doc: null,
  docLoading: false,

  // 各源文件夹的根目录并行拉取；Git 状态只属于主要目录，失败不影响文件树。
  async loadRoot() {
    const workspace = activeWorkspace();
    if (!workspace) {
      return;
    }
    const folders = workspace.folders?.length ? workspace.folders : [{ id: "", path: workspace.path, primary: true }];
    const [rootEntries, git] = await Promise.all([
      Promise.all(
        folders.map(async (folder) => {
          const nodes = await api.workspace.listDir(workspace.id, folder.id, ".");
          return [folderKey(folder.id, "."), nodes] as const;
        }),
      ),
      api.workspace.gitStatuses(workspace.id).catch(() => ({})),
    ]);
    set((state) => {
      const tree = { ...state.tree };
      for (const [key, nodes] of rootEntries) {
        tree[key] = nodes;
      }
      return { tree, git };
    });
  },

  // 懒加载单个目录：只覆盖该组合键下的缓存，其余目录保持不动。
  async loadDir(folderID, path) {
    const id = workspaceID();
    if (!id) {
      return;
    }
    const nodes = await api.workspace.listDir(id, folderID, path);
    set((state) => ({ tree: { ...state.tree, [folderKey(folderID, path)]: nodes } }));
  },

  // 先翻转展开状态；只有当目录刚被展开且还没有缓存时才去请求子节点，
  // 再次展开不会重复请求。
  async toggleDir(folderID, path) {
    const key = folderKey(folderID, path);
    const expanded = { ...get().expanded, [key]: !get().expanded[key] };
    set({ expanded });
    if (expanded[key] && !get().tree[key]) {
      await get().loadDir(folderID, path);
    }
  },

  setFilter(value) {
    set({ filter: value });
  },

  // 打开文件：Office 与 PDF 不在编辑器里显示文本，而是走文档预览；
  // 文档预览目前只在 Work 模式提供，Code 模式下给出提示而不是静默失败。
  // 返回是否成功打开，供带行号定位的入口决定是否走短路径解析兜底。
  //
  // tab 层的事情交给 panel store：文件开 tab、文档由 openDocument 开 tab，
  // 本方法只负责内容与数据槽。
  async openFile(folderID, path, options) {
    const id = workspaceID();
    if (!id) {
      return false;
    }
    const lower = path.toLowerCase();
    if (lower.endsWith(".docx") || lower.endsWith(".xlsx") || lower.endsWith(".pdf")) {
      const mode = useAppStore.getState().mode;
      if (mode === "work") {
        await get().openDocument(folderID, path);
        return true;
      }
      useUiStore.getState().toast({
        title: "Office / PDF 预览在 Work 模式",
        description: "切换到 Work 模式后可以在右侧面板查看文档。",
        tone: "info",
      });
      return false;
    }
    // tab id 由工作区 + 文件夹 + 路径推导，与 panel store 的构造规则一致，
    // 因此这里可以在 tab 建立前就把内容写进正确的槽。
    const tabId = fileTabId(id, folderID, path);
    const opened = await get().loadFileSlot(tabId, folderID, path, options);
    if (opened) {
      // 从正文链接等入口打开时，确保右侧面板出现该文件的 tab 并展开面板。
      usePanelStore.getState().openFileTab({ folderId: folderID, path, title: basename(path) });
    }
    return opened;
  },

  // 读取文件内容进指定 tab 的数据槽。
  // 打开即进入只读模式，草稿与文件内容保持一致，便于随时切到编辑；
  // targetLine 在这里复位：正常打开文件不需要定位，带行号的入口
  // （openFileAt）会在内容就绪后重新设置。
  // 面板多 tab 之下切回一个 file tab 而槽已被清（例如模式切换 reset）时，
  // 渲染层会调用本方法重新读取。
  async loadFileSlot(tabId, folderID, path, options) {
    const id = workspaceID();
    if (!id) {
      return false;
    }
    try {
      const content = await api.workspace.readFile(id, folderID, path);
      set((state) => ({
        files: {
          ...state.files,
          [tabId]: {
            ...(state.files[tabId] ?? newFileSlot(folderID, path)),
            content,
            draft: content.content,
            editorMode: "read",
            targetLine: null,
          },
        },
      }));
      return true;
    } catch (error) {
      // 静默模式用于短路径兜底的第一跳：失败后还有解析重试，不打扰用户。
      if (!options?.quiet) {
        useUiStore.getState().toast({ title: "读取文件失败", description: String(error), tone: "danger" });
      }
      return false;
    }
  },

  // 打开文件并定位到指定行。
  //
  // 顺序很重要：先等 openFile 把内容读进来（它会复位 targetLine），
  // 再设置目标行。这样编辑器的定位 effect 触发时内容与行号都已就绪；
  // 反过来先设行号会被 openFile 的复位清掉，滚动从不发生。
  //
  // 模型输出的引用常是短路径（policy.go 指代 internal/policy/policy.go），
  // 直接读取失败时调用后端的 locateFile 按文件名或路径后缀在全部源文件夹
  // 中解析唯一匹配，解析结果唯一才重试打开；仍失败则保持第一次的错误提示。
  async openFileAt(path, line) {
    let opened = await get().openFile("", path, { quiet: true });
    let resolvedFolder = "";
    if (!opened) {
      const id = workspaceID();
      if (id) {
        try {
          // locateFile 在全部源文件夹中做唯一匹配，返回所属文件夹与相对路径；
          // 即使路径相同，命中的文件夹也可能不是主要目录，重试仍有意义。
          const resolved = await api.workspace.locateFile(id, path);
          if (resolved) {
            resolvedFolder = resolved.folderId ?? "";
            opened = await get().openFile(resolvedFolder, resolved.path);
          }
        } catch {
          // 解析失败：沿用第一次尝试的错误路径，由下方统一提示。
        }
      }
    }
    if (!opened) {
      useUiStore.getState().toast({
        title: "无法打开引用的文件",
        description: `在工作区内找不到 ${path}，请确认路径是否正确`,
        tone: "warn",
      });
      return;
    }
    // 定位行写进目标文件的槽（tab id 与 panel store 的构造规则一致）。
    const tabId = fileTabId(workspaceID(), resolvedFolder, path);
    set((state) => ({
      files: {
        ...state.files,
        [tabId]: { ...(state.files[tabId] ?? newFileSlot(resolvedFolder, path)), targetLine: line },
      },
    }));
  },

  // 关闭 file tab 时清空它的数据槽，草稿与定位行一起释放。
  closeFileSlot(tabId) {
    set((state) => {
      if (!state.files[tabId]) {
        return state;
      }
      const files = { ...state.files };
      delete files[tabId];
      return { files };
    });
  },

  // 二进制文件或内容被截断的文件（只读到一部分）进入编辑会导致写回时丢内容，因此直接拒绝。
  setEditorMode(tabId, mode) {
    const slot = get().files[tabId];
    if (!slot) {
      return;
    }
    if (mode === "edit" && (slot.content?.binary || slot.content?.truncated)) {
      return;
    }
    set((state) => ({
      files: {
        ...state.files,
        [tabId]: {
          ...slot,
          editorMode: mode,
          draft: mode === "edit" ? (slot.content?.content ?? slot.draft) : slot.draft,
        },
      },
    }));
  },

  setDraft(tabId, value) {
    set((state) => {
      const slot = state.files[tabId];
      if (!slot) {
        return state;
      }
      return { files: { ...state.files, [tabId]: { ...slot, draft: value } } };
    });
  },

  // 保存流程（按 tab 槽）：
  // 1. 写入目标是打开文件时记录的工作区与源文件夹（content.workspaceId /
  //    content.folderId），而不是「当前激活工作区」——切换工作区后保存
  //    不会把内容写进错误的目录；
  // 2. 带上打开文件时拿到的 hash 写入，后端据此判断文件是否被外部改动；
  // 3. 成功后重新读取文件，用后端返回的最新内容与 hash 刷新，并回到只读模式；
  // 4. 若后端报「外部修改」，说明磁盘上的内容已经变了，弹确认框由用户选择是否重新加载；
  //    重新加载会丢弃当前草稿，所以默认按钮保持危险样式；
  // 5. 其它错误走 toast 提示，草稿保留以便用户重试。
  async saveSlot(tabId) {
    const slot = get().files[tabId];
    const content = slot?.content;
    const id = content?.workspaceId ?? "";
    if (!id || !slot || !content) {
      return;
    }
    const folderID = content.folderId ?? "";
    try {
      await api.workspace.writeFile(id, folderID, content.path, slot.draft, content.hash);
      const refreshed = await api.workspace.readFile(id, folderID, content.path);
      set((state) => ({
        files: {
          ...state.files,
          [tabId]: {
            ...(state.files[tabId] ?? slot),
            content: refreshed,
            draft: refreshed.content,
            editorMode: "read",
          },
        },
      }));
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
          await get().loadFileSlot(tabId, folderID, content.path);
        }
        return;
      }
      useUiStore.getState().toast({ title: "保存失败", description: message, tone: "danger" });
    }
  },

  // 打开差异：先让后端计算差异，结果写入 store 后在面板里聚焦差异 tab
  // （singleton：已存在时只刷新标题并聚焦）。视图方式沿用上一次的选择，
  // 默认统一视图。差异在打开时记录所属工作区，应用写入时显式传回，
  // 不依赖「当前激活工作区」。
  async openDiff(title, oldText, newText) {
    const result = await api.workspace.computeDiff(oldText, newText);
    set({
      diff: {
        title,
        oldText,
        newText,
        result,
        view: get().diff?.view ?? "unified",
        workspaceId: workspaceID(),
      },
    });
    usePanelStore.getState().openDiffTab(title);
  },

  // 应用差异属于覆盖写入，必须先确认；确认后整份写入新内容（不带 hash，表示允许覆盖），
  // 写完重新打开文件，让编辑器显示磁盘上的最新状态。
  // 差异标题是相对主要目录的路径：审批差异来自事件载荷，目前固定按主要
  // 目录解析，附加目录文件的差异应用是已知限制。
  async applyDiff() {
    const diff = get().diff;
    const id = diff?.workspaceId ?? "";
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
      await api.workspace.writeFile(id, "", diff.title, diff.newText, "");
      useUiStore.getState().toast({ title: "已应用修改", description: diff.title, tone: "ok" });
      await get().openFile("", diff.title);
    } catch (error) {
      useUiStore.getState().toast({ title: "应用失败", description: String(error), tone: "danger" });
    }
  },

  // 只切换视图方式，保留已计算好的差异结果。
  setDiffView(view) {
    const diff = get().diff;
    if (diff) {
      set({ diff: { ...diff, view } });
    }
  },

  // 文档预览期间置 docLoading，让面板显示加载态；失败时清标记并提示。
  // 预览成功后在面板里打开（或聚焦）对应的文档 tab，类型取自预览结果。
  async openDocument(folderID, path) {
    const id = workspaceID();
    if (!id) {
      return;
    }
    set({ docLoading: true });
    try {
      const doc = await api.documents.preview(id, folderID, path);
      set({ doc, docLoading: false });
      usePanelStore.getState().openDocumentTab({
        folderId: folderID,
        path,
        title: basename(path),
        kind: doc.kind,
      });
    } catch (error) {
      set({ docLoading: false });
      useUiStore.getState().toast({ title: "读取文档失败", description: String(error), tone: "danger" });
    }
  },

  // 模式切换时整体清空：文件树、展开状态、Git 标记、file tab 数据槽、差异与文档预览。
  // 面板 tab 本身不清（tab 归 panel store 管并按线程记忆），切回 file tab 时
  // 渲染层发现槽已空会重新读取内容。
  reset() {
    set({
      tree: {},
      expanded: {},
      git: {},
      files: {},
      diff: null,
      doc: null,
    });
  },
}));
