/**
 * 底部状态栏：集中展示当前模式、工作区、上下文占用、待审批数量与安全相关状态。
 *
 * 高度固定为 27px，内容随模式不同而切换：Code 模式显示 Git 与终端，
 * Work 模式显示邮件 / 日历与文档相关入口。多数条目可点击，直接跳到对应的设置或数据面板。
 */
import { Icon } from "../../components/Icon";
import { formatTokens } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { usePanelStore } from "../../stores/panel";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";

/** 审批档位到中文标签的映射；未知档位按「分级」显示。 */
const levelLabel: Record<string, string> = {
  graded: "分级",
  strict: "严格",
  readonly: "只读",
};

/**
 * 底部状态栏，不接收 props。
 *
 * 负责：把各 store 里的关键状态汇总成一行可点击的提示。
 * 关键状态：
 * - `mode` 决定左半部分显示 Code 还是 Work 的信息；
 * - `thread` 是当前线程，`model` 是它使用的模型，两者共同决定上下文占用百分比；
 * - `pending` 是待审批列表，数量大于 0 时显示警告色并可跳转到差异面板。
 */
export function Statusbar() {
  const mode = useAppStore((state) => state.mode);
  const settings = useAppStore((state) => state.settings);
  const activeWorkspace = useAppStore((state) => state.activeWorkspace);
  const providers = useAppStore((state) => state.providers);

  const threads = useThreadStore((state) => state.threads[mode]);
  const activeID = useThreadStore((state) => state.activeID[mode]);
  const pending = useThreadStore((state) => state.pendingApprovals);
  const openData = useUiStore((state) => state.openData);
  const openSettings = useUiStore((state) => state.openSettings);
  const setTab = usePanelStore((state) => state.setTab);

  // 当前线程与它使用的模型；模型缺失时上下文窗口按 32k 估算，保证进度条仍可显示。
  const thread = threads.find((item) => item.id === activeID);
  const model = providers
    .flatMap((provider) => provider.models)
    .find((item) => item.id === thread?.modelId);
  const contextK = model?.contextK ?? 32;
  const used = thread?.tokens ?? 0;
  // 上下文占用百分比，上限 100；超过 70% 警告、超过 85% 标红。
  const pct = Math.min(100, Math.round((used / (contextK * 1000)) * 100));
  const barClass = pct > 85 ? "danger" : pct > 70 ? "warn" : "";

  return (
    <footer className="statusbar">
      {/* 左侧第一组：Code 模式强调 Git，Work 模式强调工作区与只读的邮件 / 日历 */}
      {mode === "code" ? (
        <>
          <span className="status-item">
            <Icon name="git-branch" />
            Git 工作区
          </span>
          <span className="status-sep" />
          <span className="status-item">
            <Icon name="folder" />
            {activeWorkspace?.name ?? "未选择工作区"}
          </span>
        </>
      ) : (
        <>
          <span className="status-item">
            <Icon name="folder" />
            {activeWorkspace?.name ?? "未选择工作区"}
          </span>
          <span className="status-item">
            <Icon name="mail" />
            邮件 / 日历只读
          </span>
        </>
      )}

      {/* 上下文占用：点击打开用量面板；进度条至少保留 2% 宽度，0 使用时也能看到一点痕迹 */}
      <button type="button" className="status-item" onClick={() => openData("usage")}>
        <Icon name="zap" />
        <span>上下文</span>
        <span className="mini-bar">
          <i className={barClass} style={{ width: `${Math.max(2, pct)}%` }} />
        </span>
        <span className="mono">
          {formatTokens(used)} / {contextK}k
        </span>
      </button>

      {/* 待审批：有请求时显示警告并可跳到差异面板，没有时显示正常状态 */}
      {pending.length > 0 ? (
        <button type="button" className="status-item warn" onClick={() => setTab(mode, "diff")}>
          <Icon name="clock" />
          <span>待审批 {pending.length}</span>
        </button>
      ) : (
        <span className="status-item ok">
          <Icon name="check-circle" />
          无待审批
        </span>
      )}

      {/* 按模式给出对应入口提示：Code 是终端，Work 是草稿 */}
      {mode === "code" ? (
        <span className="status-item">
          <Icon name="terminal" />
          终端
        </span>
      ) : (
        <span className="status-item">
          <Icon name="file-text" />
          草稿
        </span>
      )}

      {/* 线程处于错误状态时提供跳转到审计的入口 */}
      {thread?.status === "error" ? (
        <button type="button" className="status-item danger" onClick={() => openData("audit")}>
          <Icon name="alert" />
          错误 1
        </button>
      ) : null}

      <span className="status-spacer" />

      {/* 右侧：模式能力说明、审批档位、沙箱与审计状态、本地运行声明 */}
      {mode === "work" ? (
        <span className="status-item" title="Work 模式不提供 Shell、Git 与终端">
          <Icon name="file-text" />
          文档 · 文件 · MCP
        </span>
      ) : (
        <span className="status-item" title="Code 模式启用 Shell、Git、终端与差异面板">
          <Icon name="terminal" />
          内置工具 · Shell 已启用
        </span>
      )}
      <span className="status-sep" />
      {/* 点击审批档位直接打开设置里的工具分区 */}
      <button type="button" className="status-item" onClick={() => openSettings("tools")}>
        <Icon name="shield" />
        审批 {levelLabel[settings.approvalLevel] ?? "分级"}
      </button>
      {/* 沙箱属于 M4 能力，当前明确标注为未启用 */}
      <span className="status-item" title="macOS Seatbelt 沙箱在 M4 接入">
        <Icon name="lock" />
        沙箱 未启用
      </span>
      <span className="status-item ok">
        <Icon name="shield-check" />
        审计已开启
      </span>
      <span className="status-item">
        <Icon name="lock" />
        本地 · 无遥测
      </span>
    </footer>
  );
}
