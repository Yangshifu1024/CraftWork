// 右侧面板：固定视图（文件 / 子代理 / 后台任务）+ 动态 tab（计划 / 子代理详情）——
// 「固定视图 + 动态 tab」模型（D15 的面板补注）。tab 状态（展开/收起、
// 动态 tab 列表、激活 tab）在全局 store：跨会话存活的硬要求（切会话不关闭动态 tab），
// 重启随内存 store 清空（它们是界面态，事实源在事件表）。
//
// 形态约定（2026-11-11 重构，D22 布局）：
// - 不常驻：关闭即渲染 null（零占位），打开入口在会话头右端的开关按钮——
//   旧收起图标列已删（它常驻窗口右缘，曾与自绘窗口控件发生遮挡）。
// - 展开态卡片化：边框粗细/颜色/圆角/阴影/边距与对话卡片完全一致（rounded-xl + 全边框 +
//   shadow-xs，顶部 mt-2 对齐卡片 pt-2）；tab 条 h-12 是卡片 caption 行，也是拖拽区
//   （data-tauri-drag-region="deep"）：空隙拖动窗口、双击最大化窗口；tab 触发器与按钮
//   是可点击元素，被 drag.js 自动排除。
// - 最大化抽屉：覆盖内容区（z-20，App.tsx #content 为定位上下文），布局保留占位列
//   宽度（对话列不重排）；抽屉 tab 条同样贴顶成 caption 行。
// - 灵活 tab：弹性尺寸（basis 156px、收缩下限 60px、上限 156px），溢出时
//   横向滚动 + 两侧渐隐；动态 tab 中键关闭、dnd-kit 拖拽排序（固定 tab 不参与排序）。
// 文件树按工作区浏览，文件点击做文本级预览；子代理详情复用消息流的只读时间线（无输入框）。
import { useEffect, useMemo, useRef, useState } from "react";
import { Bot, FolderOpen, ListChecks, Maximize2, Minimize2, TerminalSquare } from "lucide-react";
import { DndContext, closestCenter, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext, horizontalListSortingStrategy, useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { X } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../store/app";
import { useToasts } from "../store/toast";
import { useRunningSubagentCount } from "../hooks/running-subagent-count";
import { useShellTasks } from "../hooks/running-shell-task-count";
import { formatDurationMs } from "../lib/format";
import { Markdown } from "./markdown";
import { ResizeHandle } from "./resize-handle";
import { Select } from "./select";
import { WindowControls } from "./window-controls";
import { RunningBadge } from "./running-badge";
import { PANE } from "../lib/resize";
import { highlightLines, languageForPath, type HighlightToken } from "../lib/code-highlight";
import { SUBAGENT_STATUS_LABEL } from "./cards";
import { Timeline } from "./conversation";
import type { FileNode, ShellTaskView, SubagentPart, ThreadView, WorkspaceView } from "../../../shared/types";

/** 面板展开态宽度：只由 --craft-pane-width 提供（拖拽条拖动期直接改写，见 D17）。
 *  这里不写死 px 宽度类：宽度若同时有类名与变量两个来源，状态分支切换时会互相覆盖。 */
const PANEL_WIDTH_CLASS = "min-w-[280px] shrink-0";
const PANEL_WIDTH_STYLE = { width: "var(--craft-pane-width)" } as const;

/** 当前面板生效的工作区 id：活动会话的工作区 → 草稿态取草稿槽归属（draftWsId 非空时，发送前即可浏览项目）
 *  → 回落内置临时目录（conversation 工作区）。归属查不到（如项目已删）同样回落。 */
function effectiveWorkspaceId(activeThreadId: string, draftWsId: string, threads: ThreadView[], workspaces: WorkspaceView[]): string {
  const direct = activeThreadId
    ? (threads.find((t) => t.id === activeThreadId)?.workspaceId ?? "")
    : draftWsId;
  const found = direct ? workspaces.find((w) => w.id === direct) : undefined;
  return (found ?? workspaces.find((w) => w.purpose === "conversation"))?.id ?? "";
}

/** 右侧面板：不常驻——关闭时不渲染（零占位），打开入口在会话头右端的开关按钮。 */
export function SidePane(): React.JSX.Element | null {
  const open = useApp((s) => s.paneOpen);
  const maximized = useApp((s) => s.paneMaximized);
  const paneTabs = useApp((s) => s.paneTabs);
  const activeTab = useApp((s) => s.activePaneTab);
  const setPaneOpen = useApp((s) => s.setPaneOpen);
  const setPaneMaximized = useApp((s) => s.setPaneMaximized);
  const setActivePaneTab = useApp((s) => s.setActivePaneTab);
  const paneWidth = useApp((s) => s.paneWidth);
  const setPaneWidth = useApp((s) => s.setPaneWidth);
  const platform = useApp((s) => s.platform);
  const runningCount = useRunningSubagentCount();
  // 后台任务数据：面板打开即轮询（徽标的意义正是「不在这个 tab 时也知道有几个在跑」），
  // 面板关闭立即停。数据在这里取一份，向下传给 tab 条徽标与 ShellTasksView——
  // 两处共用同一份，避免各轮询一次导致徽标与列表短暂不一致。
  const runningTasks = useShellTasks(open);
  // 自绘窗口控件仅 Windows 渲染（macOS 交通灯原生；Linux 不在本期范围，D7）
  const showWindowControls = platform === "windows";
  // 展开/收起动画的挂载状态机：open 翻 false 后节点继续挂载 220ms（宽度→0 + 淡出），
  // 动画结束才卸载；open 翻 true 立即挂载，下一帧再展开到目标宽度（入场过渡生效）。
  const [mounted, setMounted] = useState(open);
  const [shown, setShown] = useState(open);
  // 收起前是否处于抽屉态：决定收起动画走「宽度收缩」还是「抽屉滑出」
  const wasMaximizedRef = useRef(maximized);
  useEffect(() => {
    if (open) {
      if (maximized) wasMaximizedRef.current = true;
      setMounted(true);
      // 下一帧翻 shown：保证「初始 0 宽 → 目标宽」的过渡被浏览器捕捉（否则直接落终态）
      const raf = requestAnimationFrame(() => setShown(true));
      return () => cancelAnimationFrame(raf);
    }
    setShown(false);
    const timer = setTimeout(() => {
      setMounted(false);
      wasMaximizedRef.current = false;
    }, 220);
    return () => clearTimeout(timer);
  }, [open, maximized]);

  // 面板未挂载（收起动画结束后的稳态）：零占位；打开入口在会话头右端的开关按钮
  if (!mounted) {
    return null;
  }

  const closeTab = (id: string): void => useApp.getState().closePaneTab(id);

  // 展开态内容块：宽度与边框交给外层容器决定（默认态自己就是占位列，最大化态挂在抽屉里）。
  // 内容宽度锁定：普通态锁 --craft-pane-width（收起动画中父容器宽度→0 时内容不被挤压回流）；
  // 抽屉态全宽。
  const tabs = (
    <div className="flex min-h-0 flex-1 flex-col bg-background" style={{ width: maximized ? "100%" : "var(--craft-pane-width)" }}>
      {/* 卡片 caption 行：空隙拖动窗口、双击最大化窗口，tab 触发器与按钮自动排除；
          与会话头行（对话卡片内）同一高度对齐 */}
      <div
        className="flex h-12 shrink-0 items-center gap-1 border-b border-border/50 px-2"
        data-tauri-drag-region="deep"
      >
        <TabStrip
          paneTabs={paneTabs}
          activeTab={activeTab}
          runningCount={runningCount}
          runningTaskCount={runningTasks.length}
          onActivate={setActivePaneTab}
          onClose={closeTab}
        />
        {/* 放大/还原抽屉（收起面板的开关常驻会话头右端，不随面板迁移）。 */}
        <div className="flex shrink-0 items-center gap-1">
          <button
            className="flex size-6 items-center justify-center rounded-md text-foreground-subtlest hover:bg-surface-hover"
            onClick={() => setPaneMaximized(!maximized)}
            title={maximized ? "还原面板宽度" : "最大化面板"}
            aria-label={maximized ? "还原面板宽度" : "最大化面板"}
          >
            {maximized ? <Minimize2 className="size-4" /> : <Maximize2 className="size-4" />}
          </button>
          {/* shown 门控：收起过渡期控件已迁回会话头，这里若仍渲染会两份并存——
              中间区右上角闪一下控件（实机验收踩坑） */}
          {shown && showWindowControls && <WindowControls />}
        </div>
      </div>
      {/* 固定 tab 恒定存在；动态 tab 打开才渲染。非激活的动态 tab 卸载视图但数据在
          store——子代理详情的事件流继续更新 chats，切回即见最新。 */}
      {activeTab === "files" && <FilesView />}
      {activeTab === "agents" && <AgentsView />}
      {activeTab === "tasks" && <ShellTasksView tasks={runningTasks} />}
      {paneTabs
        .filter((tab) => tab.id === activeTab)
        .map((tab) =>
          tab.kind === "plan" ? (
            <PlanTabView key="plan" text={tab.text} />
          ) : (
            <SubagentDetail key={tab.id} agentId={tab.agentId} />
          ),
        )}
    </div>
  );

  // 两种展开形态共用同一个面板节点：只切换外层 className，不重建组件树。
  // key 必须显式给出——fragment 子节点按位置匹配，最大化时插入的占位 div 会把面板挤到下一位，
  // 没有 key 就会整块卸载重挂（文件树展开态 / 手动选的工作区 / 动态 tab 全丢）。
  // 收起动画（shown=false 的过渡期节点）：普通态宽度→0 + 淡出（布局平滑让位给对话列）；
  // 抽屉态滑出右侧（absolute 定位不参与布局宽度）。220ms 后由挂载状态机卸载。
  const closing = !open;
  const drawerClosing = closing && wasMaximizedRef.current;
  return (
    <>
      {/* 占位列：最大化时只保留宽度，让对话列宽度与档位不受抽屉开关影响（见 `panel.tsx` 模块头） */}
      {maximized && (
        <div key="spacer" className={PANEL_WIDTH_CLASS} style={PANEL_WIDTH_STYLE} aria-hidden />
      )}
      {/* 展开态且有占位列时，拖拽条贴在面板左缘：向左拖变宽（drag handle 不参与最大化抽屉） */}
      {shown && !maximized && (
        <ResizeHandle
          key="resize"
          cssVar="--craft-pane-width"
          edge="right"
          value={paneWidth}
          min={PANE.min}
          max={PANE.max}
          initial={PANE.initial}
          label="调整面板宽度"
          onCommit={(w) => setPaneWidth(w)}
          onCollapse={() => setPaneOpen(false)}
        />
      )}
      <div
        key="panel"
        id="side-pane"
        style={
          maximized || drawerClosing
            ? undefined
            : { width: shown ? "var(--craft-pane-width)" : "0px", opacity: shown ? 1 : 0 }
        }
        data-resize-smooth
        className={
          maximized
            ? // 抽屉：定位上下文是 App.tsx 的 #content；top-2/bottom-2 与对话卡片上下边界对齐
              // （圆角/边框/底色与卡片一致，shadow-2xl 是悬浮抬高的刻意差异）
              "absolute top-2 bottom-2 left-0 right-2 z-20 flex flex-col overflow-hidden rounded-xl border border-border bg-background shadow-2xl animate-in slide-in-from-right duration-200 ease-out motion-reduce:animate-none"
            : drawerClosing
              ? "absolute top-2 bottom-2 left-0 right-2 z-20 flex flex-col overflow-hidden rounded-xl border border-border bg-background shadow-2xl animate-out slide-out-to-right-4 fade-out duration-200 ease-in motion-reduce:animate-none"
              : // 普通展开态：边框粗细/颜色/圆角/阴影与对话卡片完全一致（rounded-xl + 全边框 +
                // shadow-xs），顶边距 mt-2 与对话卡片 pt-2 对齐；宽度过渡承担收起动画
                "mt-2 mr-2 flex shrink-0 flex-col overflow-hidden rounded-xl border border-border bg-background shadow-xs transition-[width,opacity] duration-200 ease-out"
        }
      >
        {tabs}
      </div>
    </>
  );
}

/** tab 条（caption 行主体）：固定 tab（文件/子代理，不参与排序）+ 动态 tab（dnd-kit 可拖拽
 *  排序、中键关闭）；弹性尺寸溢出时横向滚动，两侧渐隐提示还有更多。 */
function TabStrip({ paneTabs, activeTab, runningCount, runningTaskCount, onActivate, onClose }: {
  paneTabs: { id: string; kind: string; title?: string }[];
  activeTab: string;
  runningCount: number;
  runningTaskCount: number;
  onActivate(id: string): void;
  onClose(id: string): void;
}): React.JSX.Element {
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }));
  const scrollRef = useRef<HTMLDivElement | null>(null);
  // 渐隐提示按滚动位置切边：不在起点显示左渐隐、不在终点显示右渐隐
  const [edge, setEdge] = useState({ left: false, right: false });

  const updateEdges = (): void => {
    const el = scrollRef.current;
    if (!el) return;
    const max = el.scrollWidth - el.clientWidth;
    setEdge({ left: el.scrollLeft > 2, right: el.scrollLeft < max - 2 });
  };

  useEffect(() => {
    updateEdges();
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(updateEdges);
    ro.observe(el);
    el.addEventListener("scroll", updateEdges, { passive: true });
    return () => {
      ro.disconnect();
      el.removeEventListener("scroll", updateEdges);
    };
  }, [paneTabs.length]);

  const handleDragEnd = (e: DragEndEvent): void => {
    if (!e.over || e.active.id === e.over.id) return;
    useApp.getState().reorderPaneTabs(String(e.active.id), String(e.over.id));
  };

  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
      <div className="relative flex min-w-0 flex-1 items-center">
        <div
          ref={scrollRef}
          // 滚动区隐藏滚动条（渐隐即滚动提示）；tab 弹性收缩，全部到下限后才出现滚动
          className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto [scrollbar-width:none]"
        >
          <PaneTabChip
            icon={<FolderOpen className="size-3.5" />}
            label="文件"
            active={activeTab === "files"}
            onClick={() => onActivate("files")}
          />
          <PaneTabChip
            icon={<Bot className="size-3.5" />}
            label="子代理"
            active={activeTab === "agents"}
            badge={runningCount}
            onClick={() => onActivate("agents")}
          />
          <PaneTabChip
            icon={<TerminalSquare className="size-3.5" />}
            label="后台任务"
            active={activeTab === "tasks"}
            badge={runningTaskCount}
            onClick={() => onActivate("tasks")}
          />
          <SortableContext items={paneTabs.map((t) => t.id)} strategy={horizontalListSortingStrategy}>
            {paneTabs.map((tab) => (
              <SortablePaneTab
                key={tab.id}
                id={tab.id}
                icon={tab.kind === "plan" ? <ListChecks className="size-3.5" /> : <Bot className="size-3.5" />}
                label={tab.kind === "plan" ? "计划" : (tab.title ?? "子代理")}
                active={activeTab === tab.id}
                onClose={() => onClose(tab.id)}
                onClick={() => onActivate(tab.id)}
              />
            ))}
          </SortableContext>
        </div>
        {/* 渐隐遮罩：压在滚动区边缘上，pointer-events-none 不挡滚动与点击 */}
        {edge.left && (
          <div className="pointer-events-none absolute inset-y-0 left-0 w-6 bg-gradient-to-r from-background to-transparent" />
        )}
        {edge.right && (
          <div className="pointer-events-none absolute inset-y-0 right-0 w-6 bg-gradient-to-l from-background to-transparent" />
        )}
      </div>
    </DndContext>
  );
}

