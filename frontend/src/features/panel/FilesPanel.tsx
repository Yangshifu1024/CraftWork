/**
 * 文件面板。
 *
 * 负责：展示工作区文件树、按关键字过滤文件、切换目录展开状态、打开文件。
 * 选中文件后本组件会被编辑器取代（见 Editor），关闭编辑器即回到文件树。
 */
import { useMemo } from "react";
import { Icon, type IconName } from "../../components/Icon";
import type { FileNode } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useWorkspaceStore } from "../../stores/workspace";
import { Editor } from "./Editor";

/**
 * 文件树面板。
 *
 * 两种展示方式：
 * - 过滤框为空时渲染可折叠的树形结构；
 * - 输入关键字时改为平铺的文件列表（只匹配文件名，最多 200 条），便于快速定位。
 *
 * 目录展开状态、当前选中文件与 Git 变更标记都来自 workspace store。
 */
export function FilesPanel() {
  const tree = useWorkspaceStore((state) => state.tree);
  const expanded = useWorkspaceStore((state) => state.expanded);
  const filter = useWorkspaceStore((state) => state.filter);
  const setFilter = useWorkspaceStore((state) => state.setFilter);
  const toggleDir = useWorkspaceStore((state) => state.toggleDir);
  const openFile = useWorkspaceStore((state) => state.openFile);
  const loadRoot = useWorkspaceStore((state) => state.loadRoot);
  const selected = useWorkspaceStore((state) => state.selected);
  const git = useWorkspaceStore((state) => state.git);

  // 过滤模式下的扁平文件列表：遍历所有已加载目录，按文件名包含关键字筛选并去重。
  // 返回 null 表示没有过滤关键字，界面应显示树形结构。
  const flat = useMemo(() => {
    if (!filter.trim()) {
      return null;
    }
    const keyword = filter.trim().toLowerCase();
    const seen = new Set<string>();
    const result: FileNode[] = [];
    for (const nodes of Object.values(tree)) {
      for (const node of nodes) {
        if (node.type === "file" && node.name.toLowerCase().includes(keyword) && !seen.has(node.path)) {
          seen.add(node.path);
          result.push(node);
        }
      }
    }
    return result.slice(0, 200);
  }, [filter, tree]);

  if (selected) {
    return <Editor />;
  }

  return (
    <div className="panel-scroll">
      <div className="tree-toolbar">
        {/* 过滤框只做前端筛选，不会触发后端搜索 */}
        <div className="tree-filter">
          <Icon name="search" className="ico" />
          <input
            type="text"
            value={filter}
            placeholder="过滤文件…"
            onChange={(event) => setFilter(event.target.value)}
          />
        </div>
        <button type="button" className="icon-btn sm" title="刷新文件树" onClick={() => void loadRoot()}>
          <Icon name="refresh" className="ico" />
        </button>
      </div>
      <div className="tree-meta">
        {/* 左侧显示当前是过滤结果还是完整工作区，右侧显示 Git 变更文件数 */}
        <span>{flat ? `匹配 ${flat.length} 个文件` : "工作区文件"}</span>
        <span className="spacer" />
        <span>{Object.keys(git).length} 个变更</span>
      </div>

      {flat ? (
        /* 过滤结果：平铺展示文件名与完整路径，点击直接打开 */
        <div className="file-list">
          {flat.map((node) => (
            <button key={node.path} type="button" className="file-item" onClick={() => void openFile(node.path)}>
              <Icon name={fileIcon(node.name)} className={cn("f-ico", extensionClass(node.name))} />
              <span className="file-item-main">
                <div className="file-item-name">{node.name}</div>
                <div className="file-item-path">{node.path}</div>
              </span>
              {git[node.path] ? <span className={cn("tree-git", git[node.path])}>{git[node.path]}</span> : null}
            </button>
          ))}
        </div>
      ) : (
        /* 无过滤关键字：从根目录开始递归渲染树 */
        <div className="tree">
          <TreeLevel
            path="."
            depth={0}
            expanded={expanded}
            selected={selected}
            git={git}
            onToggle={toggleDir}
            onOpen={openFile}
          />
        </div>
      )}
    </div>
  );
}

