// 群聊流式增量事件的前端应用逻辑测试（v6.4 1B）。
// 运行：npm run test:js
// 被测函数为 src/group-stream.ts 的 applyGroupMessageStreamEvent
//（零运行时依赖，只 import type，Node 24 类型擦除可直接导入）。
import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { applyGroupMessageStreamEvent } from "../src/group-stream.ts";

const RENDER_TEXT = (content) => `R:${content}`;

function makePage() {
  const dom = new JSDOM(
    `<!DOCTYPE html><html><head></head><body>
      <div class="group-messages">
        <div class="group-msg assistant" data-msg-id="m1"><div class="group-msg-text" data-msg-text-id="m1">old</div></div>
        <div class="group-msg assistant" data-msg-id="m2"><div class="group-msg-text" data-msg-text-id="m2">other</div></div>
      </div>
    </body></html>`,
    { url: "https://example.invalid/" },
  );
  const { window } = dom;
  // 助手函数在调用时读全局 CSS.escape；jsdom 的 window.CSS 若缺失则补最小实现。
  if (typeof globalThis.CSS === "undefined") {
    globalThis.CSS =
      window.CSS && typeof window.CSS.escape === "function"
        ? window.CSS
        : { escape: (value) => String(value).replace(/["\\]/g, "\\$&") };
  }
  return { dom, window, document: window.document };
}

function makeSnapshot() {
  return {
    activeGroupId: "g1",
    groupChats: [
      {
        id: "g1",
        messages: [
          { id: "m1", content: "old", status: "streaming" },
          { id: "m2", content: "other", status: "done" },
        ],
      },
    ],
  };
}

function stubScrollMetrics(container, { scrollHeight, clientHeight, scrollTop }) {
  let top = scrollTop;
  Object.defineProperty(container, "scrollHeight", { value: scrollHeight, configurable: true });
  Object.defineProperty(container, "clientHeight", { value: clientHeight, configurable: true });
  Object.defineProperty(container, "scrollTop", {
    get: () => top,
    set: (v) => {
      top = v;
    },
    configurable: true,
  });
  return () => top;
}

test("只更新目标消息正文并同步内存快照，不动其他消息", () => {
  const { document } = makePage();
  const snapshot = makeSnapshot();
  const result = applyGroupMessageStreamEvent(
    snapshot,
    { groupId: "g1", msgId: "m1", content: "new text" },
    document,
    { isGroupPageActive: true, memberAliases: [], renderText: RENDER_TEXT },
  );
  assert.equal(result, "applied");
  assert.equal(
    document.querySelector('[data-msg-text-id="m1"]').innerHTML,
    "R:new text",
  );
  assert.equal(
    document.querySelector('[data-msg-text-id="m2"]').innerHTML,
    "other",
  );
  assert.equal(snapshot.groupChats[0].messages[0].content, "new text");
  assert.equal(snapshot.groupChats[0].messages[1].content, "other");
});

test("群或消息不存在时返回 ignored，DOM 与内存都不动", () => {
  const { document } = makePage();
  const snapshot = makeSnapshot();
  const result = applyGroupMessageStreamEvent(
    snapshot,
    { groupId: "g1", msgId: "nope", content: "new text" },
    document,
    { isGroupPageActive: true, memberAliases: [], renderText: RENDER_TEXT },
  );
  assert.equal(result, "ignored");
  assert.equal(
    document.querySelector('[data-msg-text-id="m1"]').innerHTML,
    "old",
  );
  assert.equal(snapshot.groupChats[0].messages[0].content, "old");
});

test("不在群聊页时只同步内存，不动 DOM", () => {
  const { document } = makePage();
  const snapshot = makeSnapshot();
  const result = applyGroupMessageStreamEvent(
    snapshot,
    { groupId: "g1", msgId: "m1", content: "new text" },
    document,
    { isGroupPageActive: false, memberAliases: [], renderText: RENDER_TEXT },
  );
  assert.equal(result, "applied");
  assert.equal(
    document.querySelector('[data-msg-text-id="m1"]').innerHTML,
    "old",
  );
  assert.equal(snapshot.groupChats[0].messages[0].content, "new text");
});

test("距底 ≤64px 时贴底滚动，距底更远时不打断阅读", () => {
  const { document } = makePage();
  const snapshot = makeSnapshot();
  const container = document.querySelector(".group-messages");
  const getTop = stubScrollMetrics(container, {
    scrollHeight: 1000,
    clientHeight: 400,
    scrollTop: 550, // 距底 50px
  });
  applyGroupMessageStreamEvent(
    snapshot,
    { groupId: "g1", msgId: "m1", content: "new text" },
    document,
    { isGroupPageActive: true, memberAliases: [], renderText: RENDER_TEXT },
  );
  assert.equal(getTop(), 1000);

  const snapshot2 = makeSnapshot();
  const getTop2 = stubScrollMetrics(container, {
    scrollHeight: 1000,
    clientHeight: 400,
    scrollTop: 0, // 距底 600px：用户在翻历史
  });
  applyGroupMessageStreamEvent(
    snapshot2,
    { groupId: "g1", msgId: "m1", content: "newer text" },
    document,
    { isGroupPageActive: true, memberAliases: [], renderText: RENDER_TEXT },
  );
  assert.equal(getTop2(), 0);
});
