/**
 * 面板多标签页状态库的单元测试。
 *
 * 覆盖 spec 第 1–4 节的核心规则：幂等打开、归属盖戳不可覆盖、可见性矩阵、
 * 关闭与邻居激活、关闭其他/全部、最近关闭栈、线程清理、排序与 file 上限。
 */
import { describe, expect, it } from "vitest";
import {
  activateTab,
  closeAllVisibleTabs,
  closeOtherTabs,
  closeTab,
  DIFF_TAB_ID,
  emptySidePaneState,
  enforceFileTabLimit,
  fileTabId,
  isTabVisibleInScope,
  makeDiffTab,
  makeDocumentTab,
  makeFileTab,
  makeTerminalTab,
  markTabsOwnership,
  openTab,
  pruneThreadTabs,
  pushRecentClosed,
  RECENT_CLOSED_LIMIT,
  reorderTabs,
  type SidePaneState,
  type SidePaneTab,
  TERMINAL_TAB_ID,
  takeRecentClosed,
  visibleActiveTabId,
  visibleTabs,
} from "./sidePane";

/** 工作区 W1 的线程 T1 / T2 上下文，供大多数用例复用。 */
const W1_T1 = { workspaceId: "w1", threadId: "t1" };
const W1_T2 = { workspaceId: "w1", threadId: "t2" };
const W2_T1 = { workspaceId: "w2", threadId: "t1" };
const W2_T2 = { workspaceId: "w2", threadId: "t2" };

/** 造一个 file tab，路径即标题，时间戳固定便于断言。 */
function fileTab(workspaceId: string, threadId: string | null, path: string, openedAt = 1000) {
  return makeFileTab({ workspaceId, threadId, folderId: "", path, title: path, openedAt });
}

/** 在初始状态上依次打开一组 tab。 */
function stateWith(tabs: SidePaneTab[], scope = W1_T1): SidePaneState {
  let state = emptySidePaneState();
  for (const tab of tabs) {
    state = openTab(state, tab, scope);
  }
  return state;
}

describe("openTab 幂等打开", () => {
  it("新 tab 追加到末尾并激活", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go"), fileTab("w1", "t1", "b.go")]);
    expect(state.tabs).toHaveLength(2);
    expect(state.activeTabId).toBe(fileTabId("w1", "", "b.go"));
  });

  it("重复打开同一文件只激活，不产生重复 tab", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go"), fileTab("w1", "t1", "b.go")]);
    const reopened = openTab(state, fileTab("w1", "t1", "a.go", 2000), W1_T1);
    expect(reopened.tabs).toHaveLength(2);
    expect(reopened.activeTabId).toBe(fileTabId("w1", "", "a.go"));
    // 打开时间保留首开值，激活时间被刷新。
    expect(reopened.tabs[0].openedAt).toBe(1000);
    expect(reopened.tabs[0].activatedAt).toBeGreaterThan(1000);
  });

  it("重复打开会刷新标题等展示字段", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go")]);
    const renamed = openTab(state, { ...fileTab("w1", "t1", "a.go"), title: "新标题" }, W1_T1);
    expect(renamed.tabs[0].title).toBe("新标题");
  });

  it("singleton（diff/terminal）重复打开只聚焦", () => {
    const state = stateWith([makeDiffTab({ workspaceId: "w1", threadId: "t1" }), makeTerminalTab({ threadId: "t1" })]);
    const again = openTab(state, makeDiffTab({ workspaceId: "w1", threadId: "t1" }), W1_T1);
    expect(again.tabs.filter((tab) => tab.id === DIFF_TAB_ID)).toHaveLength(1);
    expect(again.activeTabId).toBe(DIFF_TAB_ID);
    expect(again.tabs.filter((tab) => tab.id === TERMINAL_TAB_ID)).toHaveLength(1);
  });
});

