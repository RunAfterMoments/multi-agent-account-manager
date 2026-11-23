use crate::platform::{self, current_platform_name, ProxyTestResult};
use crate::providers::{
  all_provider_infos, classify_top_level_url, is_verified_callback_origin,
  normalize_custom_provider_input, resolve_provider, CustomProviderRecord, NavigationDecision,
  ProviderId, ProviderInfo, ProviderSpec,
};
use crate::storage::{
  detect_conflicts, load_or_migrate_store, normalize_uuid, now_ms, save_store_atomic, AccountKey,
  AccountRecord, AccountsStoreFile, AppSettings, AuthState, ConflictItem, QuotaDetail,
};
use serde::Serialize;
#[cfg(desktop)]
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

pub const APP_TITLE: &str = "叠号";
#[cfg(desktop)]
const LEFT_PANE_WIDTH: f64 = 380.0;
#[cfg(desktop)]
const MAX_WARM_DESKTOP_VIEWS: usize = 6;
#[cfg(desktop)]
const MAX_CONCURRENT_PREWARMS: usize = 2;

#[cfg(desktop)]
const LOGIN_CAPTURE_INIT_JS: &str = r#"(() => {
  try {
    if (location.protocol !== 'https:') return;
    if (window !== window.top) return;
    const initHost = (location.hostname || '').toLowerCase();
    const isPrimaryHost = initHost === 'muse.ai' || initHost.endsWith('.muse.ai') || initHost === 'app.hark.com' || initHost.endsWith('.hark.com');
    const origFetch = window.fetch;
    if (initHost === 'app.hark.com' || initHost.endsWith('.hark.com')) {
      try {
        if (document.documentElement && !document.documentElement.style.backgroundColor) {
          document.documentElement.style.backgroundColor = '#171d2f';
        }
      } catch (_) {}
    }
    const setQuota = (text, pct) => {
      if (!text) return;
      const cleanPct = typeof pct === 'number' && Number.isFinite(pct)
        ? Math.max(0, Math.min(100, Math.round(pct)))
        : null;
      const obj = { text: String(text).slice(0, 48), pct: cleanPct };
      window.__maamQuota = obj;
      try { localStorage.setItem('__maam_cached_quota__', JSON.stringify(obj)); } catch (_) {}
    };
    const saveDetail = (detail) => {
      if (!detail || !detail.planLabel) return;
      window.__maamQuotaDetail = detail;
      try { localStorage.setItem('__maam_cached_quota_detail_v2__', JSON.stringify(detail)); } catch (_) {}
      const rem = typeof detail.planUsedPercent === 'number'
        ? Math.max(0, Math.min(100, 100 - detail.planUsedPercent))
        : null;
      setQuota(detail.planValue || detail.planLabel, rem);
    };
    const formatHarkReset = (iso) => {
      if (!iso || typeof iso !== 'string') return null;
      const d = new Date(iso);
      if (Number.isNaN(d.getTime())) return null;
      const pad = (n) => String(n).padStart(2, '0');
      const hhmm = pad(d.getHours()) + ':' + pad(d.getMinutes());
      const now = new Date();
      const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
      const targetStart = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
      const diffDays = Math.round((targetStart - todayStart) / 86400000);
      if (diffDays === 0) return '今天 ' + hhmm + ' 重置';
      if (diffDays === 1) return '明天 ' + hhmm + ' 重置';
      if (d.getFullYear() === now.getFullYear()) {
        return (d.getMonth() + 1) + '月' + d.getDate() + '日 ' + hhmm + ' 重置';
      }
      return d.getFullYear() + '年' + (d.getMonth() + 1) + '月' + d.getDate() + '日 ' + hhmm + ' 重置';
    };
    const mapHarkSub = (node) => {
      if (!(location.hostname || '').toLowerCase().endsWith('hark.com')) return null;
      if (!node || typeof node !== 'object') return null;
      const meters = (node.meters && typeof node.meters === 'object' ? (node.meters.harkTokens || node.meters) : null)
        || (node.harkTokens && typeof node.harkTokens === 'object' ? node.harkTokens : null)
        || (('dailyLimit' in node || 'poolLimit' in node) ? node : null);
      if (meters && typeof meters === 'object' && (typeof meters.dailyLimit === 'number' || typeof meters.poolLimit === 'number')) {
        const rawPlan = (node.plan && typeof node.plan === 'object' && typeof node.plan.name === 'string' && node.plan.name.trim())
          || (typeof node.planName === 'string' && node.planName.trim())
          || (typeof node.plan === 'string' && node.plan.trim())
          || '';
        const shortPlan = rawPlan.replace(/^hark\s+/i, '').trim();
        const hasDaily = typeof meters.dailyLimit === 'number' && meters.dailyLimit > 0 && typeof meters.dailyUsed === 'number';
        const hasMonthly = typeof meters.poolLimit === 'number' && meters.poolLimit > 0 && typeof meters.poolUsed === 'number';
        if (hasDaily || hasMonthly) {
          const dailyPct = hasDaily ? Math.max(0, Math.min(100, Math.round((meters.dailyUsed / meters.dailyLimit) * 100))) : null;
          const monthlyPct = hasMonthly ? Math.max(0, Math.min(100, Math.round((meters.poolUsed / meters.poolLimit) * 100))) : null;
          if (hasDaily) {
            return {
              planLabel: shortPlan ? (shortPlan + ' · 每日') : '每日上限',
              planValue: '已使用 ' + dailyPct + '%',
              planSubtext: formatHarkReset(meters.dailyResetsAt),
              planUsedPercent: dailyPct,
              extraLabel: hasMonthly ? '每月上限' : null,
              extraValue: hasMonthly ? ('已使用 ' + monthlyPct + '%') : null,
              extraSubtext: hasMonthly ? formatHarkReset(meters.poolResetsAt) : null,
              extraUsedPercent: monthlyPct
            };
          }
          return {
            planLabel: shortPlan ? (shortPlan + ' · 每月') : '每月上限',
            planValue: '已使用 ' + monthlyPct + '%',
            planSubtext: formatHarkReset(meters.poolResetsAt),
            planUsedPercent: monthlyPct,
            extraLabel: null,
            extraValue: null,
            extraSubtext: null,
            extraUsedPercent: null
          };
        }
      }
      const u = node.usage || node.billing || node.credits || node.subscription || node;
      if (!u || typeof u !== 'object' || 'page' in u || 'items' in u || 'messages' in u) return null;
      const rem = u.remaining ?? u.remaining_credits ?? u.remainingCredits ?? u.credits_remaining ?? u.creditsRemaining ?? u.balance ?? u.creditBalance ?? u.left;
      const tot = u.total ?? u.limit ?? u.total_credits ?? u.totalCredits ?? u.max ?? u.allowance;
      const used = u.used ?? u.used_credits ?? u.usedCredits ?? u.consumed;
      const planName = u.plan_name || u.planName || u.plan || node.plan_name || node.planName || node.plan || (typeof node.tier === 'string' ? node.tier : null) || (typeof u.name === 'string' ? u.name : null) || 'Hark 额度';
      if (typeof rem === 'number' || typeof used === 'number') {
        let usedPct = null;
        let planVal = '';
        if (typeof tot === 'number' && tot > 0) {
          const actualUsed = typeof used === 'number' ? used : (tot - (typeof rem === 'number' ? rem : 0));
          usedPct = Math.max(0, Math.min(100, Math.round((actualUsed / tot) * 100)));
          planVal = '已使用 ' + usedPct + '%';
        } else if (typeof rem === 'number') {
          planVal = '剩余 ' + rem;
        }
        const resetsAt = u.resets_at ?? u.resetsAt;
        return {
          planLabel: String(planName),
          planValue: planVal,
          planSubtext: resetsAt ? ('将在 ' + new Date(resetsAt).toLocaleDateString() + ' 重置') : (u.period ? ('周期: ' + u.period) : null),
          planUsedPercent: usedPct,
          extraLabel: null,
          extraValue: null,
          extraSubtext: null,
          extraUsedPercent: null
        };
      }
      return null;
    };
    const mapSub = (node) => {
      if (!node || typeof node !== 'object') return null;
      const harkRes = mapHarkSub(node);
      if (harkRes) return harkRes;
      const s = (node.subscription && typeof node.subscription === 'object' && node.subscription.usage)
        ? node.subscription
        : node;
      const u = s.usage;
      if (!u || typeof u !== 'object') return null;
      if (!('state' in u) && !('percentUsed' in u) && !('percent_used' in u) && !('quotaStatus' in u) && !('quota_status' in u)) {
        return null;
      }
      const subType = s.subscriptionType || s.subscription_type || '';
      const tierName = (s.tier && typeof s.tier.name === 'string' && s.tier.name.trim()) || '';
      const planLabel = (typeof s.usageRowLabel === 'string' && s.usageRowLabel.trim())
        || (typeof s.usage_row_label === 'string' && s.usage_row_label.trim())
        || tierName
        || (subType === 'PAID' ? '订阅版' : subType === 'COMPLIMENTARY' ? '赠送版' : subType === 'FREE' ? '免费版' : '基础额度');
      const rawPct = typeof u.percentUsed === 'number' ? u.percentUsed : u.percent_used;
      const qStatus = u.quotaStatus || u.quota_status || '';
      let planUsedPercent = null;
      if (u.state !== 'UNLIMITED' && typeof rawPct === 'number' && Number.isFinite(rawPct)) {
        planUsedPercent = Math.max(0, Math.min(100, Math.round(rawPct)));
      } else if (qStatus === 'DEPLETED') {
        planUsedPercent = 100;
      }
      const rawVal = (typeof s.usageRowValueLabel === 'string' && s.usageRowValueLabel.trim())
        || (typeof s.usage_row_value_label === 'string' && s.usage_row_value_label.trim())
        || '';
      const planValue = rawVal
        || (u.state === 'UNLIMITED' ? '无限额度'
          : planUsedPercent !== null ? ('已使用 ' + planUsedPercent + '%')
          : qStatus === 'DEPLETED' ? '已用尽'
          : qStatus === 'LOW' ? '额度偏低'
          : '额度充足');
      let planSubtext = (typeof s.statusSubtitle === 'string' && s.statusSubtitle.trim())
        || (typeof s.status_subtitle === 'string' && s.status_subtitle.trim())
        || (typeof s.usageRowFooter === 'string' && s.usageRowFooter.trim())
        || (typeof s.usage_row_footer === 'string' && s.usage_row_footer.trim())
        || null;
      if (!planSubtext) {
        const rawReset = u.resetsAt ?? u.resets_at;
        if (typeof rawReset === 'number' && rawReset > 1000000000) {
          const ms = rawReset > 100000000000 ? rawReset : rawReset * 1000;
          const d = new Date(ms);
          if (!Number.isNaN(d.getTime())) {
            planSubtext = '将在 ' + (d.getMonth() + 1) + '月' + d.getDate() + '日重置';
          }
        }
      }
      const topupVal = (typeof s.topupRowValueLabel === 'string' && s.topupRowValueLabel.trim())
        || (typeof s.topup_row_value_label === 'string' && s.topup_row_value_label.trim())
        || '';
      const topupBal = typeof s.topupBalance === 'number' ? s.topupBalance : s.topup_balance;
      const topupTot = typeof s.topupTotal === 'number' ? s.topupTotal : s.topup_total;
      let extraLabel = null;
      let extraValue = null;
      let extraSubtext = null;
      let extraUsedPercent = null;
      if (topupVal || (typeof topupTot === 'number' && topupTot > 0)) {
        extraLabel = (typeof s.topupRowLabel === 'string' && s.topupRowLabel.trim())
          || (typeof s.topup_row_label === 'string' && s.topup_row_label.trim())
          || '额外的使用额度';
        if (typeof topupTot === 'number' && topupTot > 0 && typeof topupBal === 'number') {
          extraUsedPercent = Math.max(0, Math.min(100, Math.round(((topupTot - topupBal) / topupTot) * 100)));
        }
        extraValue = topupVal || (extraUsedPercent !== null ? ('已使用 ' + extraUsedPercent + '%') : null);
        extraSubtext = '从不过期';
      }
      return {
        planLabel,
        planValue,
        planSubtext,
        planUsedPercent,
        extraLabel,
        extraValue,
        extraSubtext,
        extraUsedPercent
      };
    };
    window.__maamMapSub = mapSub;
    window.__maamSaveDetail = saveDetail;

    const scanTpChunk = (chunk) => {
      if (!Array.isArray(chunk)) return;
      for (let i = 0; i < chunk.length; i++) {
        const item = chunk[i];
        if (Array.isArray(item)) {
          scanTpChunk(item);
          continue;
        }
        if (typeof item === 'function') {
          let src = '';
          try { src = Function.prototype.toString.call(item); } catch (_) { continue; }
          if (src.includes('fetchSubscriptionAction')) {
            const mAct = src.match(/createServerReference\)\("([0-9a-f]{32,64})"[^)]*"fetchSubscriptionAction"\)/);
            if (mAct) window.__maamSubActionId = mAct[1];
            if (i > 0 && typeof chunk[i - 1] === 'number') {
              window.__maamSubModId = chunk[i - 1];
            }
          }
          if (src.includes('preloadHatchSettingsData')) {
            const mPre = src.match(/\.A\((\d+)\)\.then\(\(\{preloadHatchSettingsData\b/);
            if (mPre) window.__maamPreloadModId = parseInt(mPre[1], 10);
          }
        }
      }
    };

    if ((location.hostname || '').endsWith('muse.ai')) {
      try {
        const tpArr = Array.isArray(globalThis.TURBOPACK) ? globalThis.TURBOPACK : [];
        tpArr.forEach(scanTpChunk);
        const origArrPush = Array.prototype.push;
        tpArr.push = function(...items) {
          items.forEach(scanTpChunk);
          return origArrPush.apply(this, items);
        };
        let tpRuntime = (!Array.isArray(globalThis.TURBOPACK) && globalThis.TURBOPACK) ? globalThis.TURBOPACK : null;
        if (tpRuntime && typeof tpRuntime.push === 'function') {
          const origObjPush = tpRuntime.push.bind(tpRuntime);
          tpRuntime.push = function(...items) {
            items.forEach(scanTpChunk);
            return origObjPush(...items);
          };
        } else {
          Object.defineProperty(globalThis, 'TURBOPACK', {
            configurable: true,
            enumerable: true,
            get() {
              return tpRuntime || tpArr;
            },
            set(val) {
              if (val && typeof val === 'object' && !Array.isArray(val) && typeof val.push === 'function') {
                const origPush = val.push.bind(val);
                val.push = function(...items) {
                  items.forEach(scanTpChunk);
                  return origPush(...items);
                };
                tpRuntime = val;
              } else {
                tpRuntime = val;
              }
            }
          });
        }
      } catch (_) {}
    }

    window.__maamRefreshBackendQuota = async (force) => {
      if (window.__maamQuotaInflight) return;
      const now = Date.now();
      if (!force && window.__maamQuotaFetchedAt && now - window.__maamQuotaFetchedAt < 90000) return;
      const curHost = (location.hostname || '').toLowerCase();
      if (curHost === 'app.hark.com' || curHost.endsWith('.hark.com')) {
        window.__maamQuotaInflight = true;
        try {
          const fetchFn = typeof origFetch === 'function' ? origFetch : window.fetch;
          const res = await fetchFn.call(window, '/api/billing/summary', {
            credentials: 'include',
            headers: { Accept: 'application/json' }
          });
          if (res && res.ok) {
            const data = await res.json();
            const mapped = mapHarkSub(data);
            if (mapped) {
              window.__maamQuotaFetchedAt = Date.now();
              saveDetail(mapped);
            }
          }
          if (!window.__maamApiEmail) {
            try {
              const profRes = await fetchFn.call(window, '/api/profile', {
                credentials: 'include',
                headers: { Accept: 'application/json' }
              });
              if (profRes && profRes.ok) {
                const prof = await profRes.json();
                if (prof && typeof prof.email === 'string' && prof.email.includes('@')) {
                  window.__maamApiEmail = prof.email.trim().toLowerCase();
                }
              }
            } catch (_) {}
          }
        } catch (_) {
        } finally {
          window.__maamQuotaInflight = false;
        }
        return;
      }
      const tpGlobal = globalThis.TURBOPACK;
      if (!tpGlobal || Array.isArray(tpGlobal) || typeof tpGlobal.push !== 'function') return;
      window.__maamQuotaInflight = true;
      try {
        if (!window.__maamTp) {
          tpGlobal.push(['__maam_init__', 999999991, (ctx) => { window.__maamTp = ctx; }]);
          tpGlobal.push(['__maam_run__', { otherChunks: [], runtimeModuleIds: [999999991] }]);
        }
        const tp = window.__maamTp;
        if (!tp) return;
        if (!window.__maamSubModId && !window.__maamPreloadModId && !window.__maamScriptsScanned) {
          window.__maamScriptsScanned = true;
          const scripts = Array.from(document.querySelectorAll('script[src*="/_next/static/"]'));
          for (const sc of scripts) {
            if (!sc.src) continue;
            try {
              const txt = await origFetch.call(window, sc.src, { cache: 'force-cache' }).then((r) => r.text());
              const mPre = txt.match(/\.A\((\d+)\)\.then\(\(\{preloadHatchSettingsData\b/);
              if (mPre) window.__maamPreloadModId = parseInt(mPre[1], 10);
              const mAct = txt.match(/createServerReference\)\("([0-9a-f]{32,64})"[^)]*"fetchSubscriptionAction"\)/);
              if (mAct) window.__maamSubActionId = mAct[1];
              if (window.__maamPreloadModId && window.__maamSubActionId) break;
            } catch (_) {}
          }
        }
        if (!window.__maamSubModId && window.__maamPreloadModId && typeof tp.A === 'function') {
          await tp.A(window.__maamPreloadModId).catch(() => {});
        }
        if (window.__maamSubModId && typeof tp.i === 'function') {
          const mod = tp.i(window.__maamSubModId);
          const fn = mod && (mod.f || mod.default || Object.values(mod).find((x) => typeof x === 'function'));
          if (typeof fn === 'function') {
            const res = await fn({ includeAgreement: true });
            const mapped = mapSub(res);
            if (mapped) {
              window.__maamQuotaFetchedAt = Date.now();
              saveDetail(mapped);
              return;
            }
          }
        }
        let callServerFn = null;
        if (tp.c && typeof tp.c.values === 'function') {
          for (const m of tp.c.values()) {
            if (m && m.exports && typeof m.exports.callServer === 'function') {
              callServerFn = m.exports.callServer;
              break;
            }
          }
        }
        const aid = window.__maamSubActionId || '40e0249eeae158685b6a8d0c3b203f53dd4f2c7d63';
        if (callServerFn && aid) {
          const res = await callServerFn(aid, [{ includeAgreement: true }]);
          const mapped = mapSub(res);
          if (mapped) {
            window.__maamQuotaFetchedAt = Date.now();
            saveDetail(mapped);
          }
        }
      } catch (_) {
      } finally {
        window.__maamQuotaInflight = false;
      }
    };

    const inspectNode = (node, depth, skipSub) => {
      if (!node || typeof node !== 'object' || depth > 6) return;
      if (Array.isArray(node)) {
        for (let i = 0; i < node.length && i < 15; i++) inspectNode(node[i], depth + 1, skipSub);
        return;
      }
      if (typeof node.email === 'string' && /^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$/.test(node.email.trim())) {
        window.__maamApiEmail = node.email.trim().toLowerCase();
      }
      if (typeof node.avatar_url === 'string' && node.avatar_url.startsWith('http')) {
        window.__maamApiAvatar = node.avatar_url;
      }
      if (typeof node.picture === 'string' && node.picture.startsWith('http')) {
        window.__maamApiAvatar = node.picture;
      }
      let nextSkipSub = Boolean(skipSub);
      if (!nextSkipSub) {
        const mapped = mapSub(node);
        if (mapped) {
          saveDetail(mapped);
          nextSkipSub = true;
        }
      }
      const remCredits = node.remaining_credits ?? node.credits_remaining ?? node.credit_balance ?? node.quota_remaining;
      if (typeof remCredits === 'number' && Number.isFinite(remCredits)) {
        const total = node.total_credits ?? node.monthly_limit ?? node.quota_limit;
        if (typeof total === 'number' && total > 0) {
          const pct = Math.max(0, Math.min(100, Math.round((remCredits / total) * 100)));
          setQuota('剩余 ' + remCredits + '/' + total, pct);
        } else {
          setQuota('剩余额度 ' + remCredits, null);
        }
      }
      if (node.code === 'out_of_credits' || node.error === 'rate_limit_exceeded') {
        setQuota('额度已用尽', 0);
      }
      const keys = Object.keys(node);
      for (let i = 0; i < keys.length && i < 24; i++) {
        const v = node[keys[i]];
        if (v && typeof v === 'object') inspectNode(v, depth + 1, nextSkipSub);
      }
    };
    const inspectText = (txt) => {
      if (typeof txt !== 'string' || txt.length < 2 || txt.length > 600000) return;
      if (!/percent_used|percentUsed|quota_status|quotaStatus|usage_row_value_label|usageRowValueLabel|topup_balance|topupBalance|remaining_credits|credit_balance|out_of_credits|UNLIMITED|email|credits|allowance|balance|usage|tokens|billing|subscription|harkTokens|dailyLimit|dailyUsed|poolLimit|poolUsed|dailyResetsAt|poolResetsAt|meters/i.test(txt)) return;
      try {
        inspectNode(JSON.parse(txt), 0, false);
        return;
      } catch (_) {}
      const lines = txt.split('\n');
      for (let i = 0; i < lines.length && i < 80; i++) {
        const idx = lines[i].indexOf(':');
        if (idx > 0 && idx < 12) {
          const rest = lines[i].slice(idx + 1).trim();
          if (rest.startsWith('{') || rest.startsWith('[')) {
            try { inspectNode(JSON.parse(rest), 0, false); } catch (_) {}
          }
        }
      }
    };
    if (isPrimaryHost) {
      if (typeof origFetch === 'function') {
        window.fetch = function(...args) {
          const reqUrl = typeof args[0] === 'string' ? args[0] : (args[0] && args[0].url) || '';
          const p = origFetch.apply(this, args);
          p.then((res) => {
            try {
              const ct = (res.headers && res.headers.get('content-type')) || '';
              if (ct.includes('json') || ct.includes('text')) {
                res.clone().text().then(inspectText).catch(() => {});
              }
              if (reqUrl.includes('/api/chat') || reqUrl.includes('/api/messages')) {
                clearTimeout(window.__maamPostChatTimer);
                window.__maamPostChatTimer = setTimeout(() => {
                  if (typeof window.__maamRefreshBackendQuota === 'function') {
                    window.__maamRefreshBackendQuota(true);
                  }
                }, 8000);
              }
            } catch (_) {}
          }).catch(() => {});
          return p;
        };
      }
      const OrigWS = window.WebSocket;
      if (typeof OrigWS === 'function') {
        window.WebSocket = new Proxy(OrigWS, {
          construct(target, args, newTarget) {
            const ws = Reflect.construct(target, args, newTarget);
            ws.addEventListener('message', (ev) => {
              if (typeof ev.data === 'string') inspectText(ev.data);
            });
            return ws;
          }
        });
      }
    }
    const capture = (el) => {
      if (!el || typeof el.value !== 'string') return;
      const v = el.value.trim();
      if (/^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$/.test(v)) {
        window.__maamTypedIdentity = v.toLowerCase();
        try { sessionStorage.setItem('__maam_typed_identity__', window.__maamTypedIdentity); } catch (_) {}
      } else if (/^\+?\d[\d\s-]{6,16}$/.test(v) && (el.type === 'tel' || /手机|phone|tel/i.test(el.placeholder || el.name || ''))) {
        window.__maamTypedPhone = v.replace(/\s+/g, '');
        try { sessionStorage.setItem('__maam_typed_phone__', window.__maamTypedPhone); } catch (_) {}
      }
    };
    document.addEventListener('input', (e) => capture(e.target), true);
    document.addEventListener('change', (e) => capture(e.target), true);
    document.addEventListener('blur', (e) => capture(e.target), true);
    document.addEventListener('submit', () => {
      document.querySelectorAll('input').forEach(capture);
    }, true);
    window.addEventListener('keydown', (e) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey) {
        const m = e.key && e.key.match(/^[1-9]$/);
        if (m) {
          window.__maamPendingHotkey = parseInt(m[0], 10);
        }
      }
    }, true);
  } catch (_) {}
})();"#;