/** tab 触发器视觉（固定与可排序动态 tab 共用）：h-7 紧凑高、左侧类型图标、
 *  透明边框底、active 底色+浅阴影，关闭钮绝对定位右侧居中。弹性尺寸类由 className 传入
 *  （固定 tab 直挂弹性类；可排序 tab 的弹性类在包裹层上，chip 自身 flex-1 填满），两处必须一致。 */
function PaneTabChip({ icon, label, active, badge = 0, onClick, onClose, dragHandleProps, dragging, className = "flex-[1_1_9.75rem] min-w-15 max-w-39" }: {
  label: string;
  icon?: React.ReactNode;
  active: boolean;
  badge?: number;
  onClick(): void;
  onClose?(): void;
  dragHandleProps?: Record<string, unknown>;
  dragging?: boolean;
  /** 外层尺寸类：固定 tab 用默认弹性类；可排序 tab 传 flex-1（弹性在包裹层） */
  className?: string;
}): React.JSX.Element {
  return (
    <div
      className={`group relative flex h-7 min-w-0 items-center rounded-md border border-transparent transition-colors ${
        dragging ? "opacity-50" : active ? "border-border/40 bg-selected text-foreground shadow-xs" : "text-foreground-subtle hover:bg-surface-hover"
      } ${className}`}
    >
      <button
        {...dragHandleProps}
        className="flex h-full min-w-0 flex-1 items-center justify-start gap-1.5 rounded-md px-1.5 pr-2 text-ui-sm outline-none"
        onClick={onClick}
        title={label}
      >
        {icon ? <span className="flex size-3.5 shrink-0 items-center justify-center">{icon}</span> : null}
        <span className="truncate">{label}</span>
        {badge > 0 ? <RunningBadge count={badge} /> : null}
      </button>
      {onClose ? (
        <button
          className="mr-1 flex size-4 shrink-0 items-center justify-center rounded text-foreground-subtlest hover:bg-surface-hover"
          onClick={onClose}
          title="关闭标签页"
          aria-label={`关闭 ${label} 标签页`}
        >
          <X className="size-3" />
        </button>
      ) : null}
    </div>
  )
}