/**
 * 递归渲染一层文件树。
 *
 * props：
 * - path：当前层对应的目录路径，"." 表示工作区根目录；
 * - depth：层级深度，用于缩进；
 * - expanded：目录路径到展开状态的映射；
 * - selected：当前选中文件的路径；
 * - git：文件路径到 Git 状态字母的映射；
 * - onToggle / onOpen：点击目录时切换展开，点击文件时打开。
 *
 * 每层只渲染自己的节点，子目录通过再次渲染自身实现递归。
 */
function TreeLevel({
  path,
  depth,
  expanded,
  selected,
  git,
  onToggle,
  onOpen,
}: {
  path: string;
  depth: number;
  expanded: Record<string, boolean>;
  selected: string | null;
  git: Record<string, string>;
  onToggle: (path: string) => void;
  onOpen: (path: string) => void;
}) {
  const tree = useWorkspaceStore((state) => state.tree);
  const nodes = tree[path];
  // 该目录内容还没加载时（例如根目录尚未读取），本层不渲染任何内容。
  if (!nodes) {
    return null;
  }
  return (
    <>
      {nodes.map((node) => {
        const isDir = node.type === "dir";
        const isOpen = Boolean(expanded[node.path]);
        return (
          <div key={node.path} className={cn("tree-node", isOpen && "is-open")}>
            {/* 语义化按钮：文件树行可聚焦、可回车打开，读屏可识别 */}
            <button
              type="button"
              className={cn(
                "tree-row",
                isDir && "is-dir",
                isOpen && "is-open",
                selected === node.path && "is-selected",
              )}
              style={{ "--depth": depth } as React.CSSProperties}
              onClick={() => {
                // 目录点击只切换展开，文件点击才打开编辑器
                if (isDir) {
                  onToggle(node.path);
                } else {
                  onOpen(node.path);
                }
              }}
            >
              <Icon name="chevron-right" className={cn("twist", !isDir && "placeholder")} />
              <Icon
                name={isDir ? (isOpen ? "folder-open" : "folder") : fileIcon(node.name)}
                className={cn("f-ico", extensionClass(node.name))}
              />
              <span className="tree-name">{node.name}</span>
              {git[node.path] ? <span className={cn("tree-git", git[node.path])}>{git[node.path]}</span> : null}
            </button>
            {isDir ? (
              /* 目录展开时递归渲染下一层；未展开则不渲染子节点，减少无用工作量 */
              <div className="tree-children">
                <TreeLevel
                  path={node.path}
                  depth={depth + 1}
                  expanded={expanded}
                  selected={selected}
                  git={git}
                  onToggle={onToggle}
                  onOpen={onOpen}
                />
              </div>
            ) : null}
          </div>
        );
      })}
    </>
  );
}

/**
 * 根据文件名后缀选择文件图标。
 * 只按扩展名判断，未识别的类型统一使用通用文件图标。
 */
export function fileIcon(name: string): IconName {
  const lower = name.toLowerCase();
  if (lower.endsWith(".md")) {
    return "file-text";
  }
  if (
    lower.endsWith(".go") ||
    lower.endsWith(".ts") ||
    lower.endsWith(".tsx") ||
    lower.endsWith(".js") ||
    lower.endsWith(".py")
  ) {
    return "file-code";
  }
  if (lower.endsWith(".json") || lower.endsWith(".yaml") || lower.endsWith(".yml") || lower.endsWith(".toml")) {
    return "file-code";
  }
  if (lower.endsWith(".xlsx") || lower.endsWith(".csv")) {
    return "table";
  }
  if (lower.endsWith(".pdf") || lower.endsWith(".docx")) {
    return "file-text";
  }
  if (
    lower.endsWith(".png") ||
    lower.endsWith(".jpg") ||
    lower.endsWith(".jpeg") ||
    lower.endsWith(".webp") ||
    lower.endsWith(".svg")
  ) {
    return "image";
  }
  return "file";
}

/**
 * 返回文件后缀对应的样式类名，用于给图标着色。
 * 返回空字符串表示使用默认颜色。
 */
export function extensionClass(name: string): string {
  const lower = name.toLowerCase();
  if (lower.endsWith(".go")) {
    return "go";
  }
  if (lower.endsWith(".md")) {
    return "md";
  }
  if (lower.endsWith(".docx")) {
    return "doc";
  }
  if (lower.endsWith(".xlsx")) {
    return "sheet";
  }
  if (lower.endsWith(".pdf")) {
    return "pdf";
  }
  return "";
}