#[cfg(desktop)]
const DOM_PAINT_READY_JS: &str = r#"(() => {
  try {
    if (location.href === 'about:blank') return false;
    if (document.readyState !== 'complete') return false;
    const body = document.body;
    if (!body) return false;
    const sheets = document.querySelectorAll('link[rel~="stylesheet"]');
    for (const l of sheets) {
      if (l.disabled) continue;
      if ((l.media || '').toLowerCase() === 'print') continue;
      if (l.sheet === null) return false;
    }
    const host = (location.hostname || '').toLowerCase();
    const isHark = host === 'app.hark.com' || host.endsWith('.hark.com');
    const isMuse =
      host === 'muse.ai' ||
      host.endsWith('.muse.ai') ||
      host === 'meta.com' ||
      host.endsWith('.meta.com');
    const vw = window.innerWidth || document.documentElement.clientWidth || 0;
    const vh = window.innerHeight || document.documentElement.clientHeight || 0;
    const isTrulyVisible = (el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      if (r.width < 16 || r.height < 16) return false;
      if (vw > 0 && vh > 0 && (r.bottom <= 0 || r.right <= 0 || r.top >= vh || r.left >= vw)) {
        return false;
      }
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
    const isLoadingPlaceholderEl = (el) => {
      if (!el) return false;
      const role = (el.getAttribute && el.getAttribute('role')) || '';
      const busy = (el.getAttribute && el.getAttribute('aria-busy')) || '';
      const aria = (el.getAttribute && el.getAttribute('aria-label')) || '';
      const cls =
        typeof el.className === 'string'
          ? el.className
          : (el.getAttribute && el.getAttribute('class')) || '';
      if (role === 'progressbar' || busy === 'true') return true;
      return /spinner|skeleton|shimmer|loader|loading/i.test(`${cls} ${aria}`);
    };
    const isLoadingOnlyText = (t) => {
      const s = (t || '').trim();
      if (!s) return true;
      return /^(loading|加载中|正在加载|请稍候|please wait|\.{1,3}|meta|muse|hark)$/i.test(s);
    };

    let domReady = false;
    if (isHark) {
      const root = document.getElementById('root');
      const rootText = root ? (root.innerText || '').trim() : '';
      if (!(rootText.length < 4 && (body.innerText || '').trim() === 'Unauthorized')) {
        let ready = true;
        if (root) {
          let rootReady = rootText.length >= 2;
          if (!rootReady) {
            const uiEls = root.querySelectorAll('input, button, textarea, [contenteditable="true"], svg, img');
            for (let i = 0; i < uiEls.length && i < 30; i++) {
              if (isTrulyVisible(uiEls[i])) {
                rootReady = true;
                break;
              }
            }
          }
          if (!rootReady) ready = false;
        }
        if (ready && window.__harkSky && !window.__harkSky.painted) {
          const gl = document.getElementById('gl');
          if (gl) {
            try {
              if (getComputedStyle(gl).visibility === 'hidden') ready = false;
            } catch (_) {}
          }
        }
        domReady = ready;
      }
    } else {
      const bodyText = (body.innerText || '').trim();
      const interactiveSel = 'input:not([type="hidden"]), textarea, [contenteditable="true"], button, [role="button"], a[href]';
      const hasVisibleInteractive = (sel) => {
        const els = body.querySelectorAll(sel);
        for (let i = 0; i < els.length && i < 60; i++) {
          const el = els[i];
          if (isLoadingPlaceholderEl(el)) continue;
          if (el.tagName === 'IFRAME') {
            const r = el.getBoundingClientRect();
            if (r.width < 100 || r.height < 100) continue;
          }
          if (isTrulyVisible(el)) return true;
        }
        return false;
      };
      if (isMuse) {
        const hasControl = hasVisibleInteractive(interactiveSel);
        const hasInput = hasVisibleInteractive('input:not([type="hidden"]), textarea, [contenteditable="true"]');
        const hasText = bodyText.length >= 12 && !isLoadingOnlyText(bodyText);
        domReady = hasControl && (hasText || hasInput);
      } else {
        const hasContentText = bodyText.length >= 20 && !isLoadingOnlyText(bodyText);
        const hasLayoutOrContent = hasVisibleInteractive('main, article, section, form, nav, header, input:not([type="hidden"]), textarea, [contenteditable="true"], button, [role="button"], a[href], img, canvas, video, iframe');
        const hasDirectControl = hasVisibleInteractive('input:not([type="hidden"]), textarea, [contenteditable="true"], button, [role="button"], iframe');
        domReady = (hasContentText && hasLayoutOrContent) || hasDirectControl;
      }
    }

    if (!domReady) {
      window.__maamReadySince = 0;
      window.__maamReadyFrames = 0;
      return false;
    }

    if (!window.__maamReadySince) {
      window.__maamReadySince = Date.now();
      window.__maamReadyFrames = 0;
      if (typeof requestAnimationFrame === 'function') {
        const step = () => {
          window.__maamReadyFrames = (window.__maamReadyFrames || 0) + 1;
          if ((window.__maamReadyFrames || 0) < 3) {
            requestAnimationFrame(step);
          }
        };
        requestAnimationFrame(step);
      }
      return false;
    }

    if (document.visibilityState === 'hidden') {
      return (Date.now() - window.__maamReadySince) >= 350;
    }
    return (window.__maamReadyFrames || 0) >= 2 || (Date.now() - window.__maamReadySince) >= 600;
  } catch (_) {
    return false;
  }
})()"#;

#[cfg(desktop)]
const IDENTITY_PROBE_JS: &str = r#"(() => {
  try {
    const pendingHotkey = window.__maamPendingHotkey || null;
    window.__maamPendingHotkey = null;
    document.querySelectorAll('input').forEach((el) => {
      const v = (el.value || '').trim();
      if (/^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$/.test(v)) {
        window.__maamTypedIdentity = v.toLowerCase();
        try { sessionStorage.setItem('__maam_typed_identity__', window.__maamTypedIdentity); } catch (_) {}
      }
    });
    let typed = window.__maamTypedIdentity || '';
    let phone = window.__maamTypedPhone || '';
    try {
      typed = typed || sessionStorage.getItem('__maam_typed_identity__') || '';
      phone = phone || sessionStorage.getItem('__maam_typed_phone__') || '';
    } catch (_) {}
    const host = (location.hostname || '').toLowerCase();
    const path = (location.pathname || '').toLowerCase();
    const pCfg = window.__maamProvider || {};
    const isCustom = Boolean(pCfg.custom);
    const customHost = (pCfg.host || '').toLowerCase();
    const matchesDomain = (h, d) => Boolean(d) && (h === d || h.endsWith('.' + d));
    const baseDomain = (h) => {
      const parts = (h || '').split('.').filter(Boolean);
      return parts.length >= 2 ? parts.slice(-2).join('.') : h;
    };
    const customBase = isCustom ? baseDomain(customHost) : '';
    const isPrimary = isCustom
      ? (matchesDomain(host, customHost) || matchesDomain(host, customBase))
      : (host === 'muse.ai' || host.endsWith('.muse.ai') || host === 'app.hark.com' || host.endsWith('.hark.com'));
    const isExternalLoginHost =
      host === 'meta.com' || host.endsWith('.meta.com') ||
      host === 'facebook.com' || host.endsWith('.facebook.com') ||
      host === 'appleid.apple.com' ||
      (isCustom && (
        matchesDomain(host, 'auth.openai.com') ||
        matchesDomain(host, 'auth0.openai.com') ||
        matchesDomain(host, 'accounts.anthropic.com') ||
        matchesDomain(host, 'accounts.x.ai') ||
        matchesDomain(host, 'login.microsoftonline.com') ||
        matchesDomain(host, 'login.live.com') ||
        matchesDomain(host, 'accounts.google.com')
      ));
    const makeLoggedOut = () => {
      window.__maamQuota = null;
      window.__maamQuotaDetail = null;
      try {
        localStorage.removeItem('__maam_cached_quota__');
        localStorage.removeItem('__maam_cached_quota_detail_v2__');
      } catch (_) {}
      return JSON.stringify({ auth: false, loggedOut: true, typed: typed || phone, email: '', avatar: '', quotaText: '', quotaPct: null, quotaDetail: null, pendingHotkey });
    };
    const makeLoading = () => JSON.stringify({ auth: false, loggedOut: false, typed: typed || phone, email: '', avatar: '', quotaText: '', quotaPct: null, quotaDetail: null, pendingHotkey });
    if (document.readyState === 'loading') {
      return makeLoading();
    }
    const loginInputs = document.querySelectorAll('input[type="email"], input[type="password"], input[autocomplete="one-time-code"], input[placeholder*="邮箱"], input[placeholder*="手机"], input[placeholder*="email" i], input[placeholder*="phone" i]');
    const hasVisibleLoginInput = () => {
      for (const inp of loginInputs) {
        if (inp.disabled || inp.readOnly) continue;
        const r = inp.getBoundingClientRect();
        if (r.width > 20 && r.height > 15) return true;
      }
      return false;
    };
    const hasChatComposer = Array.from(document.querySelectorAll('textarea, [contenteditable="true"]')).some((el) => {
      if (el.disabled || el.readOnly) return false;
      const r = el.getBoundingClientRect();
      return r.width > 120 && r.height > 20;
    });
    if (isExternalLoginHost) {
      if (/^\/(login|signin|signup|auth)\b/.test(path) || hasVisibleLoginInput()) {
        return makeLoggedOut();
      }
      return makeLoading();
    }
    if (!isPrimary) {
      return makeLoading();
    }
    if (/^\/(login|signin|signup|auth)\b/.test(path)) {
      return makeLoggedOut();
    }
    if (!hasChatComposer && !/^\/(settings|profile|account)\b/.test(path) && hasVisibleLoginInput()) {
      return makeLoggedOut();
    }
    const bodyText = (document.body && document.body.innerText) || '';
    if (!hasChatComposer && /登录或创建账户|Log in or sign up|Sign in to|登录以继续|通过 Google 继续|Continue with Google|通过 Apple 继续|Continue with Apple/i.test(bodyText)) {
      return makeLoggedOut();
    }
    if (document.readyState !== 'complete') {
      return makeLoading();
    }

    const extractAvatar = () => {
      const selectors = [
        '[data-hatch-media-owner="avatar-layer"]',
        '[data-hatch-avatar-host]:not([data-hatch-avatar-host-hidden="true"]) img',
        '[data-hatch-avatar-host]:not([data-hatch-avatar-host-hidden="true"]) video',
        '[data-hatch-avatar-host] img',
        '[data-hatch-avatar-host] video',
        '[data-hatch-avatar-media-static] img',
        '[data-hatch-avatar-media-static] video',
        '[class*="hatch-avatar"] img',
        '[class*="hatch-avatar"] video',
        '[data-avatar-parity-side] img',
        '[data-testid*="avatar" i] img',
        '[data-testid*="user" i] img',
        'header [class*="avatar" i] img',
        'nav [class*="avatar" i] img',
        'button[aria-label*="Profile" i] img',
        'button[aria-label*="Account" i] img',
        'button[aria-label*="User" i] img',
        'button[aria-label*="Settings" i] img',
        'img[src^="blob:"]',
        '[class*="avatar" i] img',
        'img[alt*="avatar" i]',
        'img[src*="avatar" i]'
      ];
      let target = null;
      for (const sel of selectors) {
        const isHatchAvatar = sel.includes('hatch');
        const list = document.querySelectorAll(sel);
        for (const el of list) {
          if (el.closest && el.closest('.landing-rebus-avatar-slot')) continue;
          const r = el.getBoundingClientRect();
          if (!isHatchAvatar && (r.width < 16 || r.height < 16 || r.width > 520 || Math.abs(r.width - r.height) > 64)) continue;
          if (isHatchAvatar && r.width > 520) continue;
          if (el.tagName === 'IMG' && el.complete && el.naturalWidth > 0) {
            target = el;
            break;
          }
          if (el.tagName === 'VIDEO' && el.readyState >= 2 && el.videoWidth > 0) {
            target = el;
            break;
          }
        }
        if (target) break;
      }
      if (!target) return window.__maamApiAvatar || window.__maamAvatarData || '';
      const srcKey = target.currentSrc || target.src || target.poster || '';
      if (srcKey && window.__maamAvatarKey === srcKey && window.__maamAvatarData) {
        return window.__maamAvatarData;
      }
      try {
        const sw = target.tagName === 'VIDEO' ? target.videoWidth : target.naturalWidth;
        const sh = target.tagName === 'VIDEO' ? target.videoHeight : target.naturalHeight;
        if (!sw || !sh) return window.__maamApiAvatar || window.__maamAvatarData || '';
        const size = 72;
        const canvas = document.createElement('canvas');
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d');
        if (!ctx) return '';
        const side = Math.min(sw, sh);
        const sx = (sw - side) / 2;
        const sy = (sh - side) / 2;
        ctx.drawImage(target, sx, sy, side, side, 0, 0, size, size);
        const px = ctx.getImageData(0, 0, size, size).data;
        let visible = 0;
        for (let i = 3; i < px.length; i += 64) {
          if (px[i] > 12) { visible++; if (visible >= 4) break; }
        }
        if (visible < 4) return window.__maamApiAvatar || window.__maamAvatarData || '';
        let dataUrl = canvas.toDataURL('image/webp', 0.86);
        if (!dataUrl || dataUrl.length < 32) {
          dataUrl = canvas.toDataURL('image/png');
        }
        if (dataUrl && dataUrl.startsWith('data:image/') && dataUrl.length < 60000) {
          window.__maamAvatarKey = srcKey;
          window.__maamAvatarData = dataUrl;
          return dataUrl;
        }
      } catch (_) {}
      return window.__maamApiAvatar || window.__maamAvatarData || '';
    };

    const extractQuota = () => {
      if (isCustom) {
        return { text: '', pct: null, detail: null };
      }
      if (isPrimary) {
        if (typeof window.__maamRefreshBackendQuota === 'function') {
          window.__maamRefreshBackendQuota(false);
        }
      }

      if (window.__maamQuotaDetail && window.__maamQuotaDetail.planLabel) {
        const d = window.__maamQuotaDetail;
        const rem = typeof d.planUsedPercent === 'number' ? Math.max(0, 100 - d.planUsedPercent) : null;
        return { text: d.planValue || d.planLabel, pct: rem, detail: d };
      }

      try {
        const cachedV2 = localStorage.getItem('__maam_cached_quota_detail_v2__');
        if (cachedV2) {
          const d = JSON.parse(cachedV2);
          if (d && d.planLabel) {
            window.__maamQuotaDetail = d;
            const rem = typeof d.planUsedPercent === 'number' ? Math.max(0, 100 - d.planUsedPercent) : null;
            return { text: d.planValue || d.planLabel, pct: rem, detail: d };
          }
        }
      } catch (_) {}

      if (host.endsWith('hark.com')) {
        const mDaily = bodyText.match(/(?:已使用每日上限的\s*(\d+(?:\.\d+)?)\s*%|(\d+(?:\.\d+)?)\s*%\s*of\s*daily\s*limit\s*used)/i);
        const mMonthly = bodyText.match(/(?:已使用每月上限的\s*(\d+(?:\.\d+)?)\s*%|(\d+(?:\.\d+)?)\s*%\s*of\s*monthly\s*limit\s*used)/i);
        if (mDaily || mMonthly) {
          const dPct = mDaily ? Math.max(0, Math.min(100, Math.round(parseFloat(mDaily[1] || mDaily[2])))) : null;
          const mPct = mMonthly ? Math.max(0, Math.min(100, Math.round(parseFloat(mMonthly[1] || mMonthly[2])))) : null;
          const mDailyReset = bodyText.match(/(?:每日重置|Daily reset)[\s:：·]*([^\n\r]{2,28})/i);
          const mMonthlyReset = bodyText.match(/(?:每月重置|Monthly reset)[\s:：·]*([^\n\r]{2,28})/i);
          const fmtDomReset = (m) => {
            if (!m || !m[1]) return null;
            const s = m[1].trim();
            return s ? (s.endsWith('重置') ? s : (s + ' 重置')) : null;
          };
          const d = dPct !== null
            ? {
                planLabel: '每日上限',
                planValue: '已使用 ' + dPct + '%',
                planSubtext: fmtDomReset(mDailyReset),
                planUsedPercent: dPct,
                extraLabel: mPct !== null ? '每月上限' : null,
                extraValue: mPct !== null ? ('已使用 ' + mPct + '%') : null,
                extraSubtext: mPct !== null ? fmtDomReset(mMonthlyReset) : null,
                extraUsedPercent: mPct
              }
            : {
                planLabel: '每月上限',
                planValue: '已使用 ' + mPct + '%',
                planSubtext: fmtDomReset(mMonthlyReset),
                planUsedPercent: mPct,
                extraLabel: null,
                extraValue: null,
                extraSubtext: null,
                extraUsedPercent: null
              };
          if (typeof window.__maamSaveDetail === 'function') window.__maamSaveDetail(d);
          return { text: d.planValue, pct: 100 - d.planUsedPercent, detail: d };
        }
        const creditEls = document.querySelectorAll('[class*="credit" i], [class*="usage" i], [data-testid*="credit"], [data-testid*="usage"]');
        for (const el of creditEls) {
          const txt = (el.innerText || '').trim();
          const m = txt.match(/(\d+(?:\.\d+)?)\s*\/\s*(\d+(?:\.\d+)?)/);
          if (m) {
            const cur = parseFloat(m[1]);
            const max = parseFloat(m[2]);
            if (max > 0) {
              const pct = Math.max(0, Math.min(100, Math.round((cur / max) * 100)));
              const d = {
                planLabel: 'Hark 用量',
                planValue: '已使用 ' + pct + '%',
                planSubtext: '当前 ' + cur + ' / ' + max,
                planUsedPercent: pct,
                extraLabel: null,
                extraValue: null,
                extraSubtext: null,
                extraUsedPercent: null
              };
              if (typeof window.__maamSaveDetail === 'function') window.__maamSaveDetail(d);
              return { text: d.planValue, pct: 100 - pct, detail: d };
            }
          }
        }
      }

      if (window.__maamQuota && window.__maamQuota.text && window.__maamQuota.text !== '剩余额度 100%') {
        return { text: window.__maamQuota.text, pct: window.__maamQuota.pct, detail: null };
      }
      return { text: '正在同步额度…', pct: null, detail: null };
    };

    const avatar = extractAvatar();
    const quota = extractQuota();

    const bad = /^(support|privacy|help|legal|press|noreply|no-reply|info|security|contact|abuse|feedback|sales|terms)@/i;
    const re = /[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}/g;
    const candidates = [];
    if (window.__maamApiEmail) candidates.push(window.__maamApiEmail);
    document.querySelectorAll('[data-email],[aria-label*="@"],[title*="@"],a[href^="mailto:"]').forEach((el) => {
      const raw = (el.getAttribute('data-email') || el.getAttribute('aria-label') || el.getAttribute('title') || el.getAttribute('href') || '').replace(/^mailto:/i, '');
      const m = raw.match(re);
      if (m) candidates.push(...m);
    });
    try {
      for (let i = 0; i < localStorage.length; i++) {
        const k = localStorage.key(i);
        const val = k ? localStorage.getItem(k) : '';
        if (val && val.includes('@') && val.length < 4000) {
          const m = val.match(re);
          if (m) candidates.push(...m);
        }
      }
    } catch (_) {}
    const textMatches = bodyText.match(re);
    if (textMatches) candidates.push(...textMatches);
    let foundEmail = '';
    for (const c of candidates) {
      const clean = c.toLowerCase();
      if (!bad.test(clean) && !clean.endsWith('@example.com') && !clean.endsWith('@sentry.io')) {
        foundEmail = clean;
        break;
      }
    }
    return JSON.stringify({
      auth: true,
      loggedOut: false,
      typed: typed || phone,
      email: foundEmail,
      avatar,
      quotaText: quota.text || '',
      quotaPct: quota.pct,
      quotaDetail: quota.detail || null,
      pendingHotkey
    });
  } catch (_) {
    return JSON.stringify({ auth: false, loggedOut: false, typed: '', email: '', avatar: '', quotaText: '', quotaPct: null, quotaDetail: null, pendingHotkey: null });
  }
})()"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerSnapshot {
  pub platform: &'static str,
  pub providers: Vec<ProviderInfo>,
  pub accounts: Vec<AccountRecord>,
  pub selected_provider_id: ProviderId,
  pub active_account: Option<AccountKey>,
  pub switching_account: Option<AccountKey>,
  pub warm_accounts: Vec<AccountKey>,
  pub last_committed: Option<AccountKey>,
  pub pending_cleanup: Vec<AccountKey>,
  pub conflicts: Vec<ConflictItem>,
  pub notice: Option<String>,
  pub modal_active: bool,
  pub settings: AppSettings,
  pub global_shortcut_active: bool,
  pub global_shortcut_error: Option<String>,
}

