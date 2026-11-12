// 对话区：工作区标题栏 + 消息流（Timeline）+ 沉底输入区（Composer）。
// 滚动贴底与「回到底部」由 use-stick-to-bottom 托管；发送 / 引导（steer）/ 停止（armed 二次确认）编排收在 Composer。
// Timeline 同时被右侧面板的子代理详情 tab 复用（无输入框形态；对话区不反向依赖面板，无循环依赖）。
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useStickToBottom } from "use-stick-to-bottom";
import { ArrowUp, Check, ChevronDown, ChevronRight, Brain, AlertCircle, Copy, RefreshCw, Square, Plus, Paperclip, X, Folder, ListChecks, GitBranch, WandSparkles, Puzzle, PanelRight, PanelRightClose, Loader } from "lucide-react";
import { Popover, HoverCard } from "radix-ui";
import { ModelPicker } from "./model-picker";
import { QuotaChip } from "./quota-chip";
import { TurnNavigator } from "./turn-navigator";
import { buildTurnNavItems, PLAN_CONFIRM_TEXT as NAV_PLAN_CONFIRM_TEXT, TURN_NAV_MIN_WIDTH_PX, type TurnNavItem } from "../lib/turn-nav";
import { useApp, draftSlot, effectiveModelRef, effectiveReasoningLevel, EMPTY_DRAFT, EMPTY_THREAD_COMPOSER } from "../store/app";
import { useOverlays } from "../store/overlays";
import { useToasts } from "../store/toast";
import { api } from "../lib/api";
import { cleanErrorMessage } from "../lib/errors";
import { isImageName, pastedFileName, readFileAsBase64, selectPastedFiles } from "../lib/paste";
import { APPROVAL_LEVELS, APPROVAL_LEVEL_NAMES, APPROVAL_LEVEL_ICONS, useApprovalLevelAction } from "../lib/approval";
import { WindowControls } from "./window-controls";
import { RunningBadge } from "./running-badge";
import { useRunningSubagentCount } from "../hooks/running-subagent-count";
import { reasoningLabel } from "../lib/reasoning";
import { selectableModels } from "../lib/provider-list";
import { formatDurationMs, formatElapsed, formatSpeed, formatTokens, trimBlankLines } from "../lib/format";
import { isBackgroundPollPart, mergeBackgroundPolls } from "../lib/background-poll";
import { isImeComposing } from "../lib/ime";
import { rankSkillCandidates } from "../lib/skill-match";
import type { ApprovalPart, ToolCallPart } from "../../../shared/events";
import type { MessageSnapshot, PartSnapshot, SkillView, SubagentPart } from "../../../shared/types";
import { Markdown } from "./markdown";
import { ToolDiff, DiffStat } from "./tool-diff";
import { TodoCard, SubagentCard } from "./cards";

// 内容列宽度档位（决策 D17 的同族约定：宽度是姿态，不持久化）：以对话卡片为命名容器
// `conversation` 做容器查询——与窗口/侧栏宽度解耦，右侧面板开合改变容器宽度时档位自动升降。
// 各档默认值都是可插值长度（max-w-full），断点跨越与草稿→会话切换对 max-width 平滑过渡不硬跳。
// 草稿档（会话无消息）固定收窄到 672px 聚焦；会话档随容器 896px（≥864）→ 1152px（≥1280）放宽。
// 864 这个断点同时是提问导航竖轨的出现门槛（lib/turn-nav.ts 的 TURN_NAV_MIN_WIDTH_PX），
// 两处必须同值：窄档既不该有宽内容列，也不该有导航轨。
const CONTENT_WIDTH_CLASS =
  "max-w-full transition-[max-width] duration-150 ease-out @min-[864px]/conversation:max-w-4xl @min-[1280px]/conversation:max-w-6xl";
const DRAFT_WIDTH_CLASS = "max-w-2xl transition-[max-width] duration-150 ease-out";
// 上述两条的 150ms max-width 过渡在拖动期必须关掉（拖拽条挂 html[data-resizing]），
// 否则容器宽度连续变化时档位会一路闪跳、松手还留一段拖尾。

/** 批准计划的固定确认消息（后端在历史里排除）：历史翻阅时排除，避免回填假输入。
 *  同一常量也用于提问导航的过滤（唯一事实源在 lib/turn-nav.ts）。 */
const PLAN_CONFIRM_TEXT = NAV_PLAN_CONFIRM_TEXT;

/** 空 messages 占位：selector 稳定引用，避免未装载会话时 useMemo 因新数组字面量反复重算。 */
const NO_MESSAGES: MessageSnapshot[] = [];

/** 对话区整体：标题取活动会话，草稿态（无活动会话）显示草稿槽归属的工作区名。
 *  子代理查看已收拢到右侧面板的动态详情 tab（D15 的面板补注）：
 *  对话区不再被子会话只读视图整块替换，主对话不被子代理查看打断。 */
export function Conversation(): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const activeId = useApp((s) => s.activeThreadId[mode]);
  const chat = useApp((s) => s.chats[activeId]);
  const thread = useApp((s) => s.threads[mode].find((t) => t.id === activeId));
  const draftWsId = useApp((s) => s.draftWsId[mode]);
  const workspaces = useApp((s) => s.workspaces);
  // 草稿态（活动会话为空）标题显示槽归属：项目槽显示该工作区名（查不到回落「新对话」），临时槽显示「新对话」。
  const draftWs = activeId === "" && draftWsId ? workspaces.find((w) => w.id === draftWsId) : undefined;
  const title = activeId === "" ? (draftWs?.name ?? "新对话") : (thread?.title ?? "新对话");
  // 草稿档：会话尚无消息（草稿槽或新建会话）时内容列收窄聚焦，发出首条消息后放宽到会话档。
  const draftLayout = !chat || chat.messages.length === 0;

  // 面板关时补右外边距：卡片四周留白一致（面板开时右缘与面板卡片贴合无间距）
  const paneOpen = useApp((s) => s.paneOpen);
  return (
    // pt-2：卡片顶边距与右侧面板卡片的 mt-2 对齐（D22：两卡片边框/圆角/边距完全一致）
    <div className={`flex min-h-0 min-w-[320px] flex-1 flex-col pt-2 ${paneOpen ? "" : "mr-2"}`}>
      {/* @container/conversation：内容列宽度档位的裁决容器（Timeline 与 Composer 都在其内）。 */}
      <div className="@container/conversation relative flex h-full min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border bg-background shadow-xs">
        <WorkspaceHeader title={title} status={thread?.status ?? "idle"} mode={mode} />
        {/* 输入区以 sticky 沉底浮在消息流上：滚动区贯穿到容器底，滚动条贴底=贴窗口底 */}
        <Timeline
          activeId={activeId}
          draftLayout={draftLayout}
          workspaceId={thread?.workspaceId ?? ""}
          chat={chat ? { messages: chat.messages, running: chat.running } : undefined}
          footer={
            <Composer
              activeId={activeId}
              draftLayout={draftLayout}
              chat={chat ? { running: chat.running } : { running: false }}
            />
          }
        />
      </div>
    </div>
  );
}

/** 会话顶栏：标题 + 模式徽章 + 运行状态（运行中=文字扫光 / 等待审批 / 出错）
 *  +（面板关时）右端 [展开侧栏开关 + Windows 窗口控件]——面板开时这组按钮迁移到
 *  面板 tab 条右端（panel.tsx），此处只剩标题区（控件内联迁移，D22）。 */
function WorkspaceHeader({ title, status, mode }: { title: string; status: string; mode: string }): React.JSX.Element {
  const paneOpen = useApp((s) => s.paneOpen);
  const setPaneOpen = useApp((s) => s.setPaneOpen);
  const platform = useApp((s) => s.platform);
  const runningCount = useRunningSubagentCount();
  // 面板收起后窗口控件延时回归：折叠动画 220ms（panel.tsx 挂载状态机）完全结束 + 20ms 缓冲，
  // 才在会话头显示控件——避免与仍在淡出的面板条残影争位闪烁（用户裁决的节奏）。
  // 仅「开 → 关」跳变时延时；应用启动即面板关的场景不延时（无闪烁语境）。
  const [controlsVisible, setControlsVisible] = useState(true);
  const prevPaneOpenRef = useRef(paneOpen);
  useEffect(() => {
    if (prevPaneOpenRef.current === paneOpen) return;
    prevPaneOpenRef.current = paneOpen;
    if (paneOpen) {
      setControlsVisible(false);
      return;
    }
    const timer = setTimeout(() => setControlsVisible(true), 240);
    return () => clearTimeout(timer);
  }, [paneOpen]);
  // "deep"：子树内任意点击均可拖拽/双击最大化。裸属性只匹配直接命中的元素本身，
  // 而内层 div 铺满标题栏，点击永远落不到外层上；按钮等可点击元素会自动阻断，无需单独排除。
  // select-none：macOS 双击最大化时 drag.js 不拦截默认行为（以支持移动取消），需禁用选字。
  return (
    <div className="relative flex w-full shrink-0 select-none border-b border-border/50" data-tauri-drag-region="deep">
      <div className="flex h-12 min-w-0 flex-1 items-center justify-between gap-2 overflow-hidden p-2">
        <div className="flex min-w-0 items-center gap-2 pl-1">
          <span className="truncate text-ui-base font-medium text-foreground">{title}</span>
          {/* 模式徽章：浅色模式色底 + 对应文字色（与侧栏切换器同源 token） */}
          <span
            className={`shrink-0 rounded-full px-2 py-0.5 text-ui-xs ${
              mode === "work"
                ? "bg-mode-work-soft text-mode-work-soft-foreground"
                : "bg-mode-code-soft text-mode-code-soft-foreground"
            }`}
          >
            {mode === "work" ? "Work" : "Code"}
          </span>
          {status === "running" && <span className="animated-gradient-text shrink-0 text-ui-xs">运行中</span>}
          {status === "awaiting" && <span className="shrink-0 text-ui-xs text-warning">等待审批</span>}
          {status === "error" && <span className="shrink-0 text-ui-xs text-destructive">出错</span>}
        </div>
        {/* 右端动作区：面板开合开关【常驻此处】（不随右侧面板迁移）——
            面板关 = 展开侧栏，面板开 = 收起侧栏；Windows 窗口控件仅在面板关时驻留会话头，
            面板开时迁移到面板条右端（panel.tsx）。macOS 无窗口控件（交通灯原生）。 */}
        <div className="flex shrink-0 items-center gap-1">
          <button
            className="relative flex size-6 items-center justify-center rounded-md text-foreground-subtlest hover:bg-surface-hover"
            onClick={() => setPaneOpen(!paneOpen)}
            title={paneOpen ? "收起侧栏" : "展开侧栏"}
            aria-label={paneOpen ? "收起侧栏" : "展开侧栏"}
          >
            {paneOpen ? <PanelRightClose className="size-4" /> : <PanelRight className="size-4" />}
            {runningCount > 0 && <RunningBadge count={runningCount} floating />}
          </button>
          {!paneOpen && controlsVisible && platform === "windows" && <WindowControls />}
        </div>
      </div>
    </div>
  );
}

// ---------- 消息流 ----------

/** 消息流滚动区：贴底状态由 use-stick-to-bottom 托管；footer（输入区）以 sticky 沉底、不挡消息点击。
 *  数据全走 props、不依赖活动会话，可复用：右侧面板的子代理详情 tab 以 footer=null（无输入框）
 *  形态复用它（D15 的面板补注；对话区不反向依赖面板，无循环依赖）。 */
