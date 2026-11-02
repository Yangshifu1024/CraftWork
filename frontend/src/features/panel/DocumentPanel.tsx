/**
 * 文档面板（Work 模式）。
 *
 * 负责：展示 Word / Excel / PDF 提取出的文本预览与文件基本信息，
 * 并提供「在编辑器中打开」或「重新加载预览」的入口。
 *
 * 当前只做文本级能力：Word 仅替换同格式片段内的文本，
 * Excel 不计算公式，PDF 只读文本层，不做 OCR 与编辑。
 */
import { Icon } from "../../components/Icon";
import { formatBytes, formatTime } from "../../lib/utils";
import { useWorkspaceStore } from "../../stores/workspace";

/**
 * 文档预览组件。
 *
 * props：无。状态全部来自 workspace store：
 * - doc：提取后的文本、类型（docx / xlsx / pdf）、是否可预览与补充说明；
 * - docLoading：是否正在读取；
 * - supported 且 kind 为 text 时可以在编辑器中打开，否则只能重新加载预览。
 */
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

  // 只有能提取文本的文档才提供编辑器入口；其余类型只能重新加载预览。
  const editable = doc.supported && doc.kind === "text";

  return (
    <div className="panel-scroll">
      <div className="doc-wrap">
        <div className="doc-toolbar">
          {/* 头部：文件名、类型徽标、大小与修改时间，以及主操作按钮 */}
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
              // 可编辑文本进编辑器；其他类型重新向后端请求预览。
              // doc.folderId 记录了文件所属源文件夹，空值表示主要目录。
              if (editable) {
                void openFile(doc.folderId ?? "", doc.path);
              } else {
                void openDocument(doc.folderId ?? "", doc.path);
              }
            }}
          >
            <Icon name={editable ? "file-code" : "refresh"} className="ico" />
            {editable ? "在编辑器中打开" : "重新加载预览"}
          </button>
        </div>

        <div className="doc-badge-bar">
          {/* 预览能力提示；note 解释无法预览或文本受限的原因 */}
          {doc.supported ? (
            <span className="badge ok">
              <Icon name="check-circle" />
              文本预览
            </span>
          ) : (
            <span className="badge warn">
              <Icon name="alert" />
              无法预览
            </span>
          )}
          {doc.note ? <span className="doc-note">{doc.note}</span> : null}
        </div>

        {doc.supported && doc.text ? (
          /* 提取出的纯文本按原样展示，不做富文本样式还原 */
          <pre className="doc-text">{doc.text}</pre>
        ) : (
          /* 无可展示文本时给出图标与原因说明 */
          <div className="panel-empty">
            <Icon name={doc.kind === "xlsx" ? "table" : "file-text"} size={24} />
            <div>没有可提取的文本</div>
            <div className="panel-empty-hint">{doc.note ?? "文档内容为空。"}</div>
          </div>
        )}
      </div>
    </div>
  );
}