#[cfg(desktop)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadNoticeEvent {
  pub id: u64,
  pub filename: String,
  pub status: &'static str,
  pub error: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct PendingExternalAuth {
  account_key: AccountKey,
  switch_seq: u64,
  nonce: String,
}

#[cfg(desktop)]
struct WarmDesktopEntry {
  account_key: AccountKey,
  view_instance_id: u64,
  webview_label: String,
  popup_label: Option<String>,
  paint_ready: bool,
  staged_painted: bool,
  navigation_reached: bool,
  load_gen: Arc<AtomicU64>,
  last_url: Option<Url>,
  captured_identity_hint: Option<String>,
  watcher_running: bool,
  probe_notify: Arc<tokio::sync::Notify>,
  last_used_ms: u64,
  alive_flag: Arc<AtomicBool>,
}

struct ActiveSessionSlot {
  account_key: AccountKey,
  view_instance_id: u64,
  #[cfg_attr(desktop, allow(dead_code))]
  switch_seq: u64,
  committed: bool,
}

struct InnerState {
  store_path: PathBuf,
  app_data_dir: PathBuf,
  store: AccountsStoreFile,
  conflicts: Vec<ConflictItem>,
  selected_provider_id: ProviderId,
  active_slot: Option<ActiveSessionSlot>,
  last_active_per_provider: HashMap<ProviderId, AccountKey>,
  #[cfg(desktop)]
  warm_pool: HashMap<AccountKey, WarmDesktopEntry>,
  #[cfg(desktop)]
  prewarm_failed: HashSet<AccountKey>,
  #[cfg(desktop)]
  cookie_checked_at: HashMap<AccountKey, u64>,
  pending_external_auth: Option<PendingExternalAuth>,
  notice: Option<String>,
  modal_active: bool,
  global_shortcut_active: bool,
  global_shortcut_error: Option<String>,
}

pub struct SessionManager {
  current_seq: Arc<AtomicU64>,
  active_view_id: Arc<AtomicU64>,
  view_counter: AtomicU64,
  last_sync_timestamp: AtomicU64,
  minimize_to_tray: AtomicBool,
  #[cfg(desktop)]
  download_counter: AtomicU64,
  #[cfg(desktop)]
  recent_downloads: std::sync::Mutex<HashMap<u64, PathBuf>>,
  inner: Mutex<InnerState>,
}

impl SessionManager {
  pub fn new(app_data_dir: PathBuf) -> Self {
    let store_path = app_data_dir.join("accounts.json");
    let loaded = load_or_migrate_store(&store_path);
    let initial_minimize = loaded.store.settings.minimize_to_tray;
    let initial_provider = loaded
      .store
      .last_committed
      .as_ref()
      .map(|k| k.provider_id.clone())
      .filter(|pid| {
        resolve_provider(&loaded.store.custom_providers, pid)
          .is_some_and(|spec| spec.info.enabled)
      })
      .unwrap_or_else(ProviderId::muse);

    Self {
      current_seq: Arc::new(AtomicU64::new(1)),
      active_view_id: Arc::new(AtomicU64::new(0)),
      view_counter: AtomicU64::new(1),
      last_sync_timestamp: AtomicU64::new(0),
      minimize_to_tray: AtomicBool::new(initial_minimize),
      #[cfg(desktop)]
      download_counter: AtomicU64::new(1),
      #[cfg(desktop)]
      recent_downloads: std::sync::Mutex::new(HashMap::new()),
      inner: Mutex::new(InnerState {
        store_path,
        app_data_dir,
        store: loaded.store,
        conflicts: loaded.conflicts,
        selected_provider_id: initial_provider,
        active_slot: None,
        last_active_per_provider: HashMap::new(),
        #[cfg(desktop)]
        warm_pool: HashMap::new(),
        #[cfg(desktop)]
        prewarm_failed: HashSet::new(),
        #[cfg(desktop)]
        cookie_checked_at: HashMap::new(),
        pending_external_auth: None,
        notice: loaded.startup_notice,
        modal_active: false,
        global_shortcut_active: false,
        global_shortcut_error: None,
      }),
    }
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub fn minimize_to_tray_enabled(&self) -> bool {
    self.minimize_to_tray.load(Ordering::SeqCst)
  }

  #[cfg(desktop)]
  pub fn toggle_main_window(app: &AppHandle) {
    let Some(win) = app.get_window("main") else {
      return;
    };
    let visible = win.is_visible().unwrap_or(false);
    let focused = win.is_focused().unwrap_or(false);
    if visible && focused {
      let _ = win.hide();
    } else {
      let _ = win.unminimize();
      let _ = win.show();
      let _ = win.set_focus();
    }
  }

  #[cfg(desktop)]
  async fn apply_global_shortcut_on(app: &AppHandle, shortcut: &str) -> Result<bool, String> {
    let trimmed = shortcut.trim().to_string();
    let app_handle = app.clone();
    tokio::task::spawn_blocking(move || {
      use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

      let gs = app_handle.global_shortcut();
      let _ = gs.unregister_all();
      if trimmed.is_empty() {
        return Ok(false);
      }
      gs.on_shortcut(trimmed.as_str(), move |app_cb, _hotkey, event| {
        if event.state == ShortcutState::Pressed {
          Self::toggle_main_window(app_cb);
        }
      })
      .map_err(|e| format!("快捷键格式无效或已被其他应用占用: {e}"))?;
      Ok(true)
    })
    .await
    .map_err(|e| format!("应用全局快捷键中断: {e}"))?
  }

  fn snapshot_from_locked(locked: &InnerState) -> ManagerSnapshot {
    let (active_account, switching_account) = match &locked.active_slot {
      Some(slot) if slot.committed => (Some(slot.account_key.clone()), None),
      Some(slot) => (None, Some(slot.account_key.clone())),
      None => (None, None),
    };

    #[cfg(desktop)]
    let warm_accounts = locked
      .warm_pool
      .values()
      .filter(|e| e.paint_ready)
      .map(|e| e.account_key.clone())
      .collect::<Vec<_>>();

    #[cfg(not(desktop))]
    let warm_accounts = Vec::new();

    ManagerSnapshot {
      platform: current_platform_name(),
      providers: all_provider_infos(&locked.store.custom_providers),
      accounts: locked.store.accounts.clone(),
      selected_provider_id: locked.selected_provider_id.clone(),
      active_account,
      switching_account,
      warm_accounts,
      last_committed: locked.store.last_committed.clone(),
      pending_cleanup: locked.store.pending_cleanup.clone(),
      conflicts: locked.conflicts.clone(),
      notice: locked.notice.clone(),
      modal_active: locked.modal_active,
      settings: locked.store.settings.clone(),
      global_shortcut_active: locked.global_shortcut_active,
      global_shortcut_error: locked.global_shortcut_error.clone(),
    }
  }

  fn emit_snapshot(&self, app: &AppHandle, snap: &ManagerSnapshot) {
    let _ = app.emit("manager-state-changed", snap);
  }

  pub async fn get_snapshot(&self) -> ManagerSnapshot {
    let locked = self.inner.lock().await;
    Self::snapshot_from_locked(&locked)
  }

  pub async fn run_startup_tasks(self: &Arc<Self>, app: &AppHandle) {
    #[cfg(desktop)]
    {
      let initial_shortcut = {
        let locked = self.inner.lock().await;
        locked.store.settings.global_shortcut.clone()
      };
      let shortcut_res = Self::apply_global_shortcut_on(app, &initial_shortcut).await;
      let mut locked = self.inner.lock().await;
      match shortcut_res {
        Ok(active) => {
          locked.global_shortcut_active = active;
          locked.global_shortcut_error = None;
        }
        Err(err) => {
          locked.global_shortcut_active = false;
          locked.global_shortcut_error = Some(err);
        }
      }
    }

    #[cfg(target_os = "android")]
    {
      let mgr = Arc::clone(self);
      let app_clone = app.clone();
      let channel = tauri::ipc::Channel::new(move |body| {
        if let tauri::ipc::InvokeResponseBody::Json(raw_json) = body {
          if let Ok(evt) =
            serde_json::from_str::<crate::platform::android::AndroidSessionEvent>(&raw_json)
          {
            let mgr_inner = Arc::clone(&mgr);
            let app_inner = app_clone.clone();
            tauri::async_runtime::spawn(async move {
              mgr_inner.handle_android_event(&app_inner, evt).await;
            });
          }
        }
        Ok(())
      });
      let _ = crate::platform::android::subscribe_events(app, channel).await;
    }

    self.retry_pending_cleanups(app).await;

    let mgr_periodic = Arc::clone(self);
    let app_periodic = app.clone();
    tauri::async_runtime::spawn(async move {
      let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5 * 60));
      interval.tick().await;
      loop {
        interval.tick().await;
        mgr_periodic.trigger_silent_sync(&app_periodic, false).await;
      }
    });

    let candidate = {
      let mut locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() || locked.notice.is_some() {
        None
      } else if let Some(key) = locked.store.last_committed.clone() {
        let in_cleanup = locked.store.pending_cleanup.contains(&key);
        let exists = locked
          .store
          .accounts
          .iter()
          .any(|a| a.provider_id == key.provider_id && a.uuid == key.uuid);
        let enabled = resolve_provider(&locked.store.custom_providers, &key.provider_id)
          .is_some_and(|spec| spec.info.enabled);
        if in_cleanup || !exists {
          locked.store.last_committed = None;
          let _ = save_store_atomic(&locked.store_path, &locked.store);
          locked.notice = Some("上次记录的账号键不存在或已清理，已回到管理页".to_string());
          None
        } else if !enabled {
          None
        } else {
          locked.selected_provider_id = key.provider_id.clone();
          Some(key)
        }
      } else {
        None
      }
    };

    let had_startup_candidate = candidate.is_some();
    if let Some(key) = candidate {
      let _ = self.switch_account_internal(app, key, true).await;
    } else {
      let snap = self.get_snapshot().await;
      self.emit_snapshot(app, &snap);
    }

