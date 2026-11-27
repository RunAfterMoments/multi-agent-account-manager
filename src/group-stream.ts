// 群聊流式增量应用（v6.4 1B）：把后端 `group-msg-stream` 事件的单条
// 消息全文同步到内存快照，并在群聊页处于该群时局部更新正文容器，
// 不触发全量 render。零运行时依赖（只 import type），便于 node:test
// 直接导入测试。
import type { GroupMessageStreamEvent, ManagerSnapshot } from "./types";

export type GroupStreamApplyResult = "applied" | "ignored";

export function applyGroupMessageStreamEvent(
  snapshot: ManagerSnapshot,
  ev: GroupMessageStreamEvent,
  doc: Document,
  opts: {
    isGroupPageActive: boolean;
    memberAliases: string[];
    renderText: (content: string, memberAliases: string[]) => string;
  },
): GroupStreamApplyResult {
  const group = snapshot.groupChats.find((g) => g.id === ev.groupId);
  const msg = group?.messages.find((m) => m.id === ev.msgId);
  if (!group || !msg) return "ignored";
  // 1. 同步内存快照，防止后续任意 render() 把文本回闪成旧内容。
  msg.content = ev.content;
  if (!opts.isGroupPageActive) return "applied";
  // 2. 局部更新正文容器；元素不存在时静默跳过，下次全量快照会纠正。
  const textEl = doc.querySelector<HTMLElement>(
    `[data-msg-text-id="${CSS.escape(ev.msgId)}"]`,
  );
  if (!textEl) return "applied";
  // 3. 仅当原本就贴底时才贴底滚动（先量后改，innerHTML 会改变
  // scrollHeight），绝不打断用户向上翻阅历史或选中文本。
  const container = doc.querySelector<HTMLElement>(".group-messages");
  const nearBottom = container
    ? container.scrollHeight - container.scrollTop - container.clientHeight <= 64
    : true;
  textEl.innerHTML = opts.renderText(ev.content, opts.memberAliases);
  if (nearBottom && container) {
    container.scrollTop = container.scrollHeight;
  }
  return "applied";
}