export function Timeline({
  activeId,
  draftLayout,
  workspaceId,
  chat,
  footer,
}: {
  activeId: string;
  draftLayout: boolean;
  /** 会话绑定的工作区 id：斜杠技能 chip 查询描述时按它刷新技能清单 */
  workspaceId: string;
  chat?: { messages: MessageSnapshot[]; running: boolean };
  /** 输入区；子代理只读视图传 null（不渲染输入框） */
  footer?: React.ReactNode;
}): React.JSX.Element {
  // 流式跟随动画由设置控制：默认关闭（resize=instant，新内容直接跳到底），开启走库默认弹簧。
  // 库每次渲染把 options 写进内部 ref，切换开关立即生效，无需重建实例。
  const smoothFollow = useApp((s) => s.settings.smoothFollow);
  const { scrollRef, contentRef, isAtBottom, scrollToBottom, stopScroll } = useStickToBottom({
    resize: smoothFollow ? undefined : "instant",
  });
  const messages = chat?.messages ?? [];

  // 贴底状态镜像：完成贴底在跨越时刻读「此刻」的贴底状态（用户上翻阅读历史就不打扰）。
  // 用库返回的 isAtBottom（贴底或近底），不读 escapedFromLock——后者被 stopScroll() 置 true
  // （发送消息、切会话都会调用），且滚动位置没变时不会触发 scroll 事件复位，判断会一直失准。
  const atBottomRef = useRef(isAtBottom);
  useEffect(() => {
    atBottomRef.current = isAtBottom;
  }, [isAtBottom]);

  useEffect(() => {
    // 切换会话时贴底：instant 直达。默认弹簧动画下，内容高度骤变引发的 scrollTop 钳制
    // 会被库误判为「用户向上滚」（escape）并在下一帧取消动画，贴底会停在顶部附近。
    stopScroll();
    scrollToBottom("instant");
  }, [activeId, scrollToBottom, stopScroll]);

  // 完成贴底（`conversation.tsx` 的 Timeline 注释）：run 结束后结论正文就位，但过程折叠区
  // 还要走 300ms 高度过渡（过渡结束才卸载内容），轮尾「工作中」槽位也同时消失——这段时间内容
  // 高度持续收缩，「平滑跟随」开启时弹簧动画还可能停在半路，贴底会留下残余。等过渡走完再补一次
  // instant 贴底，让「已工作 · 持续 N 秒 + 结论」完整落在视口底部。
  // 边界：① 只认同一会话内「运行中 → 已结束」的这次跨越，切换会话不触发；
  //       ② 只在贴底时补这一下：用户上翻阅读历史就不打扰（与流式期间「翻历史不被拉走」一致）。
  //          跨越这一刻先判一次（等折叠收完再判已失真：负 resize 会让库自己把状态重置回贴底），
  //          延时到点时再判一次（覆盖用户在这 360ms 窗口里又上翻的情况）。
  const prevRun = useRef({ threadId: activeId, running: chat?.running ?? false });
  useEffect(() => {
    const running = chat?.running ?? false;
    const prev = prevRun.current;
    prevRun.current = { threadId: activeId, running };
    if (prev.threadId !== activeId || !prev.running || running) return;
    if (!atBottomRef.current) return;
    const timer = setTimeout(() => {
      // 延时窗口内用户又上翻的情况：此刻位置已经远离底部就不再拉回（滚轮上翻会即时翻转贴底状态）
      if (!atBottomRef.current) return;
      scrollToBottom("instant");
    }, FOLD_TRANSITION_MS + FOLD_SETTLE_EXTRA_MS);
    return () => clearTimeout(timer);
  }, [activeId, chat?.running, scrollToBottom]);

  // 审批决定贴底（`conversation.tsx` 的 Timeline 注释）：用户点审批卡按钮（批准 / 忽略）
  // 或 run 取消导致等待卡收口时，卡片从常驻槽位回归时序位置、按钮组卸载、正文预览收起——
  // 内容净收缩，库对负 resize 只解除逃逸、不移动滚动位置；而批准后内容随即重新增长，
  // 若此前用户读计划时上翻过（escapedFromLock=true），库的 resize 跟随会在第一帧直接放弃，
  // 整个实施阶段都不跟随（只能手动点「回到底部」）。所以这里必须主动贴底一次。
  // 取态与「发送消息即贴底」一致：这是用户显式推进任务的动作，即使正翻历史也要拉回底部。
  // 观察等待中的审批卡数量而非在卡片回调里发信号：决定后的卡片状态由 approval.resolved
  // 事件经 applyBatch 应用，卡片自身不知道渲染层何时更新完；观察消息流形状变化才是
  // Timeline 里既有的写法（同 prevRun / prevUserCount）。
  const waitingCount = messages.reduce(
    (n, m) => n + m.parts.filter((p) => p.kind === "approval" && (p.payload as unknown as ApprovalPart).status === "waiting").length,
    0,
  );
  const prevWaiting = useRef({ threadId: activeId, count: waitingCount });
  useLayoutEffect(() => {
    const prev = prevWaiting.current;
    prevWaiting.current = { threadId: activeId, count: waitingCount };
    // 只认同一会话内「有等待卡 → 等待卡变少」的这次跨越，切换会话不触发
    if (prev.threadId !== activeId || prev.count === 0 || waitingCount >= prev.count) return;
    stopScroll();
    scrollToBottom("instant");
    // 再补一次：这段窗口里正文预览卸载、按钮组卸载、轮尾「工作中」槽位挂上，
    // 布局要落定之后视口底部才是真正的底部。这一次只在仍贴底时执行——
    // 用户在处理结果时手动上翻阅读，就不再把他拉回来（与完成贴底同取态）。
    const timer = setTimeout(() => {
      if (!atBottomRef.current) return;
      scrollToBottom("instant");
    }, APPROVAL_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [activeId, waitingCount, scrollToBottom, stopScroll]);

  // 用户发送新消息（user 消息数增加）时立即贴底：发送者必然想看到刚发出的消息，
  // 即使正翻在历史里也要拉回底部，否则会被误认为发送失败。
  // 助手流式回复不走这里——翻历史阅读时保持 use-stick-to-bottom 的「不打断」行为。
  const prevUserCount = useRef(0);
  useLayoutEffect(() => {
    const count = messages.reduce((n, m) => (m.role === "user" ? n + 1 : n), 0);
    if (count > prevUserCount.current) {
      stopScroll();
      scrollToBottom("instant");
    }
    prevUserCount.current = count;
  }, [messages, scrollToBottom, stopScroll]);

  // 计划反馈消息归并（via=plan_feedback）：修改意见本是计划审阅卡的一部分（用户在卡上
  // 决定「提出修改」产生的），单独渲染会在时间线末尾挂一个右下角气泡，与卡片脱节。
  // 渲染时抽掉这些消息，反馈文本塞进它对应的那张计划审阅卡里展示。
  //
  // 归属到「具体卡片」而不是「助手消息」：一个助手消息里可以先后出现多张 exit_plan 卡
  // （提出修改 → 模型修订后重新提交），按消息记账时旧卡拿不到自己的意见，而新卡会把
  // 这条意见捡走——表现就是「意见要等到下一次计划审阅才出现」。
  // 也不能按「此前最后一张卡」归属：实时流里部件 seq 统一为 0、新卡追加在同一消息的
  // parts 末尾，遍历顺序取到的末张恰是刚提交的新卡，照样错位。
  // 真正的判据是状态：**等待中的卡不可能已经产生反馈**，所以反馈属于此前最后一张
  // 已决定（denied）的卡。快照重建（请求卡 + resolved 回填）与实时流同规则。
  // 注意只影响渲染：消息数据本身不动（历史翻页、复制等仍按完整消息流工作）。
  const isPlanFeedback = (m: MessageSnapshot): boolean =>
    m.role === "user" && m.parts.some((p) => p.payload["via"] === "plan_feedback");
  const feedbackTextOf = (m: MessageSnapshot): string =>
    m.parts
      .filter((p) => p.kind === "text")
      .map((p) => String(p.payload["text"] ?? ""))
      .join("\n")
      .trim();
  const planCards: { messageIdx: number; partId: string; waiting: boolean }[] = [];
  messages.forEach((m, messageIdx) => {
    if (m.role !== "assistant") return;
    for (const p of m.parts) {
      if (p.kind !== "approval") continue;
      const approval = p.payload as unknown as ApprovalPart;
      if (approval.tool !== "exit_plan") continue;
      planCards.push({ messageIdx, partId: p.id, waiting: approval.status === "waiting" });
    }
  });
  const planFeedbackByPartId = new Map<string, string>();
  messages.forEach((m, messageIdx) => {
    if (!isPlanFeedback(m)) return;
    const before = planCards.filter((c) => c.messageIdx <= messageIdx);
    const target = [...before].reverse().find((c) => !c.waiting) ?? before[before.length - 1];
    if (target) planFeedbackByPartId.set(target.partId, feedbackTextOf(m));
  });
  const hasPlanFeedback = planFeedbackByPartId.size > 0;
  const renderMessages = hasPlanFeedback ? messages.filter((m) => !isPlanFeedback(m)) : messages;

  // 提问导航（`src/renderer/src/lib/turn-nav.ts` 模块头）：条目从「渲染后的消息流」派生——计划反馈已被
  // 抽进计划卡，导航里也不该再出现一次。上游每来一条流式增量 `messages` 都是新数组，但
  // 用户消息集合没变；这里用「用户消息 id 序列」当指纹，指纹不变就复用上一份条目，
  // 免得每个 token 都重建数组、把虚拟化行与 memo 组件的引用打散。
  // 只拼 id 不拼正文：正文只可能随消息身份变化（用户消息没有原地编辑，rewind 会重建消息）。
  const navItemsRef = useRef<{ signature: string; items: TurnNavItem[] }>({ signature: "", items: [] });
  const navSignature = renderMessages
    .filter((m) => m.role === "user")
    .map((m) => m.id)
    .join("\u0002");
  const navItems = useMemo(() => {
    const previous = navItemsRef.current;
    if (previous.signature === navSignature) return previous.items;
    const items = buildTurnNavItems(renderMessages);
    navItemsRef.current = { signature: navSignature, items };
    return items;
    // 只依赖签名：renderMessages 每次渲染都可能是新数组（含流式增量），用签名代替它。
  }, [navSignature]);
  const navVisible = navItems.length >= 2;
  const rootRef = useRef<HTMLDivElement>(null);
  // 轨道宽度闸门：观测对话区根节点（≈ 对话卡片宽度），而不是窗口——右侧面板开合改变的是
  // 容器宽度。初值判一次，之后交给 ResizeObserver；引用稳定，不会重建观察者。
  const [navWide, setNavWide] = useState(false);
  useEffect(() => {
    const element = rootRef.current;
    if (!element) return;
    const sync = (): void => setNavWide(element.clientWidth >= TURN_NAV_MIN_WIDTH_PX);
    sync();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", sync);
      return () => window.removeEventListener("resize", sync);
    }
    const observer = new ResizeObserver(sync);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  /** 跳到某次用户输入：先解除贴底（否则下一次内容增长又会被拉回底部），再按行几何精确滚动。
   *  跳转后 isAtBottom 变 false，「回到底部」按钮自然出现。 */
  const jumpToNavItem = useCallback(
    (item: TurnNavItem) => {
      const scroller = scrollRef.current;
      if (!scroller) return;
      const row = scroller.querySelector<HTMLElement>(`[data-message-id="${item.id}"]`);
      if (!row) return;
      stopScroll();
      const target = scroller.scrollTop + row.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
      const behavior: ScrollBehavior = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
        ? "auto"
        : "smooth";
      scroller.scrollTo({ top: target, behavior });
    },
    [scrollRef, stopScroll],
  );

  return (
    // 外层只负责定位上下文：提问导航绝对定位在滚动视口之外（贴对话卡片左缘），
    // 若挂进滚动容器内会随内容一起滚走。宽度闸门也以这一层为准。
    <div ref={rootRef} className="relative flex min-h-0 flex-1 flex-col">
      <TurnNavigator
        items={navItems}
        wide={navWide}
        scrollRef={scrollRef}
        running={chat?.running ?? false}
        onJump={jumpToNavItem}
      />
      <div
        ref={scrollRef}
        className={`relative min-h-0 flex-1 overflow-y-auto ${navVisible ? "@min-[864px]/conversation:pl-12" : ""}`}
      >
        {/* min-h-full：消息少时也占满滚动视口，footer 始终沉底；footer 参与内容流，
            输入区增减高度会触发内容 resize，贴底状态由 use-stick-to-bottom 自动保持。
            宽度档位与 Composer 共用常量，保证消息列与输入卡各档始终对齐。 */}
        <div
          ref={contentRef}
          data-resize-smooth
          className={`mx-auto flex min-h-full w-full flex-col px-4 ${draftLayout ? DRAFT_WIDTH_CLASS : CONTENT_WIDTH_CLASS}`}
        >
          <div className="flex min-h-0 flex-1 flex-col gap-6 pb-4 pt-6">
            {messages.length === 0 && (
              <div className="flex flex-1 items-center justify-center py-24 text-ui-sm text-foreground-subtlest">
                开始你的第一条消息
              </div>
            )}
            {renderMessages.map((message) =>
              hasPlanFeedback ? (
                // 计划反馈按卡片归属：整张表沿渲染链传下去，只有 exit_plan 卡会用到
                <MessageRow
                  key={message.id}
                  message={message}
                  chatRunning={chat?.running ?? false}
                  workspaceId={workspaceId}
                  planFeedbackByPartId={planFeedbackByPartId}
                />
              ) : (
                <MessageRow key={message.id} message={message} chatRunning={chat?.running ?? false} workspaceId={workspaceId} />
              ),
            )}
          </div>
          {footer && (
            // 外壳 pointer-events-none：全宽 sticky 层不挡消息（滚入输入区背后的消息仍可点选），
            // 只有输入列与按钮恢复命中——composer dock 的同款处理。
            <div className="pointer-events-none sticky bottom-0 z-10 -mx-4 bg-gradient-to-t from-background via-background to-transparent pt-6">
              <div className="pointer-events-auto relative">
                {!isAtBottom && (
                  <button
                    className="absolute -top-9 left-1/2 z-10 -translate-x-1/2 rounded-full border border-border bg-popover px-3 py-1.5 text-ui-sm text-foreground-subtle shadow-md hover:bg-menu-hover"
                    onClick={() => scrollToBottom("instant")}
                  >
                    <ChevronDown className="inline size-3.5" /> 回到底部
                  </button>
                )}
                {footer}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

const pad2 = (n: number): string => String(n).padStart(2, "0");

/** 消息时间："yyyy-MM-dd HH:mm:ss"（本地时区）；ts 无效（0 = 未知，如旧数据）返回空串。 */
function fmtDateTime(ts: number): string {
  if (!ts) return "";
  const d = new Date(ts);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

/** 文件路径 → 展示名。 */
function attachmentLeaf(path: string): string {
  return path.split("/").pop() || path;
}

/** 工具图片缩略图条（MCP 工具返回的截图等）：与用户消息附件同一读取通道，
 *  懒加载 data URL、点击放大。附件目录由后端统一管理，这里只按文件名取。 */
function ToolMediaStrip({ attachments }: { attachments: string[] }): React.JSX.Element | null {
  const list = attachments.filter(Boolean);
  if (list.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1.5 pt-1.5">
      {list.map((name) => (
        <AttachmentThumb key={name} name={name} />
      ))}
    </div>
  );
}

/** 单张附件缩略图：加载失败静默降级为文件名 chip（不阻塞工具行渲染）。 */
function AttachmentThumb({ name }: { name: string }): React.JSX.Element {
  const openPreview = useOverlays((s) => s.openImagePreview);
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!isImageName(name)) {
      setFailed(true);
      return;
    }
    let alive = true;
    void api
      .readAttachment(name)
      .then((c) => {
        if (alive) setUrl(`data:${c.mime};base64,${c.base64}`);
      })
      .catch(() => {
        if (alive) setFailed(true);
      });
    return () => {
      alive = false;
    };
  }, [name]);

  if (failed) {
    return (
      <span className="rounded-lg border border-border bg-surface px-2 py-1 text-ui-xs text-foreground-subtlest">
        {attachmentLeaf(name)}（无法显示）
      </span>
    );
  }
  if (!url) {
    return (
      <span className="h-16 w-24 animate-pulse rounded-lg border border-border bg-surface" aria-label="图片加载中" />
    );
  }
  return (
    <button
      type="button"
      className="overflow-hidden rounded-lg border border-border"
      title={`放大查看 ${attachmentLeaf(name)}`}
      onClick={() => openPreview(url, attachmentLeaf(name))}
    >
      <img src={url} alt={attachmentLeaf(name)} className="max-h-32 object-cover" />
    </button>
  );
}

/** composer 待发送附件：图片显示缩略图并可点击放大，非图片保留文件名 chip。
 *
 *  与消息流里的 AttachmentThumb 分开实现：composer 的每一项都要带删除按钮，
 *  且缩略图尺寸更小（不吃掉输入区高度）。读取仍走同一条 readAttachment 通道。
 */
function ComposerAttachment({ path, onRemove }: { path: string; onRemove: () => void }): React.JSX.Element {
  const openPreview = useOverlays((s) => s.openImagePreview);
  const name = path.split("/").pop() ?? path;
  const image = isImageName(name);
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!image) return;
    let alive = true;
    void api
      .readAttachment(path)
      .then((c) => {
        if (alive) setUrl(`data:${c.mime};base64,${c.base64}`);
      })
      .catch(() => {
        if (alive) setFailed(true);
      });
    return () => {
      alive = false;
    };
  }, [path, image]);

  const removeButton = (
    <button
      type="button"
      className="rounded p-0.5 hover:bg-surface-hover"
      aria-label={`移除附件 ${name}`}
      onClick={(e) => {
        e.stopPropagation();
        onRemove();
      }}
    >
      <X className="size-3" />
    </button>
  );

  // 非图片、或图片读取失败：退回文件名 chip（信息不丢，仍可删除）
  if (!image || failed) {
    return (
      <span className="flex h-7 items-center gap-1.5 rounded-lg border border-border bg-surface px-2 text-ui-xs text-foreground-subtle">
        <Paperclip className="size-3" />
        <span className="max-w-48 truncate">{name}</span>
        {removeButton}
      </span>
    );
  }

  if (!url) {
    return (
      <span className="flex h-16 w-16 animate-pulse items-center justify-center rounded-lg border border-border bg-surface" aria-label="图片加载中" />
    );
  }

  return (
    <span className="group relative inline-flex">
      <button
        type="button"
        className="overflow-hidden rounded-lg border border-border"
        title={`放大查看 ${name}`}
        onClick={() => openPreview(url, name)}
      >
        <img src={url} alt={name} className="h-16 w-16 object-cover" />
      </button>
      {/* 删除按钮压在缩略图右上角；hover / focus 时显现，避免常态遮挡图片 */}
      <span className="absolute -right-1.5 -top-1.5 rounded-full border border-border bg-background opacity-0 shadow-sm transition-opacity group-hover:opacity-100 focus-within:opacity-100">
        {removeButton}
      </span>
    </span>
  );
}

/** 用户消息附件：图片经 readAttachment 懒加载为缩略图（点击放大预览），其余渲染文件 chip。 */function MessageAttachments({ attachments }: { attachments: unknown }): React.JSX.Element | null {
  const openPreview = useOverlays((s) => s.openImagePreview);
  const list = (Array.isArray(attachments) ? attachments : []).map(String).filter(Boolean);
  const [urls, setUrls] = useState<Record<string, string>>({});
  const requested = useRef<Set<string>>(new Set());

  useEffect(() => {
    for (const name of list) {
      if (!isImageName(name) || requested.current.has(name)) continue;
      requested.current.add(name);
      void api
        .readAttachment(name)
        .then((c) => setUrls((prev) => ({ ...prev, [name]: `data:${c.mime};base64,${c.base64}` })))
        .catch(() => setUrls((prev) => ({ ...prev, [name]: "" }))); // 读取失败降级为文件 chip
    }
  }, [list]);

  if (list.length === 0) return null;
  return (
    <div className="flex flex-wrap justify-end gap-1.5">
      {list.map((name) => {
        const url = urls[name];
        if (isImageName(name) && url) {
          return url ? (
            <button
              key={name}
              type="button"
              className="overflow-hidden rounded-lg border border-border"
              title={`放大查看 ${attachmentLeaf(name)}`}
              onClick={() => openPreview(url, attachmentLeaf(name))}
            >
              <img src={url} alt={attachmentLeaf(name)} className="max-h-40 max-w-56 object-cover" />
            </button>
          ) : null;
        }
        if (isImageName(name)) {
          return (
            <span
              key={name}
              className="flex h-16 w-24 items-center justify-center rounded-lg border border-border bg-surface text-ui-xs text-foreground-subtlest"
            >
              加载中…
            </span>
          );
        }
        return (
          <span
            key={name}
            className="flex h-7 items-center gap-1.5 rounded-lg border border-border bg-surface px-2 text-ui-xs text-foreground-subtle"
          >
            <Paperclip className="size-3" />
            <span className="max-w-48 truncate">{attachmentLeaf(name)}</span>
          </span>
        );
      })}
    </div>
  );
}

/** 消息复制按钮：写入成功后原处切换「已复制」反馈，短暂停留后恢复。 */
function CopyButton({ getText }: { getText: () => string }): React.JSX.Element {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => clearTimeout(timer.current ?? undefined), []);
  return (
    <button
      className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-ui-xs text-foreground-subtlest hover:bg-surface-hover"
      onClick={() => {
        void api.clipboardWrite(getText()).then(() => {
          setCopied(true);
          if (timer.current) clearTimeout(timer.current);
          timer.current = setTimeout(() => setCopied(false), 1600);
        });
      }}
    >
      {copied ? <Check className="size-3 text-success" /> : <Copy className="size-3" />}
      {copied ? "已复制" : "复制"}
    </button>
  );
}

/** 单条消息：用户=右对齐气泡（附件缩略图 + 待送达标记 + hover 操作条）；助手=工作折叠块 + 最终正文。 */
function MessageRow({
  message,
  chatRunning,
  workspaceId,
  planFeedbackByPartId,
}: {
  message: MessageSnapshot;
  chatRunning: boolean;
  workspaceId: string;
  /** 计划修改意见（via=plan_feedback 的用户消息，渲染层抽走原消息后传入），按审批卡部件 id 归属。 */
  planFeedbackByPartId?: Map<string, string>;
}): React.JSX.Element {
  // 技能清单（决策 D11）：斜杠技能 chip 点击弹描述时查询；打开浮层时按需刷新
  const skills = useApp((s) => s.skills);
  const refreshSkills = useApp((s) => s.refreshSkills);
  if (message.role === "user") {
    return (
      // data-message-id：提问导航靠它把轨道条目定位到具体消息行（只标用户消息）
      <div data-message-id={message.id} className="group/msg flex w-full flex-col items-end gap-1">
        <div className="select-text flex max-w-full flex-col gap-2 rounded-xl rounded-tr-xs border border-border bg-user-message-bubble px-4 py-3 text-ui-base text-foreground sm:max-w-xl">
          {message.parts.some((p) => p.payload["via"] === "steer") && (
            <span className="self-end rounded bg-warning-surface px-1.5 py-0.5 text-ui-xs text-warning-surface-foreground">
              待送达
            </span>
          )}
          {message.parts
            .filter((p) => p.kind === "text")
            .map((part) => {
              // 斜杠技能调用（决策 D11）：胶囊头（技能名）+ 参数正文；原文保留在 payload.text
              // （复制按钮取原文），渲染层从原文提取参数（`/名称 参数` → 参数）
              const invoke = part.payload["skillInvoke"] as { name?: string } | undefined;
              const raw = trimBlankLines(String(part.payload["text"] ?? ""));
              if (invoke?.name) {
                const args = raw.replace(/^\/\S*/, "").trim();
                // chip 不带斜杠（技能名即标识）；点击弹出该技能的描述卡（对齐补全面板信息结构）。
                // chip 与参数同行内联排布（外层 whitespace-pre-wrap 沿用参数原文换行），不强制分列
                const meta = skills.find((s) => s.name === invoke.name && s.enabled && !s.error);
                return (
                  <div key={part.id} className="whitespace-pre-wrap break-words">
                    <Popover.Root
                      onOpenChange={(open) => {
                        if (open && !meta) void refreshSkills(workspaceId); // 清单未就绪时按当前工作区补扫
                      }}
                    >
                      <Popover.Trigger asChild>
                        <button
                          type="button"
                          title="点击查看技能描述"
                          // -my-0.5 抵消 py-0.5 的行盒增量：胶囊背景略超出但不撑高文本行
                          className="inline-flex cursor-pointer items-center gap-1 rounded-full bg-skill/10 px-2.5 py-0.5 -my-0.5 align-middle text-ui-base font-medium text-skill transition-colors hover:bg-skill/20"
                        >
                          <WandSparkles className="size-3.5 shrink-0" />
                          {invoke.name}
                        </button>
                      </Popover.Trigger>
                      <Popover.Portal>
                        <Popover.Content
                          side="top"
                          align="end"
                          sideOffset={6}
                          className="z-[80] w-80 rounded-xl border border-popover-border bg-popover p-3 shadow-md"
                        >
                          <div className="flex items-center gap-1.5">
                            <WandSparkles className="size-3.5 shrink-0 text-skill" />
                            <span className="truncate text-ui-sm font-medium text-foreground">{invoke.name}</span>
                            {meta && (
                              <span className="ml-auto shrink-0 rounded border border-border px-1 text-[10px] leading-4 text-foreground-subtlest">
                                {meta.source === "agents" ? "agents" : "craftwork"} · {meta.scope === "workspace" ? "工作区" : "用户级"}
                              </span>
                            )}
                          </div>
                          <p className="pt-1.5 text-ui-sm leading-5 text-foreground-subtle">
                            {meta ? meta.description || "（该技能没有描述）" : "正在读取技能描述…"}
                          </p>
                          {meta?.whenToUse && <p className="pt-1 text-ui-xs text-foreground-subtlest">适用：{meta.whenToUse}</p>}
                          {meta && (
                            <p className="truncate pt-1.5 font-mono text-ui-xs text-foreground-subtlest" title={meta.path}>
                              {meta.path}
                            </p>
                          )}
                        </Popover.Content>
                      </Popover.Portal>
                    </Popover.Root>
                    {args && ` ${args}`}
                  </div>
                );
              }
              return (
                <div key={part.id} className="whitespace-pre-wrap break-words">
                  {raw}
                </div>
              );
            })}
          {message.parts
            .filter((p) => p.kind === "text" && Array.isArray(p.payload["attachments"]) && (p.payload["attachments"] as unknown[]).length > 0)
            .map((part) => (
              <MessageAttachments key={`att_${part.id}`} attachments={part.payload["attachments"]} />
            ))}
        </div>
        {/* 操作条常驻占位：只切换透明度，避免 hover 时布局跳动 */}
        <div className="mr-1 flex items-center gap-1.5 opacity-0 transition-opacity duration-150 group-hover/msg:opacity-100">
          {message.createdAt > 0 && (
            <span className="select-none text-ui-xs text-foreground-subtlest">{fmtDateTime(message.createdAt)}</span>
          )}
          <CopyButton
            getText={() =>
              message.parts
                .filter((p) => p.kind === "text")
                .map((p) => String(p.payload["text"] ?? ""))
                .join("\n")
            }
          />
        </div>
      </div>
    );
  }

  // 助手消息：按「过程分组」切段渲染（工具/推理/审批收进折叠，最终正文在外）
  return (
    <div className="group/msg flex w-full flex-col gap-2">
      <TurnBody message={message} chatRunning={chatRunning} planFeedbackByPartId={planFeedbackByPartId} />
      {/* 操作条常驻占位：只切换透明度，避免 hover 时布局跳动。
          复制按钮与时间戳只在回复结束后出现（流式中占位为空，hover 也不显示任何内容）；
          时间戳取结束时刻（finishedAt）而非开始时刻 */}
      <div className="flex items-center gap-1.5 opacity-0 transition-opacity duration-150 group-hover/msg:opacity-100">
        {message.status !== "streaming" && (
          <CopyButton
            getText={() =>
              message.parts
                .filter((p) => p.kind === "text")
                .map((p) => String(p.payload["text"] ?? ""))
                .join("\n")
            }
          />
        )}
        {message.status !== "streaming" && (message.finishedAt ?? message.createdAt) > 0 && (
          <span className="select-none text-ui-xs text-foreground-subtlest">
            {fmtDateTime(message.finishedAt ?? message.createdAt)}
          </span>
        )}
      </div>
    </div>
  );
}

/** 单条助手消息的渲染（单折叠区模型）：一个 run 只有一个过程折叠区——
 *  正文（最后一段非空 text）之前的全部内容（思考、工具、审批历史、**中间解说正文**）
 *  都收进折叠区，只留正文平铺在外；正文之后的过程行（运行中实时产出 / 中断残余）
 *  常驻平铺。run 结束后折叠区收起，只显示「已工作 · 持续 N 秒」+ 结论。
 *  流式期间正文起点锚定第一个非空 text part：正文与其后的过程行、追加正文段按序平铺，
 *  几何只增不减；run 结束按最后一个非空 text 重算切段——这次重组与折叠收起同帧完成，
 *  是一次显式形态转换而非逐帧搬家（行为约定见 `conversation.tsx` 的 Timeline 注释）。
 *  等待中的审批卡常驻渲染在折叠区之外（run 推进的关键路径）；已决定卡立即回归时序位置，
 *  run 结束后随切段归档折叠区。 */
function TurnBody({
  message,
  chatRunning,
  planFeedbackByPartId,
}: {
  message: MessageSnapshot;
  chatRunning: boolean;
  /** 计划修改意见（按审批卡部件 id 归属）：折叠区里的 exit_plan 卡同样要显示自己的意见。 */
  planFeedbackByPartId?: Map<string, string>;
}): React.JSX.Element {
  const threadId = useActiveThreadId();
  // 后台命令任务可能被模型反复查看：同一任务号的 bash_output 先合并成一条等待行，
  // 再做正文切段与折叠区划分，保证流式与历史快照的形态一致。
  const parts = useMemo(() => mergeBackgroundPolls(message.parts), [message.parts]);
  const isStreaming = message.status === "streaming";

  const isWaitingApproval = (p: PartSnapshot): boolean =>
    p.kind === "approval" && (p.payload as unknown as ApprovalPart).status === "waiting";
  // 常驻折叠区之外的审批卡：只有 waiting 卡（run 推进的关键路径，折叠进过程块会让
  // 任务无声卡死）。已决定卡立即回归时序位置——流式中折叠区默认展开，卡片归位到其
  // 工具调用附近仍可见；钉到 run 结束会让计划审批后的长实施阶段（数分钟）始终垫底。
  const isNonEmptyText = (p: PartSnapshot): boolean =>
    p.kind === "text" && String(p.payload["text"] ?? "").length > 0;

  // 正文起点：流式锚定第一个非空 text（parts 只追加，该选择逐帧稳定——旧逻辑取「最后一个」，
  // 新正文段到达时前一段正文会从平铺 Markdown 位搬进折叠区，格式与高度双突变）；
  // 完成后取最后一个非空 text（最终结论平铺在外，此前的一切进折叠区）。
  let finalIdx = -1;
  if (isStreaming) {
    finalIdx = parts.findIndex(isNonEmptyText);
  } else {
    for (let i = parts.length - 1; i >= 0; i--) {
      const p = parts[i]!;
      if (isNonEmptyText(p) && !isWaitingApproval(p)) {
        finalIdx = i;
        break;
      }
    }
  }
  // 折叠区：正文之前的全部行（含中间解说正文——它们是过程的一部分）
  const historyParts = parts.slice(0, finalIdx < 0 ? parts.length : finalIdx).filter((p) => !isWaitingApproval(p));
  const finalText = finalIdx >= 0 ? parts[finalIdx]! : null;
  // 尾段：正文之后的过程行 / 流式中的追加正文段（按序平铺，见 JSX 内分支）
  const tailParts = (finalIdx >= 0 ? parts.slice(finalIdx + 1) : []).filter((p) => !isWaitingApproval(p));
  // 流式中的思考段（消息流式中最后一个 reasoning part）：折叠行显示「思考中 + 最新一行摘要」
  const streamingReasoningId = isStreaming
    ? ([...parts].reverse().find((p) => p.kind === "reasoning")?.id ?? "")
    : "";
  const openApprovals = parts.filter(isWaitingApproval);

  return (
    <div className="flex w-full flex-col gap-2">
      {historyParts.length > 0 && (
        <WorkFold
          parts={historyParts}
          streaming={isStreaming}
          status={message.status}
          threadId={threadId}
          streamingReasoningId={streamingReasoningId}
          durationMs={message.workDurationMs}
          planFeedbackByPartId={planFeedbackByPartId}
        />
      )}
      {finalText && (
        <div className="text-ui-base text-foreground">
          <Markdown text={String(finalText.payload["text"] ?? "")} />
        </div>
      )}
      {tailParts.map((part) =>
        // 流式中的追加正文段：与正文同级 Markdown 平铺（按序追加、不搬家）；
        // 非流式尾段是中断残余，保持弱化纯文本
        isStreaming && part.kind === "text" ? (
          <div key={part.id} className="text-ui-base text-foreground">
            <Markdown text={String(part.payload["text"] ?? "")} />
          </div>
        ) : (
          <WorkLine
            key={part.id}
            part={part}
            threadId={threadId}
            streamingReasoningId={streamingReasoningId}
            planFeedbackByPartId={planFeedbackByPartId}
          />
        ),
      )}
      {openApprovals.map((part) => (
        <ApprovalCard
          key={part.id}
          part={part.payload as unknown as ApprovalPart}
          threadId={threadId}
          planFeedback={planFeedbackByPartId?.get(part.id)}
        />
      ))}
      {parts.map((part) => {
        if (part.kind === "error") {
          return <ErrorCard key={part.id} part={part} threadId={threadId} />;
        }
        // notice（上下文压缩、模型失败转移等运行提示）不在这里单独渲染：
        // 它是过程行，必须跟着 parts 的时序走——见 WorkLine 的 notice 分支。
        // 曾在此处用独立的「尾段」渲染，导致提示永久固定在对话流底部，
        // 与它描述的那一步脱节。
        return null;
      })}
      {/* 轮尾运行中槽位：run 期间常驻（固定 min-h，出现/消失不推挤上文）；等待审批时让位——
          审批卡已是唯一进度反馈（互斥裁决：只对「等待用户
          操作」互斥，已决定的审批卡原位留存不阻塞 loading） */}
      {isStreaming && chatRunning && !parts.some(isWaitingApproval) && (
        <RunLoadingSlot createdAt={message.createdAt} />
      )}
    </div>
  );
}

function useActiveThreadId(): string {
  return useApp((s) => s.activeThreadId[s.mode]);
}

/** 折叠块收起过渡时长（毫秒）；过渡结束后才卸载内容。 */
const FOLD_TRANSITION_MS = 300;
/** 折叠收起过渡结束后再多等一小段（毫秒），让卸载内容带来的高度变化落定，再执行完成贴底。 */
const FOLD_SETTLE_EXTRA_MS = 60;
/** 审批决定后补一次贴底的延迟（毫秒）：等待卡片回归时序位置、正文预览与按钮组卸载、
 *  轮尾「工作中」槽位挂上，布局落定之后视口底部才是真正的底部。 */
const APPROVAL_SETTLE_MS = 150;

/** 工作过程折叠块（一段正文章节前的过程行）。运行中自动展开让过程可见，且自动展开态
 *  隐藏区头——过程行平铺本身就是当前动作，避免区头「正在思考/工作中」与行级「思考中」
 *  重复描述；run 结束后区头随自动收起一起出现，只留「已工作 · 持续 N 秒」摘要行。
 *  用户手动切换过（manual）后完全尊重手动状态，区头始终显示。展开内容无缩进平铺
 *  （组容器无缩进、纵向 gap），已完成时显示该轮工作的耗时。
 *  收起/展开走 grid-template-rows 高度过渡、过渡结束后延迟卸载内容：瞬时条件渲染会让
 *  整块过程内容无过渡蒸发（数百 px 高度瞬失，是晃动的最大单次来源）；prefers-reduced-motion
 *  下 transition-none 直接跳变，延迟卸载依旧成立。 */
function WorkFold({
  parts,
  streaming,
  status,
  threadId,
  durationMs,
  streamingReasoningId,
  planFeedbackByPartId,
}: {
  parts: PartSnapshot[];
  streaming: boolean;
  status: MessageSnapshot["status"];
  threadId: string;
  durationMs?: number;
  streamingReasoningId?: string;
  /** 计划修改意见（按审批卡部件 id 归属）：折叠区里的 exit_plan 卡要显示自己的意见。 */
  planFeedbackByPartId?: Map<string, string>;
}): React.JSX.Element {
  const hasRunning = parts.some((p) => p.kind === "tool" && (p.payload as unknown as ToolCallPart).status === "running");
  const [open, setOpen] = useState(streaming);
  const [manual, setManual] = useState(false);
  // 流式运行中默认展开（思考文本与工具调用实时可见），run 结束后自动收起；
  // 用户点过折叠头之后不再自动干预（manual 恒为 true）
  useEffect(() => {
    if (!manual) setOpen(streaming);
  }, [streaming, manual]);
  // 内容延迟卸载：收起过渡期间保持挂载，300ms 后移除
  const [mounted, setMounted] = useState(streaming);
  useEffect(() => {
    if (open) {
      setMounted(true);
      return;
    }
    const timer = setTimeout(() => setMounted(false), FOLD_TRANSITION_MS);
    return () => clearTimeout(timer);
  }, [open]);
  const done = !streaming && status !== "streaming";

  // 完成态显示秒数（向上取整，与侧栏「思考过程 · 持续了 N 秒」口径一致）
  const duration = done && durationMs && durationMs > 0 ? `持续 ${formatElapsed(durationMs / 1000)}` : "";
  // 运行中自动展开态隐藏区头（手动折叠过则恢复显示，保证还有展开入口）
  const hideHeader = streaming && open && !manual;
  return (
    <div className="flex w-full flex-col">
      {!hideHeader && (
        <button
          className="group/reasoning inline-flex min-w-0 max-w-full items-center gap-2 self-start rounded-md px-1 py-0.5 text-ui-base text-foreground-subtlest hover:text-foreground-subtle"
          onClick={() => {
            setManual(true);
            setOpen(!open);
          }}
        >
          <Brain className="size-4 shrink-0" />
          <span className={`shrink-0 whitespace-nowrap ${hasRunning || !done ? "animated-gradient-text" : ""}`}>
            {hasRunning ? "工作中" : done ? "已工作" : "正在思考"}
          </span>
          {duration && <span className="shrink-0 whitespace-nowrap text-foreground-subtlest">{duration}</span>}
          <ChevronRight className={`size-4 shrink-0 transition-transform ${open ? "rotate-90" : ""}`} />
        </button>
      )}
      {mounted && (
        <div
          aria-hidden={!open}
          className={`grid transition-[grid-template-rows] duration-300 ease-out motion-reduce:transition-none ${open ? "" : "pointer-events-none"}`}
          style={{ gridTemplateRows: open ? "1fr" : "0fr" }}
        >
          <div className="min-h-0 overflow-hidden">
            <div className={hideHeader ? "" : "pt-2"}>
              <WorkLines
                parts={parts}
                threadId={threadId}
                streamingReasoningId={streamingReasoningId}
                coalesce={!streaming}
                planFeedbackByPartId={planFeedbackByPartId}
              />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** 轮尾运行中指示（轮尾固定槽位）：run 期间常驻固定占位（min-h 槽位，
 * 出现/消失不推挤上文）；等待审批时由调用方互斥隐藏——审批卡已是唯一进度反馈。
 * 图标 + 文字分工：旋转 `Loader` 表「在跑」（与侧栏状态槽同一个图标、同一份动效语言），
 * 扫光的「工作中」表「谁在跑」——图标不参与扫光，文字不旋转。
 * 「工作中」扫光 + 分段计时（<1 分钟 N.N 秒，更长分:秒 / 时:分:秒，tabular-nums 防数字宽度抖动）。计时起点在挂载时固定一次：
 * 消息 createdAt（run 开始时刻），旧事件流无时刻（0）时回落挂载时刻——不随渲染漂移。 */
function RunLoadingSlot({ createdAt }: { createdAt: number }): React.JSX.Element {
  const [startMs] = useState(() => (createdAt > 0 ? createdAt : Date.now()));
  return (
    <div className="min-h-6" role="status">
      <div className="flex items-center gap-2 text-ui-sm">
        {/* 运行中唯一允许的旋转图标（AGENTS 3.6 白名单：此处 + 侧栏会话状态槽）；
            motion-reduce 下停转，图标仍在，不丢「有东西在跑」的线索。 */}
        <Loader className="size-3.5 shrink-0 animate-spin text-foreground-subtle motion-reduce:animate-none" />
        <span className="shrink-0 whitespace-nowrap animated-gradient-text">工作中</span>
        <RunLoadingTimer startMs={startMs} />
      </div>
    </div>
  );
}

/** 独立计时小组件：interval 自刷内部 state，秒数推进只重渲染计时本身，不重渲染对话流。 */
function RunLoadingTimer({ startMs }: { startMs: number }): React.JSX.Element {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 100);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(0, (now - startMs) / 1000);
  return (
    <span className="shrink-0 font-mono text-ui-xs tabular-nums text-foreground-subtlest">
      {formatElapsed(seconds)}
    </span>
  );
}

/** 过程行平铺容器：折叠块展开内容（组容器——无缩进、纵向 gap）。
 *  coalesce 控制是否把连续同名且已完成的工具行归并成组卡（× N 可展开）——只在历史折叠块
 *  中开启（见 coalesceToolRows 注释）。思考行独立折叠，流式中的那段思考
 *  显示「思考中 + 最新一行摘要」。 */
function WorkLines({
  parts,
  threadId,
  streamingReasoningId,
  coalesce,
  planFeedbackByPartId,
}: {
  parts: PartSnapshot[];
  threadId: string;
  streamingReasoningId?: string;
  coalesce: boolean;
  planFeedbackByPartId?: Map<string, string>;
}): React.JSX.Element {
  const rows = coalesceToolRows(parts, coalesce);
  return (
    <div className="flex flex-col gap-3">
      {rows.map((row) =>
        row.kind === "group" ? (
          <ToolGroupCard key={row.key} name={row.name} tools={row.tools} />
        ) : (
          <WorkLine
            key={row.part.id}
            part={row.part}
            threadId={threadId}
            streamingReasoningId={streamingReasoningId}
            planFeedbackByPartId={planFeedbackByPartId}
          />
        ),
      )}
    </div>
  );
}

/** 单条过程行分发：思考折叠行 / 工具卡 / 审批卡 / 任务清单卡片，其余忽略。 */
function WorkLine({
  part,
  threadId,
  streamingReasoningId,
  planFeedbackByPartId,
}: {
  part: PartSnapshot;
  threadId: string;
  streamingReasoningId?: string;
  planFeedbackByPartId?: Map<string, string>;
}): React.JSX.Element | null {
  switch (part.kind) {
    case "reasoning":
      return <ReasoningFold part={part} streaming={part.id === streamingReasoningId} />;
    case "text":
      // 折叠区内的中间解说正文（「边做边说」的过程叙述），弱化展示。
      // 纯文本 + whitespace-pre-wrap 原样渲染，尾随换行会变成真实空行（模型常多吐几个），
      // 先裁首尾空行——最终正文走 Markdown 渲染，解析器本就会吞掉尾部空行。
      return (
        <div className="select-text whitespace-pre-wrap break-words text-ui-base text-foreground-subtle">
          {trimBlankLines(String(part.payload["text"] ?? ""))}
        </div>
      );
    case "tool":
      return isBackgroundPollPart(part) ? (
        <BackgroundWaitCard part={part.payload as unknown as ToolCallPart} />
      ) : (
        <ToolCard part={part.payload as unknown as ToolCallPart} threadId={threadId} />
      );
    case "approval":
      // 计划修改意见按卡片归属：这张卡自己收到的反馈才显示在这里（已拒绝的旧卡也保留，
      // 历史里它是「这份计划被要求过怎么改」的记录）
      return (
        <ApprovalCard
          part={part.payload as unknown as ApprovalPart}
          threadId={threadId}
          planFeedback={planFeedbackByPartId?.get(part.id)}
        />
      );
    case "todo":
      return <TodoCard items={(part.payload["items"] as never) ?? []} />;
    case "notice":
      // 运行提示（上下文已压缩、模型失败转移）：时序过程行，跟着 parts 的顺序落位。
      // 归入过程行而非固定底部，是「提示必须出现在它实际发生的那一步」的要求。
      return (
        <div className="select-text rounded-lg bg-surface px-3 py-2 text-ui-sm text-foreground-subtle">
          {String(part.payload["text"] ?? "")}
        </div>
      );
    case "subagent":
      return (
        <SubagentCard
          part={part.payload as unknown as SubagentPart}
          onOpen={(agentId) => void useApp.getState().openSubagentTab(agentId)}
        />
      );
    default:
      return null;
  }
}

type CoalescedRow =
  | { kind: "group"; name: string; tools: ToolCallPart[]; key: string }
  | { kind: "part"; part: PartSnapshot };

/** 归并连续同名且已完成的工具行；running / error 工具保持独立（运行扫光与失败显眼优先），
 *  不同名工具穿插时各自分组。单元素组还原为普通工具卡。
 *  coalesce=false（流式运行中）不归并：每个工具固定独立单行、完成时原位换状态文字——
 *  若 running 完成瞬间并入组卡，会产生「+1 行 → -1 行」的持续震荡，是对话流垂直晃动的
 *  主要来源（`conversation.tsx` 的 Timeline 注释）。合并只作为历史折叠块的压缩呈现。 */
function coalesceToolRows(parts: PartSnapshot[], coalesce: boolean): CoalescedRow[] {
  if (!coalesce) return parts.map((part) => ({ kind: "part" as const, part }));
  const out: CoalescedRow[] = [];
  let buf: { name: string; tools: ToolCallPart[]; key: string } | null = null;
  const flush = (): void => {
    if (!buf) return;
    if (buf.tools.length === 1) {
      out.push({ kind: "part", part: { id: buf.key, kind: "tool", seq: 0, payload: buf.tools[0] as unknown as Record<string, unknown> } });
    } else {
      out.push({ kind: "group", name: buf.name, tools: buf.tools, key: buf.key });
    }
    buf = null;
  };
  for (const part of parts) {
    if (part.kind === "tool") {
      const tc = part.payload as unknown as ToolCallPart;
      // 后台任务轮询行已在 TurnBody 按任务号合并成等待行，不参与「同名工具」组卡：
      // 否则 run 结束后会被并进 ToolGroupCard，丢掉「等待后台任务」的语义与终态文字。
      if (tc.status === "done" && !isBackgroundPollPart(part)) {
        if (buf && buf.name === tc.name) buf.tools.push(tc);
        else {
          flush();
          buf = { name: tc.name, tools: [tc], key: part.id };
        }
        continue;
      }
    }
    flush();
    out.push({ kind: "part", part });
  }
  flush();
  return out;
}

/** 工具归并组卡（仅历史折叠块中呈现）：一行摘要（人类可读标题 + × N + 首个目标 + 总耗时），
 *  点击展开组内全部工具卡。分组仍按原始工具名（同名才归并），只把展示名换成标题。 */
function ToolGroupCard({ name, tools }: { name: string; tools: ToolCallPart[] }): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const totalMs = tools.reduce((n, p) => n + (p.durationMs ?? 0), 0);
  const label = tools[0] ? toolDisplayName(tools[0]) : name;
  const rawSummary = (tools[0]?.summary ?? "").trim();
  const summary = rawSummary === label ? "" : rawSummary;
  return (
    <div className="w-full">
      <button
        className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-surface-hover"
        onClick={() => setOpen(!open)}
      >
        <span className="h-4 w-0.5 shrink-0 rounded-full bg-border" />
        <span className="shrink-0 text-ui-sm text-foreground" title={name}>
          {label}
        </span>
        <span className="shrink-0 rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtlest">
          × {tools.length}
        </span>
        <span className="min-w-0 flex-1 truncate text-ui-sm text-foreground-subtlest">{summary}</span>
        {totalMs > 0 && <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">{totalMs}ms</span>}
        <span className="shrink-0 text-ui-xs text-foreground-subtlest">完成</span>
        <ChevronRight className={`size-3.5 shrink-0 text-foreground-subtlest transition-transform ${open ? "rotate-90" : ""}`} />
      </button>
      {open && (
        <div className="flex flex-col gap-1 pt-1.5">
          {tools.map((p, i) => (
            <ToolCard key={`${i}_${p.toolCallId}`} part={p} />
          ))}
        </div>
      )}
    </div>
  );
}

/** 思考折叠行：单段思考独立开合。默认折叠；
 * 流式中标题「思考中」扫光 + 右侧最新一行思考摘要（折叠着也能看到模型在想什么）；
 * 完成后显示「思考」（本会话内计时过的补「· 持续了 时钟格式时长」，快照恢复的历史无时长数据不显示）。
 * 手动展开后完全由用户控制。 */
function ReasoningFold({ part, streaming }: { part: PartSnapshot; streaming: boolean }): React.JSX.Element {
  const text = String(part.payload["text"] ?? "");
  // 流式摘要：思考文本最后一个非空行（正在写出的那行）
  const summary = streaming ? ([...text.split("\n")].reverse().find((l) => l.trim().length > 0)?.trim() ?? "") : "";
  const [open, setOpen] = useState(false);
  const [seconds, setSeconds] = useState<number | undefined>(undefined);
  // 流式计时：本会话内从该段开始流式起每秒推进；结束后固定。历史恢复（挂载即非流式）不计时。
  const startRef = useRef<number | null>(null);
  useEffect(() => {
    if (!streaming) {
      if (startRef.current !== null) {
        setSeconds(Math.max(1, Math.ceil((Date.now() - startRef.current) / 1000)));
        startRef.current = null;
      }
      return;
    }
    if (startRef.current === null) startRef.current = Date.now();
    const tick = () => {
      if (startRef.current !== null) setSeconds(Math.max(1, Math.ceil((Date.now() - startRef.current) / 1000)));
    };
    tick();
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  }, [streaming]);

  return (
    <div className="flex w-full flex-col">
      <button
        className="inline-flex min-w-0 max-w-full items-center gap-2 self-start rounded-md px-1 py-0.5 text-left text-ui-base text-foreground-subtlest hover:text-foreground-subtle"
        onClick={() => setOpen(!open)}
      >
        <Brain className={`size-4 shrink-0 ${streaming ? "text-foreground-subtle" : ""}`} />
        {streaming ? (
          <>
            <span className="shrink-0 whitespace-nowrap animated-gradient-text">思考中</span>
            {summary && (
              <span className="min-w-0 flex-1 truncate text-ui-sm text-foreground-subtlest" title={summary}>
                {summary}
              </span>
            )}
          </>
        ) : (
          <span className="shrink-0 whitespace-nowrap">
            思考{seconds !== undefined ? ` · 持续了 ${formatElapsed(seconds)}` : ""}
          </span>
        )}
        <ChevronRight className={`size-4 shrink-0 transition-transform ${open ? "rotate-90" : ""}`} />
      </button>
      {open && (
        <div className="select-text whitespace-pre-wrap break-words pt-1.5 text-ui-base text-foreground-subtlest">
          {trimBlankLines(text)}
        </div>
      )}
    </div>
  );
}

// ---------- 卡片 ----------

/** 工具行的显示名：优先用后端 spec 提供的人类可读标题（「读取文件」），
 *  缺失或与工具名相同时回退工具名（MCP 动态工具没有内置标题）。
 *  标题是唯一事实源——不在渲染层维护「工具名 → 中文」映射表，避免两处漂移。 */
function toolDisplayName(part: ToolCallPart): string {
  const title = (part.title ?? "").trim();
  return title.length > 0 && title !== part.name ? title : part.name;
}

/** diff 着色所需的文件路径：取 `args.path`（写类工具都以 path 为路径参数）。
 *  不从摘要反推——摘要按工具不同可能是路径、命令或固定文案，靠它判语言会判错。 */
function diffPathOf(part: ToolCallPart): string {
  const path = part.args?.["path"];
  return typeof path === "string" ? path : "";
}

/** 工具调用卡片：状态色条 + 人类可读标题 + 摘要 + 耗时，点击展开输出；失败默认展开。
 *  原始工具名保留在悬停提示里（排查「到底是哪个工具」时仍可见）。 */
export function ToolCard({ part, threadId }: { part: ToolCallPart; threadId?: string }): React.JSX.Element {
  const [open, setOpen] = useState(part.status === "error");
  const [output, setOutput] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const running = part.status === "running";
  const label = toolDisplayName(part);
  // 后台命令任务（`crates/craft-agent/src/shell_tasks.rs` 模块头）：命令确实还在跑，但这次工具调用已经
  // 返回了，因此状态不能显示成扫光的「运行中」——那会让人以为界面卡住。改为「后台运行」，
  // 并给出任务号 + 读取输出入口。
  const backgroundTaskId = (part as unknown as { backgroundTaskId?: string | null }).backgroundTaskId ?? "";
  const isBackground = backgroundTaskId !== "";
  // 摘要与标题同文时不再重复展示（todo_write 这类工具的摘要就是动作本身）
  const rawSummary = (part.summary ?? "").trim();
  const summary = rawSummary === label ? "" : rawSummary;
  return (
    <div className="w-full">
      <button
        className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-surface-hover"
        onClick={() => setOpen(!open)}
      >
        <span className={`h-4 w-0.5 rounded-full ${part.status === "error" ? "bg-destructive" : running ? "bg-icon-blue" : "bg-border"}`} />
        <span
          title={part.name}
          className={`shrink-0 text-ui-sm ${running ? "animated-gradient-text" : "text-foreground"}`}
        >
          {label}
        </span>
        <span className="min-w-0 flex-1 truncate text-ui-sm text-foreground-subtlest">{summary}</span>
        {/* 变更统计放折叠行：不展开也能看出这次改动多大（D21 第 5 条） */}
        {part.diff ? <DiffStat diff={part.diff} /> : null}
        {part.durationMs ? <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">{part.durationMs}ms</span> : null}
        {/* wait 等待行的「跳过等待」（D21）：run 继续、模型立即核实等待目的；
            用户主动操作走 chatCancelWait（不经审批、后端落审计），无在途等待时后端拒绝。 */}
        {running && part.name === "wait" && threadId ? (
          <button
            type="button"
            className="shrink-0 rounded-md px-1.5 py-0.5 text-ui-xs text-foreground-subtle hover:bg-surface-hover"
            title="跳过等待：继续任务，模型立即核实等待目的"
            onClick={(e) => {
              e.stopPropagation();
              void api.chatCancelWait(threadId).catch(() => {});
            }}
          >
            跳过等待
          </button>
        ) : null}
        {isBackground ? (
          <button
            type="button"
            className="shrink-0 rounded-md px-1.5 py-0.5 text-ui-xs text-foreground-subtle hover:bg-surface-hover"
            title={`读取后台任务 ${backgroundTaskId} 的输出尾部`}
            onClick={(e) => {
              e.stopPropagation();
              if (output !== null) {
                setOutput(null);
                return;
              }
              setLoading(true);
              void api
                .chatBashOutput(threadId ?? "", backgroundTaskId, 8192)
                .then((r) => setOutput(r.output || "（还没有输出）"))
                .catch((err) => setOutput(`读取失败：${String(err)}`))
                .finally(() => setLoading(false));
            }}
          >
            {loading ? "读取中…" : output !== null ? "收起输出" : "读取输出"}
          </button>
        ) : null}
        <span className={`shrink-0 text-ui-xs ${part.status === "error" ? "text-destructive" : "text-foreground-subtlest"}`}>
          {running ? "运行中" : part.status === "error" ? "失败" : part.status === "canceled" ? "已取消" : isBackground ? "后台运行" : "完成"}
        </span>
        <ChevronRight className={`size-3.5 shrink-0 text-foreground-subtlest transition-transform ${open ? "rotate-90" : ""}`} />
      </button>
      {open && (
        <div className="flex flex-col gap-1.5 pt-1.5">
          {/* 有行级差异时先给 diff，再把工具输出正文压成一行弱化摘要：
              正文是「已写入 / 已替换」这类回执，改了什么以 diff 为准（D21）。
              出错时不给 diff——失败没有可信的「改动结果」。 */}
          {part.diff && part.status !== "error" ? (
            <ToolDiff diff={part.diff} path={diffPathOf(part)} />
          ) : null}
          {part.output ? (
            part.diff && part.status !== "error" ? (
              <div className="truncate px-1 text-ui-xs text-foreground-subtlest" title={part.output}>
                {part.output}
              </div>
            ) : (
              <pre className="select-text max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-surface px-3 py-2 font-mono text-ui-sm text-foreground-subtle">
                {part.output}
              </pre>
            )
          ) : null}
          {/* 工具图片（MCP）：缩略图条，点击放大预览（与用户消息附件同一读取通道） */}
          <ToolMediaStrip attachments={part.attachments ?? []} />
        </div>
      )}
      {/* 后台任务输出：独立一块，与工具结果正文分开——正文是「启动回执」，
          这里是命令持续写到输出文件里的实时内容（手动点「读取输出」拉取，不自动轮询）。 */}
      {output !== null ? (
        <div className="pt-1.5">
          <pre className="select-text max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-surface px-3 py-2 font-mono text-ui-sm text-foreground-subtle">
            {output}
          </pre>
        </div>
      ) : null}
    </div>
  );
}

/** 后台任务等待行：同一任务号的多次 `bash_output` 已在 TurnBody 合并为一条。
 *  运行中显示「等待后台任务」扫光；任务结束/被终止/失败时在同一行原位切换状态文字，
 *  不新增、不重挂。点击展开最近一次查看读到的输出尾部；旧事件没有 backgroundTaskState
 *  时回退为普通「查看命令输出」文案（仍按任务号合并）。 */
function BackgroundWaitCard({ part }: { part: ToolCallPart }): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const state = part.backgroundTaskState;
  const failed = part.status === "error" || state === "failed";
  // 最新一次查看可能还在执行（running 事件没有 meta），此时沿用合并时带上来的状态；
  // 完全没有状态字段的旧事件按工具调用自身的 running 判断。
  const running = !failed && (state === "running" || (state == null && part.status === "running"));
  const label = running
    ? "等待后台任务"
    : state === "completed"
      ? "后台任务完成"
      : state === "killed"
        ? "后台任务已终止"
        : state === "failed"
          ? "后台任务失败"
          : toolDisplayName(part);
  const statusText = running ? "运行中" : failed ? "失败" : state === "killed" ? "已终止" : "完成";
  const taskId =
    (typeof part.backgroundTaskId === "string" && part.backgroundTaskId) ||
    (typeof part.args?.["task_id"] === "string" ? (part.args["task_id"] as string) : "");
  const summary = taskId || (part.summary ?? "").trim();
  return (
    <div className="w-full">
      <button
        className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-surface-hover"
        onClick={() => setOpen(!open)}
      >
        <span className={`h-4 w-0.5 rounded-full ${failed ? "bg-destructive" : running ? "bg-icon-blue" : "bg-border"}`} />
        <span title={part.name} className={`shrink-0 text-ui-sm ${running ? "animated-gradient-text" : "text-foreground"}`}>
          {label}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-ui-xs text-foreground-subtlest">{summary}</span>
        <span className={`shrink-0 text-ui-xs ${failed ? "text-destructive" : "text-foreground-subtlest"}`}>
          {statusText}
        </span>
        <ChevronRight className={`size-3.5 shrink-0 text-foreground-subtlest transition-transform ${open ? "rotate-90" : ""}`} />
      </button>
      {open && part.output ? (
        <div className="pt-1.5">
          <pre className="select-text max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-surface px-3 py-2 font-mono text-ui-sm text-foreground-subtle">
            {part.output}
          </pre>
        </div>
      ) : null}
    </div>
  );
}

const RISK_LABELS: Record<string, string> = {
  R0: "只读",
  R1: "写入",
  R2: "执行",
  R3: "网络/越界",
  R4: "破坏性",
};

/** 审批卡片：风险徽章 + 目标 + 事实 + 决定按钮组（拒绝 / 允许一次 / 本会话 / 本工作区，永久授权按 allowPermanent 与风险等级显隐）。 */
export function ApprovalCard({ part, threadId, planFeedback }: { part: ApprovalPart; threadId: string; planFeedback?: string }): React.JSX.Element {
  const waiting = part.status === "waiting";
  const [busy, setBusy] = useState(false);
  const pushToast = useToasts((s) => s.push);
  // 计划审阅卡：三动作（批准并开始实施 / 提出修改 / 忽略），拒绝可携带反馈
  const isPlanReview = part.tool === "exit_plan";
  const [feedbackOpen, setFeedbackOpen] = useState(false);
  const [feedbackText, setFeedbackText] = useState("");

  const decide = async (allow: boolean, scope: "once" | "session" | "workspace", feedback?: string): Promise<void> => {
    setBusy(true);
    try {
      await api.chatDecideApproval(part.approvalId, allow, scope, feedback);
    } catch (err) {
      // 决定失败（如重启后等待已失效的僵尸卡）保持卡片可重试，但必须给出可读反馈
      pushToast("danger", "审批操作失败", String(err));
    } finally {
      setBusy(false);
    }
  };

  const permanentVisible = part.allowPermanent && (part.risk === "R1" || part.risk === "R2");

  return (
    <div
      className={`w-full overflow-hidden rounded-2xl border bg-popover shadow-xs ${
        waiting ? "border-warning/40" : part.status === "approved" ? "border-success/40" : "border-border opacity-90"
      }`}
    >
      <div className="flex flex-col gap-3 p-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-ui-base font-medium text-foreground-subtle">{part.tool === "exit_plan" ? "计划审阅" : "需要你的批准"}</span>
          <span className="rounded-md bg-warning-surface px-1.5 py-0.5 font-mono text-ui-xs text-warning-surface-foreground">
            {part.risk} {RISK_LABELS[part.risk] ?? ""}
          </span>
          <span className="rounded-md bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle">{part.tool}</span>
          <span className="ml-auto shrink-0 text-ui-sm">
            {waiting ? (
              <span className="animated-gradient-text">等待决定</span>
            ) : part.status === "approved" ? (
              <span className="text-success">已允许{part.scope && part.scope !== "once" ? `（${part.scope === "session" ? "本会话" : "本工作区"}）` : "一次"}</span>
            ) : part.status === "denied" ? (
              <span className="text-destructive">已拒绝</span>
            ) : (
              <span className="text-foreground-subtlest">已取消</span>
            )}
          </span>
        </div>
        {part.detail && <p className="text-ui-base leading-5 text-foreground">{part.detail}</p>}
        {part.target && (
          <div className="truncate rounded-lg bg-surface px-2.5 py-1.5 font-mono text-ui-sm text-foreground-subtle">{part.target}</div>
        )}
        {part.facts.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {part.facts.map((f, i) => (
              <span key={i} className="rounded-md bg-surface px-2 py-0.5 text-ui-xs text-foreground-subtle">
                {f.label}：{f.value}
              </span>
            ))}
          </div>
        )}
        {waiting && isPlanReview && typeof part.args.plan === "string" && (
          // 滚动容器 overscroll-contain：鼠标在计划内容区滚动只滚内部，到边界不 chaining 到外部消息流。
          // 只给等待决定看：批准后计划已落盘并可随时从「查看计划」调阅，240px 预览继续占着视口
          // 会把刚挂上的实施进展挤出可视区（决定后卡片应收缩为紧凑摘要）。
          <div className="max-h-60 select-text overflow-y-auto overscroll-contain rounded-lg bg-surface px-3 py-2">
            <Markdown text={part.args.plan} />
          </div>
        )}
        {/* 计划修改意见（via=plan_feedback 用户消息归并渲染）：嵌在卡片内，与「提出修改」动作同位；
            已决定卡也保留——历史里它是「这份计划被要求过怎么改」的完整记录 */}
        {isPlanReview && planFeedback && (
          <div className="rounded-lg border border-border bg-input px-3 py-2">
            <div className="pb-1 text-ui-xs font-medium text-foreground-subtle">你的修改意见</div>
            <div className="select-text whitespace-pre-wrap break-words text-ui-sm text-foreground">{planFeedback}</div>
          </div>
        )}
        {waiting && isPlanReview && (
          <div className="flex flex-col gap-2 pt-1">
            {feedbackOpen ? (
              <>
                <textarea
                  autoFocus
                  value={feedbackText}
                  onChange={(e) => setFeedbackText(e.target.value)}
                  placeholder="说明计划需要怎么改（将作为你的消息发给模型，模型会修订后重新提交）"
                  rows={3}
                  className="w-full resize-y rounded-lg border border-border bg-input px-2.5 py-2 text-ui-sm text-foreground placeholder:text-foreground-subtlest focus:border-brand/60 focus:outline-none"
                />
                <div className="flex items-center gap-2">
                  <button
                    disabled={busy}
                    className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:opacity-50"
                    onClick={() => {
                      setFeedbackOpen(false);
                      setFeedbackText("");
                    }}
                  >
                    返回
                  </button>
                  <div className="flex-1" />
                  <button
                    disabled={busy || feedbackText.trim().length === 0}
                    className="h-8 rounded-lg border border-brand/50 bg-brand/10 px-3 text-ui-sm text-brand hover:bg-brand/20 disabled:opacity-50"
                    onClick={() => void decide(false, "once", feedbackText)}
                  >
                    发送修改意见
                  </button>
                </div>
              </>
            ) : (
              <div className="flex items-center gap-2">
                <button
                  disabled={busy}
                  className="h-8 rounded-lg border border-border px-3 text-ui-sm text-foreground-subtle hover:bg-surface-hover disabled:opacity-50"
                  onClick={() => void decide(false, "once")}
                >
                  忽略
                </button>
                <div className="flex-1" />
                {isPlanReview && typeof part.args.plan === "string" && (
                  <button
                    disabled={busy}
                    className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:opacity-50"
                    onClick={() => useApp.getState().openPlanTab(part.args.plan as string)}
                    title="在右侧面板的计划标签页中查看计划全文"
                  >
                    查看计划
                  </button>
                )}
                <button
                  disabled={busy}
                  className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:opacity-50"
                  onClick={() => setFeedbackOpen(true)}
                >
                  提出修改
                </button>
                <button
                  disabled={busy}
                  className="h-8 rounded-lg border border-brand/50 bg-brand/10 px-3 text-ui-sm text-brand hover:bg-brand/20 disabled:opacity-50"
                  onClick={() => void decide(true, "once")}
                >
                  批准并开始实施
                </button>
              </div>
            )}
          </div>
        )}
        {waiting && !isPlanReview && (
          <div className="flex items-center gap-2 pt-1">
            <button
              disabled={busy}
              className="h-8 rounded-lg border border-destructive/50 px-3 text-ui-sm text-destructive hover:bg-destructive-surface disabled:opacity-50"
              onClick={() => void decide(false, "once")}
            >
              拒绝
            </button>
            <div className="flex-1" />
            <button
              disabled={busy}
              className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:opacity-50"
              onClick={() => void decide(true, "once")}
            >
              允许一次
            </button>
            <button
              disabled={busy}
              className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:opacity-50"
              onClick={() => void decide(true, "session")}
            >
              本会话允许
            </button>
            {permanentVisible && !isPlanReview && (
              <button
                disabled={busy}
                className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:opacity-50"
                onClick={() => void decide(true, "workspace")}
              >
                本工作区始终允许
              </button>
            )}
          </div>
        )}
        {/* 已决定的计划审阅卡保留「查看计划」入口：正文预览收起后，计划全文仍可随时调阅
            （右侧面板计划标签页），卡片本身收缩为紧凑摘要，不再挤压实施进展的视口 */}
        {!waiting && isPlanReview && typeof part.args.plan === "string" && (
          <div className="flex justify-end">
            <button
              className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover"
              onClick={() => useApp.getState().openPlanTab(part.args.plan as string)}
              title="在右侧面板的计划标签页中查看计划全文"
            >
              查看计划
            </button>
          </div>
        )}
      </div>
      <span className="hidden">{threadId}</span>
    </div>
  );
}

/** 错误卡片：错误码 + 原始信息 + 分类处置建议（hints 按 craft-model 错误码给出）+ 重试。 */
function ErrorCard({ part, threadId }: { part: PartSnapshot; threadId: string }): React.JSX.Element {
  const code = String(part.payload["code"] ?? "");
  const message = String(part.payload["message"] ?? "");
  const hints: Record<string, string> = {
    auth: "请检查提供商密钥是否有效（设置 → 模型）。",
    rate_limit: "请求过于频繁或额度不足，稍后重试。",
    timeout: "模型响应超时，可重试。",
    context_overflow: "会话内容超出模型上下文窗口，请新建会话或切换更大窗口的模型。",
    provider_unavailable: "服务暂时不可用，请检查端点与网络。",
    invalid_request: "请求参数不被接受，请检查模型配置。",
    interrupted: "上次运行被中断，可重试。",
    canceled: "已取消。",
    max_iterations: "已达本轮最大调用次数。",
    workspace_missing: "会话绑定的工作区不存在，请重新选择工作区。",
  };
  return (
    <div className="rounded-lg border border-destructive/30 bg-destructive-surface px-3 py-2.5">
      <div className="flex items-center gap-2 pb-1">
        <AlertCircle className="size-4 shrink-0 text-destructive" />
        <span className="text-ui-sm font-medium text-destructive">执行出错</span>
        <span className="rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle">{code}</span>
        <div className="flex-1" />
        <button
          className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-ui-xs text-foreground-subtle hover:bg-surface-hover"
          onClick={() => void api.chatRegenerate(threadId)}
        >
          <RefreshCw className="size-3" /> 重试
        </button>
      </div>
      <p className="select-text whitespace-pre-wrap break-words text-ui-sm text-foreground">{message}</p>
      {hints[code] && <p className="pt-1 text-ui-xs text-foreground-subtlest">{hints[code]}</p>}
    </div>
  );
}

// ---------- 输入区 ----------



/** 套餐额度 chip 挂载点：从生效模型 ref（会话 → 草稿槽 → 全局默认）解析提供商并传给
 *  QuotaChip；ref 形如 `providerId/modelId`，无 ref 时不出 chip。提供商显示名经
 *  providerList 解析（与 ContextChip 的目录查询同构），查不到时兜底 ID 本身。 */
function QuotaChipSlot(): React.JSX.Element | null {
  const mode = useApp((s) => s.mode);
  const activeId = useApp((s) => s.activeThreadId[mode]);
  const settings = useApp((s) => s.settings);
  const draftModel = useApp((s) => effectiveModelRef(s, s.mode));
  const thread = useApp((s) => s.threads[mode].find((t) => t.id === activeId));
  const modelId = activeId === "" ? draftModel : (thread?.modelId || settings.defaultModel);
  const providerId = modelId.includes("/") ? modelId.slice(0, modelId.indexOf("/")) : "";
  const [providerName, setProviderName] = useState("");
  useEffect(() => {
    if (!providerId) return;
    let alive = true;
    void api
      .providerList()
      .then((providers) => {
        const name = providers.find((p) => p.id === providerId)?.name;
        if (alive) setProviderName(name ?? providerId);
      })
      .catch(() => {
        if (alive) setProviderName(providerId);
      });
    return () => {
      alive = false;
    };
  }, [providerId]);
  if (!providerId) return null;
  return <QuotaChip providerId={providerId} providerName={providerName || providerId} />;
}

/** 模型 chip：显示会话模型（草稿态显示当前草稿槽的模型引用，缺省回落全局默认），点击在按钮上方弹出选择浮层。
 *  模型名 + ChevronDown，未选时占位文案。
 *  作为 Popover.Trigger asChild 的 child，必须铺开透传 props（含 ref），否则 radix 的点击绑定落不到 DOM。 */
function ModelChip({ className, disabled, ...props }: React.ComponentProps<"button">): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const activeId = useApp((s) => s.activeThreadId[mode]);
  const threads = useApp((s) => s.threads[mode]);
  const settings = useApp((s) => s.settings);
  const draftModel = useApp((s) => effectiveModelRef(s, s.mode));
  const draftLevel = useApp((s) => effectiveReasoningLevel(s, s.mode));
  const draftSlotData = useApp((s) => draftSlot(s, s.mode));
  const thread = threads.find((t) => t.id === activeId);
  const modelId = activeId === "" ? draftModel : (thread?.modelId || settings.defaultModel);
  const level = activeId === "" ? draftLevel : (thread?.reasoningLevel || "");
  // 显示名解析：会话名 → 草稿槽随选存的名字 → 其他会话同模型的 name → ID 尾段兜底
  const validName = (n: string | undefined): n is string => !!n && n !== "未选择模型" && n !== "模型不可用";
  const slotName = draftSlotData?.modelRef && draftSlotData.modelRef === modelId ? draftSlotData.modelName : undefined;
  const borrowedName = threads.find((t) => t.modelId === modelId && validName(t.modelName))?.modelName;
  const name = validName(thread?.modelName)
    ? thread!.modelName
    : validName(slotName)
      ? slotName!
      : validName(borrowedName)
        ? borrowedName!
        : (modelId.split("/").pop() ?? modelId);
  const label = modelId ? name : "选择模型";
  return (
    <button
      type="button"
      disabled={disabled}
      {...props}
      className={`flex h-7 shrink-0 items-center gap-1 rounded-lg pr-1 pl-2 text-ui-sm hover:bg-surface-hover disabled:hover:bg-transparent disabled:cursor-default disabled:opacity-50 ${
        modelId ? "text-foreground" : "text-foreground-subtle hover:text-foreground-subtle"
      } ${className ?? ""}`}
      title={modelId || "先在设置中添加提供商并选择模型"}
    >
      <span className="max-w-60 truncate">
        {label}
        {modelId && level !== "" && ` · ${reasoningLabel(level)}`}
      </span>
      <ChevronDown className="size-3.5 shrink-0 text-foreground-subtlest" />
    </button>
  );
}

/** 审批档位 chip：点击在按钮上方弹出档位选择浮层；切到「自由」先弹危险确认。
 *  档位图标 + ChevronDown；「自由」为高权限警示态（text-warning 常驻）。 */
function LevelChip(): React.JSX.Element {
  const settings = useApp((s) => s.settings);
  const chooseLevel = useApprovalLevelAction();
  const picker = useOverlays((s) => s.picker);
  const openPicker = useOverlays((s) => s.openPicker);
  const closePicker = useOverlays((s) => s.closePicker);
  const open = picker === "level";
  const current = APPROVAL_LEVEL_NAMES[settings.approvalLevel] ?? settings.approvalLevel;
  const Icon = APPROVAL_LEVEL_ICONS[settings.approvalLevel];
  const warning = settings.approvalLevel === "free";
  return (
    <Popover.Root open={open} onOpenChange={(o) => (o ? openPicker("level") : closePicker())}>
      <Popover.Trigger asChild>
        <button
          type="button"
          className={`flex h-7 shrink-0 items-center gap-1 rounded-lg px-2 text-ui-sm hover:bg-surface-hover ${
            warning ? "text-warning hover:text-warning" : "text-foreground-subtle"
          }`}
          title={`审批档位：${current}`}
        >
          <Icon className="size-3.5 shrink-0" />
          <span className="max-w-16 truncate">{current}</span>
          <ChevronDown className="size-3.5 shrink-0 text-foreground-subtlest" />
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="start"
          sideOffset={8}
          aria-label="选择审批档位"
          onCloseAutoFocus={(e) => {
            // 关闭后焦点回 composer 输入框。
            e.preventDefault();
            document.querySelector<HTMLTextAreaElement>("[data-composer-input]")?.focus();
          }}
          className="z-50 w-80 rounded-xl border border-popover-border bg-popover p-1.5 shadow-md"
        >
          {APPROVAL_LEVELS.map((l) => {
            const LevelIcon = APPROVAL_LEVEL_ICONS[l.id];
            const selected = settings.approvalLevel === l.id;
            return (
              <button
                key={l.id}
                className={`flex min-h-11 w-full cursor-pointer items-start gap-2.5 rounded-lg px-2.5 py-2 text-left hover:bg-menu-hover ${
                  l.id === "free" ? "text-warning" : ""
                }`}
                onClick={() => {
                  chooseLevel(l.id);
                  closePicker();
                }}
              >
                <LevelIcon className="mt-0.5 size-4 shrink-0" />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5 text-ui-sm font-medium text-foreground">
                    {l.name}
                    {selected && <Check className="size-3.5 shrink-0 text-success" />}
                  </span>
                  <span className="block text-ui-xs text-foreground-subtlest">{l.desc}</span>
                </span>
              </button>
            );
          })}
          {/* 风险等级说明：R0~R4 分级语义（审批档位切换的唯一说明位）。 */}
          <div className="mt-1 rounded-lg bg-surface px-2.5 py-2 text-ui-xs leading-relaxed text-foreground-subtlest">
            风险等级：R0 只读 · R1 写入工作区 · R2 执行命令 · R3 网络或越界（不提供永久授权） · R4 破坏性（阻止）
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

/** 上下文占用圆环（14px、2px 描边）：底环整圆 + 进度弧从 12 点方向顺时针（SVG dasharray + rotate -90），
 *  颜色三档与审批告警同源（<50% subtle、50–80% warning、≥80% destructive），超出窗口画满环。 */
function ContextRing({ percent, size = 14, strokeWidth = 2 }: { percent: number; size?: number; strokeWidth?: number }): React.JSX.Element {
  const r = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * r;
  const ratio = Math.max(0, Math.min(1, percent));
  const tone = percent >= 0.8 ? "stroke-destructive" : percent >= 0.5 ? "stroke-warning" : "stroke-foreground-subtle";
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="shrink-0 -rotate-90" aria-hidden>
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" className="stroke-border" strokeWidth={strokeWidth} />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        className={tone}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeDasharray={`${ratio * circumference} ${circumference}`}
      />
    </svg>
  );
}

function ContextRow({ label, value, muted }: { label: string; value: string; muted?: boolean }): React.JSX.Element {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="shrink-0 text-foreground-subtle">{label}</dt>
      <dd className={`text-right tabular-nums ${muted ? "text-foreground-subtlest" : "text-foreground"}`}>{value}</dd>
    </div>
  );
}

/** 上下文占用 chip：圆环 + 百分比 + 输出速度，点击弹出详情浮层（口径沿袭历史版 contextHint：
 *  占用只认实测（thread.contextTokens = 最近一轮归一化提示词+输出，覆盖语义）；
 *  没有调用记录（新会话/网关不给 usage）显示「—」空环，不做字符估算兜底——
 *  会话累计 tokens 是计费口径（每轮输入重复累计），当占用展示会严重虚高。
 *  窗口取模型目录 contextK；目录缺失（0）不渲染百分比，只显示实测 token 数。
 *  速度运行中按当前输出段实时估算（段内字符 ÷ charsPerToken ÷ 段起秒数，迭代边界重置；
 *  charsPerToken 由最近一轮 outputChars/outputTokens 自校准，修正 chars/4 对中文的低估），
 *  空闲显示最近一轮后端实测值，无实测显示「—」。
 *  空态收口：未选择模型时整块不渲染。 */
function ContextChip(): React.JSX.Element | null {
  const mode = useApp((s) => s.mode);
  const activeId = useApp((s) => s.activeThreadId[mode]);
  const settings = useApp((s) => s.settings);
  const thread = useApp((s) => s.threads[mode].find((t) => t.id === activeId));
  const draftModel = useApp((s) => effectiveModelRef(s, s.mode));
  const running = useApp((s) => s.chats[activeId]?.running ?? false);
  const lastStreamTs = useApp((s) => s.chats[activeId]?.lastStreamTs ?? 0);
  const charsPerToken = useApp((s) => s.charsPerToken[activeId] ?? 2);
  const modelId = activeId === "" ? draftModel : (thread?.modelId || settings.defaultModel);

  // 当前模型上下文窗口（千 token）：目录缺失（手动模型 / 目录无 limit.context）为 0，
  // 按「窗口未知」处理（不渲染百分比），不做 32k 静默兜底——错误分母会把占比放大几十倍
  const [contextK, setContextK] = useState(0);
  useEffect(() => {
    if (!modelId) return;
    let alive = true;
    void api
      .providerList()
      .then((providers) => {
        const found = selectableModels(providers).flatMap((g) => g.models).find((m) => m.ref === modelId);
        // 查不到（手动模型无目录信息）回落 0（窗口未知），不保留上一个模型的窗口值
        if (alive) setContextK(found?.contextK ?? 0);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [modelId]);

  // 当前流式输出段：store 记录的「最近有增量产出的部件」（lastStreamPartId，text 或
  // reasoning）。编码为 "partId|chars" 字符串保持 selector 值稳定；null = 无活跃部件
  // （空闲 / 快照装载 / 部件已随消息完结）。只认活跃部件的原因：模型常「正文前言 →
  // 长思考 → 正文续流」交替产出，若固定统计最后一个 text 部件，思考期间字符冻结而
  // 计时照走，实时速率会塌缩到 0.0x tok/s（日志实测一次停 2 字符 13 秒）。
  const streamSeg = useApp((s) => {
    const chat = s.chats[activeId];
    const pid = chat?.lastStreamPartId;
    if (!chat || !pid) return null;
    for (const m of chat.messages) {
      const p = m.parts.find((q) => q.id === pid);
      if (p && (p.kind === "text" || p.kind === "reasoning")) {
        return m.status === "streaming" ? `${p.id}|${String(p.payload["text"] ?? "").length}` : null;
      }
    }
    return null;
  });
  const segParts = streamSeg ? streamSeg.split("|") : ["", "0"];
  const segId = segParts[0] ?? "";
  const streamChars = Number(segParts[1]) || 0;
  // 实时速度按「当前活跃部件」计时：部件切换（思考↔正文、迭代边界）即重置，并记录
  // 切换时刻的字符基数，只统计之后的净增长——切回已有内容的部件（正文续流）不会把
  // 存量字符算进速率造成虚高。若跨段累计字符而起点只设一次，长任务里分母持续增长会
  // 把速度稀释成 0.x，Math.round 后恒显「0 tok/s」并劫持实测速度的展示。
  const streamStartRef = useRef(0);
  const segIdRef = useRef("");
  const segBaseCharsRef = useRef(0);
  if (streamSeg === null) {
    streamStartRef.current = 0;
    segIdRef.current = "";
    segBaseCharsRef.current = 0;
  } else if (segId !== "" && segId !== segIdRef.current) {
    streamStartRef.current = Date.now();
    segIdRef.current = segId;
    segBaseCharsRef.current = streamChars;
  }
  // 活跃窗口过期（3 秒无增量）需要一次重渲染才能切回实测值：流式期间每个增量都会触发
  // 重渲染，停顿 / 工具执行期没有天然重渲染源，跑 1s 低频 tick 兜底（只重渲染 chip
  // 自身，不重渲染对话流）。
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, [running]);

  if (!modelId) return null;

  // 占用只认实测：无实测（0）显示「—」空环。会话累计 tokens 是计费口径
  //（每轮输入随历史增长重复累计），拿来当占用会平方级虚高、ring 钳死 100%。
  const reported = thread?.contextTokens ?? 0;
  const hasMeasured = reported > 0;
  const used = reported;
  const maxTokens = contextK * 1000;
  const percent = hasMeasured && maxTokens > 0 ? Math.min(100, Math.round((used / maxTokens) * 100)) : 0;
  const elapsed = streamStartRef.current > 0 ? (Date.now() - streamStartRef.current) / 1000 : 0;
  // 活跃窗口：最近 3 秒内有增量才认为在产出。工具执行 / 网络停顿期间字符不动，累计
  // 平均会把速率单调拉低到个位数小数并长期挂住，超窗一律回落实测速度。
  const streamActive = lastStreamTs > 0 && Date.now() - lastStreamTs < 3000;
  // 段起始 1 秒内样本太少（速度虚高跳动），不启用实时估算，回落实测速度
  const liveSpeed =
    streamActive && streamChars > segBaseCharsRef.current && elapsed >= 1
      ? (streamChars - segBaseCharsRef.current) / charsPerToken / elapsed
      : 0;
  const shownSpeed = running && liveSpeed > 0 ? liveSpeed : (thread?.contextSpeed ?? 0);
  const tone = percent >= 80 ? "text-destructive" : percent >= 50 ? "text-warning" : "text-foreground-subtle";
  // 按钮主数字：有窗口 → 百分比；无窗口有实测 → 实测 token 数；无实测 → 「—」
  const mainLabel = !hasMeasured ? "—" : maxTokens > 0 ? `${percent}%` : formatTokens(used);
  const ringPercent = maxTokens > 0 ? percent / 100 : 0;
  const liveEstimating = running && liveSpeed > 0;

  return (
    <HoverCard.Root openDelay={120} closeDelay={80}>
      <HoverCard.Trigger asChild>
        <button
          type="button"
          className={`flex h-7 shrink-0 items-center gap-1.5 rounded-lg px-2 text-ui-xs transition-colors hover:bg-surface-hover ${tone}`}
          title={hasMeasured && maxTokens > 0 ? `上下文占用 ${percent}%` : "上下文"}
        >
          <ContextRing percent={ringPercent} />
          <span className="tabular-nums">{mainLabel}</span>
          {shownSpeed > 0 && (
            <span className="tabular-nums text-foreground-subtlest">
              {liveEstimating ? "≈" : ""}
              {formatSpeed(shownSpeed)} tok/s
            </span>
          )}
        </button>
      </HoverCard.Trigger>
      <HoverCard.Portal>
        <HoverCard.Content
          side="top"
          align="start"
          sideOffset={8}
          aria-label="上下文详情"
          className="z-50 w-80 rounded-xl border border-popover-border bg-popover p-3 shadow-md"
        >
          <p className="pb-2 text-ui-sm font-medium text-foreground">上下文</p>
          <dl className="flex flex-col gap-1.5 text-ui-xs">
            <ContextRow
              label="当前上下文"
              value={
                !hasMeasured
                  ? "—"
                  : maxTokens > 0
                    ? `${formatTokens(used)} / ${contextK}k（${percent}%）`
                    : `${formatTokens(used)}（窗口未知）`
              }
              muted={!hasMeasured}
            />
            <ContextRow label="累计 tokens（计费口径）" value={formatTokens(thread?.tokens ?? 0)} />
            <ContextRow
              label="最近一轮缓存命中"
              value={(thread?.contextCacheTokens ?? 0) > 0 ? formatTokens(thread!.contextCacheTokens) : "—"}
              muted={(thread?.contextCacheTokens ?? 0) <= 0}
            />
            <ContextRow
              label="最近一轮速度"
              value={shownSpeed > 0 ? `${liveEstimating ? "≈" : ""}${formatSpeed(shownSpeed)} tok/s` : "—"}
              muted={shownSpeed <= 0}
            />
            <ContextRow
              label="最近一轮首 token 延迟"
              value={formatDurationMs(thread?.contextTtftMs ?? 0)}
              muted={(thread?.contextTtftMs ?? 0) <= 0}
            />
          </dl>
          <p className="pt-2 text-ui-xs text-foreground-subtlest">
            占用为最近一轮模型请求的实测值（含缓存 token）；发送第一条消息后显示。
          </p>
        </HoverCard.Content>
      </HoverCard.Portal>
    </HoverCard.Root>
  );
}

/** 输入区：草稿态读写当前草稿槽（内容随槽保留）、会话态用本地 state；运行中发送转为引导（steer），停止走 armed 二次确认（与双击 Esc 共享状态）。
 * 顶部信息行：左侧始终显示工作区名称（项目名 / 临时会话），右侧为计划模式与 worktree 两个发送选项开关。 */
function Composer({
  activeId,
  draftLayout,
  chat,
}: {
  activeId: string;
  draftLayout: boolean;
  chat: { running: boolean };
}): React.JSX.Element {
  // 输入态分流：草稿态（活动会话为空）读写当前草稿槽（按模式 × 工作区归属分槽，draftWsId 指向当前槽，
  // 内容随槽保留互不覆盖）；会话态读写会话级 composer 桶 threadComposer（文本/附件/发送选项随会话
  // 分桶保留，切换互不串、重启不丢），草稿槽只服务草稿态。
  const mode = useApp((s) => s.mode);
  const setDraft = useApp((s) => s.setDraft);
  const setThreadComposer = useApp((s) => s.setThreadComposer);
  const refreshThreads = useApp((s) => s.refreshThreads);
  const selectThread = useApp((s) => s.selectThread);
  const workspaces = useApp((s) => s.workspaces);
  const draftWsId = useApp((s) => s.draftWsId[s.mode]);
  const thread = useApp((s) => s.threads[s.mode].find((t) => t.id === activeId));
  const isDraft = activeId === "";
  const draft = useApp((s) => (isDraft ? (draftSlot(s, s.mode) ?? EMPTY_DRAFT) : EMPTY_DRAFT));
  const bucket = useApp((s) => (isDraft ? EMPTY_THREAD_COMPOSER : (s.threadComposer[activeId] ?? EMPTY_THREAD_COMPOSER)));
  const text = isDraft ? draft.text : bucket.text;
  const attachments = isDraft ? draft.attachments : bucket.attachments;
  const setText = (value: string): void => {
    if (isDraft) setDraft(mode, { text: value });
    else setThreadComposer(activeId, { text: value });
  };
  const setAttachments = (value: string[] | ((prev: string[]) => string[])): void => {
    if (isDraft) {
      const prev = draftSlot(useApp.getState(), mode)?.attachments ?? EMPTY_DRAFT.attachments;
      setDraft(mode, { attachments: typeof value === "function" ? value(prev) : value });
    } else {
      const prev = useApp.getState().threadComposer[activeId]?.attachments ?? EMPTY_THREAD_COMPOSER.attachments;
      setThreadComposer(activeId, { attachments: typeof value === "function" ? value(prev) : value });
    }
  };
  // 顶部信息行的工作区名称：草稿态取草稿槽归属，会话态取会话绑定的工作区；
  // 内置临时目录的名称就叫「临时会话」，查不到（孤儿会话）如实显示「未绑定工作区」。
  const metaWsId = isDraft ? draftWsId : (thread?.workspaceId ?? "");
  const metaWs = workspaces.find((w) => w.id === metaWsId);
  // 斜杠技能调用（决策 D11）：输入区打开时按当前工作区刷新技能清单；文本以 `/` 开头时
  // 解析首 token 为技能名（模糊匹配出补全候选，精确命中成胶囊）。总开关关闭时不激活。
  const skills = useApp((s) => s.skills);
  const refreshSkills = useApp((s) => s.refreshSkills);
  const skillsEnabled = useApp((s) => s.settings.skillsEnabled);
  useEffect(() => {
    void refreshSkills(metaWsId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [metaWsId]);
  const availableSkills = useMemo(
    () => (skillsEnabled ? skills.filter((s) => s.enabled && !s.error) : []),
    [skills, skillsEnabled],
  );
  const slash = useMemo(() => {
    const m = /^\/(\S*)(\s[\s\S]*)?$/.exec(text);
    if (!m) return null;
    const token = m[1] ?? "";
    const exact = availableSkills.find((s) => s.name === token);
    // 模糊匹配（D11）：不要求从名称开头命中，按匹配质量降序；
    // 精确命中项同时留在候选里（胶囊与面板的互斥判定靠 exact），不影响选中语义。
    const candidates = rankSkillCandidates(availableSkills, token);
    return { token, exact, candidates, args: m[2] ?? "" };
  }, [text, availableSkills]);
  // 补全面板状态：名字未成型且有候选时显示；text 每次变化重置高亮到首项
  const [slashIdx, setSlashIdx] = useState(0);
  useEffect(() => setSlashIdx(0), [text]);
  const slashPanel = slash !== null && slash.exact === undefined && slash.candidates.length > 0;
  // 胶囊激活：名字精确命中已知技能，或有模糊候选（输入中）；无匹配的 /xxx 按普通文本不高亮
  const slashActive = slash !== null && (slash.exact !== undefined || slash.candidates.length > 0);
  const slashLayerRef = useRef<HTMLDivElement>(null);
  /** 选中补全候选：填充 `/名称 `（光标置尾，视同用户改动——取消输入预测与翻阅态）。 */
  const applySlash = (skill: SkillView): void => {
    setTextAndCursorEnd(`/${skill.name} `);
  };
  // 发送选项勾选态：草稿态随草稿槽保留；会话态随会话级 composer 桶保留（切走切回不丢、不跨会话串）。
  // worktree 已绑定的会话恒为勾选且锁定（绑定不可解绑，本会话固定在工作树内执行）。
  const boundWorktree = thread?.worktreePath ?? "";
  const planOn = isDraft ? draft.planMode : bucket.planMode;
  const worktreeOn = boundWorktree ? true : isDraft ? draft.useWorktree : bucket.useWorktree;
  const setPlanOn = (value: boolean): void => {
    if (isDraft) setDraft(mode, { planMode: value });
    else setThreadComposer(activeId, { planMode: value });
  };
  const setWorktreeOn = (value: boolean): void => {
    if (isDraft) setDraft(mode, { useWorktree: value });
    else setThreadComposer(activeId, { useWorktree: value });
  };
  const togglePlan = (): void => setPlanOn(!planOn);
  /** worktree 勾选带预检：临时槽 / 非项目工作区 / Work 模式 / 非 git 仓库直接拒绝并说明，发送时后端还会兜底校验。 */
  const toggleWorktree = async (): Promise<void> => {
    if (boundWorktree || running) return;
    if (worktreeOn) {
      setWorktreeOn(false);
      return;
    }
    if (mode === "work") {
      pushToast("danger", "无法启用 worktree", "Work 模式没有 Git 能力，请切换到 Code 模式后重试");
      return;
    }
    if (!metaWs || metaWs.purpose !== "project") {
      pushToast("danger", "无法启用 worktree", "临时会话不绑定项目；请先添加项目，再从侧栏项目分组发起会话");
      return;
    }
    try {
      const statuses = await api.workspaceGitStatuses();
      if (!statuses[metaWs.id]) {
        pushToast("danger", "无法启用 worktree", "当前项目不是 git 仓库，无法创建独立工作树");
        return;
      }
    } catch (err) {
      pushToast("danger", "worktree 预检失败", cleanErrorMessage(err));
      return;
    }
    setWorktreeOn(true);
  };
  // composer 配置 picker 排他态收在 overlays store（模型 / 档位互斥，⌘M 可从快捷键打开）。
  const picker = useOverlays((s) => s.picker);
  const openPicker = useOverlays((s) => s.openPicker);
  const closePicker = useOverlays((s) => s.closePicker);
  const pushToast = useToasts((s) => s.push);
  const running = chat?.running ?? false;
  const taRef = useRef<HTMLTextAreaElement>(null);
  // 停止「再按一次确认」armed 态：与全局 Esc 退出链共享（overlays store，800ms）
  const stopArmed = useOverlays((s) => s.stopArmed);
  const setStopArmed = useOverlays((s) => s.setStopArmed);

  // 历史翻阅（`conversation.tsx` 的 Composer 注释）：当前会话的用户消息倒序回溯，仅恢复文本。
  // steer / 计划模式消息是真实输入纳入；批准计划的固定确认消息与空白消息排除；相邻去重
  // 避免重试场景连发同样内容时「按了没动」的假象。
  const messages = useApp((s) => s.chats[activeId]?.messages ?? NO_MESSAGES);
  const userHistory = useMemo(() => {
    const items: string[] = [];
    for (const m of messages) {
      if (m.role !== "user") continue;
      const content = m.parts
        .filter((p) => p.kind === "text")
        .map((p) => String(p.payload["text"] ?? ""))
        .join("\n")
        .trim();
      if (!content || content === PLAN_CONFIRM_TEXT || items[items.length - 1] === content) continue;
      items.push(content);
    }
    return items;
  }, [messages]);
  // 翻阅游标：index = -1 未在翻阅；stash 暂存进入翻阅前的原始输入（↓ 越过最新时还原）。
  // cursorEnd 置位后由自适应高度 effect 在受控更新完成后把光标移到末尾。
  const historyRef = useRef({ index: -1, stash: "" });
  const cursorEndRef = useRef(false);
  // Composer 不按 activeId 重挂载，切换会话显式重置翻阅态（输入与勾选走会话桶，无需在此处理）。
  useEffect(() => {
    historyRef.current = { index: -1, stash: "" };
    setHint("");
  }, [activeId]);

  // 输入预测（`crates/craft-agent/src/prediction.rs` 模块头）：run 结束（running 下降沿）静置 5 秒后
  // 拉取后端预计算的下一句建议，以幽灵文本展示，Tab 应用。期间键入 / 程序化回填 /
  // 发送 / 接力新 run / 切会话均取消；到点后端未算完（返回空）则本次放弃，不重试。
  const [hint, setHint] = useState("");
  const touchedRef = useRef(false); // 下降沿后输入框是否被改动过（键入 / 回填 / 发送）
  const runningPrevRef = useRef(running);
  const lastThreadIdRef = useRef(activeId);
  useEffect(() => {
    // 会话切换导致的 running 值变化不是下降沿（如从运行中的 A 切到空闲的 B）
    const threadSwitched = lastThreadIdRef.current !== activeId;
    lastThreadIdRef.current = activeId;
    const wasRunning = runningPrevRef.current;
    runningPrevRef.current = running;
    if (running) {
      setHint(""); // 新 run 开始：旧建议即刻失效
      return;
    }
    if (isDraft || !wasRunning || threadSwitched) return;
    touchedRef.current = false;
    // 5 秒静默到点拉取；后端未算完（sidecar 慢模型返回空）时补偿重试一次，再为空则本次放弃
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const fetchHint = (delay: number): void => {
      timer = setTimeout(() => {
        if (cancelled || touchedRef.current) return;
        if (!useApp.getState().settings.inputPrediction) return; // 前端兜底（后端已门控）
        if (useApp.getState().chats[activeId]?.running) return; // steer 接力新 run
        void api
          .inputPredictionGet(activeId)
          .then((text) => {
            if (cancelled) return;
            if (text) {
              if (!touchedRef.current && !useApp.getState().chats[activeId]?.running) setHint(text);
              return;
            }
            fetchHint(4000);
          })
          .catch(() => {}); // 失败静默（spec 行为约定 10）
      }, delay);
    };
    fetchHint(5000);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [running, activeId, isDraft]);

  const pickAndAttach = async (): Promise<void> => {
    try {
      const files = await api.pickFiles();
      if (files.length > 0) {
        setAttachments((prev) => [...prev, ...files.filter((f) => !prev.includes(f))].slice(0, 10));
      }
    } catch {
      // 用户取消
    }
  };

  const stop = (): void => {
    if (stopArmed) {
      setStopArmed(false);
      void api.chatCancel(activeId);
    } else {
      setStopArmed(true);
      setTimeout(() => {
        const s = useOverlays.getState();
        if (s.stopArmed) s.setStopArmed(false);
      }, 800);
    }
  };

  /** 粘贴上传：剪贴板文件经 savePastedAttachment 落附件目录后并入附件列表。 */
  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>): void => {
    const files = selectPastedFiles(e.clipboardData.files);
    if (files.length === 0) return;
    e.preventDefault();
    void (async () => {
      for (const file of files) {
        const base64 = await readFileAsBase64(file);
        const saved = await api.savePastedAttachment(pastedFileName(file), base64);
        setAttachments((prev) => (prev.includes(saved) ? prev : [...prev, saved].slice(0, 10)));
      }
    })().catch((err) => pushToast("danger", "粘贴上传失败", cleanErrorMessage(err)));
  };

  // 自适应高度（2→8 行）；历史翻阅回填后顺带把光标移到末尾（受控更新完成后 DOM 才可定位）
  useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${Math.min(ta.scrollHeight, 8 * 24)}px`;
    if (cursorEndRef.current) {
      cursorEndRef.current = false;
      const end = ta.value.length;
      ta.setSelectionRange(end, end);
    }
  }, [text]);

  const submit = (): void => {
    historyRef.current = { index: -1, stash: "" }; // 发送即退出翻阅态（失败还原走程序化回填，不影响游标）
    touchedRef.current = true; // 发送后不再展示旧建议
    setHint("");
    const clean = text.trim();
    if (!clean && attachments.length === 0) return;
    const sendAttachments = attachments;
    const sendPlan = planOn;
    const sendWorktree = worktreeOn;
    setText("");
    setAttachments([]);
    const restore = (): void => {
      setText(clean); // 失败还原草稿与附件
      setAttachments(sendAttachments);
    };
    if (isDraft) {
      // 草稿首发：创建并首发原子路径。创建前失败（如未配置模型、worktree 创建失败）还原草稿；
      // 创建成功即视为发送成功——会话立即入栏并选中，Run 启动失败经 run.failed 在时间线展示，不还原草稿。
      // 工作区传当前草稿槽归属：临时槽空串（主进程回落 conversation 工作区），项目槽传项目 id（会话落入该项目分组）。
      // 模型取当前草稿槽引用（草稿态选择过），取不到回落全局默认；思考档位同理。
      const state = useApp.getState();
      const modelRef = effectiveModelRef(state, mode);
      const reasoningLevel = effectiveReasoningLevel(state, mode);
      void api
        .chatCreateAndStart(mode, state.draftWsId[mode], clean, sendAttachments, modelRef, reasoningLevel, sendPlan, sendWorktree)
        .then(async ({ threadId }) => {
          // 勾选随首发消息转移：新会话桶播种 planMode（按钮保持勾选，与运行中的计划 run 一致）；
          // worktree 绑定已落在会话上不转移；草稿槽复位，不把选项留给同槽的下一个草稿。
          setThreadComposer(threadId, { planMode: sendPlan });
          setDraft(mode, { planMode: false, useWorktree: false });
          await refreshThreads(mode);
          // 首发飞行中切走模式时不抢选：新会话留在原模式侧栏
          if (useApp.getState().mode === mode) await selectThread(threadId);
        })
        .catch((err) => {
          restore();
          pushToast("danger", "发送失败", cleanErrorMessage(err));
        });
      return;
    }
    if (running) {
      // 运行中引导并入当前任务：仅计划标记生效（worktree 为会话级绑定，引导时不新增）
      void api
        .chatSteer(activeId, clean, sendAttachments, sendPlan)
        .catch((err) => {
          restore();
          pushToast("danger", "引导失败", cleanErrorMessage(err));
        });
    } else {
      void api
        .chatSend(activeId, clean, sendAttachments, sendPlan, sendWorktree)
        .catch((err) => {
          restore();
          pushToast("danger", "发送失败", cleanErrorMessage(err));
        });
    }
  };

  /** 回填文本并置光标于末尾：受控值变化时经 effect 置尾；未变化（effect 不会跑）直接在 DOM 上置。 */
  const setTextAndCursorEnd = (value: string): void => {
    touchedRef.current = true; // 程序化回填视同用户改动（历史翻阅 / Tab 应用后不再展示旧建议）
    setHint("");
    setText(value);
    if (value === text) {
      const ta = taRef.current;
      if (ta) {
        const end = ta.value.length;
        ta.setSelectionRange(end, end);
      }
    } else {
      cursorEndRef.current = true;
    }
  };

  /** 历史翻阅（`conversation.tsx` 的 Composer 注释）：dir = -1 回溯（↑）、+1 前进（↓），命中返回 true（调用方 preventDefault）。
      进入翻阅前暂存当前输入；↓ 越过最新一条时还原暂存并退出翻阅态；到最旧后停留。 */
  const recallHistory = (dir: -1 | 1): boolean => {
    if (isDraft || userHistory.length === 0) return false;
    const h = historyRef.current;
    if (h.index === -1) {
      if (dir === 1) return false; // 非翻阅态 ↓ 交给光标
      h.stash = text;
      h.index = userHistory.length - 1;
    } else {
      const next = h.index + dir;
      if (next >= userHistory.length) {
        h.index = -1;
        const stash = h.stash;
        h.stash = "";
        setTextAndCursorEnd(stash);
        return true;
      }
      if (next < 0) return true; // 已到最旧
      h.index = next;
    }
    const item = userHistory[h.index];
    if (item !== undefined) setTextAndCursorEnd(item);
    return true;
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>): void => {
    // IME 组词中的 Enter 是候选确认（如中文输入法选词），交给输入法处理，不触发发送
    if (isImeComposing(e)) return;
    // 斜杠技能补全面板激活：↑↓ 移动高亮、Enter/Tab 选中填充（优先于发送 / Tab 建议 / 历史翻阅）；
    // 面板未激活时 ↓↑/Enter/Tab 走原链路
    if (slashPanel) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setSlashIdx((prev) => {
          const total = slash.candidates.length;
          return (prev + (e.key === "ArrowDown" ? 1 : -1) + total) % total;
        });
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        const picked = slash.candidates[slashIdx];
        if (picked) applySlash(picked);
        return;
      }
    }
    // 输入预测应用（`crates/craft-agent/src/prediction.rs` 模块头）：Tab 填入建议、光标置尾、hint 消失
    if (e.key === "Tab" && hint !== "") {
      e.preventDefault();
      setTextAndCursorEnd(hint);
      return;
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
      return;
    }
    // 历史翻阅：非翻阅态 ↑ 仅在光标位于第一行时进入（多行输入不抢光标上移）；翻阅态 ↑/↓ 无条件翻阅
    if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      const ta = e.currentTarget;
      const inRecall = historyRef.current.index !== -1;
      const onFirstLine = !ta.value.slice(0, ta.selectionStart).includes("\n");
      if ((inRecall || (e.key === "ArrowUp" && onFirstLine)) && recallHistory(e.key === "ArrowUp" ? -1 : 1)) {
        e.preventDefault();
      }
    }
  };

  // 右侧单按钮互斥：运行中输入了待发内容 → 发送态（点击=引导并入当前任务）；
  // 运行中清空内容 → 停止态；空闲恒为发送态（无内容禁用）。任一时刻只有一个按钮。
  const hasDraft = text.trim() !== "" || attachments.length > 0;
  const showStop = running && !hasDraft;
  // 输入预测幽灵文本的展示窗口：空闲会话 + 空输入 + 有建议；此时隐藏原生 placeholder
  const showHint = !isDraft && !running && text === "" && hint !== "";

  return (
    <div className="shrink-0 px-4 pb-4">
      {/* 宽度档位与消息列共用常量（`conversation.tsx` 的宽度档位注释），草稿档收窄聚焦。 */}
      <div data-resize-smooth className={`mx-auto flex w-full flex-col gap-2 ${draftLayout ? DRAFT_WIDTH_CLASS : CONTENT_WIDTH_CLASS}`}>
        <div
          // 不加 overflow-hidden：斜杠补全面板从 textarea 区向上浮出（bottom-full），
          // 裁剪会切掉面板上半截；卡片内子元素全部在 p-3 内边距内，无贴边裁剪需求
          className="relative flex flex-col gap-3 rounded-2xl border-[0.5px] border-input-border bg-input p-3 shadow-(--shadow-composer-soft) transition-colors focus-within:border-input-border-focused"
          data-composer-layout="conversation"
        >
          {/* 顶部信息行：左 = 工作区名称（项目名 / 临时会话），右 = 发送选项开关（计划模式 / worktree） */}
          <div className="flex items-center justify-between gap-2" data-composer-meta-row>
            <span className="flex min-w-0 items-center gap-1.5 text-ui-xs text-foreground-subtle" title={metaWs?.path || undefined}>
              <Folder className="size-3 shrink-0" />
              <span className="truncate">{metaWs ? metaWs.name : "未绑定工作区"}</span>
            </span>
            <span className="flex shrink-0 items-center gap-1.5">
              <button
                type="button"
                aria-pressed={planOn}
                onClick={togglePlan}
                title="计划模式：本轮先调研出实施计划，经你在审阅卡中批准后再开始实施；执行操作遵循当前审批档位"
                className={`flex items-center gap-1 rounded-full border px-2 py-0.5 text-ui-xs transition-colors ${
                  planOn ? "border-brand/50 bg-brand/10 text-brand" : "border-border text-foreground-subtle hover:bg-surface-hover"
                }`}
              >
                <ListChecks className="size-3" /> 计划模式
              </button>
              <button
                type="button"
                aria-pressed={worktreeOn}
                disabled={running || !!boundWorktree}
                onClick={() => void toggleWorktree()}
                title={
                  boundWorktree
                    ? `已绑定独立工作树（不可解绑，删除会话时清理）：${boundWorktree}`
                    : "worktree：发送后创建独立 git 工作树并绑定本会话，任务改动与主目录隔离"
                }
                className={`flex items-center gap-1 rounded-full border px-2 py-0.5 text-ui-xs transition-colors ${
                  worktreeOn ? "border-brand/50 bg-brand/10 text-brand" : "border-border text-foreground-subtle hover:bg-surface-hover"
                } disabled:cursor-not-allowed disabled:opacity-60`}
              >
                <GitBranch className="size-3" /> {boundWorktree ? "worktree · 已绑定" : "worktree"}
              </button>
            </span>
          </div>
          {attachments.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {attachments.map((path) => (
                <ComposerAttachment
                  key={path}
                  path={path}
                  onRemove={() => setAttachments((prev) => prev.filter((p) => p !== path))}
                />
              ))}
            </div>
          )}
          <div className="relative">
            {/* 输入预测幽灵文本（`crates/craft-agent/src/prediction.rs` 模块头）：非原生 placeholder（需与空输入
                共存）；leading-6 与 textarea 行高一致保证首行重合；行尾 Tab 徽标提示应用方式。
                显示期间隐藏原生 placeholder，避免两层提示文字重叠 */}
            {showHint && (
              <div
                className="pointer-events-none absolute inset-x-0 top-0 flex min-h-[48px] items-start gap-2 overflow-hidden text-ui-base leading-6 text-foreground-subtlest"
                aria-hidden
              >
                <span className="min-w-0 flex-1 truncate">{hint}</span>
                <kbd className="shrink-0 rounded border border-border px-1 text-ui-xs text-foreground-subtlest">Tab</kbd>
              </div>
            )}
            {/* 斜杠技能胶囊（决策 D11）：底层渲染层完整重绘文本，`/名称` 段套 brand 胶囊；
                textarea 文字转透明（caret 保持前景色），px-1/-mx-1 让胶囊包住文字而不改变行宽。
                与 hint 互斥（hint 仅空输入、胶囊仅 / 开头）。 */}
            {slashActive && (
              <div
                ref={slashLayerRef}
                className="pointer-events-none absolute inset-x-0 top-0 min-h-[48px] overflow-hidden whitespace-pre-wrap break-words text-ui-base leading-6 text-foreground"
                aria-hidden
              >
                {/* 不放图标：底层渲染必须与 textarea 文本逐字符对位，额外图标宽度会使参数文字错位 */}
                <span className="-mx-1 rounded bg-skill/10 px-1 text-skill">/{slash?.token}</span>
                {slash?.args}
              </div>
            )}
            <textarea
              ref={taRef}
              rows={2}
              value={text}
              onChange={(e) => {
                historyRef.current = { index: -1, stash: "" }; // 手动键入退出翻阅态（程序化回填不触发 onChange）
                touchedRef.current = true; // 键入即取消静默计时与既有建议
                setHint("");
                setText(e.target.value);
              }}
              onKeyDown={onKeyDown}
              onPaste={onPaste}
              onScroll={(e) => {
                // 胶囊层与 textarea 滚动同步（多行输入超出 8 行高度时）
                if (slashLayerRef.current) slashLayerRef.current.scrollTop = e.currentTarget.scrollTop;
              }}
              placeholder={showHint ? "" : running ? "任务执行中，输入补充指令（将并入当前任务）…" : "描述你的任务…"}
              data-composer-input
              className={`max-h-[192px] min-h-[48px] w-full resize-none overscroll-contain bg-transparent text-ui-base leading-6 placeholder:text-foreground-subtlest ${
                slashActive ? "text-transparent caret-foreground" : "text-foreground"
              }`}
            />
            {/* 斜杠技能补全面板：/ 前缀未精确命中时浮出候选（名称 + 描述），↑↓ 高亮、Enter/Tab 选中 */}
            {slashPanel && (
              <div className="absolute bottom-full left-0 z-30 mb-1 max-h-56 w-80 overflow-auto rounded-xl border border-popover-border bg-popover p-1 shadow-md">
                {slash.candidates.map((skill, i) => (
                  <button
                    key={skill.path}
                    type="button"
                    onMouseDown={(e) => {
                      e.preventDefault(); // 防止 textarea 失焦
                      applySlash(skill);
                    }}
                    onMouseEnter={() => setSlashIdx(i)}
                    className={`flex w-full flex-col gap-0.5 rounded-lg px-2.5 py-1.5 text-left ${i === slashIdx ? "bg-surface-hover" : ""}`}
                  >
                    <span className="flex items-center gap-1.5 text-ui-sm font-medium text-foreground">
                      <Puzzle className="size-3.5 shrink-0 text-foreground-subtle" />
                      {skill.name}
                      <span className="rounded border border-border px-1 text-[10px] leading-4 font-normal text-foreground-subtlest">
                        {skill.source === "agents" ? "agents" : "craftwork"}
                      </span>
                      <span className="text-[10px] font-normal text-foreground-subtlest">{skill.scope === "user" ? "用户级" : "工作区"}</span>
                    </span>
                    {skill.description && <span className="line-clamp-1 pl-5 text-ui-xs text-foreground-subtle">{skill.description}</span>}
                  </button>
                ))}
              </div>
            )}
          </div>
          <div className="group/toolbar flex items-end gap-3">
            <div className="flex min-w-0 flex-1 items-center gap-0.5" data-composer-leading-actions>
              <button
                type="button"
                title="添加附件"
                className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle hover:bg-surface-hover"
                onClick={() => void pickAndAttach()}
              >
                <Plus className="size-4" />
              </button>
              <LevelChip />
              <ContextChip />
            </div>
            <div className="ml-auto flex shrink-0 items-center justify-end gap-1.5" data-composer-trailing-actions>
              {/* 套餐额度 chip（D34）：模型选择器左侧，显示当前生效模型的提供商套餐余量；
                  白名单外提供商（含全部常规网关）不渲染，chip 缺席即「该提供商无额度查询」 */}
              <QuotaChipSlot />
              {/* 模型选择器贴发送簇左侧（模型/思考簇紧挨停止/发送） */}
              <ModelPicker
                open={picker === "model"}
                onOpenChange={(o) => (o ? openPicker("model") : closePicker())}
                trigger={<ModelChip />}
              />
              {/* 运行中且无待发内容时显示停止钮；输入了内容切换为发送钮（点击=引导并入当前任务），
                  避免禁用态圆钮与停止钮并排被误读为两个发送键 */}
              {showStop ? (
                <button
                  type="button"
                  onClick={stop}
                  className={`flex size-8 items-center justify-center rounded-full border hover:bg-surface-hover ${
                    stopArmed ? "border-destructive text-destructive" : "border-border text-foreground-subtle"
                  }`}
                  title={stopArmed ? "再按一次确认停止（或按 Esc）" : "停止（再次点击或按 Esc）"}
                >
                  <Square className="size-3.5" />
                </button>
              ) : (
                <button
                  type="button"
                  onClick={submit}
                  disabled={!running && !text.trim()}
                  className="flex size-8 items-center justify-center gap-1 rounded-full bg-brand text-foreground-inverse transition-opacity disabled:opacity-30"
                  title={running ? "发送（Enter），并入当前任务" : "发送（Enter）"}
                >
                  <ArrowUp className="size-4" />
                </button>
              )}
            </div>
          </div>
        </div>
        <div className="px-1 text-center text-ui-xs text-foreground-subtlest">
          Enter 发送 · Shift+Enter 换行 · ↑ 翻阅历史 · 运行中输入为引导指令 · ⌘K 命令面板 · ⌘, 设置
        </div>
      </div>
    </div>
  );
}
