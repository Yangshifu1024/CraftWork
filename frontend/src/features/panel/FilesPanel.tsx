import { useMemo } from "react";
import { Icon, type IconName } from "../../components/Icon";
import type { FileNode } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useWorkspaceStore } from "../../stores/workspace";
import { Editor } from "./Editor";

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
        <span>{flat ? `匹配 ${flat.length} 个文件` : "工作区文件"}</span>
        <span className="spacer" />
        <span>{Object.keys(git).length} 个变更</span>
      </div>

      {flat ? (
        <div className="file-list">
          {flat.map((node) => (
            <button
              key={node.path}
              type="button"
              className="file-item"
              onClick={() => void openFile(node.path)}
            >
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
        <div className="tree">
          <TreeLevel path="." depth={0} expanded={expanded} selected={selected} git={git} onToggle={toggleDir} onOpen={openFile} />
        </div>
      )}
    </div>
  );
}

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
            <div
              className={cn(
                "tree-row",
                isDir && "is-dir",
                isOpen && "is-open",
                selected === node.path && "is-selected",
              )}
              style={{ "--depth": depth } as React.CSSProperties}
              onClick={() => {
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
            </div>
            {isDir ? (
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

export function fileIcon(name: string): IconName {
  const lower = name.toLowerCase();
  if (lower.endsWith(".md")) {
    return "file-text";
  }
  if (lower.endsWith(".go") || lower.endsWith(".ts") || lower.endsWith(".tsx") || lower.endsWith(".js") || lower.endsWith(".py")) {
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
  if (lower.endsWith(".png") || lower.endsWith(".jpg") || lower.endsWith(".jpeg") || lower.endsWith(".webp") || lower.endsWith(".svg")) {
    return "image";
  }
  return "file";
}

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
