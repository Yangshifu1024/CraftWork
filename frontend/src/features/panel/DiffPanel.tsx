/**
 * 差异面板。
 *
 * 负责：展示最近一次差异预览（统一视图或并排视图）、切换展示方式、
 * 在编辑器中打开对应文件，以及应用全部修改。
 *
 * 数据来自 workspace store 的 diff，其中是按行差异与分段（hunk）的结构化结果。
 */
import { Icon } from "../../components/Icon";
import type { DiffLine } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useWorkspaceStore } from "../../stores/workspace";

/**
 * 差异面板组件。
 *
 * props：无，内容与操作全部取自 workspace store。
 * diff 为空表示还没有可展示的差异，此时显示空状态提示。
 */
export function DiffPanel() {
  const diff = useWorkspaceStore((state) => state.diff);
  const setView = useWorkspaceStore((state) => state.setDiffView);
  const applyDiff = useWorkspaceStore((state) => state.applyDiff);
  const openFile = useWorkspaceStore((state) => state.openFile);

  if (!diff || !diff.result) {
    return (
      <div className="panel-scroll">
        <div className="panel-empty">
          <Icon name="diff" size={22} />
          <div>还没有差异预览</div>
          <div className="panel-empty-hint">在审批卡片中点击「查看完整差异」，或让 Agent 修改文件。</div>
        </div>
      </div>
    );
  }

  // 前面已经确认 result 存在，后面直接使用，避免每处都判空。
  const result = diff.result;

  return (
    <div className="diff-wrap">
      <div className="diff-summary">
        {/* 概要：新增 / 删除行数、文件标题与视图切换 */}
        <span className="badge ok">+{result.additions}</span>
        <span className="badge danger">−{result.deletions}</span>
        <span className="diff-title" title={diff.title}>
          {diff.title}
        </span>
        <span className="spacer" />
        {/* 视图切换：统一视图逐行显示，并排视图左右两列对照 */}
        <div className="segmented">
          <button type="button" className={diff.view === "unified" ? "is-on" : ""} onClick={() => setView("unified")}>
            统一
          </button>
          <button type="button" className={diff.view === "side" ? "is-on" : ""} onClick={() => setView("side")}>
            并排
          </button>
        </div>
      </div>

      <div className="diff-scroll">
        {/* 依次处理三种情况：文件过大被截断、没有差异、正常渲染分段 */}
        {result.truncated ? (
          <div className="panel-empty">
            <Icon name="alert" size={20} />
            <div>文件过大，无法逐行对比</div>
          </div>
        ) : result.hunks.length === 0 ? (
          <div className="panel-empty">
            <Icon name="check-circle" size={20} />
            <div>内容一致，没有差异</div>
          </div>
        ) : (
          result.hunks.map((hunk, hunkIndex) => (
            <div className="diff-hunk" key={`${hunk.header}-${hunkIndex}`}>
              <div className="diff-hunk-head">
                <Icon name="diff" className="ico" />
                <span className="hunk-title">{hunk.header}</span>
              </div>
              {diff.view === "unified" ? (
                /* 统一视图：每行同时显示旧行号、新行号、变更符号与内容 */
                <div className="diff-table">
                  {hunk.lines.map((line, index) => (
                    <div key={index} className={cn("dline", line.kind)}>
                      <span className="dn">{line.old ?? ""}</span>
                      <span className="dn">{line.new ?? ""}</span>
                      <span className="ds">{line.kind === "add" ? "+" : line.kind === "del" ? "−" : " "}</span>
                      <span className="dt">{line.text}</span>
                    </div>
                  ))}
                </div>
              ) : (
                /* 并排视图：左右两列都由 sideRows 配对后的行渲染，缺失的一侧留空 */
                <div className="diff-side">
                  <div className="diff-side-col">
                    {sideRows(hunk.lines).map((row, index) => (
                      <div key={index} className={cn("sline", row.left?.kind === "del" && "del", !row.left && "empty")}>
                        <span className="dn">{row.left?.old ?? ""}</span>
                        <span className="dt">{row.left?.text ?? ""}</span>
                      </div>
                    ))}
                  </div>
                  <div className="diff-side-col">
                    {sideRows(hunk.lines).map((row, index) => (
                      <div key={index} className={cn("sline", row.right?.kind === "add" && "add", !row.right && "empty")}>
                        <span className="dn">{row.right?.new ?? ""}</span>
                        <span className="dt">{row.right?.text ?? ""}</span>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          ))
        )}
      </div>

      <div className="diff-foot">
        {/* 底部：行数汇总、在编辑器中打开、应用全部修改 */}
        <span>
          {result.additions} 行新增 · {result.deletions} 行删除
        </span>
        <span className="spacer" />
        <button type="button" onClick={() => void openFile(diff.title)}>
          <Icon name="file-code" className="ico" />
          在编辑器中打开
        </button>
        <button type="button" className="primary" onClick={() => void applyDiff()}>
          <Icon name="check" className="ico" />
          应用全部修改
        </button>
      </div>
    </div>
  );
}

/** 差异行的类型别名，便于配对函数里书写。 */
type DiffLineItem = DiffLine;

/**
 * 把逐行差异配对成并排视图的左右两行。
 *
 * 规则：
 * - 上下文（ctx）行左右各放一份；
 * - 连续的删除（del）行先暂存，遇到新增（add）行时与之配成一对；
 * - 删除比新增多时，多出来的删除行只有左列、右侧留空；
 * - 遍历结束后仍暂存的删除行补成只有左列的行。
 */
export function sideRows(lines: DiffLineItem[]) {
  const rows: Array<{ left?: DiffLineItem; right?: DiffLineItem }> = [];
  let pendingDelete: DiffLineItem | null = null;
  for (const line of lines) {
    if (line.kind === "ctx") {
      rows.push({ left: line, right: line });
      continue;
    }
    if (line.kind === "del") {
      if (pendingDelete) {
        rows.push({ left: pendingDelete });
      }
      pendingDelete = line;
      continue;
    }
    rows.push({ left: pendingDelete ?? undefined, right: line });
    pendingDelete = null;
  }
  if (pendingDelete) {
    rows.push({ left: pendingDelete });
  }
  return rows;
}