/** 可排序动态 tab：dnd-kit useSortable 包住视觉 chip；整片 chip 是拖拽把手，
 *  中键（auxclick）关闭。 */
function SortablePaneTab({ id, icon, label, active, onClose, onClick }: {
  id: string;
  icon: React.ReactNode;
  label: string;
  active: boolean;
  onClose(): void;
  onClick(): void;
}): React.JSX.Element {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id });
  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Translate.toString(transform), transition }}
      // 中键关闭：auxclick 的 button===1；阻止默认避免中键自动滚动
      onAuxClick={(e) => {
        if (e.button === 1) {
          e.preventDefault();
          onClose();
        }
      }}
      className="flex min-w-0 flex-[1_1_9.75rem] min-w-15 max-w-39"
    >
      <PaneTabChip
        icon={icon}
        label={label}
        active={active}
        onClose={onClose}
        onClick={onClick}
        dragging={isDragging}
        dragHandleProps={{ ...attributes, ...listeners }}
        className="flex-1"
      />
    </div>
  );
}


// ---------- 文件查看器 ----------

/** 单次渲染的最多行数：shiki 已整段 tokenize，渲染层限制 DOM 规模（超出只报行数）。 */
const MAX_VIEWER_LINES = 1000;

/** 文件内容查看器：shiki 着色（languageForPath 未收录的按纯文本），主题随 themeResolved
 *  即时切换；着色失败静默退化为纯文本（内容是信息，颜色是增强）。 */