    let mgr_prewarm = Arc::clone(self);
    let app_prewarm = app.clone();
    tauri::async_runtime::spawn(async move {
      tokio::time::sleep(std::time::Duration::from_millis(600)).await;
      mgr_prewarm.prewarm_next_cold_account(&app_prewarm).await;
      if !had_startup_candidate {
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        mgr_prewarm.prewarm_next_cold_account(&app_prewarm).await;
      }
    });
  }

  async fn retry_pending_cleanups(&self, app: &AppHandle) {
    let keys = {
      let locked = self.inner.lock().await;
      locked.store.pending_cleanup.clone()
    };
    if keys.is_empty() {
      return;
    }

    for key in keys {
      let deleted = self.delete_profile_for_key(app, &key.uuid).await;
      if deleted.is_ok() {
        let mut locked = self.inner.lock().await;
        locked.store.pending_cleanup.retain(|k| k != &key);
        let _ = save_store_atomic(&locked.store_path, &locked.store);
      }
    }
  }

  async fn delete_profile_for_key(&self, app: &AppHandle, uuid_str: &str) -> Result<(), String> {
    #[cfg(desktop)]
    {
      let app_data_dir = {
        let locked = self.inner.lock().await;
        locked.app_data_dir.clone()
      };
      platform::delete_desktop_profile(app, &app_data_dir, uuid_str).await
    }
    #[cfg(target_os = "android")]
    {
      crate::platform::android::delete_profile(app, uuid_str).await
    }
    #[cfg(all(not(desktop), not(target_os = "android")))]
    {
      let _ = (app, uuid_str);
      Ok(())
    }
  }

  #[cfg(desktop)]
  fn close_warm_entry(app: &AppHandle, entry: WarmDesktopEntry) {
    entry.alive_flag.store(false, Ordering::SeqCst);
    entry.probe_notify.notify_one();
    if let Some(popup_label) = entry.popup_label {
      if let Some(popup_win) = app.get_webview_window(&popup_label) {
        let _ = popup_win.close();
      }
    }
    if let Some(wv) = app.get_webview(&entry.webview_label) {
      let _ = wv.hide();
      let _ = wv.close();
    }
  }

  #[cfg(desktop)]
  fn evict_warm_pool_if_needed(
    app: &AppHandle,
    locked: &mut InnerState,
    protect: Option<&AccountKey>,
    max_len: usize,
  ) {
    while locked.warm_pool.len() > max_len {
      let victim_key = locked
        .warm_pool
        .iter()
        .filter(|(k, _)| {
          Some(*k) != protect && locked.active_slot.as_ref().map(|s| &s.account_key) != Some(*k)
        })
        .min_by_key(|(_, entry)| (entry.last_used_ms, entry.view_instance_id))
        .map(|(k, _)| k.clone());
      let Some(victim) = victim_key else {
        break;
      };
      if let Some(entry) = locked.warm_pool.remove(&victim) {
        Self::close_warm_entry(app, entry);
      }
    }
  }

  async fn hide_foreground_slot(&self, app: &AppHandle, next_seq: u64, locked: &mut InnerState) {
    locked.pending_external_auth = None;
    self.active_view_id.store(0, Ordering::SeqCst);
    if let Some(old_slot) = locked.active_slot.take() {
      #[cfg(desktop)]
      {
        let _ = next_seq;
        if let Some(entry) = locked.warm_pool.get_mut(&old_slot.account_key) {
          if let Some(popup_label) = entry.popup_label.take() {
            if let Some(popup_win) = app.get_webview_window(&popup_label) {
              let _ = popup_win.close();
            }
          }
          if let Some(wv) = app.get_webview(&entry.webview_label) {
            let _ = wv.hide();
          }
        }
      }
      #[cfg(target_os = "android")]
      {
        let _ = old_slot;
        let _ = crate::platform::android::close_active_session(app, next_seq).await;
      }
    }
    #[cfg(desktop)]
    {
      if let Some(main_win) = app.get_window("main") {
        let _ = main_win.set_title(APP_TITLE);
      }
      Self::apply_desktop_layout_locked(app, locked);
    }
  }

  #[cfg(desktop)]
  fn set_right_webview_input_frozen(webview: &tauri::Webview, frozen: bool) {
    #[cfg(windows)]
    {
      let _ = webview.with_webview(move |pwv| unsafe {
        let mut hwnd = windows::Win32::Foundation::HWND::default();
        if pwv.controller().ParentWindow(&mut hwnd).is_ok() && !hwnd.0.is_null() {
          let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(hwnd, !frozen);
        }
      });
    }

    if frozen {
      let _ = webview.eval(
        r#"(() => {
          try {
            document.documentElement.inert = true;
            document.activeElement?.blur?.();
            if (!document.getElementById('__mgr_freeze_shield__')) {
              const s = document.createElement('div');
              s.id = '__mgr_freeze_shield__';
              s.style.cssText = 'position:fixed;inset:0;z-index:2147483647;background:transparent;cursor:not-allowed;';
              for (const ev of ['mousedown','mouseup','click','keydown','keyup','keypress','focusin','wheel','touchstart']) {
                s.addEventListener(ev, (e) => { e.stopImmediatePropagation(); e.preventDefault(); }, true);
              }
              document.documentElement.appendChild(s);
            }
          } catch (_) {}
        })()"#,
      );
    } else {
      let _ = webview.eval(
        r#"(() => {
          try {
            document.documentElement.inert = false;
            document.getElementById('__mgr_freeze_shield__')?.remove();
          } catch (_) {}
        })()"#,
      );
    }
  }

  #[cfg(desktop)]
  fn apply_desktop_layout_locked(app: &AppHandle, locked: &InnerState) {
    let Some(main_win) = app.get_window("main") else {
      return;
    };
    let Ok(phys_size) = main_win.inner_size() else {
      return;
    };
    let Ok(scale) = main_win.scale_factor() else {
      return;
    };
    let (win_w, win_h) = (
      (phys_size.width as f64) / scale,
      (phys_size.height as f64) / scale,
    );
    let _ = (win_w, win_h);

    let left_phys = ((LEFT_PANE_WIDTH * scale).round().max(0.0) as u32).min(phys_size.width);
    let right_phys = phys_size.width.saturating_sub(left_phys);

    let active_committed_label = locked.active_slot.as_ref().and_then(|slot| {
      if slot.committed {
        locked
          .warm_pool
          .get(&slot.account_key)
          .map(|e| e.webview_label.clone())
      } else {
        None
      }
    });

    let active_loading_label = locked.active_slot.as_ref().and_then(|slot| {
      if !slot.committed {
        locked
          .warm_pool
          .get(&slot.account_key)
          .map(|e| e.webview_label.clone())
      } else {
        None
      }
    });

    let stage_loading_behind = cfg!(target_os = "macos")
      && active_committed_label.is_none()
      && active_loading_label.is_some()
      && right_phys > 1;

    for entry in locked.warm_pool.values() {
      let keep_visible = Some(&entry.webview_label) == active_committed_label.as_ref()
        || (stage_loading_behind && Some(&entry.webview_label) == active_loading_label.as_ref());
      if !keep_visible {
        if let Some(wv) = app.get_webview(&entry.webview_label) {
          let _ = wv.hide();
        }
      }
    }

    if let Some(active_label) = active_committed_label {
      if let Some(mgr_wv) = app.get_webview("manager") {
        let _ = mgr_wv.set_bounds(tauri::Rect {
          position: tauri::PhysicalPosition::new(0, 0).into(),
          size: tauri::PhysicalSize::new(left_phys, phys_size.height).into(),
        });
      }

      if let Some(acc_wv) = app.get_webview(&active_label) {
        if right_phys <= 1 {
          let _ = acc_wv.hide();
        } else {
          let _ = acc_wv.set_bounds(tauri::Rect {
            position: tauri::PhysicalPosition::new(left_phys as i32, 0).into(),
            size: tauri::PhysicalSize::new(right_phys, phys_size.height).into(),
          });
          let _ = acc_wv.show();
        }
        Self::set_right_webview_input_frozen(&acc_wv, locked.modal_active);
        if locked.modal_active {
          if let Some(mgr_wv) = app.get_webview("manager") {
            let _ = mgr_wv.set_focus();
          }
        }
      }
    } else if let Some(mgr_wv) = app.get_webview("manager") {
      let _ = mgr_wv.set_bounds(tauri::Rect {
        position: tauri::PhysicalPosition::new(0, 0).into(),
        size: tauri::PhysicalSize::new(phys_size.width, phys_size.height).into(),
      });

      #[cfg(target_os = "macos")]
      if let Some(loading_label) = active_loading_label.filter(|_| right_phys > 1) {
        platform::raise_macos_webview_to_front(&mgr_wv);
        if let Some(loading_wv) = app.get_webview(&loading_label) {
          let _ = loading_wv.set_bounds(tauri::Rect {
            position: tauri::PhysicalPosition::new(left_phys as i32, 0).into(),
            size: tauri::PhysicalSize::new(right_phys, phys_size.height).into(),
          });
          let _ = loading_wv.show();
          platform::raise_macos_webview_to_front(&mgr_wv);
        }
      }
    }
  }

  #[cfg(desktop)]
  pub async fn recompute_desktop_bounds(&self, app: &AppHandle) {
    let locked = self.inner.lock().await;
    Self::apply_desktop_layout_locked(app, &locked);
  }

  pub async fn select_provider(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;

    #[cfg(desktop)]
    let warm_restore = {
      let mut locked = self.inner.lock().await;
      let spec = resolve_provider(&locked.store.custom_providers, &provider_id)
        .ok_or_else(|| "未找到对应的服务配置".to_string())?;
      locked.selected_provider_id = provider_id.clone();
      self.hide_foreground_slot(app, next_seq, &mut locked).await;
      if spec.info.enabled && locked.conflicts.is_empty() {
        locked
          .last_active_per_provider
          .get(&provider_id)
          .cloned()
          .filter(|k| {
            locked
              .warm_pool
              .get(k)
              .map(|e| e.paint_ready)
              .unwrap_or(false)
              && locked
                .store
                .accounts
                .iter()
                .any(|a| a.provider_id == k.provider_id && a.uuid == k.uuid)
          })
      } else {
        None
      }
    };

    #[cfg(desktop)]
    if let Some(restore_key) = warm_restore {
      return self.switch_account_internal(app, restore_key, false).await;
    }

    #[cfg(not(desktop))]
    {
      let mut locked = self.inner.lock().await;
      let _ = resolve_provider(&locked.store.custom_providers, &provider_id)
        .ok_or_else(|| "未找到对应的服务配置".to_string())?;
      locked.selected_provider_id = provider_id;
      self.hide_foreground_slot(app, next_seq, &mut locked).await;
      return Ok(Self::snapshot_from_locked(&locked));
    }

    #[cfg(desktop)]
    Ok(self.get_snapshot().await)
  }

  pub async fn add_custom_provider(
    &self,
    app: &AppHandle,
    raw_name: &str,
    raw_url: &str,
  ) -> Result<ManagerSnapshot, String> {
    let (name, entry_url) = normalize_custom_provider_input(raw_name, raw_url)?;
    let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
    let mut locked = self.inner.lock().await;
    if !locked.conflicts.is_empty() {
      return Err("请先处理 UUID 归属冲突后再新增服务".to_string());
    }
    let id = loop {
      let suffix = &Uuid::new_v4().simple().to_string()[..8];
      let candidate = ProviderId::from_validated_custom(format!("custom-{suffix}"));
      if !locked.store.custom_providers.iter().any(|p| p.id == candidate) {
        break candidate;
      }
    };
    locked.store.custom_providers.push(CustomProviderRecord {
      id: id.clone(),
      name,
      entry_url,
      created_at_ms: now_ms(),
    });
    if let Err(err) = save_store_atomic(&locked.store_path, &locked.store) {
      locked.store.custom_providers.pop();
      return Err(err);
    }
    locked.selected_provider_id = id;
    locked.notice = None;
    self.hide_foreground_slot(app, next_seq, &mut locked).await;
    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn update_custom_provider(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_name: &str,
    raw_url: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    if provider_id.is_builtin() {
      return Err("内置服务不支持修改入口配置".to_string());
    }
    let (name, entry_url) = normalize_custom_provider_input(raw_name, raw_url)?;

    let reopen_key = {
      let mut locked = self.inner.lock().await;
      let idx = locked
        .store
        .custom_providers
        .iter()
        .position(|p| p.id == provider_id)
        .ok_or_else(|| "未找到对应的自定义服务".to_string())?;
      let previous = locked.store.custom_providers[idx].clone();
      let url_changed = previous.entry_url != entry_url;
      let name_changed = previous.name != name;
      if !url_changed && !name_changed {
        return Ok(Self::snapshot_from_locked(&locked));
      }
      locked.store.custom_providers[idx].name = name.clone();
      locked.store.custom_providers[idx].entry_url = entry_url;
      if let Err(err) = save_store_atomic(&locked.store_path, &locked.store) {
        locked.store.custom_providers[idx] = previous;
        return Err(err);
      }

      let active_key_for_provider = locked
        .active_slot
        .as_ref()
        .filter(|s| s.account_key.provider_id == provider_id)
        .map(|s| s.account_key.clone());

      #[cfg(desktop)]
      {
        if url_changed {
          let keys_to_close: Vec<AccountKey> = locked
            .warm_pool
            .keys()
            .filter(|k| k.provider_id == provider_id)
            .cloned()
            .collect();
          for k in keys_to_close {
            locked.prewarm_failed.remove(&k);
            if let Some(entry) = locked.warm_pool.remove(&k) {
              Self::close_warm_entry(app, entry);
            }
          }
          if active_key_for_provider.is_some() {
            let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
            self.hide_foreground_slot(app, next_seq, &mut locked).await;
            active_key_for_provider
          } else {
            Self::apply_desktop_layout_locked(app, &locked);
            None
          }
        } else {
          if name_changed {
            if let Some(ref ak) = active_key_for_provider {
              if let Some(acc) = locked
                .store
                .accounts
                .iter()
                .find(|a| a.provider_id == ak.provider_id && a.uuid == ak.uuid)
              {
                if let Some(main_win) = app.get_window("main") {
                  let _ = main_win.set_title(&format!("{name} · {}", acc.alias));
                }
              }
            }
          }
          None
        }
      }
      #[cfg(not(desktop))]
      {
        let _ = (app, url_changed, active_key_for_provider);
        None::<AccountKey>
      }
    };

    if let Some(key) = reopen_key {
      return self.switch_account_internal(app, key, false).await;
    }

    Ok(self.get_snapshot().await)
  }

  pub async fn remove_custom_provider(
    &self,
    app: &AppHandle,
    raw_provider: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    if provider_id.is_builtin() {
      return Err("内置服务不能删除".to_string());
    }

    let (removed_keys, closed_desktop_view) = {
      let mut locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() {
        return Err("存在 UUID 归属冲突，请先处理冲突后再删除服务".to_string());
      }
      let idx = locked
        .store
        .custom_providers
        .iter()
        .position(|p| p.id == provider_id)
        .ok_or_else(|| "未找到对应的自定义服务".to_string())?;

      let keys_for_provider: Vec<AccountKey> = locked
        .store
        .accounts
        .iter()
        .filter(|a| a.provider_id == provider_id)
        .map(|a| AccountKey {
          provider_id: a.provider_id.clone(),
          uuid: a.uuid.clone(),
        })
        .collect();

      let active_matches = locked
        .active_slot
        .as_ref()
        .is_some_and(|s| s.account_key.provider_id == provider_id);
      if active_matches {
        let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.hide_foreground_slot(app, next_seq, &mut locked).await;
      }

      locked.store.custom_providers.remove(idx);
      locked.store.accounts.retain(|a| a.provider_id != provider_id);
      locked.last_active_per_provider.remove(&provider_id);
      if locked
        .store
        .last_committed
        .as_ref()
        .is_some_and(|k| k.provider_id == provider_id)
      {
        locked.store.last_committed = None;
      }
      for key in &keys_for_provider {
        if !locked.store.pending_cleanup.contains(key) {
          locked.store.pending_cleanup.push(key.clone());
        }
      }
      if locked.selected_provider_id == provider_id {
        locked.selected_provider_id = ProviderId::muse();
      }
      locked.conflicts = detect_conflicts(&locked.store.accounts);
      save_store_atomic(&locked.store_path, &locked.store)?;

      #[cfg(desktop)]
      let closed = {
        let mut any_closed = false;
        for key in &keys_for_provider {
          locked.prewarm_failed.remove(key);
          locked.cookie_checked_at.remove(key);
          if let Some(entry) = locked.warm_pool.remove(key) {
            Self::close_warm_entry(app, entry);
            any_closed = true;
          }
        }
        if any_closed {
          Self::apply_desktop_layout_locked(app, &locked);
        }
        any_closed
      };
      #[cfg(not(desktop))]
      let closed = false;

      (keys_for_provider, closed)
    };

    if closed_desktop_view {
      tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }

    for key in removed_keys {
      if self.delete_profile_for_key(app, &key.uuid).await.is_ok() {
        let mut locked = self.inner.lock().await;
        locked.store.pending_cleanup.retain(|k| k != &key);
        let _ = save_store_atomic(&locked.store_path, &locked.store);
      }
    }

    Ok(self.get_snapshot().await)
  }

  pub async fn add_account(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_alias: &str,
    raw_proxy_url: Option<&str>,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let trimmed = raw_alias.trim();
    let (alias, auto_alias) = if trimmed.is_empty() {
      ("待登录账号".to_string(), true)
    } else {
      (trimmed.to_string(), false)
    };
    let proxy_url = platform::parse_account_proxy(raw_proxy_url.unwrap_or_default())?
      .map(|url| platform::format_account_proxy(&url));

    let created_key = {
      let mut locked = self.inner.lock().await;
      let spec = resolve_provider(&locked.store.custom_providers, &provider_id)
        .ok_or_else(|| "未找到对应的服务配置".to_string())?;
      if !spec.info.enabled {
        return Err(format!("{} 当前待接入，暂不支持添加内嵌账号", spec.info.name));
      }
      if !locked.conflicts.is_empty() {
        return Err("请先处理 UUID 归属冲突后再新增账号".to_string());
      }

      let uuid = loop {
        let candidate = Uuid::new_v4().hyphenated().to_string();
        let used_in_accounts = locked.store.accounts.iter().any(|a| a.uuid == candidate);
        let used_in_cleanup = locked.store.pending_cleanup.iter().any(|k| k.uuid == candidate);
        if !used_in_accounts && !used_in_cleanup {
          break candidate;
        }
      };

      locked.store.accounts.push(AccountRecord {
        provider_id: provider_id.clone(),
        uuid: uuid.clone(),
        alias,
        created_at_ms: now_ms(),
        auto_alias,
        proxy_url,
        avatar_url: None,
        quota_text: None,
        quota_percent: None,
        quota_detail: None,
        auth_state: AuthState::LoggedOut,
        cookie_expires_at_ms: None,
        cookie_session_only: false,
      });
      if let Err(err) = save_store_atomic(&locked.store_path, &locked.store) {
        locked.store.accounts.pop();
        return Err(err);
      }
      locked.selected_provider_id = provider_id.clone();
      AccountKey { provider_id, uuid }
    };

    self.switch_account_internal(app, created_key, false).await
  }

  pub async fn test_account_proxy(
    &self,
    raw_provider: &str,
    raw_proxy_url: &str,
  ) -> Result<ProxyTestResult, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let entry_url = {
      let locked = self.inner.lock().await;
      resolve_provider(&locked.store.custom_providers, &provider_id)
        .ok_or_else(|| "未找到对应的服务配置".to_string())?
        .info
        .entry_url
    };
    platform::test_account_proxy(&entry_url, raw_proxy_url).await
  }

  pub async fn update_account_settings(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
    raw_alias: &str,
    raw_proxy_url: Option<&str>,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let alias = raw_alias.trim();
    if alias.is_empty() && (raw_proxy_url.is_none() || !platform::SUPPORTS_ACCOUNT_PROXY) {
      return Err("账号别名不能为空".to_string());
    }
    let proxy_update = raw_proxy_url.map(platform::parse_account_proxy).transpose()?;

    let mut locked = self.inner.lock().await;
    let spec = resolve_provider(&locked.store.custom_providers, &provider_id)
      .ok_or_else(|| "未找到对应的服务配置".to_string())?;
    let index = locked
      .store
      .accounts
      .iter()
      .position(|a| a.provider_id == provider_id && a.uuid == uuid)
      .ok_or_else(|| "未找到匹配的账号记录".to_string())?;
    let mut updated = locked.store.accounts[index].clone();
    if !alias.is_empty() {
      updated.alias = alias.to_string();
      updated.auto_alias = false;
    }
    if let Some(proxy) = proxy_update {
      updated.proxy_url = proxy.map(|url| platform::format_account_proxy(&url));
    }
    let proxy_changed = updated.proxy_url != locked.store.accounts[index].proxy_url;
    if proxy_changed && !locked.conflicts.is_empty() {
      return Err("请先处理 UUID 归属冲突后再修改代理".to_string());
    }
    if updated == locked.store.accounts[index] {
      return Ok(Self::snapshot_from_locked(&locked));
    }
    let previous = std::mem::replace(&mut locked.store.accounts[index], updated);
    if let Err(err) = save_store_atomic(&locked.store_path, &locked.store) {
      locked.store.accounts[index] = previous;
      return Err(err);
    }

    #[cfg(any(target_os = "macos", windows))]
    if proxy_changed {
      let key = AccountKey {
        provider_id: provider_id.clone(),
        uuid: uuid.clone(),
      };
      let was_active = locked.active_slot.as_ref().is_some_and(|slot| slot.account_key == key);
      locked.prewarm_failed.remove(&key);
      if let Some(entry) = locked.warm_pool.remove(&key) {
        Self::close_warm_entry(app, entry);
      }
      if was_active {
        let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.hide_foreground_slot(app, next_seq, &mut locked).await;
        locked.notice = None;
        let account = locked.store.accounts[index].clone();
        let app_data_dir = locked.app_data_dir.clone();
        let view_instance_id = self.view_counter.fetch_add(1, Ordering::SeqCst);
        drop(locked);
        return self
          .open_desktop_session(app, account, app_data_dir, next_seq, view_instance_id, false)
          .await;
      }
    }

    if let Some(slot) = &locked.active_slot {
      if slot.committed && slot.account_key.provider_id == provider_id && slot.account_key.uuid == uuid {
        let title = format!("{} · {}", spec.info.short_name, locked.store.accounts[index].alias);
        #[cfg(desktop)]
        if let Some(main_win) = app.get_window("main") {
          let _ = main_win.set_title(&title);
        }
        #[cfg(target_os = "android")]
        {
          let _ = crate::platform::android::update_account_title(app, slot.switch_seq, &title).await;
        }
      }
    }

    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn copy_account_cookie(
    &self,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
  ) -> Result<(), String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey {
      provider_id: provider_id.clone(),
      uuid,
    };
    let locked = self.inner.lock().await;
    if !locked.conflicts.is_empty() {
      return Err("存在 UUID 归属冲突，请先完成归属裁决".to_string());
    }
    if locked.store.pending_cleanup.contains(&key)
      || !locked.store.accounts.iter().any(|account| {
        account.provider_id == key.provider_id && account.uuid == key.uuid
      })
    {
      return Err("账号归属核对失败，未找到可用的账号".to_string());
    }
    let spec = resolve_provider(&locked.store.custom_providers, &provider_id)
      .ok_or_else(|| "未找到对应的服务配置".to_string())?;
    if !spec.info.enabled {
      return Err("该服务尚未开放应用内嵌".to_string());
    }
    let url = Url::parse(&spec.info.entry_url).map_err(|_| "服务入口地址无效".to_string())?;

    #[cfg(target_os = "macos")]
    {
      drop(locked);
      let uuid = Uuid::parse_str(&key.uuid).map_err(|_| "账号资料标识无效".to_string())?;
      let (header, summary) = platform::read_macos_cookies_for_url(app, uuid, url).await?;
      self.apply_cookie_expiry_summary(app, &key, summary).await;
      platform::write_cookie_clipboard(app, header).await
    }
    #[cfg(windows)]
    {
      let res = self.read_windows_cookies_for_url(app, &locked, &key, url).await;
      drop(locked);
      let (header, summary) = res?;
      self.apply_cookie_expiry_summary(app, &key, summary).await;
      platform::write_cookie_clipboard(app, header).await
    }
    #[cfg(target_os = "android")]
    {
      drop(locked);
      platform::android::copy_profile_cookie(app, &key.uuid, url.as_str()).await
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "android")))]
    {
      drop(locked);
      let _ = (app, url);
      Err("当前平台不支持复制 Cookie".to_string())
    }
  }

  #[cfg(windows)]
  async fn read_windows_webview_cookies(
    webview: tauri::Webview,
    url: Url,
  ) -> Result<(String, platform::CookieExpirySummary), String> {
    tauri::async_runtime::spawn_blocking(move || {
      let current_ms = now_ms();
      let cookies = webview
        .cookies_for_url(url)
        .map_err(|_| "读取账号 Cookie 失败".to_string())?;
      let mut pairs = Vec::with_capacity(cookies.len());
      let mut metas = Vec::with_capacity(cookies.len());
      for cookie in &cookies {
        let (session_only, expires_at_ms) = match cookie.expires_datetime() {
          None => (true, None),
          Some(dt) => match platform::unix_secs_i64_to_ms(dt.unix_timestamp()) {
            Some(exp) if exp > current_ms => (false, Some(exp)),
            _ => continue,
          },
        };
        let name = cookie.name().to_string();
        metas.push(platform::CookieMeta {
          name: name.clone(),
          http_only: cookie.http_only().unwrap_or(false),
          session_only,
          expires_at_ms,
        });
        pairs.push(format!("{}={}", cookie.name(), cookie.value()));
      }
      let header = pairs.join("; ");
      let summary = platform::summarize_cookie_expiry(&metas, current_ms);
      Ok((header, summary))
    })
    .await
    .map_err(|_| "读取账号 Cookie 中断".to_string())?
  }

  #[cfg(windows)]
  async fn read_windows_cookies_for_url(
    &self,
    app: &AppHandle,
    locked: &InnerState,
    key: &AccountKey,
    url: Url,
  ) -> Result<(String, platform::CookieExpirySummary), String> {
    let profile_dir = locked.app_data_dir.join("profiles").join(&key.uuid);
    let uuid = Uuid::parse_str(&key.uuid).map_err(|_| "账号资料标识无效".to_string())?;
    let existing = locked.warm_pool.get(key).and_then(|entry| app.get_webview(&entry.webview_label));
    let temporary = existing.is_none();
    let webview = match existing {
      Some(webview) => webview,
      None => {
        if !profile_dir.is_dir() {
          return Err("该账号暂无可复制的 Cookie，请先登录".to_string());
        }
        let account = locked.store.accounts.iter().find(|account| {
          account.provider_id == key.provider_id && account.uuid == key.uuid
        }).ok_or_else(|| "未找到可用的账号".to_string())?;
        let proxy = platform::parse_account_proxy(account.proxy_url.as_deref().unwrap_or_default())?;
        let window = app.get_window("main").ok_or_else(|| "主窗口不存在".to_string())?;
        let instance_id = self.view_counter.fetch_add(1, Ordering::SeqCst);
        let builder = tauri::webview::WebviewBuilder::new(
          format!("cookie-{instance_id}"),
          tauri::WebviewUrl::External("about:blank".parse().expect("valid blank url")),
        )
        .data_directory(profile_dir.clone())
        .on_navigation(|url| url.as_str() == "about:blank")
        .focused(false);
        let builder =
          platform::with_windows_account_proxy(builder, proxy.as_ref(), &app.config().identifier)?;
        let (position, size) = Self::initial_hidden_child_rect(&window);
        let webview = window.add_child(builder, position, size)
          .map_err(|_| "无法打开该账号的会话资料，请重试".to_string())?;
        let _ = webview.hide();
        webview
      }
    };

    let result = async {
      platform::verify_desktop_profile_binding(&webview, uuid, &profile_dir).await?;
      Self::read_windows_webview_cookies(webview.clone(), url).await
    }.await;
    if temporary {
      let _ = webview.close();
    }
    result
  }

  pub async fn remove_account(
    &self,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey {
      provider_id: provider_id.clone(),
      uuid: uuid.clone(),
    };

    let closed_desktop_view = {
      let mut locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() {
        return Err("存在 UUID 归属冲突，请先完成归属裁决".to_string());
      }
      let idx = locked
        .store
        .accounts
        .iter()
        .position(|a| a.provider_id == provider_id && a.uuid == uuid)
        .ok_or_else(|| "账号归属核对失败，未找到该账号".to_string())?;

      locked.store.accounts.remove(idx);
      if locked.store.last_committed.as_ref() == Some(&key) {
        locked.store.last_committed = None;
      }
      if locked.last_active_per_provider.get(&provider_id) == Some(&key) {
        locked.last_active_per_provider.remove(&provider_id);
      }
      if !locked.store.pending_cleanup.contains(&key) {
        locked.store.pending_cleanup.push(key.clone());
      }
      save_store_atomic(&locked.store_path, &locked.store)?;

      let active_matches = locked
        .active_slot
        .as_ref()
        .map(|s| s.account_key == key)
        .unwrap_or(false);
      if active_matches {
        let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.hide_foreground_slot(app, next_seq, &mut locked).await;
      }

      #[cfg(desktop)]
      {
        locked.prewarm_failed.remove(&key);
        locked.cookie_checked_at.remove(&key);
        if let Some(warm_entry) = locked.warm_pool.remove(&key) {
          Self::close_warm_entry(app, warm_entry);
          Self::apply_desktop_layout_locked(app, &locked);
          true
        } else {
          false
        }
      }
      #[cfg(not(desktop))]
      {
        false
      }
    };

    if closed_desktop_view {
      tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }

    if self.delete_profile_for_key(app, &uuid).await.is_ok() {
      let mut locked = self.inner.lock().await;
      locked.store.pending_cleanup.retain(|k| k != &key);
      let _ = save_store_atomic(&locked.store_path, &locked.store);
    }

    Ok(self.get_snapshot().await)
  }

  pub async fn reorder_accounts(
    &self,
    raw_provider: &str,
    ordered_uuids: Vec<String>,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let mut locked = self.inner.lock().await;
    if !locked.conflicts.is_empty() {
      return Err("存在 UUID 归属冲突，请先处理冲突后再调整顺序".to_string());
    }

    let slot_indices: Vec<usize> = locked
      .store
      .accounts
      .iter()
      .enumerate()
      .filter_map(|(idx, acc)| (acc.provider_id == provider_id).then_some(idx))
      .collect();

    if ordered_uuids.len() != slot_indices.len() {
      return Err("账号数量变化，请刷新后重试".to_string());
    }

    let existing_by_uuid: HashMap<String, usize> = slot_indices
      .iter()
      .map(|&idx| (locked.store.accounts[idx].uuid.clone(), idx))
      .collect();

    let mut seen = HashSet::with_capacity(ordered_uuids.len());
    let mut reordered = Vec::with_capacity(ordered_uuids.len());
    for raw_uuid in ordered_uuids {
      let uuid = normalize_uuid(&raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
      if !seen.insert(uuid.clone()) {
        return Err("存在重复的账号排序项".to_string());
      }
      let &src_idx = existing_by_uuid
        .get(&uuid)
        .ok_or_else(|| "账号列表已变更，请刷新后重试".to_string())?;
      reordered.push(locked.store.accounts[src_idx].clone());
    }

    let unchanged = slot_indices
      .iter()
      .zip(reordered.iter())
      .all(|(&idx, record)| locked.store.accounts[idx].uuid == record.uuid);
    if unchanged {
      return Ok(Self::snapshot_from_locked(&locked));
    }

    let previous = locked.store.accounts.clone();
    for (slot_idx, record) in slot_indices.into_iter().zip(reordered.into_iter()) {
      locked.store.accounts[slot_idx] = record;
    }
    if let Err(err) = save_store_atomic(&locked.store_path, &locked.store) {
      locked.store.accounts = previous;
      return Err(err);
    }

    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn reload_account(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey {
      provider_id: provider_id.clone(),
      uuid: uuid.clone(),
    };

    #[cfg(desktop)]
    {
      let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
      let action = {
        let mut locked = self.inner.lock().await;
        let Some(acc) = locked
          .store
          .accounts
          .iter()
          .find(|a| a.provider_id == provider_id && a.uuid == uuid)
          .cloned()
        else {
          return Err("未找到对应的账号记录".to_string());
        };
        let spec = resolve_provider(&locked.store.custom_providers, &provider_id)
          .ok_or_else(|| "未找到对应的服务配置".to_string())?;
        if !spec.info.enabled {
          return Err("该服务尚未开放应用内嵌".to_string());
        }
        let was_active = locked
          .active_slot
          .as_ref()
          .is_some_and(|s| s.account_key == key);
        locked.prewarm_failed.remove(&key);
        if let Some(entry) = locked.warm_pool.remove(&key) {
          Self::close_warm_entry(app, entry);
        }
        if was_active {
          self.hide_foreground_slot(app, next_seq, &mut locked).await;
          let app_data_dir = locked.app_data_dir.clone();
          let vid = self.view_counter.fetch_add(1, Ordering::SeqCst);
          Some((acc, app_data_dir, vid))
        } else {
          Self::apply_desktop_layout_locked(app, &locked);
          None
        }
      };

      if let Some((account, app_data_dir, vid)) = action {
        return self
          .open_desktop_session(app, account, app_data_dir, next_seq, vid, false)
          .await;
      }
      return Ok(self.get_snapshot().await);
    }

    #[cfg(not(desktop))]
    {
      let _ = app;
      let locked = self.inner.lock().await;
      if !locked
        .store
        .accounts
        .iter()
        .any(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
      {
        return Err("未找到对应的账号记录".to_string());
      }
      Ok(Self::snapshot_from_locked(&locked))
    }
  }

  pub async fn clear_account_data(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey {
      provider_id: provider_id.clone(),
      uuid: uuid.clone(),
    };

    let (was_active, closed_desktop_view) = {
      let mut locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() {
        return Err("存在 UUID 归属冲突，请先完成归属裁决".to_string());
      }
      let idx = locked
        .store
        .accounts
        .iter()
        .position(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        .ok_or_else(|| "未找到对应的账号记录".to_string())?;

      let active_matches = locked
        .active_slot
        .as_ref()
        .is_some_and(|s| s.account_key == key);
      if active_matches {
        let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.hide_foreground_slot(app, next_seq, &mut locked).await;
      }

      locked.store.accounts[idx].auth_state = AuthState::LoggedOut;
      locked.store.accounts[idx].quota_detail = None;
      locked.store.accounts[idx].quota_text = None;
      locked.store.accounts[idx].quota_percent = None;
      locked.store.accounts[idx].avatar_url = None;
      locked.store.accounts[idx].cookie_expires_at_ms = None;
      locked.store.accounts[idx].cookie_session_only = false;
      if !locked.store.pending_cleanup.contains(&key) {
        locked.store.pending_cleanup.push(key.clone());
      }
      save_store_atomic(&locked.store_path, &locked.store)?;

      #[cfg(desktop)]
      let closed = {
        locked.prewarm_failed.remove(&key);
        locked.cookie_checked_at.remove(&key);
        if let Some(entry) = locked.warm_pool.remove(&key) {
          Self::close_warm_entry(app, entry);
          Self::apply_desktop_layout_locked(app, &locked);
          true
        } else {
          false
        }
      };
      #[cfg(not(desktop))]
      let closed = false;

      (active_matches, closed)
    };

    if closed_desktop_view {
      tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    match self.delete_profile_for_key(app, &uuid).await {
      Ok(()) => {
        let snap = {
          let mut locked = self.inner.lock().await;
          locked.store.pending_cleanup.retain(|k| k != &key);
          save_store_atomic(&locked.store_path, &locked.store)?;
          Self::snapshot_from_locked(&locked)
        };
        if was_active {
          return self.switch_account_internal(app, key, false).await;
        }
        Ok(snap)
      }
      Err(err) => {
        let snap = self.get_snapshot().await;
        self.emit_snapshot(app, &snap);
        Err(err)
      }
    }
  }

  pub async fn update_app_settings(
    &self,
    app: &AppHandle,
    minimize_to_tray: bool,
    raw_shortcut: &str,
  ) -> Result<ManagerSnapshot, String> {
    let shortcut = raw_shortcut.trim().to_string();
    if shortcut.chars().count() > 80 {
      return Err("全局快捷键长度过长".to_string());
    }

    #[cfg(desktop)]
    let (global_shortcut_active, global_shortcut_error) = {
      let (old_shortcut, old_active) = {
        let locked = self.inner.lock().await;
        (
          locked.store.settings.global_shortcut.clone(),
          locked.global_shortcut_active,
        )
      };
      match Self::apply_global_shortcut_on(app, &shortcut).await {
        Ok(active) => (active, None),
        Err(err) => {
          if old_active && !old_shortcut.trim().is_empty() {
            let _ = Self::apply_global_shortcut_on(app, &old_shortcut).await;
          }
          return Err(err);
        }
      }
    };

    #[cfg(not(desktop))]
    let (global_shortcut_active, global_shortcut_error) = {
      let _ = app;
      (false, None)
    };

    let mut locked = self.inner.lock().await;
    let previous = locked.store.settings.clone();
    locked.store.settings.minimize_to_tray = minimize_to_tray;
    locked.store.settings.global_shortcut = shortcut;
    if let Err(err) = save_store_atomic(&locked.store_path, &locked.store) {
      locked.store.settings = previous;
      return Err(err);
    }
    self.minimize_to_tray.store(minimize_to_tray, Ordering::SeqCst);
    locked.global_shortcut_active = global_shortcut_active;
    locked.global_shortcut_error = global_shortcut_error;
    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn reveal_downloaded_file(&self, download_id: u64) -> Result<(), String> {
    #[cfg(desktop)]
    {
      let path = self
        .recent_downloads
        .lock()
        .unwrap()
        .get(&download_id)
        .cloned()
        .ok_or_else(|| "未找到对应的下载文件记录".to_string())?;
      platform::reveal_path_in_file_manager(&path)
    }
    #[cfg(not(desktop))]
    {
      let _ = download_id;
      Err("当前平台不支持打开本地文件夹".to_string())
    }
  }

  pub async fn set_modal_active(
    &self,
    app: &AppHandle,
    active: bool,
  ) -> Result<ManagerSnapshot, String> {
    let mut locked = self.inner.lock().await;
    locked.modal_active = active;
    #[cfg(desktop)]
    Self::apply_desktop_layout_locked(app, &locked);
    #[cfg(not(desktop))]
    let _ = app;
    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn open_official_site(
    &self,
    app: &AppHandle,
    raw_provider: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let homepage = {
      let locked = self.inner.lock().await;
      resolve_provider(&locked.store.custom_providers, &provider_id)
        .ok_or_else(|| "未找到对应的服务配置".to_string())?
        .info
        .homepage_url
    };

    #[cfg(desktop)]
    {
      let _ = app;
      platform::open_external_browser(&homepage)?;
    }
    #[cfg(target_os = "android")]
    {
      crate::platform::android::open_external_url(app, &homepage).await?;
    }

    Ok(self.get_snapshot().await)
  }

  pub async fn resolve_uuid_conflict(
    &self,
    keep_provider_raw: &str,
    raw_uuid: &str,
  ) -> Result<ManagerSnapshot, String> {
    let keep_provider =
      ProviderId::parse(keep_provider_raw).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;

    let mut locked = self.inner.lock().await;
    let mut kept_first = false;
    for acc in &mut locked.store.accounts {
      if acc.uuid == uuid {
        if acc.provider_id == keep_provider && !kept_first {
          kept_first = true;
        } else {
          acc.uuid = Uuid::new_v4().hyphenated().to_string();
        }
      }
    }
    locked.conflicts = detect_conflicts(&locked.store.accounts);
    if locked.conflicts.is_empty() {
      locked.notice = None;
    }
    save_store_atomic(&locked.store_path, &locked.store)?;
    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn dismiss_notice(&self) -> ManagerSnapshot {
    let mut locked = self.inner.lock().await;
    locked.notice = None;
    Self::snapshot_from_locked(&locked)
  }

  pub async fn switch_account(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id = ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey { provider_id, uuid };
    self.switch_account_internal(app, key, false).await
  }

  pub async fn switch_account_by_index(
    self: &Arc<Self>,
    app: &AppHandle,
    index_1_based: u8,
  ) -> Result<ManagerSnapshot, String> {
    if index_1_based == 0 {
      return Ok(self.get_snapshot().await);
    }
    let (provider_id, target_uuid) = {
      let locked = self.inner.lock().await;
      let pid = locked.selected_provider_id.clone();
      let accounts: Vec<_> = locked
        .store
        .accounts
        .iter()
        .filter(|a| a.provider_id == pid)
        .collect();
      let idx = (index_1_based - 1) as usize;
      if let Some(acc) = accounts.get(idx) {
        (pid, acc.uuid.clone())
      } else {
        return Ok(Self::snapshot_from_locked(&locked));
      }
    };
    self.switch_account(app, provider_id.as_str(), &target_uuid).await
  }

  pub async fn trigger_silent_sync(self: &Arc<Self>, app: &AppHandle, force: bool) {
    let now = now_ms();
    if !force {
      let last = self.last_sync_timestamp.load(Ordering::SeqCst);
      if now > last && now - last < 30_000 {
        return;
      }
    }
    self.last_sync_timestamp.store(now, Ordering::SeqCst);

    #[cfg(not(desktop))]
    {
      let _ = app;
    }

    #[cfg(desktop)]
    {
      let warm_targets = {
        let mut locked = self.inner.lock().await;
        if force {
          locked.prewarm_failed.clear();
        }
        locked
          .warm_pool
          .iter()
          .map(|(k, v)| (k.clone(), v.webview_label.clone(), v.view_instance_id))
          .collect::<Vec<_>>()
      };

      #[cfg(target_os = "macos")]
      let warm_set: HashSet<AccountKey> = warm_targets.iter().map(|(k, _, _)| k.clone()).collect();

      for (key, wv_label, vid) in warm_targets {
        if let Some(wv) = app.get_webview(&wv_label) {
          let _ = wv.eval(r#"(() => {
            try {
              if (typeof window.__maamRefreshBackendQuota === 'function') {
                window.__maamRefreshBackendQuota(true);
              }
            } catch (_) {}
          })()"#);

          let mgr = Arc::clone(self);
          let app_handle = app.clone();
          tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(850)).await;
            if let Some(probe) = Self::eval_identity_probe(&wv).await {
              if probe.is_auth {
                let resolved_identity = if !probe.email.is_empty() {
                  Some(probe.email)
                } else if !probe.typed.is_empty() {
                  Some(probe.typed)
                } else {
                  None
                };
                let (just_authenticated, _) = mgr
                  .apply_detected_metadata(
                    &app_handle,
                    key.clone(),
                    vid,
                    resolved_identity,
                    if probe.avatar.is_empty() {
                      None
                    } else {
                      Some(probe.avatar)
                    },
                    if probe.quota_text.is_empty() {
                      None
                    } else {
                      Some(probe.quota_text)
                    },
                    probe.quota_pct,
                    probe.quota_detail,
                  )
                  .await;
                mgr
                  .refresh_account_cookie_expiry(&app_handle, key, force || just_authenticated)
                  .await;
              } else if probe.logged_out {
                mgr.apply_logged_out_state(&app_handle, key, vid).await;
              }
            }
          });
        }
      }

      #[cfg(target_os = "macos")]
      {
        let cold_auth_keys: Vec<AccountKey> = {
          let locked = self.inner.lock().await;
          locked
            .store
            .accounts
            .iter()
            .filter(|a| a.auth_state == AuthState::Authenticated)
            .map(|a| AccountKey {
              provider_id: a.provider_id.clone(),
              uuid: a.uuid.clone(),
            })
            .filter(|k| !warm_set.contains(k))
            .collect()
        };
        for key in cold_auth_keys {
          let mgr_cold = Arc::clone(self);
          let app_cold = app.clone();
          tauri::async_runtime::spawn(async move {
            mgr_cold.refresh_account_cookie_expiry(&app_cold, key, force).await;
          });
        }
      }

      let mgr_prewarm = Arc::clone(self);
      let app_prewarm = app.clone();
      tauri::async_runtime::spawn(async move {
        mgr_prewarm.prewarm_next_cold_account(&app_prewarm).await;
      });
    }
  }

  async fn switch_account_internal(
    self: &Arc<Self>,
    app: &AppHandle,
    key: AccountKey,
    is_cold_start: bool,
  ) -> Result<ManagerSnapshot, String> {
    {
      let locked = self.inner.lock().await;
      if let Some(slot) = &locked.active_slot {
        if slot.committed && slot.account_key == key && !is_cold_start {
          #[cfg(desktop)]
          return Ok(Self::snapshot_from_locked(&locked));
        }
      }
    }

    let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;

    #[cfg(desktop)]
    {
      enum DesktopSwitchPlan {
        InstantWarm {
          view_instance_id: u64,
          need_identity_watcher: bool,
          probe_notify: Arc<tokio::sync::Notify>,
        },
        #[cfg(target_os = "macos")]
        StageWarmPaint {
          view_instance_id: u64,
          expected_load_gen: u64,
          loaded_url: Url,
          loading_snap: ManagerSnapshot,
        },
        AwaitWarmLoading {
          view_instance_id: u64,
        },
        CreateCold {
          account: AccountRecord,
          app_data_dir: PathBuf,
          view_instance_id: u64,
        },
        Abort(ManagerSnapshot),
      }

      let plan = {
        let mut locked = self.inner.lock().await;
        if !locked.conflicts.is_empty() {
          return Err("存在 UUID 归属冲突，已停留在管理页".to_string());
        }
        let spec = resolve_provider(&locked.store.custom_providers, &key.provider_id)
          .ok_or_else(|| "未找到对应的服务配置".to_string())?;
        if !spec.info.enabled {
          return Err(format!("{} 尚未开放应用内嵌", spec.info.name));
        }
        if locked.store.pending_cleanup.contains(&key) {
          self.hide_foreground_slot(app, next_seq, &mut locked).await;
          locked.notice = Some("该账号正在清理本地资料，请稍后重试或重新创建".to_string());
          DesktopSwitchPlan::Abort(Self::snapshot_from_locked(&locked))
        } else {
          let Some(acc) = locked
            .store
            .accounts
            .iter()
            .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
            .cloned()
          else {
            self.hide_foreground_slot(app, next_seq, &mut locked).await;
            locked.notice = Some("账号归属核对失败，已回到管理页".to_string());
            return Ok(Self::snapshot_from_locked(&locked));
          };

          locked.selected_provider_id = key.provider_id.clone();
          locked.notice = None;
          locked.prewarm_failed.remove(&key);
          self.hide_foreground_slot(app, next_seq, &mut locked).await;

          let warm_state = locked.warm_pool.get(&key).and_then(|entry| {
            if app.get_webview(&entry.webview_label).is_some() {
              Some((
                entry.view_instance_id,
                entry.paint_ready,
                entry.staged_painted,
                entry.watcher_running,
                Arc::clone(&entry.probe_notify),
                Arc::clone(&entry.load_gen),
                entry.last_url.clone(),
              ))
            } else {
              None
            }
          });

          if let Some((
            vid,
            paint_ready,
            staged_painted,
            watcher_running,
            probe_notify,
            load_gen,
            last_url,
          )) = warm_state
          {
            #[cfg(not(target_os = "macos"))]
            let _ = (staged_painted, &load_gen, &last_url);
            #[cfg(target_os = "macos")]
            let needs_stage_paint = paint_ready && !staged_painted;
            #[cfg(not(target_os = "macos"))]
            let needs_stage_paint = false;

            if needs_stage_paint {
              #[cfg(target_os = "macos")]
              {
                if let Some(entry) = locked.warm_pool.get_mut(&key) {
                  entry.last_used_ms = now_ms();
                }
                locked.active_slot = Some(ActiveSessionSlot {
                  account_key: key.clone(),
                  view_instance_id: vid,
                  switch_seq: next_seq,
                  committed: false,
                });
                self.active_view_id.store(vid, Ordering::SeqCst);
                Self::apply_desktop_layout_locked(app, &locked);
                let fallback_url = Url::parse(&spec.info.entry_url)
                  .unwrap_or_else(|_| Url::parse("about:blank").expect("valid blank url"));
                DesktopSwitchPlan::StageWarmPaint {
                  view_instance_id: vid,
                  expected_load_gen: load_gen.load(Ordering::SeqCst),
                  loaded_url: last_url.unwrap_or(fallback_url),
                  loading_snap: Self::snapshot_from_locked(&locked),
                }
              }
              #[cfg(not(target_os = "macos"))]
              unreachable!()
            } else if paint_ready {
              let prev_last = locked.store.last_committed.clone();
              locked.store.last_committed = Some(key.clone());
              if let Err(err) = save_store_atomic(&locked.store_path, &locked.store) {
                locked.store.last_committed = prev_last;
                locked.notice = Some(format!("提交账号记录写盘失败: {err}"));
                DesktopSwitchPlan::Abort(Self::snapshot_from_locked(&locked))
              } else {
                locked
                  .last_active_per_provider
                  .insert(key.provider_id.clone(), key.clone());
                locked.active_slot = Some(ActiveSessionSlot {
                  account_key: key.clone(),
                  view_instance_id: vid,
                  switch_seq: next_seq,
                  committed: true,
                });
                self.active_view_id.store(vid, Ordering::SeqCst);
                let title = format!("{} · {}", spec.info.short_name, acc.alias);
                if let Some(main_win) = app.get_window("main") {
                  let _ = main_win.set_title(&title);
                }
                let need_watcher = !watcher_running;
                if let Some(entry) = locked.warm_pool.get_mut(&key) {
                  entry.last_used_ms = now_ms();
                  if need_watcher {
                    entry.watcher_running = true;
                  }
                }
                Self::apply_desktop_layout_locked(app, &locked);
                DesktopSwitchPlan::InstantWarm {
                  view_instance_id: vid,
                  need_identity_watcher: need_watcher,
                  probe_notify,
                }
              }
            } else {
              if let Some(entry) = locked.warm_pool.get_mut(&key) {
                entry.last_used_ms = now_ms();
              }
              locked.active_slot = Some(ActiveSessionSlot {
                account_key: key.clone(),
                view_instance_id: vid,
                switch_seq: next_seq,
                committed: false,
              });
              self.active_view_id.store(vid, Ordering::SeqCst);
              Self::apply_desktop_layout_locked(app, &locked);
              DesktopSwitchPlan::AwaitWarmLoading {
                view_instance_id: vid,
              }
            }
          } else {
            if let Some(entry) = locked.warm_pool.remove(&key) {
              Self::close_warm_entry(app, entry);
            }
            let vid = self.view_counter.fetch_add(1, Ordering::SeqCst);
            let dir = locked.app_data_dir.clone();
            DesktopSwitchPlan::CreateCold {
              account: acc,
              app_data_dir: dir,
              view_instance_id: vid,
            }
          }
        }
      };

      match plan {
        DesktopSwitchPlan::Abort(snap) => Ok(snap),
        DesktopSwitchPlan::InstantWarm {
          view_instance_id,
          need_identity_watcher,
          probe_notify,
        } => {
          if need_identity_watcher {
            self.spawn_desktop_identity_watcher(app.clone(), key, view_instance_id, probe_notify);
          } else {
            probe_notify.notify_one();
          }
          Ok(self.get_snapshot().await)
        }
        #[cfg(target_os = "macos")]
        DesktopSwitchPlan::StageWarmPaint {
          view_instance_id,
          expected_load_gen,
          loaded_url,
          loading_snap,
        } => {
          self.emit_snapshot(app, &loading_snap);
          let mgr_stage = Arc::clone(self);
          let app_stage = app.clone();
          let key_stage = key;
          tauri::async_runtime::spawn(async move {
            mgr_stage
              .wait_paint_and_finalize_desktop(
                &app_stage,
                key_stage,
                view_instance_id,
                loaded_url,
                expected_load_gen,
                true,
              )
              .await;
          });
          Ok(loading_snap)
        }
        DesktopSwitchPlan::AwaitWarmLoading { view_instance_id } => {
          let _ = view_instance_id;
          Ok(self.get_snapshot().await)
        }
        DesktopSwitchPlan::CreateCold {
          account,
          app_data_dir,
          view_instance_id,
        } => {
          self
            .open_desktop_session(
              app,
              account,
              app_data_dir,
              next_seq,
              view_instance_id,
              is_cold_start,
            )
            .await
        }
      }
    }

    #[cfg(target_os = "android")]
    {
      let view_instance_id = self.view_counter.fetch_add(1, Ordering::SeqCst);
      let account_record = {
        let mut locked = self.inner.lock().await;
        if !locked.conflicts.is_empty() {
          return Err("存在 UUID 归属冲突，已停留在管理页".to_string());
        }
        let spec = resolve_provider(&locked.store.custom_providers, &key.provider_id)
          .ok_or_else(|| "未找到对应的服务配置".to_string())?;
        if !spec.info.enabled {
          return Err(format!("{} 尚未开放应用内嵌", spec.info.name));
        }
        if locked.store.pending_cleanup.contains(&key) {
          self.hide_foreground_slot(app, next_seq, &mut locked).await;
          locked.notice = Some("该账号正在清理本地资料，请稍后重试或重新创建".to_string());
          return Ok(Self::snapshot_from_locked(&locked));
        }
        let Some(acc) = locked
          .store
          .accounts
          .iter()
          .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
          .cloned()
        else {
          self.hide_foreground_slot(app, next_seq, &mut locked).await;
          locked.notice = Some("账号归属核对失败，已回到管理页".to_string());
          return Ok(Self::snapshot_from_locked(&locked));
        };
        platform::parse_account_proxy(acc.proxy_url.as_deref().unwrap_or_default())?;
        locked.selected_provider_id = key.provider_id;
        locked.notice = None;
        self.hide_foreground_slot(app, next_seq, &mut locked).await;
        acc
      };

      self
        .open_android_session(app, account_record, next_seq, view_instance_id, is_cold_start)
        .await
    }
  }

  #[cfg(desktop)]
  fn extract_download_filename(path: Option<&std::path::Path>, url: &Url) -> String {
    if let Some(name) = path
      .and_then(|p| p.file_name())
      .and_then(|s| s.to_str())
      .map(str::trim)
      .filter(|s| !s.is_empty())
    {
      return name.to_string();
    }
    url
      .path_segments()
      .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
      .filter(|s| !s.is_empty())
      .unwrap_or("下载文件")
      .to_string()
  }

  #[cfg(desktop)]
  fn record_recent_download(&self, download_id: u64, path: PathBuf) {
    let mut recent = self.recent_downloads.lock().unwrap();
    recent.insert(download_id, path);
    while recent.len() > 32 {
      if let Some(&oldest_id) = recent.keys().min() {
        recent.remove(&oldest_id);
      } else {
        break;
      }
    }
  }

  #[cfg(desktop)]
  fn handle_desktop_download_event(
    &self,
    app: &AppHandle,
    pending: &std::sync::Mutex<HashMap<String, VecDeque<(u64, PathBuf)>>>,
    event: tauri::webview::DownloadEvent<'_>,
  ) -> bool {
    match event {
      tauri::webview::DownloadEvent::Requested { url, destination } => {
        let download_id = self.download_counter.fetch_add(1, Ordering::SeqCst);
        let filename = Self::extract_download_filename(Some(destination.as_path()), &url);
        pending
          .lock()
          .unwrap()
          .entry(url.as_str().to_string())
          .or_default()
          .push_back((download_id, destination.clone()));
        let _ = app.emit(
          "download-event",
          DownloadNoticeEvent {
            id: download_id,
            filename,
            status: "started",
            error: None,
          },
        );
        true
      }
      tauri::webview::DownloadEvent::Finished { url, path, success } => {
        let entry = {
          let mut map = pending.lock().unwrap();
          let popped = map.get_mut(url.as_str()).and_then(VecDeque::pop_front);
          if map.get(url.as_str()).is_some_and(VecDeque::is_empty) {
            map.remove(url.as_str());
          }
          popped
        };
        let (download_id, fallback_path) = match entry {
          Some((id, dest)) => (id, Some(dest)),
          None => (self.download_counter.fetch_add(1, Ordering::SeqCst), None),
        };
        let final_path = path.or(fallback_path);
        let filename = Self::extract_download_filename(final_path.as_deref(), &url);
        if success {
          if let Some(saved_path) = final_path {
            self.record_recent_download(download_id, saved_path);
          }
          let _ = app.emit(
            "download-event",
            DownloadNoticeEvent {
              id: download_id,
              filename,
              status: "completed",
              error: None,
            },
          );
        } else {
          let _ = app.emit(
            "download-event",
            DownloadNoticeEvent {
              id: download_id,
              filename,
              status: "failed",
              error: Some("下载未完成或已取消".to_string()),
            },
          );
        }
        true
      }
      _ => true,
    }
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  fn build_desktop_account_webview(
    self: &Arc<Self>,
    app: &AppHandle,
    key: AccountKey,
    provider_spec: ProviderSpec,
    parsed_uuid: Uuid,
    profile_dir: &std::path::Path,
    proxy_url: Option<&Url>,
    webview_label: &str,
    view_instance_id: u64,
    allow_blank: Arc<AtomicBool>,
    alive_flag: Arc<AtomicBool>,
    load_gen: Arc<AtomicU64>,
  ) -> tauri::webview::WebviewBuilder<tauri::Wry> {
    let alive_nav = Arc::clone(&alive_flag);
    let mgr_for_nav = Arc::clone(self);
    let app_for_nav = app.clone();
    let allow_blank_nav = Arc::clone(&allow_blank);
    let spec_for_nav = provider_spec.clone();
    let key_for_nav = key.clone();

    let alive_popup = Arc::clone(&alive_flag);
    let active_vid_popup = Arc::clone(&self.active_view_id);
    let mgr_for_popup = Arc::clone(self);
    let app_for_popup = app.clone();
    let spec_for_popup = provider_spec.clone();
    let key_for_popup = key.clone();
    let popup_counter = Arc::new(AtomicU64::new(1));

    let alive_load = Arc::clone(&alive_flag);
    let load_gen_for_load = Arc::clone(&load_gen);
    let mgr_for_load = Arc::clone(self);
    let app_for_load = app.clone();
    let key_for_load = key;
    let handle_page_load = !cfg!(windows) || proxy_url.is_none();

    let pending_downloads: Arc<std::sync::Mutex<HashMap<String, VecDeque<(u64, PathBuf)>>>> =
      Arc::new(std::sync::Mutex::new(HashMap::new()));
    let mgr_for_dl = Arc::clone(self);
    let app_for_dl = app.clone();
    let pending_for_dl = Arc::clone(&pending_downloads);
    let mgr_for_popup_dl = Arc::clone(self);
    let app_for_popup_dl = app.clone();
    let pending_for_popup_dl = Arc::clone(&pending_downloads);

    let provider_host = Url::parse(&provider_spec.info.entry_url)
      .ok()
      .and_then(|u| u.host_str().map(|s| s.to_ascii_lowercase()))
      .unwrap_or_default();
    let provider_init_js = format!(
      "window.__maamProvider = {{ id: {}, host: {}, custom: {} }};",
      serde_json::to_string(provider_spec.info.id.as_str()).unwrap_or_else(|_| "\"\"".to_string()),
      serde_json::to_string(&provider_host).unwrap_or_else(|_| "\"\"".to_string()),
      provider_spec.info.is_custom
    );

    let mut builder = tauri::webview::WebviewBuilder::new(
      webview_label,
      tauri::WebviewUrl::External("about:blank".parse().expect("valid blank url")),
    )
    .focused(false)
    .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
    .initialization_script(&provider_init_js)
    .initialization_script(LOGIN_CAPTURE_INIT_JS)
    .on_navigation(move |url| {
      if !alive_nav.load(Ordering::SeqCst) {
        return false;
      }
      let can_blank = allow_blank_nav.load(Ordering::SeqCst);
      match classify_top_level_url(&spec_for_nav, url, can_blank) {
        NavigationDecision::AllowBlank => true,
        NavigationDecision::AllowHttps => true,
        NavigationDecision::ExternalGoogleOAuth => {
          let mgr_ext = Arc::clone(&mgr_for_nav);
          let app_ext = app_for_nav.clone();
          let key_ext = key_for_nav.clone();
          tauri::async_runtime::spawn(async move {
            mgr_ext
              .notify_external_auth_policy(&app_ext, key_ext, view_instance_id)
              .await;
          });
          false
        }
        NavigationDecision::Deny => false,
      }
    })
    .on_new_window(move |url, features| {
      if !alive_popup.load(Ordering::SeqCst)
        || active_vid_popup.load(Ordering::SeqCst) != view_instance_id
      {
        return tauri::webview::NewWindowResponse::Deny;
      }
      let decision = classify_top_level_url(&spec_for_popup, &url, true);
      match decision {
        NavigationDecision::ExternalGoogleOAuth => {
          let mgr_ext = Arc::clone(&mgr_for_popup);
          let app_ext = app_for_popup.clone();
          let key_ext = key_for_popup.clone();
          tauri::async_runtime::spawn(async move {
            mgr_ext
              .notify_external_auth_policy(&app_ext, key_ext, view_instance_id)
              .await;
          });
          return tauri::webview::NewWindowResponse::Deny;
        }
        NavigationDecision::Deny => {
          return tauri::webview::NewWindowResponse::Deny;
        }
        NavigationDecision::AllowBlank | NavigationDecision::AllowHttps => {}
      }

      let popup_idx = popup_counter.fetch_add(1, Ordering::SeqCst);
      let popup_label = format!("auth-popup-{view_instance_id}-{popup_idx}");
      let popup_label_for_close = popup_label.clone();
      let popup_label_for_slot = popup_label.clone();

      let alive_popup_nav = Arc::clone(&alive_popup);
      let active_vid_popup_nav = Arc::clone(&active_vid_popup);
      let app_popup_nav = app_for_popup.clone();
      let mgr_popup_nav = Arc::clone(&mgr_for_popup);
      let spec_popup_nav = spec_for_popup.clone();
      let key_popup_nav = key_for_popup.clone();

      let popup_title = url
        .host_str()
        .map(|h| format!("登录认证 · {h}"))
        .unwrap_or_else(|| "登录认证".to_string());

      let popup_mgr_dl = Arc::clone(&mgr_for_popup_dl);
      let popup_app_dl = app_for_popup_dl.clone();
      let popup_pending_dl = Arc::clone(&pending_for_popup_dl);

      let popup_builder = tauri::WebviewWindowBuilder::new(
        &app_for_popup,
        &popup_label,
        tauri::WebviewUrl::External("about:blank".parse().expect("valid blank url")),
      )
      .window_features(features)
      .initialization_script(&provider_init_js)
      .initialization_script(LOGIN_CAPTURE_INIT_JS)
      .title(&popup_title)
      .on_download(move |_wv, event| {
        popup_mgr_dl.handle_desktop_download_event(&popup_app_dl, &popup_pending_dl, event)
      })
      .on_navigation(move |nav_url| {
        if !alive_popup_nav.load(Ordering::SeqCst)
          || active_vid_popup_nav.load(Ordering::SeqCst) != view_instance_id
        {
          if let Some(pw) = app_popup_nav.get_webview_window(&popup_label_for_close) {
            let _ = pw.close();
          }
          return false;
        }
        match classify_top_level_url(&spec_popup_nav, nav_url, true) {
          NavigationDecision::AllowBlank => true,
          NavigationDecision::AllowHttps => {
            if let Some(host) = nav_url.host_str() {
              if let Some(pw) = app_popup_nav.get_webview_window(&popup_label_for_close) {
                let _ = pw.set_title(&format!("登录认证 · {host}"));
              }
            }
            true
          }
          NavigationDecision::ExternalGoogleOAuth => {
            if let Some(pw) = app_popup_nav.get_webview_window(&popup_label_for_close) {
              let _ = pw.close();
            }
            let mgr_ext = Arc::clone(&mgr_popup_nav);
            let app_ext = app_popup_nav.clone();
            let key_ext = key_popup_nav.clone();
            tauri::async_runtime::spawn(async move {
              mgr_ext
                .notify_external_auth_policy(&app_ext, key_ext, view_instance_id)
                .await;
            });
            false
          }
          NavigationDecision::Deny => false,
        }
      });

      match popup_builder.build() {
        Ok(popup_window) => {
          let mgr_slot = Arc::clone(&mgr_for_popup);
          let app_slot = app_for_popup.clone();
          let key_slot = key_for_popup.clone();
          tauri::async_runtime::spawn(async move {
            mgr_slot
              .register_desktop_popup(&app_slot, key_slot, view_instance_id, popup_label_for_slot)
              .await;
          });
          tauri::webview::NewWindowResponse::Create {
            window: popup_window,
          }
        }
        Err(_) => tauri::webview::NewWindowResponse::Deny,
      }
    })
    .on_download(move |_wv, event| {
      mgr_for_dl.handle_desktop_download_event(&app_for_dl, &pending_for_dl, event)
    })
    .on_page_load(move |_wv, payload| {
      if payload.url().as_str() == "about:blank" {
        return;
      }
      if !alive_load.load(Ordering::SeqCst) {
        return;
      }
      match payload.event() {
        tauri::webview::PageLoadEvent::Started => {
          load_gen_for_load.fetch_add(1, Ordering::SeqCst);
        }
        tauri::webview::PageLoadEvent::Finished => {
          if !handle_page_load {
            return;
          }
          let expected_gen = load_gen_for_load.fetch_add(1, Ordering::SeqCst) + 1;
          let loaded_url = payload.url().clone();
          let mgr_commit = Arc::clone(&mgr_for_load);
          let app_commit = app_for_load.clone();
          let key_commit = key_for_load.clone();
          tauri::async_runtime::spawn(async move {
            mgr_commit
              .wait_paint_and_finalize_desktop(
                &app_commit,
                key_commit,
                view_instance_id,
                loaded_url,
                expected_gen,
                false,
              )
              .await;
          });
        }
      }
    });

    #[cfg(target_os = "macos")]
    {
      let _ = profile_dir;
      builder = builder
        .data_store_identifier(*parsed_uuid.as_bytes())
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15");
    }

    #[cfg(not(target_os = "macos"))]
    {
      let _ = parsed_uuid;
      builder = builder.data_directory(profile_dir.to_path_buf());
    }

    if provider_spec.info.id.is_hark() {
      builder = builder.background_color(tauri::webview::Color(23, 29, 47, 255));
    }

    builder
  }

  #[cfg(desktop)]
  fn initial_hidden_child_rect(
    main_win: &tauri::Window,
  ) -> (tauri::LogicalPosition<f64>, tauri::LogicalSize<f64>) {
    let (win_w, win_h) = match (main_win.inner_size(), main_win.scale_factor()) {
      (Ok(phys), Ok(scale)) if scale > 0.0 => {
        ((phys.width as f64) / scale, (phys.height as f64) / scale)
      }
      _ => (1280.0, 800.0),
    };
    let left_w = win_w.min(LEFT_PANE_WIDTH);
    let right_w = (win_w - left_w).max(600.0);
    (
      tauri::LogicalPosition::new(win_w.max(2000.0) + 1000.0, 0.0),
      tauri::LogicalSize::new(right_w, win_h.max(400.0)),
    )
  }

  #[cfg(desktop)]
  fn discard_desktop_view_locked(
    &self,
    app: &AppHandle,
    locked: &mut InnerState,
    key: &AccountKey,
    view_instance_id: u64,
    notice: String,
  ) -> bool {
    if !Self::is_view_instance_valid(locked, key, view_instance_id) {
      return false;
    }
    if let Some(entry) = locked.warm_pool.remove(key) {
      Self::close_warm_entry(app, entry);
    }
    if locked.active_slot.as_ref().is_some_and(|slot| slot.view_instance_id == view_instance_id) {
      locked.active_slot = None;
      self.active_view_id.store(0, Ordering::SeqCst);
      locked.notice = Some(notice);
      Self::apply_desktop_layout_locked(app, locked);
    }
    true
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  async fn attach_desktop_account_webview(
    self: &Arc<Self>,
    app: &AppHandle,
    key: &AccountKey,
    parsed_uuid: Uuid,
    profile_dir: &std::path::Path,
    webview_label: &str,
    view_instance_id: u64,
    alive_flag: &Arc<AtomicBool>,
    load_gen: &Arc<AtomicU64>,
  ) -> Result<(), String> {
    let main_win = app.get_window("main").ok_or_else(|| "主窗口不存在".to_string())?;
    let (init_pos, init_size) = Self::initial_hidden_child_rect(&main_win);
    #[cfg(windows)]
    let retry_until = tokio::time::Instant::now() + std::time::Duration::from_secs(5);

    loop {
      let (proxy_url, provider_spec) = {
        let locked = self.inner.lock().await;
        if !Self::is_view_instance_valid(&locked, key, view_instance_id) {
          return Ok(());
        }
        let spec = resolve_provider(&locked.store.custom_providers, &key.provider_id)
          .ok_or_else(|| "未找到对应的服务配置".to_string())?;
        let account = locked
          .store
          .accounts
          .iter()
          .find(|account| account.provider_id == key.provider_id && account.uuid == key.uuid)
          .ok_or_else(|| "账号不存在，已阻止加载".to_string())?;
        (
          platform::parse_account_proxy(account.proxy_url.as_deref().unwrap_or_default())?,
          spec,
        )
      };
      let builder = self.build_desktop_account_webview(
        app,
        key.clone(),
        provider_spec,
        parsed_uuid,
        profile_dir,
        proxy_url.as_ref(),
        webview_label,
        view_instance_id,
        Arc::new(AtomicBool::new(true)),
        Arc::clone(alive_flag),
        Arc::clone(load_gen),
      );
      #[cfg(windows)]
      let builder =
        platform::with_windows_account_proxy(builder, proxy_url.as_ref(), &app.config().identifier)?;
      let result = main_win.add_child(builder, init_pos, init_size);

      match result {
        Ok(webview) => {
          let _ = webview.hide();
          return self
            .navigate_desktop_account(app, &webview, key, view_instance_id, load_gen)
            .await;
        }
        #[cfg(windows)]
        Err(err) if platform::windows_webview_options_in_use(&err) => {
          if tokio::time::Instant::now() >= retry_until {
            return Err("该账号的旧 WebView2 会话尚未退出，请稍后重新打开账号".to_string());
          }
          tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        Err(err) => return Err(format!("创建独立账号视图失败: {err}")),
      }
    }
  }

  #[cfg(desktop)]
  async fn navigate_desktop_account(
    self: &Arc<Self>,
    app: &AppHandle,
    webview: &tauri::Webview,
    key: &AccountKey,
    view_instance_id: u64,
    load_gen: &Arc<AtomicU64>,
  ) -> Result<(), String> {
    let _ = load_gen;
    let (proxy_url, uuid, profile_dir, entry_url_str) = {
      let locked = self.inner.lock().await;
      if !Self::is_view_instance_valid(&locked, key, view_instance_id) {
        drop(locked);
        let _ = webview.close();
        return Ok(());
      }
      let spec = resolve_provider(&locked.store.custom_providers, &key.provider_id)
        .ok_or_else(|| "未找到对应的服务配置".to_string())?;
      let account = locked
        .store
        .accounts
        .iter()
        .find(|account| account.provider_id == key.provider_id && account.uuid == key.uuid)
        .ok_or_else(|| "账号不存在，已阻止加载".to_string())?;
      let proxy_url = platform::parse_account_proxy(account.proxy_url.as_deref().unwrap_or_default())?;
      let uuid = Uuid::parse_str(&key.uuid).map_err(|err| format!("非法 UUID: {err}"))?;
      let profile_dir = locked.app_data_dir.join("profiles").join(&key.uuid);
      (proxy_url, uuid, profile_dir, spec.info.entry_url)
    };
    let uses_proxy = proxy_url.is_some();

    platform::verify_desktop_profile_binding(webview, uuid, &profile_dir).await?;
    #[cfg(target_os = "macos")]
    platform::configure_desktop_proxy(webview, proxy_url).await?;
    #[cfg(target_os = "macos")]
    if uses_proxy {
      let mgr = Arc::clone(self);
      let app = app.clone();
      let key = key.clone();
      platform::on_macos_navigation_failed(webview, move || {
        let mgr = Arc::clone(&mgr);
        let app = app.clone();
        let key = key.clone();
        tauri::async_runtime::spawn(async move {
          mgr
            .fail_initial_proxy_load(
              &app,
              key,
              view_instance_id,
              "页面加载失败，请检查该账号的代理地址和连接",
            )
            .await;
        });
      })
      .await?;
    }
    #[cfg(windows)]
    if uses_proxy {
      let mgr = Arc::clone(self);
      let app = app.clone();
      let key = key.clone();
      let load_gen_win = Arc::clone(load_gen);
      platform::on_windows_navigation_completed(webview, move |result| {
        let mgr = Arc::clone(&mgr);
        let app = app.clone();
        let key = key.clone();
        let load_gen_cb = Arc::clone(&load_gen_win);
        tauri::async_runtime::spawn(async move {
          match result {
            Ok(url) => {
              let expected_gen = load_gen_cb.fetch_add(1, Ordering::SeqCst) + 1;
              mgr
                .wait_paint_and_finalize_desktop(
                  &app,
                  key,
                  view_instance_id,
                  url,
                  expected_gen,
                  false,
                )
                .await;
            }
            Err(()) => {
              mgr.fail_initial_proxy_load(
                &app,
                key,
                view_instance_id,
                "页面加载失败，请检查该账号的代理地址和连接",
              )
              .await;
            }
          }
        });
      })
      .await?;
    }

    let entry_url = entry_url_str
      .parse()
      .map_err(|err| format!("服务入口地址错误: {err}"))?;
    {
      let locked = self.inner.lock().await;
      if !Self::is_view_instance_valid(&locked, key, view_instance_id) {
        drop(locked);
        let _ = webview.close();
        return Ok(());
      }
    }
    webview.navigate(entry_url).map_err(|err| format!("导航到服务入口失败: {err}"))?;

    if uses_proxy {
      let mgr = Arc::clone(self);
      let app = app.clone();
      let key = key.clone();
      tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        mgr.fail_initial_proxy_load(
          &app,
          key,
          view_instance_id,
          "页面加载超时，请检查该账号的代理地址和连接",
        )
        .await;
      });
    }
    Ok(())
  }

  #[cfg(desktop)]
  async fn fail_initial_proxy_load(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    notice: &str,
  ) {
    let mut locked = self.inner.lock().await;
    if locked
      .warm_pool
      .get(&key)
      .is_some_and(|entry| entry.paint_ready || entry.navigation_reached)
    {
      return;
    }
    if self.discard_desktop_view_locked(app, &mut locked, &key, view_instance_id, notice.to_owned()) {
      locked.prewarm_failed.insert(key);
      self.emit_snapshot(app, &Self::snapshot_from_locked(&locked));
    }
  }

  #[cfg(desktop)]
  async fn open_desktop_session(
    self: &Arc<Self>,
    app: &AppHandle,
    account: AccountRecord,
    app_data_dir: PathBuf,
    expected_seq: u64,
    view_instance_id: u64,
    is_cold_start: bool,
  ) -> Result<ManagerSnapshot, String> {
    let parsed_uuid = Uuid::parse_str(&account.uuid).map_err(|e| format!("非法 UUID: {e}"))?;
    let profile_dir = app_data_dir.join("profiles").join(&account.uuid);

    #[cfg(windows)]
    if is_cold_start && !profile_dir.exists() {
      let mut locked = self.inner.lock().await;
      locked.notice = Some("上次已提交账号的 Windows WebView2 资料目录缺失，已停留在管理页".to_string());
      return Ok(Self::snapshot_from_locked(&locked));
    }
    #[cfg(not(windows))]
    let _ = is_cold_start;

    #[cfg(windows)]
    std::fs::create_dir_all(&profile_dir).map_err(|e| format!("创建资料目录失败: {e}"))?;

    let webview_label = format!("account-{view_instance_id}");
    let alive_flag = Arc::new(AtomicBool::new(true));
    let load_gen = Arc::new(AtomicU64::new(0));
    let key = AccountKey {
      provider_id: account.provider_id,
      uuid: account.uuid.clone(),
    };

    let loading_snap = {
      let mut locked = self.inner.lock().await;
      if self.current_seq.load(Ordering::SeqCst) != expected_seq {
        return Ok(Self::snapshot_from_locked(&locked));
      }
      if let Some(entry) = locked.warm_pool.remove(&key) {
        Self::close_warm_entry(app, entry);
      }
      locked.active_slot = Some(ActiveSessionSlot {
        account_key: key.clone(),
        view_instance_id,
        switch_seq: expected_seq,
        committed: false,
      });
      self.active_view_id.store(view_instance_id, Ordering::SeqCst);
      locked.warm_pool.insert(
        key.clone(),
        WarmDesktopEntry {
          account_key: key.clone(),
          view_instance_id,
          webview_label: webview_label.clone(),
          popup_label: None,
          paint_ready: false,
          staged_painted: false,
          navigation_reached: false,
          load_gen: Arc::clone(&load_gen),
          last_url: None,
          captured_identity_hint: None,
          watcher_running: false,
          probe_notify: Arc::new(tokio::sync::Notify::new()),
          last_used_ms: now_ms(),
          alive_flag: Arc::clone(&alive_flag),
        },
      );
      Self::evict_warm_pool_if_needed(app, &mut locked, Some(&key), MAX_WARM_DESKTOP_VIEWS);
      Self::apply_desktop_layout_locked(app, &locked);
      Self::snapshot_from_locked(&locked)
    };
    self.emit_snapshot(app, &loading_snap);

    let result = self
      .attach_desktop_account_webview(
        app,
        &key,
        parsed_uuid,
        &profile_dir,
        &webview_label,
        view_instance_id,
        &alive_flag,
        &load_gen,
      )
      .await;
    if let Err(err) = result {
      let mut locked = self.inner.lock().await;
      self.discard_desktop_view_locked(app, &mut locked, &key, view_instance_id, err);
      return Ok(Self::snapshot_from_locked(&locked));
    }

    {
      let locked = self.inner.lock().await;
      if locked
        .active_slot
        .as_ref()
        .is_some_and(|slot| {
          slot.account_key == key && slot.view_instance_id == view_instance_id && !slot.committed
        })
      {
        Self::apply_desktop_layout_locked(app, &locked);
      }
    }

    Ok(self.get_snapshot().await)
  }

  #[cfg(desktop)]
  async fn eval_paint_ready(webview: &tauri::Webview) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx_cell = Arc::new(std::sync::Mutex::new(Some(tx)));
    let tx_inner = Arc::clone(&tx_cell);
    if webview
      .eval_with_callback(DOM_PAINT_READY_JS, move |raw| {
        if let Some(sender) = tx_inner.lock().unwrap().take() {
          let _ = sender.send(raw);
        }
      })
      .is_err()
    {
      return false;
    }

    match tokio::time::timeout(std::time::Duration::from_millis(600), rx).await {
      Ok(Ok(raw)) => raw.trim() == "true",
      _ => false,
    }
  }

  #[cfg(desktop)]
  async fn wait_paint_and_finalize_desktop(
    self: &Arc<Self>,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    loaded_url: Url,
    expected_load_gen: u64,
    reset_paint_state: bool,
  ) {
    let wait_params = {
      let mut locked = self.inner.lock().await;
      let Some(spec) = resolve_provider(&locked.store.custom_providers, &key.provider_id) else {
        return;
      };
      if classify_top_level_url(&spec, &loaded_url, false) != NavigationDecision::AllowHttps {
        return;
      }
      locked.warm_pool.get_mut(&key).and_then(|e| {
        if e.view_instance_id == view_instance_id
          && e.load_gen.load(Ordering::SeqCst) == expected_load_gen
        {
          e.navigation_reached = true;
          Some((
            e.webview_label.clone(),
            e.paint_ready,
            Arc::clone(&e.alive_flag),
            Arc::clone(&e.load_gen),
          ))
        } else {
          None
        }
      })
    };

    let Some((wv_label, already_ready, alive_flag, load_gen)) = wait_params else {
      return;
    };

    if !already_ready || reset_paint_state {
      if reset_paint_state {
        if let Some(wv) = app.get_webview(&wv_label) {
          let _ = wv.eval("window.__maamReadySince = 0; window.__maamReadyFrames = 0;");
        }
      }
      for tick in 0..100 {
        if !alive_flag.load(Ordering::SeqCst)
          || load_gen.load(Ordering::SeqCst) != expected_load_gen
        {
          return;
        }
        let Some(wv) = app.get_webview(&wv_label) else {
          return;
        };
        #[cfg(target_os = "macos")]
        if platform::is_macos_webview_loading(&wv).await {
          tokio::time::sleep(std::time::Duration::from_millis(if tick < 20 { 80 } else { 150 }))
            .await;
          continue;
        }
        if Self::eval_paint_ready(&wv).await {
          tokio::time::sleep(std::time::Duration::from_millis(60)).await;
          if !alive_flag.load(Ordering::SeqCst)
            || load_gen.load(Ordering::SeqCst) != expected_load_gen
          {
            return;
          }
          #[cfg(target_os = "macos")]
          if platform::is_macos_webview_loading(&wv).await {
            continue;
          }
          break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(if tick < 20 { 80 } else { 150 }))
          .await;
      }
    }

    let check_params = {
      let locked = self.inner.lock().await;
      locked.warm_pool.get(&key).and_then(|e| {
        if e.view_instance_id == view_instance_id {
          Some((
            e.webview_label.clone(),
            locked.app_data_dir.join("profiles").join(&key.uuid),
          ))
        } else {
          None
        }
      })
    };

    let Some((wv_label, expected_dir)) = check_params else {
      return;
    };
    let Ok(parsed_uuid) = Uuid::parse_str(&key.uuid) else {
      return;
    };
    let Some(wv) = app.get_webview(&wv_label) else {
      return;
    };

    if let Err(err) =
      platform::verify_desktop_profile_binding(&wv, parsed_uuid, &expected_dir).await
    {
      let snap = {
        let mut locked = self.inner.lock().await;
        if !self.discard_desktop_view_locked(app, &mut locked, &key, view_instance_id, err) {
          return;
        }
        Self::snapshot_from_locked(&locked)
      };
      self.emit_snapshot(app, &snap);
      return;
    }

    let (snap_opt, start_watcher, probe_notify_opt) = {
      let mut locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() || locked.store.pending_cleanup.contains(&key) {
        if let Some(entry) = locked.warm_pool.remove(&key) {
          Self::close_warm_entry(app, entry);
        }
        return;
      }
      let Some(acc) = locked
        .store
        .accounts
        .iter()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        .cloned()
      else {
        if let Some(entry) = locked.warm_pool.remove(&key) {
          Self::close_warm_entry(app, entry);
        }
        return;
      };

      let is_foreground_waiting = locked
        .active_slot
        .as_ref()
        .map(|s| s.account_key == key && s.view_instance_id == view_instance_id && !s.committed)
        .unwrap_or(false);

      let mut need_watcher = false;
      let probe_notify;
      if let Some(entry) = locked.warm_pool.get_mut(&key) {
        if entry.view_instance_id != view_instance_id
          || entry.load_gen.load(Ordering::SeqCst) != expected_load_gen
        {
          return;
        }
        entry.paint_ready = true;
        if is_foreground_waiting {
          entry.staged_painted = true;
        }
        entry.last_url = Some(loaded_url);
        if !entry.watcher_running {
          entry.watcher_running = true;
          need_watcher = true;
        }
        probe_notify = Arc::clone(&entry.probe_notify);
      } else {
        return;
      }

      if is_foreground_waiting {
        let prev_last = locked.store.last_committed.clone();
        locked.store.last_committed = Some(key.clone());
        if let Err(err) = save_store_atomic(&locked.store_path, &locked.store) {
          locked.store.last_committed = prev_last;
          locked.active_slot = None;
          self.active_view_id.store(0, Ordering::SeqCst);
          locked.notice = Some(format!("提交账号记录写盘失败，已回退管理页: {err}"));
          Self::apply_desktop_layout_locked(app, &locked);
          (Some(Self::snapshot_from_locked(&locked)), false, None)
        } else {
          if let Some(slot) = &mut locked.active_slot {
            slot.committed = true;
          }
          if let Some(entry) = locked.warm_pool.get_mut(&key) {
            entry.last_used_ms = now_ms();
          }
          locked
            .last_active_per_provider
            .insert(key.provider_id.clone(), key.clone());
          let short_name = resolve_provider(&locked.store.custom_providers, &key.provider_id)
            .map(|s| s.info.short_name)
            .unwrap_or_else(|| key.provider_id.as_str().to_string());
          let title = format!("{short_name} · {}", acc.alias);
          if let Some(main_win) = app.get_window("main") {
            let _ = main_win.set_title(&title);
          }
          Self::apply_desktop_layout_locked(app, &locked);
          (Some(Self::snapshot_from_locked(&locked)), need_watcher, Some(probe_notify))
        }
      } else {
        (Some(Self::snapshot_from_locked(&locked)), need_watcher, Some(probe_notify))
      }
    };

    if let Some(probe_notify) = probe_notify_opt {
      if start_watcher {
        self.spawn_desktop_identity_watcher(app.clone(), key, view_instance_id, probe_notify);
      } else {
        probe_notify.notify_one();
      }
    }

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }

    let mgr_next = Arc::clone(self);
    let app_next = app.clone();
    tauri::async_runtime::spawn(async move {
      tokio::time::sleep(std::time::Duration::from_millis(350)).await;
      mgr_next.prewarm_next_cold_account(&app_next).await;
    });
  }

  #[cfg(desktop)]
  async fn prewarm_desktop_account(
    self: &Arc<Self>,
    app: &AppHandle,
    account: AccountRecord,
  ) -> Result<(), String> {
    let key = AccountKey {
      provider_id: account.provider_id,
      uuid: account.uuid.clone(),
    };
    let app_data_dir = {
      let locked = self.inner.lock().await;
      if locked.warm_pool.contains_key(&key)
        || locked.store.pending_cleanup.contains(&key)
        || locked.warm_pool.len() >= MAX_WARM_DESKTOP_VIEWS
      {
        return Ok(());
      }
      locked.app_data_dir.clone()
    };

    let parsed_uuid = Uuid::parse_str(&account.uuid).map_err(|e| format!("非法 UUID: {e}"))?;
    let profile_dir = app_data_dir.join("profiles").join(&account.uuid);

    #[cfg(windows)]
    std::fs::create_dir_all(&profile_dir).map_err(|e| format!("创建资料目录失败: {e}"))?;

    let view_instance_id = self.view_counter.fetch_add(1, Ordering::SeqCst);
    let webview_label = format!("account-{view_instance_id}");
    let alive_flag = Arc::new(AtomicBool::new(true));
    let load_gen = Arc::new(AtomicU64::new(0));

    {
      let mut locked = self.inner.lock().await;
      if locked.warm_pool.contains_key(&key)
        || locked.store.pending_cleanup.contains(&key)
        || locked.warm_pool.len() >= MAX_WARM_DESKTOP_VIEWS
        || !locked.store.accounts.iter().any(|record| {
          record.provider_id == key.provider_id && record.uuid == key.uuid
        })
      {
        return Ok(());
      }
      locked.warm_pool.insert(
        key.clone(),
        WarmDesktopEntry {
          account_key: key.clone(),
          view_instance_id,
          webview_label: webview_label.clone(),
          popup_label: None,
          paint_ready: false,
          staged_painted: false,
          navigation_reached: false,
          load_gen: Arc::clone(&load_gen),
          last_url: None,
          captured_identity_hint: None,
          watcher_running: false,
          probe_notify: Arc::new(tokio::sync::Notify::new()),
          last_used_ms: 0,
          alive_flag: Arc::clone(&alive_flag),
        },
      );
    }

    let result = self
      .attach_desktop_account_webview(
        app, &key, parsed_uuid, &profile_dir, &webview_label, view_instance_id, &alive_flag, &load_gen,
      )
      .await;
    if let Err(err) = result {
      let mut locked = self.inner.lock().await;
      if self.discard_desktop_view_locked(app, &mut locked, &key, view_instance_id, err.clone()) {
        return Err(err);
      }
    }

    Ok(())
  }

  #[cfg(desktop)]
  pub fn prewarm_next_cold_account(
    self: &Arc<Self>,
    app: &AppHandle,
  ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    let mgr = Arc::clone(self);
    let app_handle = app.clone();
    Box::pin(async move {
      let next_cold = {
        let locked = mgr.inner.lock().await;
        let in_flight = locked.warm_pool.values().filter(|e| !e.paint_ready).count();
        if locked.warm_pool.len() >= MAX_WARM_DESKTOP_VIEWS || in_flight >= MAX_CONCURRENT_PREWARMS {
          None
        } else {
          let is_eligible_cold = |a: &&AccountRecord| {
            if !resolve_provider(&locked.store.custom_providers, &a.provider_id)
              .is_some_and(|s| s.info.enabled)
            {
              return false;
            }
            let k = AccountKey {
              provider_id: a.provider_id.clone(),
              uuid: a.uuid.clone(),
            };
            !locked.warm_pool.contains_key(&k)
              && !locked.store.pending_cleanup.contains(&k)
              && !locked.prewarm_failed.contains(&k)
          };
          locked
            .store
            .accounts
            .iter()
            .filter(is_eligible_cold)
            .find(|a| a.provider_id == locked.selected_provider_id)
            .or_else(|| locked.store.accounts.iter().find(is_eligible_cold))
            .cloned()
        }
      };

      if let Some(account) = next_cold {
        let failed_key = AccountKey {
          provider_id: account.provider_id.clone(),
          uuid: account.uuid.clone(),
        };
        if mgr.prewarm_desktop_account(&app_handle, account).await.is_err() {
          {
            let mut locked = mgr.inner.lock().await;
            locked.prewarm_failed.insert(failed_key);
          }
          tokio::time::sleep(std::time::Duration::from_millis(400)).await;
          mgr.prewarm_next_cold_account(&app_handle).await;
        }
      }
    })
  }

  #[cfg(not(desktop))]
  pub fn prewarm_next_cold_account(
    self: &Arc<Self>,
    _app: &AppHandle,
  ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {})
  }

  #[cfg(desktop)]
  async fn register_desktop_popup(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    popup_label: String,
  ) {
    let mut locked = self.inner.lock().await;
    if self.active_view_id.load(Ordering::SeqCst) != view_instance_id {
      if let Some(pw) = app.get_webview_window(&popup_label) {
        let _ = pw.close();
      }
      return;
    }
    let Some(entry) = locked.warm_pool.get_mut(&key) else {
      if let Some(pw) = app.get_webview_window(&popup_label) {
        let _ = pw.close();
      }
      return;
    };
    if entry.view_instance_id != view_instance_id {
      if let Some(pw) = app.get_webview_window(&popup_label) {
        let _ = pw.close();
      }
      return;
    }
    if let Some(prev_popup) = entry.popup_label.replace(popup_label) {
      if let Some(pw) = app.get_webview_window(&prev_popup) {
        let _ = pw.close();
      }
    }
  }

  #[cfg(target_os = "android")]
  async fn open_android_session(
    self: &Arc<Self>,
    app: &AppHandle,
    account: AccountRecord,
    expected_seq: u64,
    view_instance_id: u64,
    is_cold_start: bool,
  ) -> Result<ManagerSnapshot, String> {
    let caps = match crate::platform::android::check_capabilities(app).await {
      Ok(c) => c,
      Err(err) => {
        let mut locked = self.inner.lock().await;
        locked.notice = Some(format!("检测 Android WebView 多资料能力失败: {err}"));
        return Ok(Self::snapshot_from_locked(&locked));
      }
    };

    if !caps.multi_profile_supported {
      let mut locked = self.inner.lock().await;
      locked.notice = Some(
        "当前设备系统 WebView 不支持 MULTI_PROFILE 多资料隔离，请先更新 Android System WebView 后重试"
          .to_string(),
      );
      return Ok(Self::snapshot_from_locked(&locked));
    }

    if is_cold_start && !caps.profile_names.iter().any(|n| n == &account.uuid) {
      let mut locked = self.inner.lock().await;
      locked.notice = Some("上次已提交账号的系统 Profile 已丢失，已停留在管理页".to_string());
      return Ok(Self::snapshot_from_locked(&locked));
    }

    let key = AccountKey {
      provider_id: account.provider_id.clone(),
      uuid: account.uuid.clone(),
    };

    let spec = {
      let mut locked = self.inner.lock().await;
      if self.current_seq.load(Ordering::SeqCst) != expected_seq {
        return Ok(Self::snapshot_from_locked(&locked));
      }
      let spec = resolve_provider(&locked.store.custom_providers, &account.provider_id)
        .ok_or_else(|| "未找到对应的服务配置".to_string())?;
      locked.active_slot = Some(ActiveSessionSlot {
        account_key: key,
        view_instance_id,
        switch_seq: expected_seq,
        committed: false,
      });
      spec
    };

    let payload = crate::platform::android::OpenAndroidAccountPayload {
      provider_id: account.provider_id.as_str().to_string(),
      provider_name: spec.info.short_name,
      uuid: account.uuid.clone(),
      alias: account.alias.clone(),
      auto_alias: account.auto_alias,
      entry_url: spec.info.entry_url,
      allowed_hosts: spec.allowed_hosts,
      switch_seq: expected_seq,
      view_instance_id,
      is_cold_start,
    };

    if let Err(err) = crate::platform::android::open_account_session(app, payload).await {
      let mut locked = self.inner.lock().await;
      if self.current_seq.load(Ordering::SeqCst) == expected_seq {
        locked.active_slot = None;
        locked.notice = Some(format!("打开 Android 独立会话页面失败: {err}"));
      }
      return Ok(Self::snapshot_from_locked(&locked));
    }

    Ok(self.get_snapshot().await)
  }

  #[cfg(target_os = "android")]
  async fn handle_android_event(
    self: &Arc<Self>,
    app: &AppHandle,
    event: crate::platform::android::AndroidSessionEvent,
  ) {
    use crate::platform::android::AndroidSessionEvent;
    match event {
      AndroidSessionEvent::PageCommitVisible {
        provider_id,
        uuid,
        switch_seq,
        view_instance_id,
        url,
      } => {
        let Some(pid) = ProviderId::parse(&provider_id) else {
          return;
        };
        let Some(norm_uuid) = normalize_uuid(&uuid) else {
          return;
        };
        let Ok(parsed_url) = Url::parse(&url) else {
          return;
        };
        self
          .commit_android_session(
            app,
            AccountKey {
              provider_id: pid,
              uuid: norm_uuid,
            },
            view_instance_id,
            switch_seq,
            parsed_url,
          )
          .await;
      }
      AndroidSessionEvent::IdentityDetected {
        provider_id,
        uuid,
        switch_seq,
        view_instance_id,
        identity,
        avatar_url,
        quota_text,
        quota_percent,
        quota_detail_json,
      } => {
        let Some(pid) = ProviderId::parse(&provider_id) else {
          return;
        };
        let Some(norm_uuid) = normalize_uuid(&uuid) else {
          return;
        };
        let _ = switch_seq;
        let parsed_detail = quota_detail_json
          .as_deref()
          .and_then(|s| serde_json::from_str::<QuotaDetail>(s).ok());
        self
          .apply_detected_metadata(
            app,
            AccountKey {
              provider_id: pid,
              uuid: norm_uuid,
            },
            view_instance_id,
            if identity.is_empty() { None } else { Some(identity) },
            avatar_url,
            quota_text,
            quota_percent,
            parsed_detail,
          )
          .await;
      }
      AndroidSessionEvent::SessionFailed {
        switch_seq,
        view_instance_id,
        reason,
      } => {
        let snap = {
          let mut locked = self.inner.lock().await;
          if self.current_seq.load(Ordering::SeqCst) != switch_seq {
            return;
          }
          let matches = locked
            .active_slot
            .as_ref()
            .map(|s| s.switch_seq == switch_seq && s.view_instance_id == view_instance_id)
            .unwrap_or(false);
          if !matches {
            return;
          }
          locked.active_slot = None;
          locked.notice = Some(reason);
          Self::snapshot_from_locked(&locked)
        };
        self.emit_snapshot(app, &snap);
      }
      AndroidSessionEvent::ActivityClosed {
        switch_seq,
        view_instance_id,
      } => {
        let snap = {
          let mut locked = self.inner.lock().await;
          let matches = locked
            .active_slot
            .as_ref()
            .map(|s| s.switch_seq == switch_seq && s.view_instance_id == view_instance_id)
            .unwrap_or(false);
          if !matches {
            return;
          }
          locked.active_slot = None;
          locked.pending_external_auth = None;
          Self::snapshot_from_locked(&locked)
        };
        self.emit_snapshot(app, &snap);
      }
      AndroidSessionEvent::ExternalAuthUnavailable {
        switch_seq,
        view_instance_id,
        reason,
      } => {
        let snap = {
          let mut locked = self.inner.lock().await;
          let matches = locked
            .active_slot
            .as_ref()
            .map(|s| s.switch_seq == switch_seq && s.view_instance_id == view_instance_id)
            .unwrap_or(false);
          if !matches {
            return;
          }
          locked.pending_external_auth = None;
          locked.notice = Some(reason);
          Self::snapshot_from_locked(&locked)
        };
        self.emit_snapshot(app, &snap);
      }
      AndroidSessionEvent::RecreateRequested { previous_switch_seq } => {
        let fallback = {
          let mut locked = self.inner.lock().await;
          if self.current_seq.load(Ordering::SeqCst) == previous_switch_seq {
            locked.active_slot = None;
          }
          locked.store.last_committed.clone()
        };
        if let Some(committed_key) = fallback {
          let _ = self.switch_account_internal(app, committed_key, true).await;
        } else {
          let snap = self.get_snapshot().await;
          self.emit_snapshot(app, &snap);
        }
      }
      AndroidSessionEvent::UpdateProgress {
        stage,
        downloaded,
        total,
        percent,
        message,
      } => {
        let _ = app.emit(
          "update-progress",
          crate::updater::UpdateProgressEvent {
            stage,
            downloaded,
            total,
            percent,
            message,
          },
        );
      }
      AndroidSessionEvent::SessionLoggedOut {
        provider_id,
        uuid,
        switch_seq,
        view_instance_id,
      } => {
        let _ = switch_seq;
        let Some(pid) = ProviderId::parse(&provider_id) else {
          return;
        };
        let Some(norm_uuid) = normalize_uuid(&uuid) else {
          return;
        };
        self
          .apply_logged_out_state(
            app,
            AccountKey {
              provider_id: pid,
              uuid: norm_uuid,
            },
            view_instance_id,
          )
          .await;
      }
    }
  }

  #[cfg(target_os = "android")]
  async fn commit_android_session(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    expected_seq: u64,
    committed_url: Url,
  ) {
    if self.current_seq.load(Ordering::SeqCst) != expected_seq {
      return;
    }

    let snap_opt = {
      let mut locked = self.inner.lock().await;
      if self.current_seq.load(Ordering::SeqCst) != expected_seq {
        return;
      }
      let Some(spec) = resolve_provider(&locked.store.custom_providers, &key.provider_id) else {
        return;
      };
      if classify_top_level_url(&spec, &committed_url, false) != NavigationDecision::AllowHttps {
        return;
      }
      if !locked.conflicts.is_empty() || locked.store.pending_cleanup.contains(&key) {
        self.hide_foreground_slot(app, expected_seq, &mut locked).await;
        return;
      }
      let Some(acc) = locked
        .store
        .accounts
        .iter()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        .cloned()
      else {
        self.hide_foreground_slot(app, expected_seq, &mut locked).await;
        return;
      };

      let slot_matches = locked
        .active_slot
        .as_ref()
        .map(|s| {
          s.account_key == key
            && s.view_instance_id == view_instance_id
            && s.switch_seq == expected_seq
        })
        .unwrap_or(false);
      if !slot_matches {
        return;
      }

      let prev_last = locked.store.last_committed.clone();
      locked.store.last_committed = Some(key.clone());
      if let Err(err) = save_store_atomic(&locked.store_path, &locked.store) {
        locked.store.last_committed = prev_last;
        self.hide_foreground_slot(app, expected_seq, &mut locked).await;
        locked.notice = Some(format!("提交账号记录写盘失败，已回退管理页: {err}"));
        Some(Self::snapshot_from_locked(&locked))
      } else {
        if let Some(slot) = &mut locked.active_slot {
          slot.committed = true;
        }
        let title = format!("{} · {}", spec.info.short_name, acc.alias);
        let _ = crate::platform::android::update_account_title(app, expected_seq, &title).await;
        Some(Self::snapshot_from_locked(&locked))
      }
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }

  #[cfg(desktop)]
  async fn notify_external_auth_policy(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
  ) {
    let snap_opt = {
      let mut locked = self.inner.lock().await;
      let matches = locked
        .active_slot
        .as_ref()
        .map(|s| s.account_key == key && s.view_instance_id == view_instance_id)
        .unwrap_or(false);
      if !matches {
        return;
      }
      let cur_seq = self.current_seq.load(Ordering::SeqCst);
      let entry_ok = resolve_provider(&locked.store.custom_providers, &key.provider_id)
        .and_then(|spec| {
          Url::parse(&spec.info.entry_url)
            .ok()
            .map(|u| is_verified_callback_origin(&spec, &u))
        })
        .unwrap_or(false);
      locked.pending_external_auth = Some(PendingExternalAuth {
        account_key: key.clone(),
        switch_seq: cur_seq,
        nonce: format!("{}-{}", key.provider_id.as_str(), Uuid::new_v4().simple()),
      });
      if !entry_ok || locked.pending_external_auth.is_some() {
        locked.pending_external_auth = None;
      }
      locked.notice = Some(
        "Google OAuth 按政策禁止内嵌 WebView，且系统浏览器回跳暂无法绑定到当前独立浏览资料，请使用邮箱、手机号或已验收方式登录"
          .to_string(),
      );
      Some(Self::snapshot_from_locked(&locked))
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }

  #[cfg(desktop)]
  fn spawn_desktop_identity_watcher(
    self: &Arc<Self>,
    app: AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    probe_notify: Arc<tokio::sync::Notify>,
  ) {
    let mgr = Arc::clone(self);
    tauri::async_runtime::spawn(async move {
      let mut tick_count: u32 = 0;
      let mut consecutive_logged_out: u8 = 0;
      let mut next_delay_ms: Option<u64> = None;
      loop {
        if let Some(delay_ms) = next_delay_ms.take() {
          if consecutive_logged_out == 1 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
          } else {
            tokio::select! {
              _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => {},
              _ = probe_notify.notified() => {},
            }
          }
        }
        tick_count = tick_count.saturating_add(1);

        let labels_opt = {
          let locked = mgr.inner.lock().await;
          let account_exists = locked
            .store
            .accounts
            .iter()
            .any(|a| a.provider_id == key.provider_id && a.uuid == key.uuid);
          if !account_exists {
            None
          } else {
            locked.warm_pool.get(&key).and_then(|e| {
              if e.view_instance_id == view_instance_id && e.alive_flag.load(Ordering::SeqCst) {
                Some((e.webview_label.clone(), e.popup_label.clone()))
              } else {
                None
              }
            })
          }
        };

        let Some((wv_label, popup_label)) = labels_opt else {
          let mut locked = mgr.inner.lock().await;
          if let Some(entry) = locked.warm_pool.get_mut(&key) {
            if entry.view_instance_id == view_instance_id {
              entry.watcher_running = false;
            }
          }
          break;
        };

        if let Some(p_label) = popup_label {
          if let Some(p_wv) = app.get_webview(&p_label) {
            if let Some(probe) = Self::eval_identity_probe(&p_wv).await {
              if !probe.typed.is_empty() {
                let mut locked = mgr.inner.lock().await;
                if let Some(entry) = locked.warm_pool.get_mut(&key) {
                  if entry.view_instance_id == view_instance_id {
                    entry.captured_identity_hint = Some(probe.typed);
                  }
                }
              }
            }
          }
        }

        let Some(main_wv) = app.get_webview(&wv_label) else {
          let mut locked = mgr.inner.lock().await;
          if let Some(entry) = locked.warm_pool.get_mut(&key) {
            if entry.view_instance_id == view_instance_id {
              entry.watcher_running = false;
            }
          }
          break;
        };
        let Some(probe) = Self::eval_identity_probe(&main_wv).await else {
          next_delay_ms = Some(if tick_count < 6 { 800 } else { 6500 });
          continue;
        };

        if let Some(idx) = probe.pending_hotkey {
          let mgr_hotkey = Arc::clone(&mgr);
          let app_hotkey = app.clone();
          tauri::async_runtime::spawn(async move {
            let _ = mgr_hotkey.switch_account_by_index(&app_hotkey, idx).await;
          });
        }

        let resolved_identity = {
          let mut locked = mgr.inner.lock().await;
          if let Some(entry) = locked.warm_pool.get_mut(&key) {
            if entry.view_instance_id == view_instance_id {
              if !probe.typed.is_empty() {
                entry.captured_identity_hint = Some(probe.typed);
              }
              if probe.is_auth {
                if !probe.email.is_empty() {
                  Some(probe.email)
                } else {
                  entry.captured_identity_hint.clone()
                }
              } else {
                None
              }
            } else {
              None
            }
          } else {
            None
          }
        };

        if probe.is_auth {
          consecutive_logged_out = 0;
          let (just_authenticated, needs_cookie_check) = mgr
            .apply_detected_metadata(
              &app,
              key.clone(),
              view_instance_id,
              resolved_identity,
              if probe.avatar.is_empty() {
                None
              } else {
                Some(probe.avatar)
              },
              if probe.quota_text.is_empty() {
                None
              } else {
                Some(probe.quota_text)
              },
              probe.quota_pct,
              probe.quota_detail,
            )
            .await;
          if just_authenticated || needs_cookie_check {
            let mgr_cookie = Arc::clone(&mgr);
            let app_cookie = app.clone();
            let key_cookie = key.clone();
            tauri::async_runtime::spawn(async move {
              mgr_cookie
                .refresh_account_cookie_expiry(&app_cookie, key_cookie, just_authenticated)
                .await;
            });
          }
          next_delay_ms = Some(if tick_count < 6 { 950 } else { 6500 });
        } else if probe.logged_out {
          consecutive_logged_out = consecutive_logged_out.saturating_add(1);
          if consecutive_logged_out >= 2 {
            mgr
              .apply_logged_out_state(&app, key.clone(), view_instance_id)
              .await;
            next_delay_ms = Some(if tick_count < 6 { 950 } else { 6500 });
          } else {
            next_delay_ms = Some(300);
          }
        } else {
          consecutive_logged_out = 0;
          next_delay_ms = Some(if tick_count < 6 { 500 } else { 6500 });
        }
      }
    });
  }

  #[cfg(desktop)]
  async fn eval_identity_probe(webview: &tauri::Webview) -> Option<DesktopProbeResult> {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx_cell = Arc::new(std::sync::Mutex::new(Some(tx)));
    let tx_inner = Arc::clone(&tx_cell);
    webview
      .eval_with_callback(IDENTITY_PROBE_JS, move |raw| {
        if let Some(sender) = tx_inner.lock().unwrap().take() {
          let _ = sender.send(raw);
        }
      })
      .ok()?;

    let raw = tokio::time::timeout(std::time::Duration::from_millis(1200), rx)
      .await
      .ok()?
      .ok()?;

    let inner_json = serde_json::from_str::<String>(&raw).ok()?;
    let val = serde_json::from_str::<serde_json::Value>(&inner_json).ok()?;
    let is_auth = val.get("auth").and_then(|v| v.as_bool()).unwrap_or(false);
    let logged_out = val.get("loggedOut").and_then(|v| v.as_bool()).unwrap_or(false);
    let typed = val
      .get("typed")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let email = val
      .get("email")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let avatar = val
      .get("avatar")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let quota_text = val
      .get("quotaText")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let quota_pct = val
      .get("quotaPct")
      .and_then(|v| v.as_u64())
      .map(|n| n.min(100) as u8);
    let quota_detail = val
      .get("quotaDetail")
      .filter(|v| !v.is_null())
      .and_then(|v| serde_json::from_value::<QuotaDetail>(v.clone()).ok())
      .filter(|d| !d.plan_label.trim().is_empty() || !d.plan_value.trim().is_empty());
    let pending_hotkey = val
      .get("pendingHotkey")
      .and_then(|v| v.as_u64())
      .map(|n| n as u8);

    Some(DesktopProbeResult {
      is_auth,
      logged_out,
      typed,
      email,
      avatar,
      quota_text,
      quota_pct,
      quota_detail,
      pending_hotkey,
    })
  }

  fn is_view_instance_valid(locked: &InnerState, key: &AccountKey, view_instance_id: u64) -> bool {
    #[cfg(desktop)]
    {
      locked
        .warm_pool
        .get(key)
        .map(|e| e.view_instance_id == view_instance_id)
        .unwrap_or(false)
    }
    #[cfg(not(desktop))]
    {
      locked
        .active_slot
        .as_ref()
        .map(|s| s.account_key == *key && s.view_instance_id == view_instance_id)
        .unwrap_or(false)
    }
  }

  async fn apply_logged_out_state(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
  ) {
    let snap_opt = {
      let mut locked = self.inner.lock().await;
      if !Self::is_view_instance_valid(&locked, &key, view_instance_id) {
        return;
      }

      #[cfg(desktop)]
      locked.cookie_checked_at.remove(&key);

      let Some(target) = locked
        .store
        .accounts
        .iter_mut()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
      else {
        return;
      };

      if target.auth_state == AuthState::LoggedOut
        && target.quota_detail.is_none()
        && target.quota_text.is_none()
        && target.quota_percent.is_none()
        && target.cookie_expires_at_ms.is_none()
        && !target.cookie_session_only
      {
        return;
      }

      target.auth_state = AuthState::LoggedOut;
      target.quota_detail = None;
      target.quota_text = None;
      target.quota_percent = None;
      target.cookie_expires_at_ms = None;
      target.cookie_session_only = false;

      if save_store_atomic(&locked.store_path, &locked.store).is_err() {
        return;
      }

      Some(Self::snapshot_from_locked(&locked))
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }

  #[cfg(desktop)]
  #[allow(dead_code)]
  async fn apply_cookie_expiry_summary(
    &self,
    app: &AppHandle,
    key: &AccountKey,
    summary: platform::CookieExpirySummary,
  ) {
    let checked_at = now_ms();
    let snap_opt = {
      let mut locked = self.inner.lock().await;
      let Some(idx) = locked
        .store
        .accounts
        .iter()
        .position(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
      else {
        return;
      };
      if locked.store.accounts[idx].auth_state == AuthState::LoggedOut {
        return;
      }
      if summary.expires_at_ms.is_some() || summary.session_only {
        locked.cookie_checked_at.insert(key.clone(), checked_at);
      }
      let target = &mut locked.store.accounts[idx];
      if target.cookie_expires_at_ms == summary.expires_at_ms
        && target.cookie_session_only == summary.session_only
      {
        return;
      }
      target.cookie_expires_at_ms = summary.expires_at_ms;
      target.cookie_session_only = summary.session_only;
      if save_store_atomic(&locked.store_path, &locked.store).is_err() {
        return;
      }
      Some(Self::snapshot_from_locked(&locked))
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }

  #[cfg(desktop)]
  #[allow(dead_code)]
  async fn refresh_account_cookie_expiry(
    self: &Arc<Self>,
    app: &AppHandle,
    key: AccountKey,
    force: bool,
  ) {
    const COOKIE_CHECK_COOLDOWN_MS: u64 = 10 * 60 * 1000;
    let now = now_ms();

    #[cfg(not(any(target_os = "macos", windows)))]
    {
      let _ = (app, key, force, now, COOKIE_CHECK_COOLDOWN_MS);
      return;
    }

    #[cfg(any(target_os = "macos", windows))]
    {
      let url_opt = {
        let locked = self.inner.lock().await;
        let Some(account) = locked
          .store
          .accounts
          .iter()
          .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        else {
          return;
        };
        if account.auth_state == AuthState::LoggedOut {
          return;
        }
        if !force {
          if let Some(&last_checked) = locked.cookie_checked_at.get(&key) {
            if now.saturating_sub(last_checked) < COOKIE_CHECK_COOLDOWN_MS {
              return;
            }
          }
        }
        let Some(spec) = resolve_provider(&locked.store.custom_providers, &key.provider_id) else {
          return;
        };
        if !spec.info.enabled {
          return;
        }
        let Ok(url) = Url::parse(&spec.info.entry_url) else {
          return;
        };

        #[cfg(target_os = "macos")]
        let platform_target = Uuid::parse_str(&key.uuid).ok().map(|uuid| (url, uuid));

        #[cfg(windows)]
        let platform_target = locked
          .warm_pool
          .get(&key)
          .and_then(|entry| app.get_webview(&entry.webview_label))
          .map(|webview| (url, webview));

        platform_target
      };

      let Some((url, handle)) = url_opt else {
        return;
      };

      #[cfg(target_os = "macos")]
      let summary_res = platform::read_macos_cookies_for_url(app, handle, url)
        .await
        .map(|(_, summary)| summary);

      #[cfg(windows)]
      let summary_res = Self::read_windows_webview_cookies(handle, url)
        .await
        .map(|(_, summary)| summary);

      if let Ok(summary) = summary_res {
        self.apply_cookie_expiry_summary(app, &key, summary).await;
      }
    }
  }

  #[allow(clippy::too_many_arguments)]
  async fn apply_detected_metadata(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    identity_opt: Option<String>,
    avatar_opt: Option<String>,
    quota_text_opt: Option<String>,
    quota_pct_opt: Option<u8>,
    quota_detail_opt: Option<QuotaDetail>,
  ) -> (bool, bool) {
    let (snap_opt, became_authenticated, needs_cookie_check) = {
      let mut locked = self.inner.lock().await;
      if !Self::is_view_instance_valid(&locked, &key, view_instance_id) {
        return (false, false);
      }

      let Some(target) = locked
        .store
        .accounts
        .iter_mut()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
      else {
        return (false, false);
      };

      let mut changed = false;
      let mut became_authenticated = false;
      let mut alias_updated = None;

      if target.auth_state != AuthState::Authenticated {
        target.auth_state = AuthState::Authenticated;
        became_authenticated = true;
        changed = true;
      }

      if target.auto_alias {
        if let Some(raw_id) = identity_opt {
          let cleaned = raw_id.trim();
          if !cleaned.is_empty() && cleaned.len() <= 80 {
            target.alias = cleaned.to_string();
            target.auto_alias = false;
            alias_updated = Some(cleaned.to_string());
            changed = true;
          }
        }
      }

      if let Some(av) = avatar_opt {
        if av.starts_with("data:image/") && av.len() < 80_000 && target.avatar_url.as_ref() != Some(&av) {
          target.avatar_url = Some(av);
          changed = true;
        }
      }

      if let Some(detail) = quota_detail_opt {
        if target.quota_detail.as_ref() != Some(&detail) {
          target.quota_detail = Some(detail);
          changed = true;
        }
      }

      if let Some(qt) = quota_text_opt {
        let cleaned_qt = qt.trim();
        if !cleaned_qt.is_empty() && cleaned_qt.len() <= 60 {
          let is_placeholder = cleaned_qt == "额度充足" || cleaned_qt == "正在同步额度…";
          let should_overwrite = match (&target.quota_text, is_placeholder) {
            (Some(existing), true) if existing != "额度充足" && existing != "正在同步额度…" => false,
            (Some(existing), _) if existing == cleaned_qt && target.quota_percent == quota_pct_opt => false,
            _ => true,
          };
          if should_overwrite {
            target.quota_text = Some(cleaned_qt.to_string());
            target.quota_percent = quota_pct_opt;
            changed = true;
          }
        }
      }

      let needs_cookie_check =
        became_authenticated || (target.cookie_expires_at_ms.is_none() && !target.cookie_session_only);

      let snap_opt = if changed && save_store_atomic(&locked.store_path, &locked.store).is_ok() {
        if let Some(new_alias) = alias_updated {
          let is_fg = locked
            .active_slot
            .as_ref()
            .map(|s| s.committed && s.account_key == key)
            .unwrap_or(false);
          if is_fg {
            let short_name = resolve_provider(&locked.store.custom_providers, &key.provider_id)
              .map(|s| s.info.short_name)
              .unwrap_or_else(|| key.provider_id.as_str().to_string());
            let title = format!("{short_name} · {new_alias}");
            #[cfg(desktop)]
            if let Some(main_win) = app.get_window("main") {
              let _ = main_win.set_title(&title);
            }
            #[cfg(target_os = "android")]
            if let Some(slot) = &locked.active_slot {
              let _ = crate::platform::android::update_account_title(app, slot.switch_seq, &title).await;
            }
          }
        }
        Some(Self::snapshot_from_locked(&locked))
      } else {
        None
      };

      (snap_opt, became_authenticated, needs_cookie_check)
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }

    (became_authenticated, needs_cookie_check)
  }
}

#[cfg(desktop)]
struct DesktopProbeResult {
  is_auth: bool,
  logged_out: bool,
  typed: String,
  email: String,
  avatar: String,
  quota_text: String,
  quota_pct: Option<u8>,
  quota_detail: Option<QuotaDetail>,
  pending_hotkey: Option<u8>,
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::storage::save_store_atomic;

  fn make_test_account(provider_id: ProviderId, uuid: &str, alias: &str) -> AccountRecord {
    AccountRecord {
      provider_id,
      uuid: uuid.to_string(),
      alias: alias.to_string(),
      created_at_ms: 1,
      auto_alias: false,
      proxy_url: None,
      avatar_url: None,
      quota_text: None,
      quota_percent: None,
      quota_detail: None,
      auth_state: AuthState::Unknown,
      cookie_expires_at_ms: None,
      cookie_session_only: false,
    }
  }

  #[tokio::test]
  async fn reorder_accounts_updates_only_target_provider_and_validates() {
    let temp_dir = std::env::temp_dir().join(format!("maam-reorder-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");

    let m1 = "00000000-0000-4000-8000-000000000001";
    let h1 = "00000000-0000-4000-8000-000000000002";
    let m2 = "00000000-0000-4000-8000-000000000003";
    let m3 = "00000000-0000-4000-8000-000000000004";
    let h2 = "00000000-0000-4000-8000-000000000005";

    let initial = AccountsStoreFile {
      version: crate::storage::STORE_VERSION,
      accounts: vec![
        make_test_account(ProviderId::muse(), m1, "m1"),
        make_test_account(ProviderId::hark(), h1, "h1"),
        make_test_account(ProviderId::muse(), m2, "m2"),
        make_test_account(ProviderId::muse(), m3, "m3"),
        make_test_account(ProviderId::hark(), h2, "h2"),
      ],
      last_committed: None,
      pending_cleanup: Vec::new(),
      custom_providers: Vec::new(),
      settings: AppSettings::default(),
    };
    save_store_atomic(&store_path, &initial).unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let snap = mgr
      .reorder_accounts("muse", vec![m3.to_string(), m1.to_string(), m2.to_string()])
      .await
      .unwrap();
    let aliases: Vec<&str> = snap.accounts.iter().map(|a| a.alias.as_str()).collect();
    assert_eq!(aliases, vec!["m3", "h1", "m1", "m2", "h2"]);

    let reloaded = SessionManager::new(temp_dir.clone()).get_snapshot().await;
    let reloaded_aliases: Vec<&str> = reloaded.accounts.iter().map(|a| a.alias.as_str()).collect();
    assert_eq!(reloaded_aliases, vec!["m3", "h1", "m1", "m2", "h2"]);

    assert!(mgr
      .reorder_accounts("muse", vec![m1.to_string(), m2.to_string()])
      .await
      .is_err());
    assert!(mgr
      .reorder_accounts("muse", vec![m1.to_string(), m1.to_string(), m3.to_string()])
      .await
      .is_err());
    assert!(mgr
      .reorder_accounts("muse", vec![m1.to_string(), h1.to_string(), m3.to_string()])
      .await
      .is_err());

    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn custom_provider_validation_and_navigation() {
    assert!(normalize_custom_provider_input("", "https://chatgpt.com").is_err());
    assert!(normalize_custom_provider_input("GPT", "http://chatgpt.com").is_err());
    let (name, url) = normalize_custom_provider_input("  ChatGPT ", "chatgpt.com").unwrap();
    assert_eq!(name, "ChatGPT");
    assert_eq!(url, "https://chatgpt.com/");

    let rec = CustomProviderRecord {
      id: ProviderId::parse("custom-chatgpt1").unwrap(),
      name,
      entry_url: url,
      created_at_ms: 1,
    };
    let spec = crate::providers::custom_provider_spec(&rec);
    assert!(spec.info.is_custom);
    let google_oauth = Url::parse("https://accounts.google.com/o/oauth2/v2/auth").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &google_oauth, false),
      NavigationDecision::AllowHttps
    );
    let http_url = Url::parse("http://example.com/").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &http_url, false),
      NavigationDecision::Deny
    );
  }
}
