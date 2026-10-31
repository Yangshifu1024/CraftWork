import { Icon } from "../../components/Icon";
import { formatTokens } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { usePanelStore } from "../../stores/panel";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";

const levelLabel: Record<string, string> = {
  graded: "分级",
  strict: "严格",
  readonly: "只读",
};

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

  const thread = threads.find((item) => item.id === activeID);
  const model = providers
    .flatMap((provider) => provider.models)
    .find((item) => item.id === thread?.modelId);
  const contextK = model?.contextK ?? 32;
  const used = thread?.tokens ?? 0;
  const pct = Math.min(100, Math.round((used / (contextK * 1000)) * 100));
  const barClass = pct > 85 ? "danger" : pct > 70 ? "warn" : "";

  return (
    <footer className="statusbar">
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

      {thread?.status === "error" ? (
        <button type="button" className="status-item danger" onClick={() => openData("audit")}>
          <Icon name="alert" />
          错误 1
        </button>
      ) : null}

      <span className="status-spacer" />

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
      <button type="button" className="status-item" onClick={() => openSettings("tools")}>
        <Icon name="shield" />
        审批 {levelLabel[settings.approvalLevel] ?? "分级"}
      </button>
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
