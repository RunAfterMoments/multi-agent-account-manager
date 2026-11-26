// 自动审批注入脚本：扫描页面上的权限审批卡，自动点击授权按钮。
// 由 Rust 侧以 include_str! 引入，作为独立 initialization script 注册到账号 webview。
// 注意：本文件在目标站点的页面上下文中运行，只能访问 window/document，
// 不能直接调用 Tauri 命令；审计日志与熔断状态通过 window.__maamAutoApproveReport()
// 由 Rust 侧在静默同步时读回（见 session_manager.rs drain_auto_approve_report）。
// 行为约束（P0-3）：只点击 SINGLE_APPROVE_BTN_RE 命中的单次授权按钮，
// 没有单次按钮（只剩「始终/总是/always」类）时不点击，并置
// window.__maamAutoApproveNeedsManual（reason 为 'always_only'），
// 由 Rust 侧读回后向 UI 发「需要手动处理」提示。
// 每次自动点击追加审计条目到 window.__maamAutoApproveLog；
// 60 秒内自动点击超过 10 次即熔断关闭（window.__maamAutoApproveTripped）。
(() => {
  try {
if (!window.__maamAutoApproveWatcher) {
  window.__maamAutoApproveWatcher = true;
  const DANGER_RE = /删除|移除|注销|清空|退出登录|退出账户|确认退出|支付|付款|购买|订阅|充值|转账|密码|更改邮箱|停用|封禁|删除文件|删除数据|清空数据|格式化|恢复出厂|清除数据|密钥|口令|凭证|令牌|私钥|环境变量|\.env\b|id_rsa|id_ed25519|\.pem\b|api[-_ ]?key|access[-_ ]?token|secret|credential|\.ssh\b|\/etc\/passwd|\/etc\/shadow|\bdelete\b|\bremove\b|\berase\b|\bwipe\b|logout|log out|sign out|payment|purchase|billing|checkout|subscribe|charge|transfer|password|deactivate|reset account|factory reset|rm\s+(?:-\w*r\w*|--recursive)|\bsudo\b|\bchmod\b|\bchown\b|\bkill(?:all)?\b|\bpkill\b|\bgit\s+push\b[^|]{0,80}--force|\bgit\s+reset\s+--hard|\bcurl\b[^|]{0,200}\|\s*(?:sh|bash)|\bwget\b[^|]{0,200}\|\s*(?:sh|bash)|\bdrop\s+table|\bdrop\s+database|\bmkfs\b|\bdd\b|\bfdisk\b|\bshutdown\b|\breboot\b/i;
  const PROMPT_RE = /授权|审批|权限|批准|请求允许|请求执行|需要你的允许|需要您的允许|需要确认|是否(?:允许|批准|授权|同意)|要允许|你的助手想要|助手想要|想(?:要|使用|访问|运行|执行|打开|上传|下载|读取|写入|查看|获取|调用|连接|发送)|允许.{0,24}(?:此操作|此次|使用|运行|执行|访问|操作|工具|命令|浏览|上传|下载|读取|写入|修改|打开|连接|发送|调用|查看|获取)|approve|approval|authorize|permission|wants to|would like to|asks? to|needs approval|allow\s+.{0,32}\bto\b|allow\s+.{0,24}(?:this|use|run|execute|access|tool|command|browser|upload|download|read|write|open|connect|send)/i;
  // 命令执行类审批卡：命中时默认不自动批准，除非用户在设置里单独开启
  // 「允许自动批准命令执行」（window.__maamAutoApproveAllowCommands === true）。
  const COMMAND_RE = /执行命令|运行命令|执行\s*shell|运行\s*shell|shell\s*命令|bash\s*命令|终端命令|命令行|执行终端|运行终端|execute\s+(?:a\s+)?command|run\s+(?:a\s+)?(?:shell\s+)?command|\bshell\b|\bbash\b|\bterminal\b/i;
  const SINGLE_APPROVE_BTN_RE = /^(?:允许(?!.*(?:始终|总是))[^，。！？]{0,10}|本次允许|仅允许一次|批准(?!.*(?:始终|总是))[^，。！？]{0,10}|同意(?:并继续)?|确认授权|确认允许|继续执行|allow(?!\s+always)(?:\s+(?:once|this\s+time|this\s+\w+|access|upload|download|run|execute|use))?|approve(?:\s+(?:once|this\s+\w+))?|authorize|grant)$/i;
  const APPROVE_BTN_RE = /^(?:(?:允许|本次允许|仅允许一次|始终允许|总是允许|批准|始终批准)[^，。！？]{0,18}|同意(?:并继续)?|确认授权|确认允许|继续执行|(?:allow|always\s+allow|approve|always\s+approve|authorize|grant)(?:\s+[\w\s'-]{1,28})?)$/i;
  const REJECT_BTN_RE = /^(?:(?:拒绝|不允许|不批准)[^，。！？]{0,12}|取消|暂不|以后再说|(?:deny|decline|reject|don't allow|do not allow)(?:\s+[\w\s'-]{1,20})?|cancel|block|never|not now)$/i;
  const MODAL_BOX_SEL = '[role="dialog"], [role="alertdialog"], [aria-modal="true"], dialog, [class*="permission" i], [class*="approval" i], [class*="approve" i], [class*="authorize" i], [class*="consent" i], [data-testid*="approv" i], [data-testid*="permiss" i]';
  const clickedAt = new WeakMap();
  // 审计日志与熔断：每次自动点击追加一条 { at, text, button }（text 为卡片
  // 文本前 140 字），页内最多保留最近 50 条；60 秒内自动点击超过 10 次
  // 即熔断：本页立即停用自动审批，并置 tripped 标志，由 Rust 侧读回后
  // 持久化关闭总开关并向 UI 提示。同一账号对应独立 webview，计数按页隔离。
  const AUTO_APPROVE_BREAKER_LIMIT = 10;
  const AUTO_APPROVE_BREAKER_WINDOW_MS = 60000;
  const AUTO_APPROVE_LOG_CAP = 50;
  window.__maamAutoApproveLog = window.__maamAutoApproveLog || [];
  window.__maamAutoApproveTripped = window.__maamAutoApproveTripped || false;
  window.__maamAutoApproveTrippedAt = window.__maamAutoApproveTrippedAt || null;
  const clickTimes = [];
  let autoApproveObserver = null;
  let approveInterval = null;
  // 仅有「始终/总是/always」类按钮、或命令执行类审批被开关拦下时，
  // 不点击，并把卡片留给 Rust 侧读回后向 UI 发「需要手动处理」提示。
  // 同一张卡 60 秒内只记录一次，避免反复刷提示。
  window.__maamAutoApproveNeedsManual = window.__maamAutoApproveNeedsManual || null;
  const setNeedsManual = (text, reason) => {
    const entry = { at: Date.now(), text: (text || '').slice(0, 140), reason };
    const prev = window.__maamAutoApproveNeedsManual;
    if (prev && prev.text === entry.text && prev.reason === reason && entry.at - prev.at < 60000) return;
    window.__maamAutoApproveNeedsManual = entry;
  };
  const getBtnLabel = (b) => ((b.innerText || b.textContent || b.getAttribute('aria-label') || '') + '').replace(/\s+/g, ' ').trim();
  const isElVisible = (el) => {
    if (!el || !el.isConnected) return false;
    const r = el.getBoundingClientRect();
    if (r.width < 16 || r.height < 14) return false;
    let cur = el;
    for (let d = 0; cur && cur.nodeType === 1 && d < 8; d++) {
      try {
        const st = getComputedStyle(cur);
        if (st.display === 'none' || st.visibility === 'hidden') return false;
        const op = parseFloat(st.opacity);
        if (!Number.isNaN(op) && op < 0.05) return false;
      } catch (_) {}
      cur = cur.parentElement;
    }
    return true;
  };
  const isActionableBtn = (b) => {
    if (!isElVisible(b) || b.disabled || b.getAttribute('aria-disabled') === 'true') return false;
    if (b.closest && b.closest('nav, aside')) return false;
    return true;
  };
  const findApprovalTarget = () => {
    try {
      const allBtns = document.querySelectorAll('button, [role="button"]');
      for (const b of allBtns) {
        if (!isActionableBtn(b)) continue;
        const lbl = getBtnLabel(b);
        if (!APPROVE_BTN_RE.test(lbl) || REJECT_BTN_RE.test(lbl) || DANGER_RE.test(lbl)) continue;
        let cur = b.parentElement;
        for (let step = 0; step < 7 && cur && cur !== document.body && cur !== document.documentElement; step++) {
          if (cur.tagName === 'NAV' || cur.tagName === 'ASIDE') break;
          let isModal = false;
          try { isModal = typeof cur.matches === 'function' && cur.matches(MODAL_BOX_SEL); } catch (_) {}
          if (!isModal && cur.querySelector && cur.querySelector('textarea, [contenteditable="true"]')) break;
          const txt = (cur.innerText || cur.textContent || '').trim();
          if (txt.length > 2200) break;
          if (txt.length >= 6) {
            if (DANGER_RE.test(txt)) break;
            if (PROMPT_RE.test(txt)) {
              // 命令执行类审批默认不自动批准：未单独开启开关时跳过本卡，
              // 标记需要手动处理，继续看页面上有没有其他可批的卡。
              // 门控只在 PROMPT_RE 命中的审批卡内生效，避免聊天记录里
              // 提到 shell/bash 就误拦无关卡片。
              if (COMMAND_RE.test(txt) && window.__maamAutoApproveAllowCommands !== true) {
                setNeedsManual(txt, 'command');
                break;
              }
              const boxBtns = Array.from(cur.querySelectorAll('button, [role="button"]')).filter(isActionableBtn);
              const hasReject = boxBtns.some((x) => REJECT_BTN_RE.test(getBtnLabel(x)));
              if (hasReject || isModal) {
                const approveBtns = boxBtns.filter((x) => {
                  const xLbl = getBtnLabel(x);
                  return APPROVE_BTN_RE.test(xLbl) && !REJECT_BTN_RE.test(xLbl) && !DANGER_RE.test(xLbl);
                });
                if (approveBtns.length > 0) {
                  // 只允许点击 SINGLE_APPROVE_BTN_RE 命中的单次授权按钮；
                  // 没有单次按钮（只剩「始终允许」类）时绝不回退点击第一个，
                  // 而是标记需要手动处理，等用户自己点。
                  const picked = approveBtns.find((x) => SINGLE_APPROVE_BTN_RE.test(getBtnLabel(x)));
                  if (!picked) {
                    setNeedsManual(txt, 'always_only');
                    return null;
                  }
                  return { box: cur, button: picked, text: txt };
                }
              }
            }
          }
          cur = cur.parentElement;
        }
      }
    } catch (_) {}
    return null;
  };
  window.__maamFindAuthPrompt = () => {
    const target = findApprovalTarget();
    return target ? target.text.slice(0, 140) : '';
  };
  window.__maamScanAutoApprove = () => {
    if (window.__maamAutoApprove !== true) return false;
    const target = findApprovalTarget();
    if (!target) return false;
    const now = Date.now();
    const last = clickedAt.get(target.button) || 0;
    if (now - last < 4000) return false;
    // 熔断：60 秒窗口内已自动点击 10 次，第 11 次起不再点击，
    // 立即停用本页自动审批，等 Rust 侧读回后持久化关闭总开关。
    while (clickTimes.length > 0 && now - clickTimes[0] > AUTO_APPROVE_BREAKER_WINDOW_MS) clickTimes.shift();
    if (clickTimes.length >= AUTO_APPROVE_BREAKER_LIMIT) {
      window.__maamAutoApprove = false;
      window.__maamAutoApproveTripped = true;
      window.__maamAutoApproveTrippedAt = now;
      return false;
    }
    clickedAt.set(target.button, now);
    clickTimes.push(now);
    window.__maamAutoApproveLog.push({
      at: now,
      text: target.text.slice(0, 140),
      button: getBtnLabel(target.button),
    });
    if (window.__maamAutoApproveLog.length > AUTO_APPROVE_LOG_CAP) {
      window.__maamAutoApproveLog.splice(0, window.__maamAutoApproveLog.length - AUTO_APPROVE_LOG_CAP);
    }
    target.button.click();
    return true;
  };
  // 供 Rust 侧静默同步时读回（drain 语义：读走即清，避免重复提示/重复记账）。
  // 返回 JSON 字符串，与仓库内其他 eval_json_callback 调用保持一致。
  window.__maamAutoApproveReport = () => {
    const report = {
      entries: window.__maamAutoApproveLog.splice(0, window.__maamAutoApproveLog.length),
      tripped: window.__maamAutoApproveTripped === true,
      trippedAt: window.__maamAutoApproveTrippedAt || null,
      needsManual: window.__maamAutoApproveNeedsManual,
    };
    window.__maamAutoApproveTripped = false;
    window.__maamAutoApproveTrippedAt = null;
    window.__maamAutoApproveNeedsManual = null;
    return JSON.stringify(report);
  };
  // 仅供 jsdom 测试清理定时器/观察器；生产环境页面卸载时由 webview 回收。
  window.__maamAutoApproveDestroy = () => {
    try { if (approveTimer) clearTimeout(approveTimer); } catch (_) {}
    try { if (approveInterval) clearInterval(approveInterval); } catch (_) {}
    try { if (autoApproveObserver) autoApproveObserver.disconnect(); } catch (_) {}
  };
  let approveTimer = null;
  const scheduleAutoApproveScan = () => {
    if (window.__maamAutoApprove !== true || approveTimer) return;
    approveTimer = setTimeout(() => {
      approveTimer = null;
      window.__maamScanAutoApprove();
    }, 250);
  };
  const startObserver = () => {
    try {
      if (!document.documentElement) return;
      const obs = new MutationObserver(() => scheduleAutoApproveScan());
      autoApproveObserver = obs;
      obs.observe(document.documentElement, { childList: true, subtree: true });
    } catch (_) {}
  };
  if (document.documentElement) {
    startObserver();
  } else {
    document.addEventListener('DOMContentLoaded', startObserver, { once: true });
  }
  approveInterval = setInterval(() => {
    if (window.__maamAutoApprove === true) {
      window.__maamScanAutoApprove();
    }
  }, 1200);
}
  } catch (_) {}
})();