describe("归属盖戳", () => {
  it("未认领的 tab 在打开时按当前上下文补归属", () => {
    const state = openTab(emptySidePaneState(), fileTab("w1", null, "a.go"), W1_T1);
    expect(state.tabs[0].threadId).toBe("t1");
  });

  it("已带归属的 tab 打开时不被当前上下文覆盖", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go")]);
    const reopened = openTab(state, fileTab("w1", "t1", "a.go"), W1_T2);
    expect(reopened.tabs[0].threadId).toBe("t1");
  });

  it("markTabsOwnership 只盖未认领的 tab", () => {
    const state: SidePaneState = {
      ...emptySidePaneState(),
      tabs: [fileTab("w1", null, "a.go"), fileTab("w1", "t2", "b.go")],
    };
    const stamped = markTabsOwnership(state, W1_T1);
    expect(stamped.tabs[0].threadId).toBe("t1");
    expect(stamped.tabs[1].threadId).toBe("t2");
    // 无未认领 tab 时返回原引用（no-op 约定）。
    expect(markTabsOwnership(stamped, W1_T1)).toBe(stamped);
  });
});

describe("可见性", () => {
  it("工作区不匹配不可见", () => {
    const tab = fileTab("w1", "t1", "a.go");
    expect(isTabVisibleInScope(tab, W1_T1)).toBe(true);
    expect(isTabVisibleInScope(tab, W2_T1)).toBe(false);
  });

  it("工作区匹配但线程不匹配不可见；草稿与草稿匹配", () => {
    const tab = fileTab("w1", "t1", "a.go");
    expect(isTabVisibleInScope(tab, W1_T2)).toBe(false);
    const draftTab = fileTab("w1", null, "a.go");
    expect(isTabVisibleInScope(draftTab, { workspaceId: "w1", threadId: "draft" })).toBe(true);
  });

  it("终端是公共 tab：跨线程、跨工作区可见", () => {
    const terminal = makeTerminalTab({ threadId: "t1" });
    expect(terminal.workspaceId).toBe("");
    expect(isTabVisibleInScope(terminal, W1_T1)).toBe(true);
    expect(isTabVisibleInScope(terminal, W2_T2)).toBe(true);
  });

  it("差异 tab 按线程过滤", () => {
    const diff = makeDiffTab({ workspaceId: "w1", threadId: "t1" });
    expect(isTabVisibleInScope(diff, W1_T1)).toBe(true);
    expect(isTabVisibleInScope(diff, W1_T2)).toBe(false);
  });

  it("visibleTabs 与 visibleActiveTabId 的过滤与兜底", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go"), fileTab("w1", "t2", "b.go")]);
    const t1View = visibleTabs(state, W1_T1);
    expect(t1View.map((tab) => tab.title)).toEqual(["a.go"]);
    // activeTabId 指向 t2 的 tab（切走前的遗留），兜底到 t1 的最后一个可见 tab。
    expect(visibleActiveTabId(state, W1_T1)).toBe(fileTabId("w1", "", "a.go"));
    expect(visibleActiveTabId(stateWith([]), W1_T1)).toBeNull();
  });
});

