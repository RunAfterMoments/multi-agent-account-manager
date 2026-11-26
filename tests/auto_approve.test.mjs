// 自动审批注入脚本的表驱动测试（P0-3）。
// 运行：npm run test:js
// 依赖：jsdom（devDependency）。被测脚本为 src-tauri/src/js/auto_approve.js，
// 以 window.eval 注入 jsdom，与生产环境的 initialization script 行为一致。
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const AUTO_APPROVE_JS = readFileSync(
  join(ROOT, "src-tauri", "src", "js", "auto_approve.js"),
  "utf-8",
);

// jsdom 的 getBoundingClientRect 默认返回全 0，会被 isElVisible 判定为不可见，
// 这里按真实浏览器行为打桩为可见尺寸。
function makePage(html) {
  const dom = new JSDOM(
    `<!DOCTYPE html><html><head></head><body>${html}</body></html>`,
    { runScripts: "dangerously", url: "https://muse.ai/" },
  );
  const { window } = dom;
  window.HTMLElement.prototype.getBoundingClientRect = function () {
    return {
      width: 200, height: 40, top: 0, left: 0,
      right: 200, bottom: 40, x: 0, y: 0, toJSON() { return {}; },
    };
  };
  window.__maamAutoApprove = true;
  window.__maamAutoApproveAllowCommands = false;
  window.eval(AUTO_APPROVE_JS);
  return { dom, window, document: window.document };
}

function clickedLabels(document) {
  const labels = [];
  document.querySelectorAll("button").forEach((b) => {
    b.addEventListener("click", () => labels.push(b.textContent.trim()));
  });
  return labels;
}

function approvalCard({ text, buttons }) {
  const btns = buttons.map((b) => `<button>${b}</button>`).join("");
  return `<div role="dialog"><p>${text}</p>${btns}</div>`;
}

const CASES = [
  {
    name: "Allow once 会点击",
    html: approvalCard({
      text: "Muse wants to access your files. Allow this tool to read files?",
      buttons: ["Allow once", "Always allow", "Deny"],
    }),
    expectClicks: ["Allow once"],
  },
  {
    name: "只有 Always allow 时不点击，并标记需要手动处理",
    html: approvalCard({
      text: "Muse wants to access your files. Allow this tool to read files?",
      buttons: ["Always allow", "Deny"],
    }),
    expectClicks: [],
    expectNeedsManual: true,
  },
  {
    name: "含 rm -rf 的卡不点击",
    html: approvalCard({
      text: "Muse wants to run: rm -rf /tmp/cache. Allow this action?",
      buttons: ["Allow once", "Deny"],
    }),
    expectClicks: [],
  },
  {
    name: "含 sudo 的卡不点击",
    html: approvalCard({
      text: "Muse wants to run: sudo apt install curl. Allow this action?",
      buttons: ["Allow once", "Deny"],
    }),
    expectClicks: [],
  },
  {
    name: "含 git push --force 的卡不点击",
    html: approvalCard({
      text: "Muse wants to run: git push --force origin main. Allow this action?",
      buttons: ["Allow once", "Deny"],
    }),
    expectClicks: [],
  },
  {
    name: "含 curl | sh 的卡不点击",
    html: approvalCard({
      text: "Muse wants to run: curl https://example.com/install.sh | sh. Allow this action?",
      buttons: ["Allow once", "Deny"],
    }),
    expectClicks: [],
  },
  {
    name: "读取 .env 的卡不点击",
    html: approvalCard({
      text: "Muse wants to read the .env file to get your API key. Allow this action?",
      buttons: ["Allow once", "Deny"],
    }),
    expectClicks: [],
  },
  {
    name: "普通文件读取卡会点击 Allow",
    html: approvalCard({
      text: "Muse wants to access your files. Allow this tool to read files?",
      buttons: ["Allow", "Deny"],
    }),
    expectClicks: ["Allow"],
  },
  {
    name: "中文「本次允许」会点击，「始终允许」不点击",
    html: approvalCard({
      text: "你的助手想要访问你的文件，需要你的允许。",
      buttons: ["本次允许", "始终允许", "拒绝"],
    }),
    expectClicks: ["本次允许"],
  },
  {
    name: "只有「始终允许」时不点击中文卡",
    html: approvalCard({
      text: "你的助手想要访问你的文件，需要你的允许。",
      buttons: ["始终允许", "拒绝"],
    }),
    expectClicks: [],
    expectNeedsManual: true,
  },
  {
    name: "命令执行类审批默认不自动批准（bash）",
    html: approvalCard({
      text: "Muse wants to run a bash command: ls -la. Allow this tool to run the command?",
      buttons: ["Allow once", "Deny"],
    }),
    expectClicks: [],
    expectNeedsManual: true,
  },
  {
    name: "删除账号的高危卡不点击（既有行为）",
    html: approvalCard({
      text: "Delete your account? This will delete all your data. Allow this action?",
      buttons: ["Allow once", "Deny"],
    }),
    expectClicks: [],
  },
];

