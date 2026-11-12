// 斜杠技能候选的模糊匹配（决策 D11，行为规格见 D11）。
//
// 打分复用命令面板（⌘K）同款 `commandScore`（cmdk 导出的 defaultFilter），两处搜索手感一致，
// 也不引入新依赖：
//   - 输入片段的字符按顺序出现在技能名里即算命中，不要求从名称开头匹配
//     （`/audit` 命中 `ponytail-audit`，`/pr` 命中 `mattpocock-skills:pr`）；
//   - 质量分层：从开头连续命中 > 词边界（`-` `_` `/` 等分隔符之后）命中 > 字符跳跃命中；
//   - 大小写不敏感（打分器内部统一小写）。
// 命令面板对低分结果照样渲染（它没有噪声问题：条目少、文案长）；技能名普遍很短，
// 字符零散散布的低分结果会把真正想找的技能挤下去，所以这里加分数阈值把噪声挡掉。
import { defaultFilter } from "cmdk";

/** 分数低于该阈值视为「字符零散散布」的噪声，不入选（如 `/pr` 不命中 `explore`）。 */
const MIN_SCORE = 0.1;

/**
 * 按输入片段对带名字的条目做模糊匹配，返回命中条目（分高者靠前）。
 *
 * @param items 候选条目（按展示优先级排列，同分时保持该顺序）
 * @param token 用户已输入的片段（不含前导 `/`）
 * @returns 命中条目按分数降序；`token` 为空时原样返回（空片段等于「列出全部」）。
 */
export function rankSkillCandidates<T extends { name: string }>(items: T[], token: string): T[] {
  if (token === "") return items;
  const scored = items.map((item) => ({ item, score: defaultFilter(item.name, token) }));
  // 稳定排序：现代引擎的 Array.prototype.sort 对等值保持原顺序，同分即按扫描顺序展开。
  return scored
    .filter((entry) => entry.score >= MIN_SCORE)
    .sort((a, b) => b.score - a.score)
    .map((entry) => entry.item);
}