describe("关闭与邻居激活", () => {
  it("关闭中间的激活 tab 时激活右侧邻居", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go"), fileTab("w1", "t1", "b.go"), fileTab("w1", "t1", "c.go")]);
    const { state: next, removed } = closeTab(state, fileTabId("w1", "", "b.go"));
    expect(removed).toHaveLength(1);
    expect(next.activeTabId).toBe(fileTabId("w1", "", "c.go"));
  });

  it("关闭末尾的激活 tab 时回落到前一个", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go"), fileTab("w1", "t1", "b.go")]);
    const { state: next } = closeTab(state, fileTabId("w1", "", "b.go"));
    expect(next.activeTabId).toBe(fileTabId("w1", "", "a.go"));
  });

  it("关闭唯一 tab 后激活项为 null（面板收敛为收起）", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go")]);
    const { state: next } = closeTab(state, fileTabId("w1", "", "a.go"));
    expect(next.tabs).toHaveLength(0);
    expect(next.activeTabId).toBeNull();
  });

  it("关闭非激活 tab 不改变激活项", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go"), fileTab("w1", "t1", "b.go")]);
    const { state: next } = closeTab(state, fileTabId("w1", "", "a.go"));
    expect(next.activeTabId).toBe(fileTabId("w1", "", "b.go"));
  });

  it("closeOtherTabs 只保留目标（在可见集内），隐藏 tab 不受影响", () => {
    const state = stateWith([
      fileTab("w1", "t1", "a.go"),
      fileTab("w1", "t1", "b.go"),
      fileTab("w1", "t2", "hidden.go"),
    ]);
    const { state: next, removed } = closeOtherTabs(state, fileTabId("w1", "", "b.go"), W1_T1);
    expect(removed.map((tab) => tab.title)).toEqual(["a.go"]);
    expect(next.tabs.map((tab) => tab.title)).toEqual(["b.go", "hidden.go"]);
    expect(next.activeTabId).toBe(fileTabId("w1", "", "b.go"));
  });

  it("closeOtherTabs 的目标不可见时为 no-op", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go"), fileTab("w1", "t2", "b.go")]);
    const { state: next, removed } = closeOtherTabs(state, fileTabId("w1", "", "b.go"), W1_T1);
    expect(removed).toHaveLength(0);
    expect(next).toBe(state);
  });

  it("closeAllVisibleTabs 清空可见集但保留隐藏 tab（终端是公共 tab，也在可见集内）", () => {
    const state = stateWith([
      fileTab("w1", "t1", "a.go"),
      fileTab("w1", "t2", "hidden.go"),
      makeTerminalTab({ threadId: "t1" }),
    ]);
    const { state: next, removed } = closeAllVisibleTabs(state, W1_T1);
    expect(removed.map((tab) => tab.id)).toEqual([fileTabId("w1", "", "a.go"), TERMINAL_TAB_ID]);
    expect(next.tabs.map((tab) => tab.title)).toEqual(["hidden.go"]);
    // 激活项按邻居选择落到存活的隐藏 tab 上；渲染层会因可见性兜底为空态，
    // 切回 t2 时该 tab 自动恢复显示。
    expect(next.activeTabId).toBe(fileTabId("w1", "", "hidden.go"));
    expect(visibleActiveTabId(next, W1_T1)).toBeNull();
  });

  it("activateTab 刷新激活时间且 no-op 返回原引用", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go"), fileTab("w1", "t1", "b.go")]);
    const activated = activateTab(state, fileTabId("w1", "", "a.go"));
    expect(activated.activeTabId).toBe(fileTabId("w1", "", "a.go"));
    expect(activated.tabs[0].activatedAt).toBeGreaterThan(1000);
    expect(activateTab(state, "missing")).toBe(state);
  });
});

describe("最近关闭栈", () => {
  it("新的在前，同 id 去重，超过上限从尾部淘汰", () => {
    let state = emptySidePaneState();
    for (let index = 0; index < RECENT_CLOSED_LIMIT + 2; index += 1) {
      state = pushRecentClosed(state, [fileTab("w1", "t1", `f${index}.go`)]);
    }
    expect(state.recentClosed).toHaveLength(RECENT_CLOSED_LIMIT);
    expect(state.recentClosed[0].title).toBe("f9.go");
    expect(state.recentClosed.map((tab) => tab.title)).not.toContain("f0.go");

    const deduped = pushRecentClosed(state, [fileTab("w1", "t1", "f9.go")]);
    expect(deduped.recentClosed.filter((tab) => tab.title === "f9.go")).toHaveLength(1);
  });

  it("takeRecentClosed 校验作用域：跨线程不可恢复", () => {
    const state = pushRecentClosed(emptySidePaneState(), [fileTab("w1", "t1", "a.go")]);
    const ok = takeRecentClosed(state, fileTabId("w1", "", "a.go"), W1_T1);
    expect(ok.tab?.title).toBe("a.go");
    expect(ok.state.recentClosed).toHaveLength(0);

    const denied = takeRecentClosed(state, fileTabId("w1", "", "a.go"), W1_T2);
    expect(denied.tab).toBeNull();
    expect(denied.state).toBe(state);
  });

  it("终端重开不受线程作用域限制（公共 tab）", () => {
    const state = pushRecentClosed(emptySidePaneState(), [makeTerminalTab({ threadId: "t1" })]);
    const { tab } = takeRecentClosed(state, TERMINAL_TAB_ID, W2_T2);
    expect(tab?.id).toBe(TERMINAL_TAB_ID);
  });
});

