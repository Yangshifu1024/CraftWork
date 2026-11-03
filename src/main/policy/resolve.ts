// 路径安全解析。
// 核心不变量：词法校验拒绝 ../ 越界 + 真实路径（符号链接解析后）二次校验拒绝链接逃逸。
import { realpathSync, statSync } from "node:fs";
import { isAbsolute, resolve, sep } from "node:path";

export class ErrOutsideWorkspace extends Error {
  constructor() {
    super("路径超出工作区及附加目录");
    this.name = "ErrOutsideWorkspace";
  }
}

export interface Resolved {
  rootIndex: number;
  root: string;
  abs: string;
  rel: string;
}

function evalSymlinks(path: string): string | null {
  try {
    return realpathSync.native(path);
  } catch {
    return null;
  }
}

/** 目标是否在 root 内：相等，或前缀为 root + 分隔符（防止 /tmp/work 误吞 /tmp/work-other）。 */
function within(root: string, candidate: string): boolean {
  if (candidate === root) return true;
  return candidate.startsWith(root + sep);
}

/**
 * 单根解析：
 * 1. 相对路径拼接 lexRoot（保留符号链接别名）后取词法规范形式；绝对路径保持原样
 * 2. Windows UNC/设备路径直接拒绝
 * 3. 词法校验（目标不存在也不接受越界）
 * 4. EvalSymlinks：目标不存在时解析父目录再拼回文件名（新建文件场景）
 * 5. 真实路径校验（realRoot = EvalSymlinks 后的根，如 macOS /var vs /private/var）
 */
export function resolveAt(lexRoot: string, rel: string): { abs: string; rel: string } {
  const candidate = isAbsolute(rel) ? resolve(rel) : resolve(lexRoot, rel);
  if (candidate.startsWith("\\\\") || candidate.startsWith("//")) throw new ErrOutsideWorkspace();
  if (!within(lexRoot, candidate)) throw new ErrOutsideWorkspace();

  let real = evalSymlinks(candidate);
  if (real === null) {
    // 目标不存在：解析父目录的真实路径再拼回文件名
    const parent = candidate.slice(0, candidate.lastIndexOf(sep)) || sep;
    const base = candidate.slice(candidate.lastIndexOf(sep) + 1);
    const realParent = evalSymlinks(parent);
    if (realParent === null) throw new ErrOutsideWorkspace();
    real = realParent + sep + base;
  }
  const realRoot = evalSymlinks(lexRoot) ?? lexRoot;
  if (!within(realRoot, real)) throw new ErrOutsideWorkspace();
  const relOut = real === realRoot ? "." : real.slice(realRoot.length + 1);
  return { abs: real, rel: relOut };
}

function rootCandidates(roots: string[]): string[] {
  return roots.filter((r) => r.length > 0);
}

/**
 * 多根解析（roots[0] = 主要目录）：
 * - 主要目录不可用仍报错；附加目录不可达则跳过
 * - preferExisting=true 时词法命中先不返回，Stat 确认真实存在才命中；
 *   全程无真实存在则回落第一个词法命中（新建文件是合法场景）
 * - rel 为空串返回第一个可用根本身
 */
export function resolveAny(roots: string[], rel: string, preferExisting = false): Resolved {
  const candidates = rootCandidates(roots);
  if (candidates.length === 0) throw new ErrOutsideWorkspace();
  const cleanRel = rel.trim();
  let firstLexical: Resolved | null = null;

  for (let i = 0; i < candidates.length; i++) {
    const root = candidates[i]!;
    try {
      const target = cleanRel === "" ? root : resolveAt(root, cleanRel).abs;
      if (preferExisting) {
        const lexical: Resolved = { rootIndex: i, root, abs: target, rel: cleanRel === "" ? "." : cleanRel };
        let exists = false;
        try {
          statSync(target);
          exists = true;
        } catch {
          exists = false;
        }
        if (exists) return lexical;
        if (firstLexical === null) firstLexical = lexical;
        continue;
      }
      const resolved = resolveAt(root, cleanRel);
      return {
        rootIndex: i,
        root,
        abs: resolved.abs,
        rel: resolved.rel,
      };
    } catch (err) {
      if (err instanceof ErrOutsideWorkspace || (err as NodeJS.ErrnoException)?.code === "ENOENT") {
        if (preferExisting && !(err instanceof ErrOutsideWorkspace) && firstLexical === null) {
          // ENOENT 记录词法命中候选
          try {
            const lexicalAbs = isAbsolute(cleanRel) ? resolve(cleanRel) : resolve(root, cleanRel);
            firstLexical = { rootIndex: i, root, abs: lexicalAbs, rel: cleanRel };
          } catch {
            // 忽略
          }
        }
        continue; // 尝试下一根
      }
      throw err;
    }
  }
  if (firstLexical) return firstLexical;
  throw new ErrOutsideWorkspace();
}

/** 多根解析，优先命中真实存在的目标。 */
export function resolveExisting(roots: string[], rel: string): Resolved {
  return resolveAny(roots, rel, true);
}

/** 展示路径：相对主要根；越界原样返回。 */
export function relative(workspace: string, path: string): string {
  const abs = resolve(path);
  const root = resolve(workspace);
  if (abs === root) return ".";
  if (abs.startsWith(root + sep)) return abs.slice(root.length + 1);
  return path;
}