function FileViewer({ file, onClose }: { file: ViewingFile; onClose(): void }): React.JSX.Element {
  const dark = useApp((s) => s.themeResolved) === "theme-craft-dark";
  const [tokens, setTokens] = useState<HighlightToken[][] | null>(null);
  const lang = languageForPath(file.path);

  useEffect(() => {
    let alive = true;
    setTokens(null);
    if (!lang) return;
    void highlightLines(file.content, lang, dark)
      .then((result) => {
        if (alive) setTokens(result);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [file.content, dark, lang]);

  const rows = file.content.split("\n");
  const rendered = rows.slice(0, MAX_VIEWER_LINES);
  const omitted = rows.length - rendered.length;

  return (
    <div className="flex min-h-0 min-w-[150px] flex-[4] flex-col overflow-hidden border-l border-border/50 pl-1">
      <div className="flex h-7 shrink-0 items-center gap-1 px-1">
        <span className="min-w-0 flex-1 truncate text-ui-xs text-foreground-subtle">
          {file.name}
          {file.truncated ? "（已截断）" : ""}
        </span>
        <button
          className="flex size-4 shrink-0 items-center justify-center rounded text-foreground-subtlest hover:bg-surface-hover"
          onClick={onClose}
          title="关闭文件查看"
          aria-label="关闭文件查看"
        >
          <X className="size-3" />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto rounded-md bg-surface p-2">
        <div className="w-max min-w-full font-mono text-ui-xs leading-relaxed">
          {rendered.map((line, i) => {
            const lineTokens = tokens?.[i];
            return (
              <div key={i} className="whitespace-pre">
                {lineTokens
                  ? lineTokens.map((token, j) => (
                      <span key={j} style={token.color ? { color: token.color } : undefined}>
                        {token.content}
                      </span>
                    ))
                  : line || " "}
              </div>
            );
          })}
          {omitted > 0 ? (
            <div className="px-1 py-1 text-ui-xs text-foreground-subtlest">…（还有 {omitted} 行）</div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

// ---------- 文件树 ----------

/** 根节点显示名：源文件夹路径末段；单文件夹项目直接用工作区名（与下拉一致）。 */
function folderDisplayName(folderPath: string, workspaceName: string, folderCount: number): string {
  if (folderCount <= 1) return workspaceName;
  const segments = folderPath.split(/[\\/]+/).filter(Boolean);
  return segments[segments.length - 1] ?? folderPath;
}

/** 可预览的文档格式（craft-doc 结构化提取：docx / xlsx / pdf 的文本层）；其余格式走
 *  workspaceReadFile（文本直读，二进制拒绝）。 */
const DOC_EXTS = new Set(["docx", "xlsx", "pdf"]);

/** 查看器中打开的文件（文本内容或文档提取文本）。 */
interface ViewingFile {
  name: string;
  path: string;
  content: string;
  truncated: boolean;
}

/** 文件树视图：顶部工作区下拉（默认跟随当前归属，手动选择后尊重手动值）+ 懒加载目录树
 *  （多源文件夹项目每个文件夹一棵根树），点击文件在树右侧打开带代码着色的查看器。 */
function FilesView(): React.JSX.Element {
  const workspaces = useApp((s) => s.workspaces);
  const mode = useApp((s) => s.mode);
  const activeThreadId = useApp((s) => s.activeThreadId[mode]);
  const draftWsId = useApp((s) => s.draftWsId[mode]);
  const threads = useApp((s) => s.threads[mode]);
  const pushToast = useToasts((s) => s.push);
  const [pickedId, setPickedId] = useState("");
  // 默认选中跟随当前归属（活动会话工作区 / 草稿槽归属 / 内置临时目录），用户下拉选择后尊重手动选择。
  const wsId = pickedId || effectiveWorkspaceId(activeThreadId, draftWsId, threads, workspaces);
  const ws = workspaces.find((w) => w.id === wsId);
  const [viewing, setViewing] = useState<ViewingFile | null>(null);

  /** 点击文件：docx/xlsx/pdf 走结构化提取（craft-doc），其余文本直读，二进制提示拒绝。
   *  再点同一文件收起查看器。 */
  const openFile = (folderId: string, node: FileNode): void => {
    if (viewing && viewing.path === node.path) {
      setViewing(null);
      return;
    }
    const ext = node.name.includes(".") ? node.name.split(".").pop()!.toLowerCase() : "";
    if (DOC_EXTS.has(ext)) {
      void api
        .documentPreview(ws!.id, folderId, node.path)
        .then((p) => setViewing({ name: node.name, path: node.path, content: p.text || p.note || "（空）", truncated: false }))
        .catch((err) => pushToast("danger", "文档预览失败", String(err)));
      return;
    }
    void api
      .workspaceReadFile(ws!.id, folderId, node.path)
      .then((fc) => {
        if (fc.binary) {
          pushToast("danger", "无法预览", "二进制文件不支持预览");
          return;
        }
        setViewing({ name: node.name, path: node.path, content: fc.content, truncated: fc.truncated });
      })
      .catch((err) => pushToast("danger", "读取文件失败", String(err)));
  };

  return (
    // px-2 由容器承担：Select 的 fullWidth（w-full）与水平 margin 同用会溢出父容器
    // （宽度仍 100%，右 margin 被裁——同属性/尺寸类组合的坑），内边距放容器才对称
    <div className="flex min-h-0 flex-1 flex-col px-2 pb-2">
      <Select
        ariaLabel="选择工作区"
        size="xs"
        fullWidth
        placeholder="请选择工作区"
        className="my-1.5"
        value={ws?.id ?? ""}
        onChange={setPickedId}
        options={workspaces.map((w) => ({ value: w.id, label: w.name }))}
      />
      {/* 树 + 查看器双列：点击文件在树右侧打开内容（面板可拖宽 / 最大化抽屉给足空间；
          再点同一文件或 × 收起查看器回到全宽树）。多根项目每个源文件夹一棵树。 */}
      <div className="flex min-h-0 flex-1 flex-row">
        {/* 树：查看器 = 1:4（用户裁决）；min-w 130px 是可读下限——窄面板下限先生效，
            拖宽面板或最大化抽屉后比例才完整呈现 */}
        <div className="flex min-h-0 min-w-[130px] flex-[1] flex-col overflow-y-auto">
          {ws
            ? ws.folders.map((f) => (
                <DirNode
                  key={f.id}
                  workspaceId={ws.id}
                  folderId={f.id}
                  rel=""
                  depth={0}
                  name={folderDisplayName(f.path, ws.name, ws.folders.length)}
                  onSelect={openFile}
                />
              ))
            : (
              <div className="px-3 py-8 text-center text-ui-sm text-foreground-subtlest">没有工作区</div>
            )}
        </div>
        {viewing && <FileViewer file={viewing} onClose={() => setViewing(null)} />}
      </div>
    </div>
  );
}
/** 目录节点：懒加载——首次展开（nodes === null）才请求子项；根节点默认展开。
 *  onSelect：文件叶子的点击回调（folderId + 节点），由 FilesView 提供并驱动查看器。 */
function DirNode({ workspaceId, folderId, rel, depth, name, onSelect }: { workspaceId: string; folderId: string; rel: string; depth: number; name: string; onSelect(folderId: string, node: FileNode): void }): React.JSX.Element {
  const [open, setOpen] = useState(depth === 0);
  const [nodes, setNodes] = useState<FileNode[] | null>(null);

  useEffect(() => {
    if (open && nodes === null) {
      // 拉取失败静默保持 null：折叠/展开根节点即可重试（不锁死空列表）
      void api.workspaceListDir(workspaceId, folderId, rel).then(setNodes).catch(() => undefined);
    }
  }, [open, nodes, workspaceId, folderId, rel]);

  return (
    <div>
      <button
        className="flex w-full items-center gap-1 rounded-md px-1.5 py-1 text-left text-ui-sm text-foreground hover:bg-surface-hover"
        style={{ paddingLeft: `${depth * 12 + 6}px` }}
        onClick={() => setOpen(!open)}
      >
        <span className="shrink-0 text-foreground-subtlest">{open ? "▾" : "▸"}</span>
        <span className="truncate">{name}</span>
      </button>
      {open &&
        (nodes ?? []).map((node) =>
          node.type === "dir" ? (
            <DirNode key={node.path} workspaceId={workspaceId} folderId={folderId} rel={node.path} depth={depth + 1} name={node.name} onSelect={onSelect} />
          ) : (
            <FileLeaf key={node.path} folderId={folderId} node={node} depth={depth + 1} onSelect={onSelect} />
          ),
        )}
    </div>
  );
}

/** 文件叶子：单击交给 FilesView 的 onSelect（查看器在树右侧打开）。 */
function FileLeaf({ folderId, node, depth, onSelect }: { folderId: string; node: FileNode; depth: number; onSelect(folderId: string, node: FileNode): void }): React.JSX.Element {
  return (
    <button
      className="flex w-full items-center gap-1 rounded-md px-1.5 py-1 text-left text-ui-sm text-foreground-subtle hover:bg-surface-hover"
      style={{ paddingLeft: `${depth * 12 + 6}px` }}
      onClick={() => onSelect(folderId, node)}
    >
      <span className="truncate">{node.name}</span>
    </button>
  );
}

// ---------- 子代理 ----------

/** 子代理固定视图：当前会话派生的子代理列表（跨会话索引按 parentThreadId 过滤——
 *  列表跟随当前会话，切换项目后不被其他会话的子代理干扰）。挂载时兜底恢复：
 *  重启后索引为空，经 subagentList 通道拉回本会话历史派发记录（含终态）。 */
function AgentsView(): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const activeThreadId = useApp((s) => s.activeThreadId[mode]);
  const subagentIndex = useApp((s) => s.subagentIndex);
  // 派生列表在组件内用 useMemo 缓存：selector 里直接 filter/sort 每次返回新数组，
  // useSyncExternalStore 会判定快照持续变化（"getSnapshot should be cached" 无限循环）。
  const list = useMemo(
    () =>
      Object.values(subagentIndex)
        .filter((v) => v.parentThreadId === activeThreadId)
        .sort((a, b) => a.startedAt - b.startedAt),
    [subagentIndex, activeThreadId],
  );
  const openSubagentTab = useApp((s) => s.openSubagentTab);
  const ensureSubagentRestored = useApp((s) => s.ensureSubagentRestored);

  // 兜底恢复按会话去重（subagentRestored），切回会话不重复拉取
  useEffect(() => {
    void ensureSubagentRestored();
  }, [ensureSubagentRestored, activeThreadId]);

  if (list.length === 0) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-center text-ui-sm text-foreground-subtlest">
        本会话还没有派发过子代理
      </div>
    );
  }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-1 py-1.5">
      {list.map((item) => (
        <AgentRow key={item.agentId} item={item} onOpen={() => void openSubagentTab(item.agentId)} />
      ))}
    </div>
  );
}

/** 子代理列表行：名称、类型、状态与派发摘要一眼可辨；工具次数、已用轮数与 token
 *  随中间态进度事件实时增长（判断它是在推进还是卡住了）。点击整行打开详情 tab。 */
function AgentRow({ item, onOpen }: { item: SubagentPart; onOpen(): void }): React.JSX.Element {
  const running = item.status === "running";
  const failed = item.status === "failed" || item.status === "interrupted";
  return (
    <button
      className="flex w-full items-start gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-surface-hover"
      onClick={onOpen}
      title="在右侧栏查看这个子代理的完整过程（只读）"
    >
      <Bot className={`mt-0.5 size-3.5 shrink-0 ${running ? "text-foreground" : "text-foreground-subtlest"}`} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className={`shrink-0 font-mono text-ui-sm ${running ? "animated-gradient-text" : "text-foreground"}`}>
            {item.name}
          </span>
          <span className="shrink-0 rounded-md bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle">
            {item.agentTypeLabel || item.agentType}
          </span>
          <span className={`shrink-0 text-ui-xs ${failed ? "text-destructive" : running ? "animated-gradient-text" : "text-foreground-subtlest"}`}>
            {SUBAGENT_STATUS_LABEL[item.status]}
          </span>
        </div>
        <div className="mt-0.5 flex items-center gap-2 text-ui-xs text-foreground-subtlest">
          <span className="min-w-0 flex-1 truncate">{item.description}</span>
          <span className="shrink-0 font-mono">轮 {item.iterations}/{item.maxIterations > 0 ? item.maxIterations : "不限"}</span>
          <span className="shrink-0 font-mono">工具 {item.toolUses}</span>
          <span className="shrink-0 font-mono">token {item.tokens}</span>
        </div>
      </div>
    </button>
  );
}

// ---------- 后台任务 ----------

/** 后台任务固定视图：当前会话仍在运行的后台命令任务（`bash_*`）。
 *
 *  数据源是 `chatShellTasks` 轮询（`useShellTasks`），只列 Running——终态任务不保留展示
 *  （`crates/craft-tools/src/shell_tasks.rs` 的模块头：已通知终态会被丢弃，列出来列表长度
 *  会随通知投递抖动）。要回看已结束任务的输出走聊天里的 `bash_output` 工具卡。
 *
 *  两个操作与聊天工具卡同源：读取输出走 `chatBashOutput`（不经工具链、无审批），
 *  终止走 `chatKillShell`（整组杀含子进程）。
 *
 *  **渲染约束（`docs/agents/pitfalls.md` 的 WebView2 油漆残影条目）**：轮询每 2s 换一个
 *  新数组引用，但行的存在与否只由任务增删决定——行内只有一个 shimmer（任务 id），
 *  状态与时长不参与整块重绘。不做「空态整块换成列表」之外的列表替换。 */
function ShellTasksView({ tasks }: { tasks: ShellTaskView[] }): React.JSX.Element {
  const toast = useToasts((s) => s.push);
  const [viewing, setViewing] = useState<{ id: string; output: string } | null>(null);

  const readOutput = async (taskId: string): Promise<void> => {
    try {
      const r = await api.chatBashOutput(activeThread(), taskId, 8192);
      setViewing({ id: taskId, output: r.output || "（暂无输出）" });
    } catch (e) {
      toast("danger", "读取输出失败", String(e));
    }
  };

  const kill = async (taskId: string): Promise<void> => {
    try {
      const r = await api.chatKillShell(activeThread(), taskId);
      if (!r.killed) toast("info", "该任务已经结束");
      // 不在这里手动移除行：下一次轮询（≤2s）自然消失，避免与后端事实两套口径
    } catch (e) {
      toast("danger", "终止失败", String(e));
    }
  };

  if (tasks.length === 0) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-center text-ui-sm text-foreground-subtlest">
        当前没有运行中的后台任务
      </div>
    );
  }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-1 py-1.5">
      {tasks.map((task) => (
        <div key={task.id} className="rounded-lg px-2 py-1.5 hover:bg-surface-hover">
          <div className="flex items-center gap-2">
            <TerminalSquare className="size-3.5 shrink-0 text-foreground-subtlest" />
            {/* 每行只保留这一处 shimmer（§7.6：单卡至多一个 shimmer 元素），它表示「仍在跑」 */}
            <span className="shrink-0 font-mono text-ui-sm animated-gradient-text">
              {task.id}
            </span>
            {task.fromTimeout ? (
              <span className="shrink-0 rounded-md bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtle">
                超时转后台
              </span>
            ) : null}
            <span className="shrink-0 text-ui-xs text-foreground-subtlest">运行中</span>
          </div>
          <div className="mt-0.5 truncate font-mono text-ui-xs text-foreground-subtle" title={task.command}>
            {task.command}
          </div>
          <div className="mt-0.5 flex items-center gap-2 text-ui-xs text-foreground-subtlest">
            <span className="shrink-0 font-mono">pid {task.pid}</span>
            <ShellTaskElapsed startedAt={task.startedAt} />
            <span className="flex-1" />
            <button
              className="shrink-0 rounded px-1.5 py-0.5 hover:bg-surface text-foreground-subtle"
              onClick={() => void readOutput(task.id)}
            >
              读取输出
            </button>
            <button
              className="shrink-0 rounded px-1.5 py-0.5 hover:bg-surface text-destructive"
              onClick={() => void kill(task.id)}
            >
              终止
            </button>
          </div>
        </div>
      ))}
      {viewing ? (
        <div className="mx-1 mt-1 rounded-lg border border-border/50 bg-surface p-2">
          <div className="mb-1 flex items-center gap-2">
            <span className="min-w-0 flex-1 truncate font-mono text-ui-xs text-foreground-subtle">{viewing.id}</span>
            <button
              className="shrink-0 rounded px-1.5 py-0.5 text-ui-xs hover:bg-surface-hover text-foreground-subtle"
              onClick={() => setViewing(null)}
            >
              关闭
            </button>
          </div>
          <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all font-mono text-ui-xs text-foreground-subtle">{viewing.output}</pre>
        </div>
      ) : null}
    </div>
  );
}