describe("线程清理与排序、上限", () => {
  it("pruneThreadTabs 删除该线程名下 tab，保留公共 tab", () => {
    const state = stateWith([
      fileTab("w1", "t1", "a.go"),
      makeDiffTab({ workspaceId: "w1", threadId: "t1" }),
      makeTerminalTab({ threadId: "t1" }),
      fileTab("w1", "t2", "b.go"),
    ]);
    const { state: next, removed } = pruneThreadTabs(state, "t1");
    expect(removed.map((tab) => tab.id)).toEqual([fileTabId("w1", "", "a.go"), DIFF_TAB_ID]);
    expect(next.tabs.map((tab) => tab.id)).toEqual([TERMINAL_TAB_ID, fileTabId("w1", "", "b.go")]);
  });

  it("reorderTabs 只改顺序不改激活项", () => {
    const state = stateWith([fileTab("w1", "t1", "a.go"), fileTab("w1", "t1", "b.go"), fileTab("w1", "t1", "c.go")]);
    const reordered = reorderTabs(state, fileTabId("w1", "", "c.go"), fileTabId("w1", "", "a.go"));
    expect(reordered.tabs.map((tab) => tab.title)).toEqual(["c.go", "a.go", "b.go"]);
    expect(reordered.activeTabId).toBe(state.activeTabId);
    expect(reorderTabs(state, "missing", "a.go")).toBe(state);
  });

  it("enforceFileTabLimit 淘汰最久未激活的 file tab，其他类型不受限", () => {
    // 直接拼接构造，避免 openTab 刷新 activatedAt 干扰淘汰排序；
    // openedAt 递增（1000..1008），未显式激活过时 activatedAt 等于 openedAt。
    let state = emptySidePaneState();
    for (let index = 0; index < 9; index += 1) {
      state = { ...state, tabs: [...state.tabs, fileTab("w1", "t1", `f${index}.go`, 1000 + index)] };
    }
    // 激活 f0 刷新它的激活时间；此后从未激活且 openedAt 最小的是 f1，
    // 所以被淘汰的是 f1 而不是 openedAt 最大的 f8。
    state = activateTab(state, fileTabId("w1", "", "f0.go"));
    const { state: next, evicted } = enforceFileTabLimit(state, 8);
    expect(evicted.map((tab) => tab.title)).toEqual(["f1.go"]);
    expect(next.tabs.some((tab) => tab.title === "f1.go")).toBe(false);
    expect(next.tabs.some((tab) => tab.title === "f8.go")).toBe(true);

    // 未超限时 no-op。
    expect(enforceFileTabLimit(next, 8).evicted).toHaveLength(0);
  });

  it("document tab 携带类型与路径并参与幂等", () => {
    const state = stateWith([
      makeDocumentTab({
        workspaceId: "w1",
        threadId: "t1",
        folderId: "",
        path: "报告.docx",
        title: "报告.docx",
        kind: "docx",
      }),
    ]);
    const again = openTab(
      state,
      makeDocumentTab({
        workspaceId: "w1",
        threadId: "t1",
        folderId: "",
        path: "报告.docx",
        title: "报告.docx",
        kind: "docx",
      }),
      W1_T1,
    );
    expect(again.tabs).toHaveLength(1);
    expect(again.tabs[0].type).toBe("document");
  });
});
