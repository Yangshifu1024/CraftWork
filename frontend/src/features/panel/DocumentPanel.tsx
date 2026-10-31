import { Icon } from "../../components/Icon";
import { formatBytes, formatTime } from "../../lib/utils";
import { useWorkspaceStore } from "../../stores/workspace";

export function DocumentPanel() {
  const doc = useWorkspaceStore((state) => state.doc);
  const loading = useWorkspaceStore((state) => state.docLoading);
  const openFile = useWorkspaceStore((state) => state.openFile);
  const openDocument = useWorkspaceStore((state) => state.openDocument);

  if (loading) {
    return (
      <div className="panel-scroll">
        <div className="panel-empty">
          <Icon name="loader" size={20} className="spin" />
          <div>正在读取文档…</div>
        </div>
      </div>
    );
  }

  if (!doc) {
    return (
      <div className="panel-scroll">
        <div className="panel-empty">
          <Icon name="file-text" size={22} />
          <div>还没有打开文档</div>
          <div className="panel-empty-hint">在 Work 模式的文件树中选择 Word、Excel 或 PDF 文件。</div>
        </div>
      </div>
    );
  }

  return (
    <div className="panel-scroll">
      <div className="doc-wrap">
        <div className="doc-toolbar">
          <Icon name="file-text" className="ico" />
          <span className="doc-title" title={doc.path}>
            {doc.name}
          </span>
          <span className="badge neutral">{doc.kind.toUpperCase()}</span>
          <span className="spacer" />
          <span className="doc-meta">
            {formatBytes(doc.size)} · {formatTime(doc.modified)}
          </span>
          <button
            type="button"
            className="tool-btn"
            onClick={() => {
              if (doc.supported) {
                void openFile(doc.path);
              } else {
                void openDocument(doc.path);
              }
            }}
          >
            <Icon name={doc.supported ? "file-code" : "refresh"} className="ico" />
            {doc.supported ? "在编辑器中打开" : "重新加载预览"}
          </button>
        </div>

        <div className="doc-badge-bar">
          {doc.supported ? (
            <span className="badge ok">
              <Icon name="check-circle" />
              文本预览
            </span>
          ) : (
            <span className="badge warn">
              <Icon name="alert" />
              预览模型
            </span>
          )}
          {doc.note ? <span className="doc-note">{doc.note}</span> : null}
        </div>

        {doc.supported && doc.text ? (
          <pre className="doc-text">{doc.text}</pre>
        ) : (
          <div className="panel-empty">
            <Icon name={doc.kind === "xlsx" ? "table" : "file-text"} size={24} />
            <div>结构化解析将在 M3 接入</div>
            <div className="panel-empty-hint">{doc.note ?? "当前版本只展示文件信息。"}</div>
          </div>
        )}
      </div>
    </div>
  );
}