/** 独立计时小组件（同 `RunLoadingTimer` 的范式）：interval 自刷内部 state，秒数推进只
 *  重渲染计时本身，不牵动整行。轮询拿到的 `durationMs` 是快照，运行中会停在采样时刻，
 *  故这里按 `startedAt` 实时算。 */
function ShellTaskElapsed({ startedAt }: { startedAt: number }): React.JSX.Element {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  return (
    <span className="shrink-0 font-mono tabular-nums">已运行 {formatDurationMs(Math.max(0, now - startedAt))}</span>
  );
}

/** 当前活动会话 id（后台任务通道都按会话校验，跨会话查不到）。 */
function activeThread(): string {
  const s = useApp.getState();
  return s.activeThreadId[s.mode] ?? "";
}

/** 子代理详情 tab：只读的子会话时间线（无输入框——它不接受用户直接发言）。
 *  数据 = chats[agentId]（子会话事件本来就按会话 id 推送到渲染层，与活动会话无关——
 *  切走后 tab 继续实时更新）；元信息 = 跨会话索引直取。
 *  已知边界：父会话删除时子会话级联删除，残留 tab 拉不到快照——显示空态占位，手动关闭。 */
function SubagentDetail({ agentId }: { agentId: string }): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const meta = useApp((s) => s.subagentIndex[agentId]);
  const missing = useApp((s) => s.subagentChatMissing[agentId]);
  const chat = useApp((s) => s.chats[agentId]);
  const thread = useApp((s) => s.threads[mode].find((t) => t.id === s.activeThreadId[mode]));
  const ensureSubagentChat = useApp((s) => s.ensureSubagentChat);

  // 兜底装载（正常路径 openSubagentTab 已装载；这里幂等——已装载则跳过）
  useEffect(() => {
    void ensureSubagentChat(agentId);
  }, [ensureSubagentChat, agentId]);

  if (missing && (!chat || chat.messages.length === 0)) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
        <Bot className="size-5 text-foreground-subtlest" />
        <div className="text-ui-sm text-foreground-subtlest">该子代理会话已删除</div>
        <div className="text-ui-xs text-foreground-subtlest">父会话删除时子会话一并删除；此标签页可手动关闭</div>
      </div>
    );
  }

  const running = meta?.status === "running";
  const failed = meta?.status === "failed" || meta?.status === "interrupted";
  return (
    // @container/conversation：Timeline 内部有基于该命名容器的宽度档位与提问导航闸门，
    // 子代理详情同样以自身宽度裁决——380px 远低于 864px，导航轨与宽档内容列自然不出现。
    <div className="@container/conversation flex min-h-0 flex-1 flex-col">
      {/* 轻量元信息行：名称、类型、只读标记 + 进度（轮数 / 工具 / 状态） */}
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-border/40 px-3">        <span className={`min-w-0 truncate font-mono text-ui-sm ${running ? "animated-gradient-text" : "text-foreground"}`}>
          {meta?.name ?? agentId}
        </span>
        {meta ? (
          <span className="shrink-0 rounded-md bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle">
            {meta.agentTypeLabel || meta.agentType}
          </span>
        ) : null}
        <span className="shrink-0 rounded-md bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtlest">只读</span>
        <div className="flex-1" />
        {meta ? (
          <>
            <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">
              轮 {meta.iterations}/{meta.maxIterations > 0 ? meta.maxIterations : "不限"}
            </span>
            <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">工具 {meta.toolUses}</span>
            <span className={`shrink-0 text-ui-xs ${failed ? "text-destructive" : running ? "animated-gradient-text" : "text-foreground-subtlest"}`}>
              {SUBAGENT_STATUS_LABEL[meta.status]}
            </span>
          </>
        ) : null}
      </div>
      <Timeline
        activeId={`subagent:${agentId}`}
        draftLayout={false}
        workspaceId={thread?.workspaceId ?? ""}
        chat={chat ? { messages: chat.messages, running: chat.running } : undefined}
        footer={null}
      />
    </div>
  );
}

// ---------- 计划 ----------

/** 计划动态 tab（单例）：计划模式生成的计划正文（审批卡「查看计划」经 openPlanTab
 *  写入；先后查看不同计划复用同一个 tab——更新内容并激活）。 */
function PlanTabView({ text }: { text: string }): React.JSX.Element {
  return (
    // overscroll-contain：滚到内容边界不再 chaining 到外部（窗口/消息流）
    <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-3">
      {text ? (
        <Markdown text={text} />
      ) : (
        <div className="flex h-full items-center justify-center px-6 text-center text-ui-sm text-foreground-subtlest">
          暂无计划：计划模式生成的计划会在这里展示
        </div>
      )}
    </div>
  );
}
