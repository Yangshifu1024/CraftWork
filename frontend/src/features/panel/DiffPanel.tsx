import { Icon } from "../../components/Icon";
import type { DiffLine } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useWorkspaceStore } from "../../stores/workspace";

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

  const result = diff.result;

  return (
    <div className="diff-wrap">
      <div className="diff-summary">
        <span className="badge ok">+{result.additions}</span>
        <span className="badge danger">−{result.deletions}</span>
        <span className="diff-title" title={diff.title}>
          {diff.title}
        </span>
        <span className="spacer" />
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

type DiffLineItem = DiffLine;

function sideRows(lines: DiffLineItem[]) {
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
