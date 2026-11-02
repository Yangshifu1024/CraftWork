/**
 * 「激活工作区」相关用例的唯一编排点。
 *
 * 工作区变更会使文件树、差异、文档预览等视图数据全部作废，并影响会话
 * 列表的归属展示。这些联动此前散在各调用方（侧栏切换、添加工作区、新
 * 建临时会话各自复述一遍，顺序与子集还各不相同），现在统一收到这里：
 * 调用方只表达意图（切换 / 重同步 / 新建临时会话），不再自己拼装步骤。
 */
import { useAppStore } from "./app";
import { useThreadStore } from "./threads";
import { useUiStore } from "./ui";
import { useWorkspaceStore } from "./workspace";

/**
 * 切换激活工作区并重同步依赖视图。
 *
 * 后端持久化激活状态后，文件树重置重载、会话列表刷新都由这里驱动；
 * 调用方不再各自 reset / loadRoot / refresh。
 */
export async function switchWorkspace(workspaceID: string) {
  await useAppStore.getState().setActiveWorkspace(workspaceID);
  await resyncWorkspaceViews();
}

/**
 * 按当前激活工作区重同步依赖视图。
 *
 * 供「添加工作区」这类激活已在 store 内完成的入口使用：工作区视图
 * 重置重载，会话列表按当前模式刷新。
 */
export async function resyncWorkspaceViews() {
  useWorkspaceStore.getState().reset();
  await useWorkspaceStore.getState().loadRoot();
  await useThreadStore.getState().refresh(useAppStore.getState().mode);
}

/**
 * 新建临时会话：切到内置临时会话目录（purpose = conversation 的工作区）
 * 并新建绑定它的会话。已经在临时目录时直接新建，不重复切换。内置目录
 * 缺失（初始化失败的极端情况）时提示并保持现状。
 */
export async function createTempThread() {
  const app = useAppStore.getState();
  const conversation = app.workspaces.find((workspace) => workspace.purpose === "conversation");
  if (!conversation) {
    useUiStore.getState().toast({
      title: "临时会话不可用",
      description: "内置临时目录尚未初始化，请重启应用后重试",
      tone: "warn",
    });
    return;
  }
  if (app.activeWorkspace?.id !== conversation.id) {
    await switchWorkspace(conversation.id);
  }
  await useThreadStore.getState().create();
}
