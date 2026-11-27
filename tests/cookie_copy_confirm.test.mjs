// Cookie 复制两段确认的前端测试（v6.5 MAAM-M-01）。
// 运行：npm run test:js
// 被测函数为 src/cookie-copy.ts 的 createCookieCopyConfirm（零运行时依赖，
// 与 group-stream.ts 同一模式，Node 类型擦除可直接导入）。
import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import {
  createCookieCopyConfirm,
  COOKIE_COPY_CONFIRM_MS,
  COOKIE_COPY_LABEL,
  COOKIE_COPY_CONFIRM_LABEL,
} from "../src/cookie-copy.ts";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function makeButton() {
  const dom = new JSDOM(`<!DOCTYPE html><html><head></head><body></body></html>`, {
    url: "https://example.invalid/",
  });
  const document = dom.window.document;
  const button = document.createElement("button");
  button.className = "action-icon-btn copy-cookie-btn";
  button.dataset.copyLabel = COOKIE_COPY_LABEL;
  button.setAttribute("aria-label", COOKIE_COPY_LABEL);
  document.body.append(button);
  return { dom, document, button };
}

test("首次点击只进入确认态：不复制，只变文案与样式", () => {
  const { button } = makeButton();
  let copied = 0;
  const confirm = createCookieCopyConfirm(button);
  const shouldCopy = confirm.click();
  if (shouldCopy) copied += 1;
  assert.equal(shouldCopy, false);
  assert.equal(copied, 0, "首次点击不得触发复制");
  assert.equal(button.dataset.copyLabel, COOKIE_COPY_CONFIRM_LABEL);
  assert.equal(button.dataset.copyConfirm, "1");
  assert.equal(button.classList.contains("confirming"), true);
  assert.equal(button.getAttribute("aria-label"), "确认复制 Cookie？");
  button.remove();
});

test("确认窗内二次点击才复制", () => {
  const { button } = makeButton();
  let copied = 0;
  const confirm = createCookieCopyConfirm(button, 1000);
  assert.equal(confirm.click(), false);
  const shouldCopy = confirm.click();
  if (shouldCopy) copied += 1;
  assert.equal(shouldCopy, true);
  assert.equal(copied, 1);
  assert.equal(button.dataset.copyConfirm, undefined);
  assert.equal(button.classList.contains("confirming"), false);
  button.remove();
});

test("超时后点击重新进入确认态，不直接复制", async () => {
  const { button } = makeButton();
  let copied = 0;
  const confirm = createCookieCopyConfirm(button, 30);
  assert.equal(confirm.click(), false);
  await sleep(80);
  assert.equal(button.dataset.copyLabel, COOKIE_COPY_LABEL);
  assert.equal(button.dataset.copyConfirm, undefined);
  assert.equal(button.classList.contains("confirming"), false);
  const shouldCopy = confirm.click();
  if (shouldCopy) copied += 1;
  assert.equal(shouldCopy, false, "超时复原后下一次点击应重新进入确认态");
  assert.equal(copied, 0);
  button.remove();
});

test("按钮被重渲染移除后，超时回调不操作已不存在的 DOM", async () => {
  const { document, button } = makeButton();
  const confirm = createCookieCopyConfirm(button, 30);
  assert.equal(confirm.click(), false);
  button.remove(); // 模拟 rerender 的 innerHTML 重建
  await sleep(80); // 超时回调应因 isConnected 为 false 直接返回
  assert.equal(button.isConnected, false);
  assert.equal(button.dataset.copyLabel, COOKIE_COPY_CONFIRM_LABEL);
  assert.equal(document.querySelector(".copy-cookie-btn.confirming"), null);
  confirm.dispose();
});

test("dispose 立即取消确认态并清理 timer", async () => {
  const { button } = makeButton();
  const confirm = createCookieCopyConfirm(button, 30);
  assert.equal(confirm.click(), false);
  confirm.dispose();
  assert.equal(button.dataset.copyLabel, COOKIE_COPY_LABEL);
  assert.equal(button.dataset.copyConfirm, undefined);
  assert.equal(button.classList.contains("confirming"), false);
  await sleep(80);
  assert.equal(button.dataset.copyLabel, COOKIE_COPY_LABEL);
  button.remove();
});

test("默认确认窗为 3000ms", () => {
  assert.equal(COOKIE_COPY_CONFIRM_MS, 3000);
});