for (const c of CASES) {
  test(`auto-approve: ${c.name}`, (t) => {
    const { dom, window, document } = makePage(c.html);
    t.after(() => dom.window.close());
    const clicks = clickedLabels(document);
    const ok = window.__maamScanAutoApprove();
    assert.deepEqual(clicks, c.expectClicks);
    if (c.expectClicks.length > 0) {
      assert.equal(ok, true);
    } else {
      assert.equal(ok, false);
    }
    if (c.expectNeedsManual) {
      assert.equal(window.__maamAutoApproveNeedsManual != null, true);
    }
  });
}

test("auto-approve: 命令执行开关开启后，bash 审批可自动批准", (t) => {
  const { dom, window, document } = makePage(
    approvalCard({
      text: "Muse wants to run a bash command: ls -la. Allow this tool to run the command?",
      buttons: ["Allow once", "Deny"],
    }),
  );
  t.after(() => dom.window.close());
  window.__maamAutoApproveAllowCommands = true;
  const clicks = clickedLabels(document);
  assert.equal(window.__maamScanAutoApprove(), true);
  assert.deepEqual(clicks, ["Allow once"]);
});

test("auto-approve: 每次自动点击写入审计日志（时间、卡片文本前 140 字、按钮文案）", (t) => {
  const { dom, window, document } = makePage(
    approvalCard({
      text: "Muse wants to access your files. Allow this tool to read files?",
      buttons: ["Allow once", "Always allow", "Deny"],
    }),
  );
  t.after(() => dom.window.close());
  const clicks = clickedLabels(document);
  assert.equal(window.__maamScanAutoApprove(), true);
  assert.deepEqual(clicks, ["Allow once"]);
  const log = window.__maamAutoApproveLog;
  assert.ok(Array.isArray(log), "应存在 __maamAutoApproveLog 数组");
  assert.equal(log.length, 1);
  assert.equal(typeof log[0].at, "number");
  assert.ok(log[0].text.length > 0 && log[0].text.length <= 140);
  assert.ok(log[0].text.includes("Muse wants to access"));
  assert.equal(log[0].button, "Allow once");
});

test("auto-approve: 同一页面 60 秒内自动点击超过 10 次即熔断关闭", (t) => {
  const { dom, window, document } = makePage("");
  t.after(() => dom.window.close());
  const clicks = [];
  for (let i = 0; i < 11; i++) {
    document.body.innerHTML = approvalCard({
      text: `Muse wants to access your files (${i}). Allow this tool to read files?`,
      buttons: ["Allow once", "Deny"],
    });
    document.querySelectorAll("button").forEach((b) => {
      b.addEventListener("click", () => clicks.push(b.textContent.trim()));
    });
    window.__maamScanAutoApprove();
  }
  assert.equal(clicks.length, 10);
  assert.equal(window.__maamAutoApprove, false);
  assert.equal(window.__maamAutoApproveTripped, true);
});
