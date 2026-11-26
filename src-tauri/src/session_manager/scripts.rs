//! Injected JavaScript for webviews (P2-11 split).
//!
//! Moved verbatim from `session_manager.rs`: the `*_INIT_JS` init scripts
//! installed into desktop webviews and the `build_*_js` builders that render
//! per-adapter / per-message JavaScript. No behavior change: the parent module
//! re-exports everything (`pub(crate) use scripts::{...}`) so call sites are
//! untouched.

#[cfg(desktop)]
use super::GroupAttachment;
#[cfg(desktop)]
use crate::site_adapters::SiteAdapterConfig;

#[cfg(desktop)]
pub(crate) const BACKGROUND_RAF_INIT_JS: &str = r#"(() => {
  try {
    if (window.__maamRafInstalled) return;
    window.__maamRafInstalled = true;
    const origRaf = window.requestAnimationFrame ? window.requestAnimationFrame.bind(window) : null;
    const origCancel = window.cancelAnimationFrame ? window.cancelAnimationFrame.bind(window) : null;
    if (!origRaf) return;
    let nextId = 1;
    const timers = new Map();
    const natives = new Map();
    window.requestAnimationFrame = (cb) => {
      if (typeof cb !== 'function') return 0;
      const id = nextId++;
      if (document.visibilityState !== 'hidden') {
        const nid = origRaf((ts) => {
          natives.delete(id);
          cb(ts);
        });
        natives.set(id, nid);
        return id;
      }
      const tid = setTimeout(() => {
        timers.delete(id);
        cb(performance.now());
      }, 32);
      timers.set(id, tid);
      return id;
    };
    window.cancelAnimationFrame = (id) => {
      if (timers.has(id)) {
        clearTimeout(timers.get(id));
        timers.delete(id);
      } else if (natives.has(id)) {
        const nid = natives.get(id);
        natives.delete(id);
        if (origCancel) origCancel(nid);
      } else if (origCancel) {
        origCancel(id);
      }
    };
  } catch (_) {}
})()"#;

#[cfg(desktop)]
pub(crate) const LOGIN_CAPTURE_INIT_JS: &str = r#"(() => {
  try {
    if (location.protocol !== 'https:') return;
    if (window !== window.top) return;
    const initHost = (location.hostname || '').toLowerCase();
    const isPrimaryHost = initHost === 'muse.ai' || initHost.endsWith('.muse.ai') || initHost === 'app.hark.com' || initHost.endsWith('.hark.com');
    const origFetch = window.fetch;
    const authCheck = window.__maamAuthCheck || {
      status: 'unknown',
      reason: '',
      checkedAt: 0,
      inflight: false
    };
    window.__maamAuthCheck = authCheck;
    try {
      const desc = Object.getOwnPropertyDescriptor(window, '__maamAuthCheck');
      if (!desc || desc.configurable) {
        Object.defineProperty(window, '__maamAuthCheck', { value: authCheck, writable: false, configurable: false });
      }
    } catch (_) {} // Sealing raises the bar only; Rust must re-verify identity/quota server-side.
    const markAuthInvalid = (reason) => {
      authCheck.status = 'invalid';
      authCheck.reason = String(reason || 'unauthorized');
      authCheck.checkedAt = Date.now();
      window.__maamQuota = null;
      window.__maamQuotaDetail = null;
      try {
        localStorage.removeItem('__maam_cached_quota__');
        localStorage.removeItem('__maam_cached_quota_detail_v2__');
      } catch (_) {}
    };
    const markAuthOk = () => {
      authCheck.status = 'ok';
      authCheck.reason = '';
      authCheck.checkedAt = Date.now();
    };
    const BAD_EMAIL_RE = /^(?:support|privacy|help|legal|press|noreply|no-reply|info|security|contact|abuse|feedback|sales|terms|hello|team|admin|postmaster|notifications)@/i;
    const EMAIL_STRICT_RE = /^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$/;
    const PROVIDER_EMAIL_RE = /@(?:[a-z0-9-]+\.)?(?:muse\.ai|hark\.com|meta\.com|facebook\.com)$/i;
    const BAD_NAME_RE = /^(?:Meta|Muse|Hark|Button|Settings|Profile|Account|User|Avatar|Loading|Unknown|Hatch|Home|Chat|New Chat|Log in|Sign up|待登录账号|已登录账号|通用|设置)$/i;
    const isGoodEmail = (s) => {
      if (typeof s !== 'string') return false;
      const clean = s.trim().toLowerCase();
      return EMAIL_STRICT_RE.test(clean) && !BAD_EMAIL_RE.test(clean) && !clean.endsWith('@example.com') && !clean.endsWith('@sentry.io') && !clean.endsWith('@wixpress.com');
    };
    const recordApiEmail = (raw) => {
      if (!isGoodEmail(raw)) return;
      const clean = raw.trim().toLowerCase();
      if (!window.__maamApiEmail || (PROVIDER_EMAIL_RE.test(window.__maamApiEmail) && !PROVIDER_EMAIL_RE.test(clean))) {
        window.__maamApiEmail = clean;
      }
    };
    const recordApiName = (raw) => {
      if (typeof raw !== 'string') return;
      const clean = raw.replace(/\s+/g, ' ').trim();
      if (clean.length < 2 || clean.length > 60) return;
      if (/^https?:\/\//i.test(clean) || /^\d+$/.test(clean) || BAD_NAME_RE.test(clean)) return;
      if (!window.__maamApiName) {
        window.__maamApiName = clean;
      }
    };
    window.__maamRecordApiEmail = recordApiEmail;
    window.__maamRecordApiName = recordApiName;
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
          if (src.includes('createServerReference')) {
            const reProf = /createServerReference\)\("([0-9a-f]{32,64})"[^)]*"((?:fetch|get)(?:User|Account|Profile|Viewer|Settings|Email|Identity|Me|CurrentUser|HatchUser|HatchProfile|HatchSettings|HatchEmail)[A-Za-z0-9_]*Action)"\)/g;
            let mp;
            while ((mp = reProf.exec(src)) !== null) {
              if (!Array.isArray(window.__maamProfileActions)) window.__maamProfileActions = [];
              if (!window.__maamProfileActions.includes(mp[1]) && window.__maamProfileActions.length < 5) {
                window.__maamProfileActions.push(mp[1]);
              }
            }
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

    window.__maamVerifySession = async (force) => {
      if (authCheck.inflight) return;
      const now = Date.now();
      if (!force && authCheck.checkedAt && now - authCheck.checkedAt < 12000) return;
      const curHost = (location.hostname || '').toLowerCase();
      const fetchFn = typeof origFetch === 'function' ? origFetch : window.fetch;
      if (typeof fetchFn !== 'function') return;
      authCheck.inflight = true;
      try {
        if (curHost === 'muse.ai' || curHost.endsWith('.muse.ai')) {
          const r1 = await fetchFn.call(window, '/api/client-version', {
            credentials: 'same-origin',
            cache: 'no-store',
            headers: { Accept: 'application/json' }
          });
          if (r1 && r1.status === 401) {
            markAuthInvalid('client_version_401');
            return;
          }
          if (r1 && r1.ok) {
            try { r1.clone().text().then(inspectText).catch(() => {}); } catch (_) {}
            const r2 = await fetchFn.call(window, '/api/auth/check', {
              method: 'POST',
              credentials: 'include',
              cache: 'no-store',
              headers: { Accept: 'application/json' }
            });
            if (r2 && (r2.status === 401 || r2.status === 403)) {
              markAuthInvalid('auth_check_' + r2.status);
              return;
            }
            if (r2 && r2.ok) {
              try { r2.clone().text().then(inspectText).catch(() => {}); } catch (_) {}
              markAuthOk();
              return;
            }
          }
          authCheck.checkedAt = Date.now();
          return;
        }
        if (curHost === 'app.hark.com' || curHost.endsWith('.hark.com')) {
          const r = await fetchFn.call(window, '/api/profile', {
            credentials: 'include',
            cache: 'no-store',
            headers: { Accept: 'application/json' }
          });
          if (r && r.status === 401) {
            markAuthInvalid('hark_profile_401');
            return;
          }
          if (r && r.ok) {
            try { r.clone().text().then(inspectText).catch(() => {}); } catch (_) {}
            markAuthOk();
            return;
          }
          authCheck.checkedAt = Date.now();
        }
      } catch (_) {
        authCheck.checkedAt = Date.now();
      } finally {
        authCheck.inflight = false;
      }
    };

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
          if (res && res.status === 401) {
            markAuthInvalid('hark_401');
          } else if (res && res.ok) {
            markAuthOk();
            const data = await res.json();
            const mapped = mapHarkSub(data);
            if (mapped) {
              window.__maamQuotaFetchedAt = Date.now();
              saveDetail(mapped);
            }
          }
          if (!window.__maamApiEmail && authCheck.status !== 'invalid') {
            try {
              const profRes = await fetchFn.call(window, '/api/profile', {
                credentials: 'include',
                headers: { Accept: 'application/json' }
              });
              if (profRes && profRes.status === 401) {
                markAuthInvalid('hark_401');
              } else if (profRes && profRes.ok) {
                markAuthOk();
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
            inspectNode(res, 0, true);
            const mapped = mapSub(res);
            if (mapped) {
              markAuthOk();
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
        // When no action id was scanned from page scripts, do not call an unknown server action; rely on cached quota.
        const aid = window.__maamSubActionId || '';
        if (callServerFn && aid) {
          const res = await callServerFn(aid, [{ includeAgreement: true }]);
          inspectNode(res, 0, true);
          const mapped = mapSub(res);
          if (mapped) {
            markAuthOk();
            window.__maamQuotaFetchedAt = Date.now();
            saveDetail(mapped);
          }
        }
      } catch (err) {
        const msg = String(err && (err.message || err));
        if (/401|403|unauthenticated|unauthorized|authentication required|checkpoint/i.test(msg)) {
          markAuthInvalid('muse_sub_auth');
        }
      } finally {
        window.__maamQuotaInflight = false;
      }
    };

    window.__maamRefreshIdentity = async () => {
      if (!isPrimaryHost || window.__maamApiEmail || window.__maamIdFetchDone || window.__maamIdFetchInflight) return;
      window.__maamIdFetchInflight = true;
      try {
        const fetchFn = typeof origFetch === 'function' ? origFetch : window.fetch;
        const curHost = (location.hostname || '').toLowerCase();
        if (curHost === 'app.hark.com' || curHost.endsWith('.hark.com')) {
          if (typeof fetchFn === 'function') {
            try {
              const res = await fetchFn.call(window, '/api/profile', {
                credentials: 'include',
                cache: 'no-store',
                headers: { Accept: 'application/json' }
              });
              if (res && res.ok) {
                const txt = await res.text();
                inspectText(txt);
              }
            } catch (_) {}
          }
          return;
        }
        if (curHost === 'muse.ai' || curHost.endsWith('.muse.ai')) {
          if (typeof fetchFn === 'function') {
            try {
              const chk = await fetchFn.call(window, '/api/auth/check', {
                method: 'POST',
                credentials: 'include',
                cache: 'no-store',
                headers: { Accept: 'application/json' }
              });
              if (chk && chk.ok) {
                const txt = await chk.text();
                inspectText(txt);
              }
            } catch (_) {}
          }
          if (!window.__maamApiEmail) {
            const tpGlobal = globalThis.TURBOPACK;
            if (tpGlobal && !Array.isArray(tpGlobal) && typeof tpGlobal.push === 'function') {
              if (!window.__maamTp) {
                tpGlobal.push(['__maam_init__', 999999991, (ctx) => { window.__maamTp = ctx; }]);
                tpGlobal.push(['__maam_run__', { otherChunks: [], runtimeModuleIds: [999999991] }]);
              }
              const tp = window.__maamTp;
              if (tp && window.__maamPreloadModId && typeof tp.A === 'function') {
                try {
                  const preMod = await tp.A(window.__maamPreloadModId).catch(() => null);
                  if (preMod && typeof preMod.preloadHatchSettingsData === 'function') {
                    const preRes = await Promise.resolve(preMod.preloadHatchSettingsData()).catch(() => null);
                    if (preRes) inspectNode(preRes, 0, true);
                  }
                } catch (_) {}
              }
              if (!window.__maamApiEmail && tp && tp.c && typeof tp.c.values === 'function' && Array.isArray(window.__maamProfileActions)) {
                let callServerFn = null;
                for (const m of tp.c.values()) {
                  if (m && m.exports && typeof m.exports.callServer === 'function') {
                    callServerFn = m.exports.callServer;
                    break;
                  }
                }
                if (callServerFn) {
                  for (const aid of window.__maamProfileActions) {
                    try {
                      const actRes = await callServerFn(aid, []);
                      inspectNode(actRes, 0, true);
                      if (window.__maamApiEmail) break;
                    } catch (_) {}
                  }
                }
              }
            }
          }
        }
      } catch (_) {
      } finally {
        window.__maamIdFetchInflight = false;
        window.__maamIdFetchDone = true;
      }
    };

    const inspectNode = (node, depth, skipSub) => {
      if (!node || typeof node !== 'object' || depth > 10) return;
      if (Array.isArray(node)) {
        for (let i = 0; i < node.length && i < 30; i++) inspectNode(node[i], depth + 1, skipSub);
        return;
      }
      const EMAIL_KEYS = [
        'email', 'user_email', 'userEmail', 'contact_email', 'contactEmail',
        'primary_email', 'primaryEmail', 'email_address', 'emailAddress',
        'billing_email', 'billingEmail', 'customer_email', 'customerEmail',
        'login_email', 'loginEmail', 'account_email', 'accountEmail',
        'verified_email', 'verifiedEmail', 'display_label', 'displayLabel',
        'contact_point', 'contactPoint'
      ];
      for (const ek of EMAIL_KEYS) {
        if (typeof node[ek] === 'string' && node[ek].includes('@')) {
          recordApiEmail(node[ek]);
        }
      }
      const hasUserHint =
        'user_id' in node || 'userId' in node || 'viewer_id' in node || 'viewerId' in node ||
        'profile_pic_url' in node || 'profilePictureUrl' in node || 'avatar_url' in node || 'avatarUrl' in node ||
        'linked_accounts' in node || 'linkedAccounts' in node || 'account_type' in node || 'accountType' in node ||
        'subscriptionType' in node || 'subscription_type' in node || 'is_verified' in node ||
        'username' in node || 'user_name' in node || 'email' in node;
      if (hasUserHint) {
        const NAME_KEYS = ['display_name', 'displayName', 'full_name', 'fullName', 'username', 'user_name', 'userName', 'preferred_username', 'name'];
        for (const nk of NAME_KEYS) {
          if (typeof node[nk] === 'string') {
            recordApiName(node[nk]);
          }
        }
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
      for (let i = 0; i < keys.length && i < 40; i++) {
        const v = node[keys[i]];
        if (v && typeof v === 'object') inspectNode(v, depth + 1, nextSkipSub);
      }
    };
    const inspectText = (txt) => {
      if (typeof txt !== 'string' || txt.length < 2 || txt.length > 600000) return;
      if (txt.includes('@')) {
        const keyedEmailRe = /\\?"(?:email|user_email|userEmail|contact_email|contactEmail|primary_email|primaryEmail|email_address|emailAddress|billing_email|billingEmail|customer_email|customerEmail|login_email|loginEmail|account_email|accountEmail|verified_email|verifiedEmail|display_label|displayLabel|contact_point|contactPoint)\\?"\s*:\s*\\?"([a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,})\\?"/g;
        let km;
        while ((km = keyedEmailRe.exec(txt)) !== null) {
          recordApiEmail(km[1]);
        }
      }
      if (/email|user_id|userId|viewer|profile|account/i.test(txt)) {
        const keyedNameRe = /\\?"(?:display_name|displayName|full_name|fullName|preferred_username)\\?"\s*:\s*\\?"([^"\\]{2,60})\\?"/g;
        let nm;
        while ((nm = keyedNameRe.exec(txt)) !== null) {
          recordApiName(nm[1]);
        }
      }
      if (!/percent_used|percentUsed|quota_status|quotaStatus|usage_row_value_label|usageRowValueLabel|topup_balance|topupBalance|remaining_credits|credit_balance|out_of_credits|UNLIMITED|email|credits|allowance|balance|usage|tokens|billing|subscription|harkTokens|dailyLimit|dailyUsed|poolLimit|poolUsed|dailyResetsAt|poolResetsAt|meters|displayName|display_name/i.test(txt)) return;
      try {
        inspectNode(JSON.parse(txt), 0, false);
        return;
      } catch (_) {}
      const lines = txt.split('\n');
      for (let i = 0; i < lines.length && i < 400; i++) {
        const idx = lines[i].indexOf(':');
        if (idx > 0 && idx < 16) {
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
              let reqPath = null;
              try {
                const parsedUrl = new URL(reqUrl || '', location.origin);
                if (parsedUrl.origin === location.origin && parsedUrl.pathname.startsWith('/api/')) {
                  reqPath = parsedUrl.pathname;
                }
              } catch (_) {}
              const ct = (res.headers && res.headers.get('content-type')) || '';
              if (
                reqPath &&
                reqPath !== '/api/logout' &&
                reqPath !== '/api/auth/checkpoint/terminate' &&
                !reqPath.startsWith('/api/falco') &&
                reqPath !== '/api/metaconfig/exposure_sessionless' &&
                !reqPath.startsWith('/api/marketing/')
              ) {
                if (res.status === 401) {
                  markAuthInvalid('http_401');
                } else if (res.status === 403 && initHost.endsWith('muse.ai')) {
                  if (reqPath === '/api/auth/check') {
                    markAuthInvalid('auth_check_403');
                  } else if (ct.includes('json')) {
                    res.clone().text().then((body) => {
                      if (/"type"\s*:\s*"checkpointRequired"|"error"\s*:\s*"Authentication required"|"invalidation"/i.test(body)) {
                        markAuthInvalid('auth_403');
                      }
                    }).catch(() => {});
                  }
                } else if (
                  res.ok &&
                  (reqPath === '/api/client-version' ||
                    reqPath === '/api/auth/check' ||
                    reqPath === '/api/profile' ||
                    reqPath === '/api/billing/summary' ||
                    reqPath.startsWith('/api/messages') ||
                    reqPath.startsWith('/api/chat'))
                ) {
                  markAuthOk();
                }
              }
              if (!ct.includes('event-stream') && !ct.includes('ndjson') && (ct.includes('json') || ct.includes('text'))) {
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

      // 自动审批 watcher 已抽到 src-tauri/src/js/auto_approve.js，
      // 以独立 initialization script 注册（见 AUTO_APPROVE_INIT_JS）。
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
pub(crate) const DOM_PAINT_READY_JS: &str = r#"(() => {
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
pub(crate) const AUTO_APPROVE_INIT_JS: &str = include_str!("../js/auto_approve.js");

#[cfg(desktop)]
pub(crate) const IDENTITY_PROBE_JS: &str = r#"(() => {
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
    const isPrimary =
      host === 'muse.ai' || host.endsWith('.muse.ai') ||
      host === 'app.hark.com' || host.endsWith('.hark.com');
    const isMetaAccountCenter = host === 'accountscenter.meta.com';
    const isExternalLoginHost =
      host === 'meta.com' || host.endsWith('.meta.com') ||
      host === 'facebook.com' || host.endsWith('.facebook.com') ||
      host === 'appleid.apple.com';
    const makeLoggedOut = (staleSession = false) => {
      window.__maamQuota = null;
      window.__maamQuotaDetail = null;
      try {
        localStorage.removeItem('__maam_cached_quota__');
        localStorage.removeItem('__maam_cached_quota_detail_v2__');
      } catch (_) {}
      return JSON.stringify({ auth: false, loggedOut: true, staleSession: Boolean(staleSession), pageError: '', typed: typed || phone, email: '', displayName: '', avatar: '', quotaText: '', quotaPct: null, quotaDetail: null, pendingHotkey });
    };
    const makeLoading = () => JSON.stringify({ auth: false, loggedOut: false, staleSession: false, pageError: '', typed: typed || phone, email: '', displayName: '', avatar: '', quotaText: '', quotaPct: null, quotaDetail: null, pendingHotkey });
    const makePageError = (msg) => JSON.stringify({ auth: false, loggedOut: false, staleSession: false, pageError: String(msg || ''), typed: typed || phone, email: '', displayName: '', avatar: '', quotaText: '', quotaPct: null, quotaDetail: null, pendingHotkey });
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
        return makeLoggedOut(false);
      }
      if (isMetaAccountCenter && document.readyState !== 'loading') {
        const metaBodyText = (document.body && document.body.innerText) || '';
        const badMeta = /^(support|privacy|help|legal|press|noreply|no-reply|info|security|contact|abuse|feedback|sales|terms|hello|team|admin|postmaster|notifications)@/i;
        const providerMeta = /@(?:[a-z0-9-]+\.)?(?:muse\.ai|hark\.com|meta\.com|facebook\.com)$/i;
        const metaEmailRe = /[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}/g;
        const metaCands = [];
        const pushMetaRaw = (raw) => {
          if (typeof raw !== 'string' || !raw.includes('@')) return;
          const m = raw.match(metaEmailRe);
          if (m) metaCands.push(...m);
        };
        document.querySelectorAll('[data-email],[aria-label*="@"],[title*="@"],a[href^="mailto:"]').forEach((el) => {
          const raw = (el.getAttribute('data-email') || el.getAttribute('aria-label') || el.getAttribute('title') || el.getAttribute('href') || '').replace(/^mailto:/i, '');
          pushMetaRaw(raw);
        });
        pushMetaRaw(metaBodyText);
        if (metaCands.length === 0) {
          document.querySelectorAll('script[type="application/json"]').forEach((sc) => {
            const txt = sc.textContent || '';
            if (txt && txt.includes('@') && txt.length < 200000) pushMetaRaw(txt);
          });
        }
        const validMeta = [];
        for (const c of metaCands) {
          const clean = String(c || '').toLowerCase();
          if (clean && !badMeta.test(clean) && !clean.endsWith('@example.com') && !clean.endsWith('@sentry.io') && !clean.endsWith('@wixpress.com')) {
            if (!validMeta.includes(clean)) validMeta.push(clean);
          }
        }
        const metaEmail = validMeta.find((e) => !providerMeta.test(e)) || validMeta[0] || '';
        if (!metaEmail && /Log in to Meta|登录\s*Meta|Create a Meta account|创建\s*Meta\s*账户|Sign up for Meta/i.test(metaBodyText)) {
          return makeLoggedOut(false);
        }
        return JSON.stringify({ auth: false, loggedOut: false, staleSession: false, pageError: '', typed: typed || phone, email: metaEmail, displayName: '', avatar: '', quotaText: '', quotaPct: null, quotaDetail: null, pendingHotkey });
      }
      return makeLoading();
    }
    if (!isPrimary) {
      return makeLoading();
    }
    if (/^\/(login|signin|signup|auth)\b/.test(path)) {
      return makeLoggedOut(false);
    }
    if (!hasChatComposer && !/^\/(settings|profile|account)\b/.test(path) && hasVisibleLoginInput()) {
      return makeLoggedOut(false);
    }
    const bodyText = (document.body && document.body.innerText) || '';
    if (!hasChatComposer && /登录或创建账户|Log in or sign up|Sign in to|登录以继续|通过 Google 继续|Continue with Google|通过 Apple 继续|Continue with Apple/i.test(bodyText)) {
      return makeLoggedOut(false);
    }
    if (host.endsWith('hark.com') && bodyText.trim() === 'Unauthorized') {
      return makeLoggedOut(true);
    }
    if (document.readyState !== 'complete') {
      return makeLoading();
    }
    if (host.endsWith('muse.ai') && /^\/blocked\b/.test(path)) {
      return makePageError('账号需安全验证或受限，请在右侧处理');
    }
    if (hasChatComposer && window.__maamAuthCheck && window.__maamAuthCheck.status === 'invalid') {
      if (Date.now() - (window.__maamAuthCheck.checkedAt || 0) > 5000) {
        window.__maamAuthCheck.status = 'ok';
        window.__maamAuthCheck.reason = '';
        window.__maamAuthCheck.checkedAt = Date.now();
      }
    }
    if (!hasChatComposer) {
      const ac = window.__maamAuthCheck;
      if (ac && ac.status === 'invalid') {
        return makeLoggedOut(true);
      }
      const hasErrorActionBtn = Array.from(document.querySelectorAll('button, a, [role="button"]')).some((el) => {
        if (!el || !el.isConnected) return false;
        const r = el.getBoundingClientRect();
        if (r.width < 16 || r.height < 12) return false;
        try {
          const st = getComputedStyle(el);
          if (st.display === 'none' || st.visibility === 'hidden') return false;
        } catch (_) {}
        const label = ((el.innerText || el.getAttribute('aria-label') || '') + '').trim();
        return /^(?:重试|Try again|退出|Log out)$/i.test(label);
      });
      const isErrorScreen =
        /(?:^|\n)\s*(?:出错了|Something went wrong|Application Error|应用出错)\s*(?:\n|$)/.test(bodyText) &&
        hasErrorActionBtn;
      const isDegradedMuseSettings =
        host.endsWith('muse.ai') &&
        /(?:Meta\s*账户|Meta\s*Account)/i.test(bodyText) &&
        /(?:帮助与支持|Help\s*&\s*support)/i.test(bodyText) &&
        /(?:^|\n)\s*(?:退出|Log out)\s*(?:\n|$)/i.test(bodyText) &&
        !/订阅|使用量|用量|Subscription|Usage|Billing/i.test(bodyText);
      if (typeof window.__maamVerifySession === 'function') {
        window.__maamVerifySession(false);
      }
      if (isErrorScreen || isDegradedMuseSettings) {
        if (!ac || ac.inflight || !ac.checkedAt) {
          return makeLoading();
        }
        if (ac.status === 'invalid') {
          return makeLoggedOut(true);
        }
        return makePageError(isDegradedMuseSettings ? '登录态异常，请退出或清理重登' : '页面出错，请检查代理或清理重登');
      }
      if (!/^\/(settings|profile|account)\b/.test(path) && (!ac || ac.status === 'unknown') && (!ac || ac.inflight || !ac.checkedAt)) {
        return makeLoading();
      }
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
    if (isPrimary) {
      if (!window.__maamApiEmail && typeof window.__maamRefreshIdentity === 'function') {
        window.__maamRefreshIdentity();
      }
      if (window.__maamAutoApprove === true && typeof window.__maamScanAutoApprove === 'function') {
        window.__maamScanAutoApprove();
      }
    }

    const bad = /^(support|privacy|help|legal|press|noreply|no-reply|info|security|contact|abuse|feedback|sales|terms|hello|team|admin|postmaster|notifications)@/i;
    const providerMail = /@(?:[a-z0-9-]+\.)?(?:muse\.ai|hark\.com|meta\.com|facebook\.com)$/i;
    const re = /[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}/g;
    const candidates = [];
    const pushRawEmails = (raw) => {
      if (typeof raw !== 'string' || !raw.includes('@')) return;
      const m = raw.match(re);
      if (m) candidates.push(...m);
    };
    if (window.__maamApiEmail) candidates.push(window.__maamApiEmail);

    try {
      if (Array.isArray(self.__next_f)) {
        const nfLen = self.__next_f.length;
        if (window.__maamNextFLen !== nfLen) {
          window.__maamNextFLen = nfLen;
          const nfEmails = [];
          let nfName = window.__maamNextFName || '';
          const badNameRe = /^(?:Meta|Muse|Hark|Button|Settings|Profile|Account|User|Avatar|Loading|Unknown|Hatch|Home|Chat|New Chat|Log in|Sign up|待登录账号|已登录账号|通用|设置)$/i;
          const keyedEmailRe = /\\?"(?:email|user_email|userEmail|contact_email|contactEmail|primary_email|primaryEmail|email_address|emailAddress|billing_email|billingEmail|customer_email|customerEmail|login_email|loginEmail|account_email|accountEmail|verified_email|verifiedEmail|display_label|displayLabel|contact_point|contactPoint)\\?"\s*:\s*\\?"([a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,})\\?"/g;
          const keyedNameRe = /\\?"(?:display_name|displayName|full_name|fullName|preferred_username)\\?"\s*:\s*\\?"([^"\\]{2,60})\\?"/g;
          for (let i = 0; i < nfLen; i++) {
            const entry = self.__next_f[i];
            const chunkStr = Array.isArray(entry) && typeof entry[1] === 'string'
              ? entry[1]
              : (typeof entry === 'string' ? entry : '');
            if (!chunkStr) continue;
            if (chunkStr.includes('@') && chunkStr.length < 180000) {
              let km;
              while ((km = keyedEmailRe.exec(chunkStr)) !== null) nfEmails.push(km[1]);
              if (nfEmails.length === 0) {
                const gm = chunkStr.match(re);
                if (gm) nfEmails.push(...gm);
              }
            }
            if (!nfName && chunkStr.length < 180000 && /user_id|userId|viewer|profile|account|email/i.test(chunkStr)) {
              const nm = keyedNameRe.exec(chunkStr);
              keyedNameRe.lastIndex = 0;
              if (nm && nm[1]) {
                const cn = nm[1].replace(/\s+/g, ' ').trim();
                if (cn.length >= 2 && cn.length <= 60 && !/^https?:\/\//i.test(cn) && !/^\d+$/.test(cn) && !badNameRe.test(cn)) {
                  nfName = cn;
                }
              }
            }
          }
          window.__maamNextFEmails = nfEmails;
          if (nfName) window.__maamNextFName = nfName;
        }
        if (Array.isArray(window.__maamNextFEmails)) {
          candidates.push(...window.__maamNextFEmails);
        }
      }
    } catch (_) {}

    try {
      if (window.__NEXT_DATA__ && typeof window.__NEXT_DATA__ === 'object') {
        const nd = JSON.stringify(window.__NEXT_DATA__);
        if (nd && nd.includes('@') && nd.length < 120000) pushRawEmails(nd);
      }
    } catch (_) {}

    document.querySelectorAll('[data-email],[aria-label*="@"],[title*="@"],a[href^="mailto:"]').forEach((el) => {
      const raw = (el.getAttribute('data-email') || el.getAttribute('aria-label') || el.getAttribute('title') || el.getAttribute('href') || '').replace(/^mailto:/i, '');
      pushRawEmails(raw);
    });
    const scanStorage = (store) => {
      try {
        for (let i = 0; i < store.length; i++) {
          const k = store.key(i);
          const val = k ? store.getItem(k) : '';
          if (!val || val.length >= 12000) continue;
          pushRawEmails(val);
          if (val.includes('%40')) {
            try { pushRawEmails(decodeURIComponent(val)); } catch (_) {}
          }
        }
      } catch (_) {}
    };
    scanStorage(localStorage);
    scanStorage(sessionStorage);
    try {
      const rawCookie = document.cookie || '';
      if (rawCookie.includes('@') || rawCookie.includes('%40')) {
        pushRawEmails(rawCookie);
        try { pushRawEmails(decodeURIComponent(rawCookie)); } catch (_) {}
      }
    } catch (_) {}
    pushRawEmails(bodyText);

    const validEmails = [];
    for (const c of candidates) {
      const clean = String(c || '').toLowerCase();
      if (clean && !bad.test(clean) && !clean.endsWith('@example.com') && !clean.endsWith('@sentry.io') && !clean.endsWith('@wixpress.com')) {
        if (!validEmails.includes(clean)) validEmails.push(clean);
      }
    }
    const foundEmail = validEmails.find((e) => !providerMail.test(e)) || validEmails[0] || '';
    const displayName = (window.__maamApiName || window.__maamNextFName || '').trim();
    return JSON.stringify({
      auth: true,
      loggedOut: false,
      staleSession: false,
      pageError: '',
      typed: typed || phone,
      email: foundEmail,
      displayName,
      avatar,
      quotaText: quota.text || '',
      quotaPct: quota.pct,
      quotaDetail: quota.detail || null,
      pendingHotkey
    });
  } catch (_) {
    return JSON.stringify({ auth: false, loggedOut: false, staleSession: false, pageError: '', typed: '', email: '', displayName: '', avatar: '', quotaText: '', quotaPct: null, quotaDetail: null, pendingHotkey: null });
  }
})()"#;

#[cfg(desktop)]
pub(crate) fn build_new_chat_click_js(adapter: &SiteAdapterConfig) -> String {
  let adapter_json = serde_json::to_string(adapter).unwrap_or_else(|_| "{}".to_string());
  r#"((adapter) => {
  try {
    const sels = Array.isArray(adapter.newChatSelectors) ? adapter.newChatSelectors : [];
    for (const sel of sels) {
      if (!sel) continue;
      let nodes = [];
      try { nodes = Array.from(document.querySelectorAll(sel)); } catch (_) { continue; }
      for (const el of nodes) {
        if (!el || !el.isConnected || el.disabled || el.getAttribute('aria-disabled') === 'true') continue;
        const r = el.getBoundingClientRect();
        if (r.width < 8 || r.height < 8) continue;
        el.click();
        return JSON.stringify({ clicked: true, selector: sel });
      }
    }
    return JSON.stringify({ clicked: false });
  } catch (err) {
    return JSON.stringify({ clicked: false, error: String(err) });
  }
})"#
    .to_string()
    + &format!("({adapter_json})")
}

#[cfg(desktop)]
pub(crate) fn build_group_attach_file_js(
  adapter: &SiteAdapterConfig,
  attachment: &GroupAttachment,
) -> String {
  let adapter_json = serde_json::to_string(adapter).unwrap_or_else(|_| "{}".to_string());
  let name_json =
    serde_json::to_string(&attachment.filename).unwrap_or_else(|_| "\"draft.md\"".to_string());
  let mime_json = serde_json::to_string(&attachment.mime_type)
    .unwrap_or_else(|_| "\"text/markdown\"".to_string());
  let content_json =
    serde_json::to_string(&attachment.content).unwrap_or_else(|_| "\"\"".to_string());
  r#"((adapter, fileName, mimeType, content) => {
  try {

    const isVisibleEl = (el, minW = 16, minH = 12) => {
      if (!el || !el.isConnected) return false;
      const r = el.getBoundingClientRect();
      if (r.width < minW || r.height < minH) return false;
      let cur = el;
      for (let d = 0; cur && cur.nodeType === 1 && d < 8; d++) {
        try {
          const st = getComputedStyle(cur);
          if (st.display === 'none' || st.visibility === 'hidden') return false;
        } catch (_) {}
        cur = cur.parentElement;
      }
      return true;
    };

    let composer = null;
    const compSels = Array.isArray(adapter.composerSelectors) ? adapter.composerSelectors : [];
    for (const sel of compSels) {
      if (!sel) continue;
      try {
        const list = Array.from(document.querySelectorAll(sel)).filter((el) => isVisibleEl(el, 80, 16));
        if (list.length > 0) {
          list.sort((a, b) => b.getBoundingClientRect().bottom - a.getBoundingClientRect().bottom);
          composer = list[0];
          break;
        }
      } catch (_) {}
    }
    if (!composer) {
      const fb = Array.from(document.querySelectorAll('textarea, [contenteditable="true"], [role="textbox"]'))
        .filter((el) => isVisibleEl(el, 100, 16));
      fb.sort((a, b) => b.getBoundingClientRect().bottom - a.getBoundingClientRect().bottom);
      composer = fb[0] || null;
    }

    const file = new File([content], fileName, { type: mimeType });
    const dt = new DataTransfer();
    dt.items.add(file);

    const fileInputs = Array.from(document.querySelectorAll('input[type="file"]')).filter((inp) => {
      if (inp.disabled) return false;
      const acc = (inp.getAttribute('accept') || '').toLowerCase();
      if (acc && /(?:image|audio|video)\//.test(acc) && !/(?:\.md|\.txt|text\/|application\/|\*)/.test(acc)) {
        return false;
      }
      return true;
    });

    if (fileInputs.length > 0) {
      let picked = fileInputs[fileInputs.length - 1];
      if (composer) {
        const bar = composer.closest('form') || composer.parentElement?.parentElement || composer.parentElement;
        if (bar) {
          const inBar = fileInputs.find((inp) => bar.contains(inp));
          if (inBar) picked = inBar;
        }
      }
      try {
        const desc = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'files');
        if (desc && typeof desc.set === 'function') {
          desc.set.call(picked, dt.files);
        } else {
          picked.files = dt.files;
        }
      } catch (_) {
        picked.files = dt.files;
      }
      picked.dispatchEvent(new Event('input', { bubbles: true }));
      picked.dispatchEvent(new Event('change', { bubbles: true }));
      return JSON.stringify({ dispatched: true, via: 'file_input' });
    }

    const dropTarget = composer || document.body;
    if (!dropTarget) return JSON.stringify({ dispatched: false });
    let pasted = false;
    try {
      dropTarget.focus();
      const pasteEvt = new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: dt });
      dropTarget.dispatchEvent(pasteEvt);
      pasted = true;
    } catch (_) {}
    try {
      const dragOpts = { bubbles: true, cancelable: true, dataTransfer: dt };
      dropTarget.dispatchEvent(new DragEvent('dragenter', dragOpts));
      dropTarget.dispatchEvent(new DragEvent('dragover', dragOpts));
      dropTarget.dispatchEvent(new DragEvent('drop', dragOpts));
      return JSON.stringify({ dispatched: true, via: pasted ? 'paste_drop' : 'drop' });
    } catch (_) {
      return JSON.stringify({ dispatched: pasted, via: pasted ? 'paste' : 'none' });
    }
  } catch (err) {
    return JSON.stringify({ dispatched: false, error: String(err) });
  }
})"#
    .to_string()
    + &format!("({adapter_json}, {name_json}, {mime_json}, {content_json})")
}

#[cfg(desktop)]
pub(crate) fn build_group_check_attached_js(adapter: &SiteAdapterConfig, filename: &str) -> String {
  let adapter_json = serde_json::to_string(adapter).unwrap_or_else(|_| "{}".to_string());
  let name_json = serde_json::to_string(filename).unwrap_or_else(|_| "\"\"".to_string());
  r#"((adapter, fileName) => {
  try {
    const isVisibleEl = (el) => {
      if (!el || !el.isConnected) return false;
      const r = el.getBoundingClientRect();
      return r.width > 12 && r.height > 12;
    };
    let composer = null;
    const compSels = Array.isArray(adapter.composerSelectors) ? adapter.composerSelectors : [];
    for (const sel of compSels) {
      if (!sel) continue;
      try {
        const list = Array.from(document.querySelectorAll(sel)).filter(isVisibleEl);
        if (list.length > 0) {
          list.sort((a, b) => b.getBoundingClientRect().bottom - a.getBoundingClientRect().bottom);
          composer = list[0];
          break;
        }
      } catch (_) {}
    }
    if (!composer) {
      const fb = Array.from(document.querySelectorAll('textarea, [contenteditable="true"], [role="textbox"]')).filter(isVisibleEl);
      fb.sort((a, b) => b.getBoundingClientRect().bottom - a.getBoundingClientRect().bottom);
      composer = fb[0] || null;
    }
    const bar = composer ? (composer.closest('form') || composer.parentElement?.parentElement?.parentElement || composer.parentElement) : document.body;
    const barText = (bar && bar.innerText ? bar.innerText : '').toLowerCase();
    const stem = fileName.toLowerCase();
    const hasName = Boolean(stem && (barText.includes(stem) || (document.body?.innerText || '').toLowerCase().includes(stem)));
    const uploading = /上传中|uploading|parsing|解析中/i.test(barText);
    return JSON.stringify({ ready: hasName && !uploading, uploading, hasName });
  } catch (err) {
    return JSON.stringify({ ready: false, error: String(err) });
  }
})"#
    .to_string()
    + &format!("({adapter_json}, {name_json})")
}

#[cfg(desktop)]
pub(crate) fn build_hark_click_preview_card_js(preview_text: &str) -> String {
  let prev_json = serde_json::to_string(preview_text).unwrap_or_else(|_| "\"\"".to_string());
  r#"((previewTextArg) => {
  try {
    const previewText = (previewTextArg || '').trim();
    if (!previewText) return JSON.stringify({ clicked: false });
    const winH = window.innerHeight || document.documentElement.clientHeight || 800;
    const isVisibleEl = (el, minW = 140, minH = 56) => {
      if (!el || !el.isConnected) return false;
      const r = el.getBoundingClientRect();
      if (r.width < minW || r.height < minH || r.height > winH * 0.72) return false;
      let cur = el;
      for (let d = 0; cur && cur.nodeType === 1 && d < 8; d++) {
        try {
          const st = getComputedStyle(cur);
          if (st.display === 'none' || st.visibility === 'hidden') return false;
          if (parseFloat(st.opacity) < 0.05) return false;
        } catch (_) {}
        cur = cur.parentElement;
      }
      return true;
    };

    const composer = Array.from(document.querySelectorAll('textarea, [contenteditable="true"]'))
      .find((el) => isVisibleEl(el, 80, 16));
    let col = document.querySelector('main, [role="main"]') || document.body;
    if (composer) {
      let cur = composer.parentElement;
      while (cur && cur !== document.body && cur !== document.documentElement) {
        const r = cur.getBoundingClientRect();
        if (r.height >= winH * 0.5 && r.width >= 260) { col = cur; break; }
        cur = cur.parentElement;
      }
    }
    const bar = composer ? (composer.closest('form') || composer.parentElement?.parentElement) : null;
    const previewLines = previewText.split(/\r?\n/).map((l) => l.trim()).filter((l) => l.length >= 2);
    const tailLine = previewLines.length ? previewLines[previewLines.length - 1] : '';
    const midLine = previewLines.length >= 2 ? previewLines[Math.floor(previewLines.length / 2)] : '';

    const nodes = Array.from(col.querySelectorAll('div, section, article, button, [role="button"]'));
    let bestCard = null;
    for (let i = nodes.length - 1; i >= 0; i--) {
      const el = nodes[i];
      if (!isVisibleEl(el, 180, 64)) continue;
      if ((bar && bar.contains(el)) || el.closest('nav, aside, header, footer, form')) continue;
      if (el.querySelector('textarea, input')) continue;
      const txt = (el.innerText || '').trim();
      if (!txt || txt.length < 24 || txt.length > 2600) continue;
      const hitsTail = tailLine && txt.includes(tailLine);
      const hitsMid = midLine && txt.includes(midLine);
      if (!hitsTail && !hitsMid) continue;
      if (!previewText.includes(txt.slice(0, Math.min(24, txt.length))) && txt.length > previewText.length + 120) continue;

      let looksLikeCard = el.tagName === 'BUTTON' || el.getAttribute('role') === 'button';
      try {
        const st = getComputedStyle(el);
        if (st.cursor === 'pointer' || st.overflowY === 'hidden' || st.overflow === 'hidden') looksLikeCard = true;
        if ((st.maskImage && st.maskImage !== 'none') || (st.webkitMaskImage && st.webkitMaskImage !== 'none')) looksLikeCard = true;
      } catch (_) {}
      if (el.scrollHeight > el.clientHeight + 8) looksLikeCard = true;
      if (!looksLikeCard) {
        const kids = el.querySelectorAll('[style*="gradient" i], [style*="mask" i], button, [role="button"]');
        if (kids.length > 0) looksLikeCard = true;
      }
      if (!looksLikeCard) continue;

      if (!bestCard || el.getBoundingClientRect().top > bestCard.getBoundingClientRect().top) {
        bestCard = el;
      } else if (bestCard && bestCard.contains(el)) {
        bestCard = el;
      }
    }

    if (!bestCard) return JSON.stringify({ clicked: false });
    const cardPreview = (bestCard.innerText || '').trim();
    const firstLine = cardPreview.split(/\r?\n/).map((l) => l.trim()).find(Boolean) || '';
    let introPrefix = '';
    if (firstLine) {
      const idx = previewText.indexOf(firstLine);
      if (idx > 0) introPrefix = previewText.slice(0, idx).trim();
    }
    bestCard.click();
    return JSON.stringify({ clicked: true, introPrefix, cardPreview });
  } catch (err) {
    return JSON.stringify({ clicked: false, error: String(err) });
  }
})"#
    .to_string()
    + &format!("({prev_json})")
}

#[cfg(desktop)]
pub(crate) fn build_hark_read_expanded_js(card_preview: &str) -> String {
  let card_json = serde_json::to_string(card_preview).unwrap_or_else(|_| "\"\"".to_string());
  r#"((cardPreviewArg) => {
  try {
    const cardPreview = (cardPreviewArg || '').trim();
    const anchor = cardPreview.split(/\r?\n/).map((l) => l.trim()).find((l) => l.length >= 2) || '';
    const winW = window.innerWidth || document.documentElement.clientWidth || 1000;
    const winH = window.innerHeight || document.documentElement.clientHeight || 800;

    const isVisibleEl = (el) => {
      if (!el || !el.isConnected) return false;
      const r = el.getBoundingClientRect();
      if (r.width < winW * 0.3 || r.height < winH * 0.35) return false;
      let cur = el;
      for (let d = 0; cur && cur.nodeType === 1 && d < 8; d++) {
        try {
          const st = getComputedStyle(cur);
          if (st.display === 'none' || st.visibility === 'hidden' || parseFloat(st.opacity) < 0.05) return false;
        } catch (_) {}
        cur = cur.parentElement;
      }
      return true;
    };

    const extractStructuredText = (rootNode) => {
      if (!rootNode) return '';
      const skipSel = 'nav, aside, header, footer, button, script, style, svg, noscript, [hidden], [aria-hidden="true"]';
      const blockSel = 'p, div, li, tr, h1, h2, h3, h4, h5, h6, pre, blockquote, article, section';
      const parts = [];
      let lastBlk = null;
      let lastRow = null;
      let lastCell = null;
      const walker = document.createTreeWalker(rootNode, 4);
      let n;
      while ((n = walker.nextNode())) {
        const v = n.nodeValue;
        if (!v || !v.trim()) continue;
        const p = n.parentElement;
        if (!p || (p.closest && p.closest(skipSel))) continue;
        const row = p.closest ? p.closest('tr') : null;
        const cell = p.closest ? p.closest('td, th') : null;
        const blk = (p.closest && p.closest(blockSel)) || p;
        if (parts.length > 0) {
          if (row && row === lastRow && cell && cell !== lastCell) parts.push(' | ');
          else if (blk !== lastBlk) parts.push('\n');
        }
        parts.push(v.trim());
        lastBlk = blk;
        lastRow = row;
        lastCell = cell;
      }
      return parts.join('').replace(/\n{3,}/g, '\n\n').trim();
    };

    const candidates = Array.from(document.querySelectorAll('[role="dialog"], [aria-modal="true"], dialog, article, main, section, div'))
      .filter((el) => {
        if (!isVisibleEl(el) || el === document.body) return false;
        if (el.closest('nav, aside')) return false;
        if (el.querySelector('textarea')) return false;
        const txt = (el.innerText || '').trim();
        if (!txt || txt.length <= cardPreview.length + 20) return false;
        if (anchor && !txt.includes(anchor)) return false;
        return true;
      });

    if (candidates.length === 0) return JSON.stringify({ text: '' });
    candidates.sort((a, b) => {
      const aModal = a.matches('[role="dialog"], [aria-modal="true"], dialog') ? 1 : 0;
      const bModal = b.matches('[role="dialog"], [aria-modal="true"], dialog') ? 1 : 0;
      if (aModal !== bModal) return bModal - aModal;
      const ra = a.getBoundingClientRect();
      const rb = b.getBoundingClientRect();
      return (ra.width * ra.height) - (rb.width * rb.height);
    });

    let extracted = extractStructuredText(candidates[0]);
    extracted = extracted
      .replace(/^(?:摘要|Summary|Brief)\s*\r?\n+/i, '')
      .replace(/(?:\r?\n+\s*(?:摘要|Summary|Brief)\s*)+$/i, '')
      .trim();
    return JSON.stringify({ text: extracted });
  } catch (err) {
    return JSON.stringify({ text: '', error: String(err) });
  }
})"#
    .to_string()
    + &format!("({card_json})")
}

#[cfg(desktop)]
pub(crate) fn build_hark_close_expanded_js() -> String {
  r#"(() => {
  try {
    const btns = Array.from(document.querySelectorAll('button, [role="button"]')).filter((b) => {
      if (!b || !b.isConnected || b.disabled) return false;
      if (b.closest('nav, aside')) return false;
      const r = b.getBoundingClientRect();
      return r.width >= 20 && r.height >= 20 && r.top >= 0 && r.top <= 110 && r.left >= 0 && r.left <= 140;
    });
    const labeled = btns.find((b) => /close|collapse|back|收起|关闭|返回/i.test((b.getAttribute('aria-label') || '') + ' ' + (b.getAttribute('title') || '')));
    const target = labeled || btns.find((b) => Boolean(b.querySelector('svg')));
    if (target) {
      target.click();
      return JSON.stringify({ closed: true, via: 'button' });
    }
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', code: 'Escape', keyCode: 27, bubbles: true }));
    return JSON.stringify({ closed: true, via: 'escape' });
  } catch (err) {
    return JSON.stringify({ closed: false, error: String(err) });
  }
})()"#
    .to_string()
}

#[cfg(desktop)]
pub(crate) fn build_group_send_js(
  adapter: &SiteAdapterConfig,
  job_id: &str,
  prompt_text: &str,
) -> String {
  let adapter_json = serde_json::to_string(adapter).unwrap_or_else(|_| "{}".to_string());
  let job_json = serde_json::to_string(job_id).unwrap_or_else(|_| "\"\"".to_string());
  let prompt_json = serde_json::to_string(prompt_text).unwrap_or_else(|_| "\"\"".to_string());
  r#"((adapter, jobId, promptText) => {
  try {
    const host = (location.hostname || '').toLowerCase();
    const path = (location.pathname || '').toLowerCase();
    const isVisibleEl = (el, minW = 16, minH = 12) => {
      if (!el || !el.isConnected) return false;
      const r = el.getBoundingClientRect();
      if (r.width < minW || r.height < minH) return false;
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

    const findAuthPrompt = () => {
      if (typeof window.__maamFindAuthPrompt === 'function') {
        const hit = window.__maamFindAuthPrompt();
        if (hit) return hit;
      }
      const containers = document.querySelectorAll('[role="dialog"], [role="alertdialog"], [aria-modal="true"], dialog, [class*="permission" i], [class*="approval" i], [class*="authorize" i], [class*="consent" i]');
      for (const box of containers) {
        if (!isVisibleEl(box, 120, 60)) continue;
        const txt = (box.innerText || '').trim();
        if (!/授权|允许|始终允许|批准|请求访问|权限|approve|authorize|permission/i.test(txt)) continue;
        const btns = box.querySelectorAll('button, [role="button"]');
        for (const b of btns) {
          if (!isVisibleEl(b, 20, 16) || b.disabled) continue;
          const label = ((b.innerText || b.getAttribute('aria-label') || '') + '').trim();
          if (/^(允许|始终允许|本次允许|批准|确认授权|同意|allow|allow once|always allow|approve|authorize)$/i.test(label)) {
            return txt.slice(0, 140);
          }
        }
      }
      return '';
    };

    if (window.__maamAutoApprove === true && typeof window.__maamScanAutoApprove === 'function') {
      if (window.__maamScanAutoApprove()) {
        return JSON.stringify({ ok: false, status: 'no_composer', error: '' });
      }
    }

    const authReason = findAuthPrompt();
    if (authReason) {
      return JSON.stringify({ ok: false, status: 'needs_auth', error: '检测到页面授权确认，请前往该账号页面点击允许' });
    }

    const loginInputs = document.querySelectorAll('input[type="password"], input[type="email"]');
    let hasLoginForm = false;
    for (const inp of loginInputs) {
      if (isVisibleEl(inp, 40, 18)) { hasLoginForm = true; break; }
    }

    const isUsableComposer = (el) => {
      if (!el || el.disabled || el.readOnly) return false;
      if (el.getAttribute('aria-disabled') === 'true') return false;
      if (el.getAttribute('contenteditable') === 'false') return false;
      const type = (el.getAttribute('type') || '').toLowerCase();
      if (type === 'hidden' || type === 'password' || type === 'email' || type === 'search') return false;
      const ph = ((el.getAttribute('placeholder') || el.getAttribute('aria-label') || '') + '').toLowerCase();
      if (/\b(search|filter)\b|搜索|筛选/.test(ph)) return false;
      return isVisibleEl(el, 80, 16);
    };

    let composer = null;
    const adapterComposers = Array.isArray(adapter.composerSelectors) ? adapter.composerSelectors : [];
    for (const sel of adapterComposers) {
      if (!sel) continue;
      let list = [];
      try { list = Array.from(document.querySelectorAll(sel)); } catch (_) { continue; }
      const matched = list.filter(isUsableComposer);
      if (matched.length > 0) {
        matched.sort((a, b) => b.getBoundingClientRect().bottom - a.getBoundingClientRect().bottom);
        composer = matched[0];
        break;
      }
    }

    if (!composer) {
      const rawComposers = Array.from(document.querySelectorAll('textarea, [contenteditable="true"], [role="textbox"]'));
      const candidates = rawComposers.filter((el) => isUsableComposer(el) && isVisibleEl(el, 100, 16));
      candidates.sort((a, b) => {
        const score = (el) => {
          const r = el.getBoundingClientRect();
          let s = r.bottom;
          if (el.closest('form')) s += 400;
          if (el.tagName === 'TEXTAREA' || el.id === 'prompt-textarea') s += 120;
          return s;
        };
        return score(b) - score(a);
      });
      composer = candidates[0] || null;
    }

    if (!composer) {
      if ((window.__maamAuthCheck && window.__maamAuthCheck.status === 'invalid') || hasLoginForm || /^\/(login|signin|signup|auth)\b/.test(path)) {
        return JSON.stringify({ ok: false, status: 'logged_out', error: '该账号登录态已失效，请前往重新登录' });
      }
      const bodyTxt = (document.body && document.body.innerText) || '';
      const hasRetryBtn = Array.from(document.querySelectorAll('button, a, [role="button"]')).some((el) => {
        if (!isVisibleEl(el, 16, 12)) return false;
        const label = ((el.innerText || el.getAttribute('aria-label') || '') + '').trim();
        return /^(?:重试|Try again|退出|Log out)$/i.test(label);
      });
      if (/^\/blocked\b/.test(path) || (/(?:^|\n)\s*(?:出错了|Something went wrong|Application Error|应用出错)\s*(?:\n|$)/.test(bodyTxt) && hasRetryBtn)) {
        return JSON.stringify({ ok: false, status: 'page_error', error: '该账号页面出错，请检查代理或清理重登' });
      }
      if (host.includes('hark.com')) {
        const tabs = Array.from(document.querySelectorAll('button, a, [role="tab"]'));
        for (const tb of tabs) {
          if (!isVisibleEl(tb, 20, 14)) continue;
          const label = (tb.innerText || tb.getAttribute('aria-label') || '').trim();
          if (/^(聊天|Chat)$/i.test(label)) {
            try { tb.click(); } catch (_) {}
            break;
          }
        }
      }
      return JSON.stringify({ ok: false, status: 'no_composer', error: '当前页面未找到聊天输入框，请切换至该账号会话页检查' });
    }

    const resolveChatColumn = (comp) => {
      const winH = window.innerHeight || document.documentElement.clientHeight || 800;
      if (comp && comp.isConnected) {
        let cur = comp.parentElement;
        while (cur && cur !== document.body && cur !== document.documentElement) {
          const r = cur.getBoundingClientRect();
          if (r.height >= winH * 0.5 && r.width >= 220) {
            return cur;
          }
          cur = cur.parentElement;
        }
      }
      return document.querySelector('main, [role="main"]') || document.body;
    };
    const resolveComposerBar = (comp, col) => {
      const winH = window.innerHeight || document.documentElement.clientHeight || 800;
      if (!comp || !comp.isConnected) return null;
      let bar = comp;
      let cur = comp.parentElement;
      while (cur && cur !== col && cur !== document.body) {
        const r = cur.getBoundingClientRect();
        if (r.height >= winH * 0.36) break;
        bar = cur;
        cur = cur.parentElement;
      }
      return bar;
    };

    const chatCol = resolveChatColumn(composer);
    const composerBar = resolveComposerBar(composer, chatCol);
    const colRect = chatCol.getBoundingClientRect();

    const adapterAsst = Array.isArray(adapter.assistantMessageSelectors) ? adapter.assistantMessageSelectors.filter(Boolean) : [];
    const baseMsgSelector = '[data-message-author-role], [data-role], [data-testid="message-bubble"], [data-testid="message-text"], [data-testid*="assistant" i], [data-testid*="response" i], [data-testid*="turn" i], article, [class*="markdown" i], [class*="prose" i]';
    const msgSelector = adapterAsst.length ? adapterAsst.join(', ') + ', ' + baseMsgSelector : baseMsgSelector;
    const isExcludedContainer = (el) => Boolean((composerBar && composerBar.contains(el)) || (el.closest && el.closest('nav, aside, header, footer, form')));
    const isMessageWrapper = (el) => {
      const tid = (el.getAttribute('data-testid') || '').toLowerCase();
      if (/list|container|scroll|wrapper|feed|history|timestamp|time|date|header|footer|avatar/.test(tid)) return true;
      const nested = el.querySelectorAll('[data-message-author-role], [data-role], [data-testid="message-bubble"], [data-testid="message-text"], article').length;
      return nested > 1;
    };
    const extractCleanText = (rootNode, skipChrome) => {
      if (!rootNode) return '';
      const skipSel = skipChrome
        ? 'nav, aside, header, footer, form, button, script, style, time, svg, noscript, template, [hidden], [aria-hidden="true"]'
        : 'script, style, time, svg, noscript, template, [hidden], [aria-hidden="true"]';
      const blockSel = 'p, div, li, tr, h1, h2, h3, h4, h5, h6, pre, blockquote, article, section';
      const parts = [];
      let lastBlk = null;
      try {
        const walker = document.createTreeWalker(rootNode, 4);
        let n;
        while ((n = walker.nextNode())) {
          const v = n.nodeValue;
          if (!v || !v.trim()) continue;
          const p = n.parentElement;
          if (!p || (skipChrome && composerBar && composerBar.contains(p)) || (p.closest && p.closest(skipSel))) continue;
          const blk = (p.closest && p.closest(blockSel)) || p;
          if (parts.length > 0 && blk !== lastBlk) parts.push('\n');
          parts.push(v.trim());
          lastBlk = blk;
        }
      } catch (_) {}
      return parts.join('').replace(/\n{3,}/g, '\n\n').trim();
    };
    const getNodeText = (el) => {
      if (!el) return '';
      const t = (el.innerText || '').trim();
      if (t) return t;
      return extractCleanText(el, false);
    };
    const getSearchRoot = () => {
      const listSels = Array.isArray(adapter.messageListSelectors) ? adapter.messageListSelectors : [];
      for (const sel of listSels) {
        if (!sel || sel === 'main') continue;
        try {
          const el = chatCol.querySelector(sel);
          if (el && el.getBoundingClientRect().height > 10) return el;
        } catch (_) {}
      }
      return chatCol.querySelector('[data-testid="message-list"], [data-testid*="chat-messages" i], [data-testid*="message-feed" i]') || chatCol;
    };
    const collectBaseline = () => {
      const sRoot = getSearchRoot();
      const texts = [];
      const els = [];
      let list = [];
      try { list = sRoot.querySelectorAll(msgSelector); } catch (_) { list = sRoot.querySelectorAll(baseMsgSelector); }
      for (const el of list) {
        if (isExcludedContainer(el) || isMessageWrapper(el)) continue;
        if (!isVisibleEl(el, 20, 12)) continue;
        els.push(el);
        const txt = getNodeText(el);
        if (txt) texts.push(txt);
      }
      return { texts, els };
    };

    const getCleanMainText = () => extractCleanText(getSearchRoot(), true);

    const baselineInfo = collectBaseline();
    const baselineBlocks = baselineInfo.texts;
    const baselineMainText = getCleanMainText();
    const normWs = (s) => (s || '').replace(/\s+/g, ' ').trim();
    const stripBaselineTail = (raw) => {
      let s = (raw || '').trim();
      for (let k = 0; k < 6; k++) {
        const next = s.replace(/(?:\r?\n|\s|^)(?:\d{1,2}:\d{2}|Muse|Hark|已阅读|已发送|已连接|空闲中|复制|Copy)$/i, '').trim();
        if (next === s) break;
        s = next;
      }
      return s;
    };
    const baselineCleanSet = new Set();
    for (const bt of baselineBlocks) {
      const c = stripBaselineTail(bt);
      if (c) baselineCleanSet.add(c);
    }
    for (const ln of baselineMainText.split(/\r?\n/)) {
      const c = stripBaselineTail(ln);
      if (c) baselineCleanSet.add(c);
    }
    const sRoot = getSearchRoot();
    const baselineDomEls = new WeakSet();
    try {
      for (const el of sRoot.querySelectorAll('*')) {
        baselineDomEls.add(el);
      }
    } catch (_) {}
    let lastBaselineEl = baselineInfo.els.length > 0 ? baselineInfo.els[baselineInfo.els.length - 1] : null;
    if (!lastBaselineEl && sRoot) {
      try {
        const allRootEls = Array.from(sRoot.querySelectorAll('*'));
        for (let i = allRootEls.length - 1; i >= 0; i--) {
          const el = allRootEls[i];
          if (!el || isExcludedContainer(el) || el.children.length > 0) continue;
          if (!isVisibleEl(el, 8, 8)) continue;
          if ((el.textContent || '').trim()) {
            lastBaselineEl = el;
            break;
          }
        }
      } catch (_) {}
    }
    const baselineMainNorm = normWs(baselineMainText);
    const baselineBlockNorms = baselineBlocks
      .map((bt) => normWs(stripBaselineTail(bt)))
      .filter((s) => s.length >= 8);
    const lines = promptText.split('\n').map((l) => l.trim()).filter(Boolean);
    const isFooterLine = (l) => l.includes('不要复述上方方括号标题') || l.includes('不要重复上方协议标签') || l.includes('不要重复以上协议标签');
    const promptNeedle = (lines.length >= 2 && isFooterLine(lines[lines.length - 1]))
      ? lines[lines.length - 2]
      : (lines.length > 0 ? lines[lines.length - 1] : promptText.trim());
    const headCandidates = lines.filter((l) => !/^\[[^\]]{1,24}\]$/.test(l) && !isFooterLine(l));
    const headPick = headCandidates.find((l) => {
      const n40 = normWs(l).slice(0, 40);
      return n40 && (!baselineMainNorm || !baselineMainNorm.includes(n40));
    }) || headCandidates[0] || '';
    const promptHead = headPick.slice(0, 48);
    const endMarker = '（请直接回答「当前问题」，不要复述上方方括号标题）';
    const baselineMarkerCount = baselineMainText ? (baselineMainText.split(endMarker).length - 1) : 0;
    const baselineNeedleCount = (baselineMainText && promptNeedle) ? (baselineMainText.split(promptNeedle).length - 1) : 0;
    const baselineHeadCount = (baselineMainText && promptHead) ? (baselineMainText.split(promptHead).length - 1) : 0;
    const busyRe = /Researching\s+\w+|Searching\s+for\b|正在搜索|正在联网搜索|正在检索|正在读取|正在解析|正在加载|正在生成|正在处理|努力思考中|努力工作中/gi;
    const baselineBodyBusyCount = ((document.body && document.body.innerText) ? (document.body.innerText.match(busyRe) || []).length : 0);

    if (window.__maamNet) {
      window.__maamNet.esText = '';
      window.__maamNet.wsText = '';
    }

    window.__maamGroupJobs = window.__maamGroupJobs || {};
    if (window.__maamGroupJobs[jobId] && window.__maamGroupJobs[jobId].observer) {
      try { window.__maamGroupJobs[jobId].observer.disconnect(); } catch (_) {}
    }
    const jobRec = {
      jobId,
      host,
      composerEl: composer,
      chatColRect: { left: colRect.left, right: colRect.right, width: colRect.width },
      promptText,
      promptNeedle,
      promptHead,
      baselineCount: baselineBlocks.length,
      baselineSet: new Set(baselineBlocks.map((t) => t.slice(0, 220))),
      baselineExact: new Set(baselineBlocks),
      baselineCleanSet,
      baselineEls: new Set(baselineInfo.els),
      baselineDomEls,
      lastBaselineEl,
      baselineMainText,
      baselineMainNorm,
      baselineBlockNorms,
      baselineLines: new Set(baselineMainText.split(/\r?\n/).map((l) => l.trim()).filter(Boolean)),
      baselineMarkerCount,
      baselineNeedleCount,
      baselineHeadCount,
      baselineBodyBusyCount,
      netStartSeq: (window.__maamNet && window.__maamNet.seq) || 0,
      netStartAt: Date.now(),
      startedAt: Date.now(),
      lastDomMutAt: Date.now(),
      observer: null,
      lastText: '',
      lastChangedAt: Date.now(),
    };
    try {
      if (typeof MutationObserver === 'function') {
        const obs = new MutationObserver(() => {
          jobRec.lastDomMutAt = Date.now();
        });
        obs.observe(getSearchRoot(), { childList: true, subtree: true, characterData: true });
        jobRec.observer = obs;
      }
    } catch (_) {}
    window.__maamGroupJobs[jobId] = jobRec;

    composer.focus();
    if (composer.tagName === 'TEXTAREA' || composer.tagName === 'INPUT') {
      const tracker = composer._valueTracker;
      if (tracker && typeof tracker.setValue === 'function') {
        try { tracker.setValue(''); } catch (_) {}
      }
      const proto = composer.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const desc = Object.getOwnPropertyDescriptor(proto, 'value');
      if (desc && typeof desc.set === 'function') {
        desc.set.call(composer, promptText);
      } else {
        composer.value = promptText;
      }
      try {
        composer.dispatchEvent(new InputEvent('input', { bubbles: true, cancelable: true, inputType: 'insertText', data: promptText }));
      } catch (_) {}
      composer.dispatchEvent(new Event('input', { bubbles: true }));
      composer.dispatchEvent(new Event('change', { bubbles: true }));
    } else {
      try {
        const sel = window.getSelection();
        if (sel) {
          sel.removeAllRanges();
          const range = document.createRange();
          range.selectNodeContents(composer);
          sel.addRange(range);
        }
        document.execCommand('insertText', false, promptText);
      } catch (_) {}
      if ((composer.innerText || '').trim() !== promptText.trim()) {
        composer.textContent = promptText;
        try {
          composer.dispatchEvent(new InputEvent('beforeinput', { bubbles: true, cancelable: true, inputType: 'insertText', data: promptText }));
        } catch (_) {}
        try {
          composer.dispatchEvent(new InputEvent('input', { bubbles: true, cancelable: true, inputType: 'insertText', data: promptText }));
        } catch (_) {
          composer.dispatchEvent(new Event('input', { bubbles: true }));
        }
      }
    }

    const isUsableSendBtn = (b) => {
      if (!b || b.disabled || b.getAttribute('aria-disabled') === 'true') return false;
      if (!isVisibleEl(b, 16, 16)) return false;
      const label = ((b.getAttribute('aria-label') || '') + ' ' + (b.getAttribute('title') || '') + ' ' + (b.getAttribute('data-testid') || '') + ' ' + (b.innerText || '')).trim().toLowerCase();
      if (/stop|停止|attach|upload|voice|mic|record|menu|上传|附件|附加|语音|录音|听写|dictat|模型/.test(label)) return false;
      return true;
    };

    const findSendBtn = (allowSvgFallback) => {
      const scopes = [];
      const form = composer.closest('form');
      if (form) scopes.push(form);
      let cur = composer.parentElement;
      for (let d = 0; cur && d < 6; d++) {
        if (!scopes.includes(cur)) scopes.push(cur);
        cur = cur.parentElement;
      }
      const adapterSendSels = Array.isArray(adapter.sendButtonSelectors) ? adapter.sendButtonSelectors : [];
      for (const sel of adapterSendSels) {
        if (!sel) continue;
        for (const scope of scopes) {
          try {
            const btns = Array.from(scope.querySelectorAll(sel));
            for (const b of btns) {
              if (isUsableSendBtn(b)) return b;
            }
          } catch (_) {}
        }
        try {
          const globalBtns = Array.from(document.querySelectorAll(sel));
          for (const b of globalBtns) {
            if (isUsableSendBtn(b)) return b;
          }
        } catch (_) {}
      }
      for (const scope of scopes) {
        const btns = Array.from(scope.querySelectorAll('button, [role="button"]'));
        for (const b of btns) {
          if (!isUsableSendBtn(b)) continue;
          const label = ((b.getAttribute('aria-label') || '') + ' ' + (b.getAttribute('title') || '') + ' ' + (b.getAttribute('data-testid') || '') + ' ' + (b.innerText || '')).trim().toLowerCase();
          if (b.getAttribute('type') === 'submit' || /send|submit|发送|提交|提问/.test(label)) {
            return b;
          }
        }
      }
      if (allowSvgFallback) {
        const near = composer.closest('form') || composer.parentElement?.parentElement || composer.parentElement;
        if (near) {
          const btns = Array.from(near.querySelectorAll('button, [role="button"]')).filter((b) => {
            if (!isUsableSendBtn(b)) return false;
            const label = ((b.getAttribute('aria-label') || '') + ' ' + (b.getAttribute('title') || '') + ' ' + (b.getAttribute('data-testid') || '') + ' ' + (b.innerText || '')).trim().toLowerCase();
            if (/plus|add|新建|历史/.test(label)) return false;
            return Boolean(b.querySelector('svg'));
          });
          if (btns.length > 0) return btns[btns.length - 1];
        }
      }
      return null;
    };

    const dispatchEnter = () => {
      try {
        composer.focus();
        ['keydown', 'keypress', 'keyup'].forEach((evType) => {
          composer.dispatchEvent(new KeyboardEvent(evType, {
            key: 'Enter',
            code: 'Enter',
            keyCode: 13,
            which: 13,
            bubbles: true,
            cancelable: true,
          }));
        });
      } catch (_) {}
    };

    let submitClicked = false;
    const trySubmit = (attempt) => {
      try {
        if (submitClicked) return;
        if (adapter.preferEnterSubmit && attempt === 0) {
          submitClicked = true;
          dispatchEnter();
          setTimeout(() => {
            try {
              const curBlocks = collectBaseline().texts;
              const rem = (composer.value || composer.innerText || '').trim();
              if (curBlocks.length <= baselineBlocks.length && rem.length >= Math.min(20, promptText.trim().length)) {
                const sb = findSendBtn(true);
                if (sb) sb.click();
              }
            } catch (_) {}
          }, 450);
          return;
        }
        const sendBtn = findSendBtn(attempt >= 4);
        if (sendBtn && !sendBtn.disabled && sendBtn.getAttribute('aria-disabled') !== 'true') {
          submitClicked = true;
          composer.focus();
          sendBtn.click();
          setTimeout(() => {
            try {
              const curBlocks = collectBaseline().texts;
              const rem = (composer.value || composer.innerText || '').trim();
              if (curBlocks.length <= baselineBlocks.length && rem.length >= Math.min(20, promptText.trim().length)) {
                dispatchEnter();
              }
            } catch (_) {}
          }, 450);
          return;
        }
        if (attempt < 20) {
          setTimeout(() => trySubmit(attempt + 1), 80);
          return;
        }
        submitClicked = true;
        dispatchEnter();
      } catch (_) {}
    };

    Promise.resolve().then(() => {
      setTimeout(() => trySubmit(0), 60);
    });

    return JSON.stringify({ ok: true });
  } catch (err) {
    return JSON.stringify({ ok: false, status: 'error', error: String(err) });
  }
})"#
    .to_string()
    + &format!("({adapter_json}, {job_json}, {prompt_json})")
}

#[cfg(desktop)]
pub(crate) fn build_group_poll_js(adapter: &SiteAdapterConfig, job_id: &str) -> String {
  let adapter_json = serde_json::to_string(adapter).unwrap_or_else(|_| "{}".to_string());
  let job_json = serde_json::to_string(job_id).unwrap_or_else(|_| "\"\"".to_string());
  r#"((adapter, jobId) => {
  try {
    const job = window.__maamGroupJobs && window.__maamGroupJobs[jobId];
    if (!job) {
      return JSON.stringify({ state: 'error', text: '', error: '页面已刷新或任务上下文已丢失' });
    }

    const isVisibleEl = (el, minW = 16, minH = 12) => {
      if (!el || !el.isConnected) return false;
      const r = el.getBoundingClientRect();
      if (r.width < minW || r.height < minH) return false;
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

    const findAuthPrompt = () => {
      if (typeof window.__maamFindAuthPrompt === 'function') {
        const hit = window.__maamFindAuthPrompt();
        if (hit) return hit;
      }
      const containers = document.querySelectorAll('[role="dialog"], [role="alertdialog"], [aria-modal="true"], dialog, [class*="permission" i], [class*="approval" i], [class*="authorize" i], [class*="consent" i]');
      for (const box of containers) {
        if (box.closest && box.closest('nav, aside')) continue;
        if (!isVisibleEl(box, 120, 60)) continue;
        const txt = (box.innerText || '').trim();
        if (!/授权|允许|始终允许|批准|请求访问|权限|approve|authorize|permission/i.test(txt)) continue;
        const btns = box.querySelectorAll('button, [role="button"]');
        for (const b of btns) {
          if (!isVisibleEl(b, 20, 16) || b.disabled) continue;
          const label = ((b.innerText || b.getAttribute('aria-label') || '') + '').trim();
          if (/^(允许|始终允许|本次允许|批准|确认授权|同意|allow|allow once|always allow|approve|authorize)$/i.test(label)) {
            return txt.slice(0, 140);
          }
        }
      }
      return '';
    };

    if (window.__maamAutoApprove === true && typeof window.__maamScanAutoApprove === 'function') {
      if (window.__maamScanAutoApprove()) {
        return JSON.stringify({ state: 'streaming', text: job.lastText || '' });
      }
    }

    const authReason = findAuthPrompt();
    if (authReason) {
      return JSON.stringify({ state: 'needs_auth', text: job.lastText || '', error: '该账号触发授权确认，请前往原生页面点击允许后重试' });
    }

    const isInteractiveBtn = (b) => {
      if (!isVisibleEl(b, 12, 12) || b.disabled || b.getAttribute('aria-disabled') === 'true' || b.getAttribute('aria-hidden') === 'true') return false;
      try {
        if (getComputedStyle(b).pointerEvents === 'none') return false;
      } catch (_) {}
      return true;
    };
    const sendBtns = Array.from(document.querySelectorAll('button, [role="button"]')).filter((b) => {
      if (!isInteractiveBtn(b)) return false;
      const t = (b.innerText || b.getAttribute('aria-label') || b.getAttribute('title') || b.getAttribute('data-testid') || '').trim();
      return /^(发送|发送消息|Send|Send message)$/i.test(t);
    });
    const overlapsSendBtn = (el) => {
      if (!el || !sendBtns.length) return false;
      const r1 = el.getBoundingClientRect();
      return sendBtns.some((sb) => {
        if (sb === el) return false;
        const r2 = sb.getBoundingClientRect();
        return Math.abs(r1.top - r2.top) < 12 && Math.abs(r1.left - r2.left) < 12;
      });
    };

    const adapterStops = Array.isArray(adapter.stopButtonSelectors) ? adapter.stopButtonSelectors.filter(Boolean) : [];
    const baseStopSelector = 'button[aria-label*="Stop" i], button[aria-label*="停止"], button[data-testid*="stop" i]';
    const stopSelector = adapterStops.length ? adapterStops.join(', ') + ', ' + baseStopSelector : baseStopSelector;
    let domGenerating = false;
    let stopEls = [];
    try { stopEls = document.querySelectorAll(stopSelector); } catch (_) { stopEls = document.querySelectorAll(baseStopSelector); }
    for (const sEl of stopEls) {
      if (sEl.closest && sEl.closest('nav, aside, header')) continue;
      if (!isInteractiveBtn(sEl) || overlapsSendBtn(sEl)) continue;
      domGenerating = true;
      break;
    }
    if (!domGenerating) {
      const allBtns = document.querySelectorAll('button, [role="button"]');
      for (const b of allBtns) {
        if (!isInteractiveBtn(b) || overlapsSendBtn(b)) continue;
        const txt = (b.innerText || b.getAttribute('aria-label') || b.getAttribute('title') || '').trim();
        if (/^(停止|停止生成|Stop|Stop generating)$/i.test(txt)) {
          domGenerating = true;
          break;
        }
      }
    }

    const resolveActiveComposer = () => {
      if (job.composerEl && job.composerEl.isConnected) return job.composerEl;
      const compSels = Array.isArray(adapter.composerSelectors) ? adapter.composerSelectors : [];
      for (const sel of compSels) {
        if (!sel) continue;
        try {
          const list = Array.from(document.querySelectorAll(sel)).filter((el) => isVisibleEl(el, 80, 16));
          if (list.length > 0) {
            list.sort((a, b) => b.getBoundingClientRect().bottom - a.getBoundingClientRect().bottom);
            return list[0];
          }
        } catch (_) {}
      }
      const fallback = Array.from(document.querySelectorAll('textarea, [contenteditable="true"], [role="textbox"]'))
        .filter((el) => isVisibleEl(el, 100, 16));
      fallback.sort((a, b) => b.getBoundingClientRect().bottom - a.getBoundingClientRect().bottom);
      return fallback[0] || null;
    };
    const resolveChatColumn = (comp) => {
      const winH = window.innerHeight || document.documentElement.clientHeight || 800;
      if (comp && comp.isConnected) {
        let cur = comp.parentElement;
        while (cur && cur !== document.body && cur !== document.documentElement) {
          const r = cur.getBoundingClientRect();
          if (r.height >= winH * 0.5 && r.width >= 220) {
            return cur;
          }
          cur = cur.parentElement;
        }
      }
      return document.querySelector('main, [role="main"]') || document.body;
    };
    const resolveComposerBar = (comp, col) => {
      const winH = window.innerHeight || document.documentElement.clientHeight || 800;
      if (!comp || !comp.isConnected) return null;
      let bar = comp;
      let cur = comp.parentElement;
      while (cur && cur !== col && cur !== document.body) {
        const r = cur.getBoundingClientRect();
        if (r.height >= winH * 0.36) break;
        bar = cur;
        cur = cur.parentElement;
      }
      return bar;
    };

    const activeComposer = resolveActiveComposer();
    if (activeComposer) job.composerEl = activeComposer;
    const composerVal = activeComposer
      ? (activeComposer.value || activeComposer.innerText || '').trim()
      : '';
    const chatCol = resolveChatColumn(activeComposer);
    const composerBar = resolveComposerBar(activeComposer, chatCol);
    const colRect = (chatCol && chatCol !== document.body)
      ? chatCol.getBoundingClientRect()
      : (job.chatColRect || null);

    if (!domGenerating && composerBar && composerVal === '') {
      const barBtns = composerBar.querySelectorAll('button, [role="button"]');
      for (const b of barBtns) {
        if (!isInteractiveBtn(b) || overlapsSendBtn(b)) continue;
        const rectSvg = b.querySelector('svg rect');
        if (rectSvg && !b.querySelector('svg path, svg circle, svg line, svg polyline, svg polygon')) {
          const rRect = rectSvg.getBoundingClientRect();
          const rw = parseFloat(rectSvg.getAttribute('width') || '0') || rRect.width;
          const rh = parseFloat(rectSvg.getAttribute('height') || '0') || rRect.height;
          if (rw >= 6 && rw <= 20 && rh >= 6 && Math.abs(rw - rh) <= 2) {
            domGenerating = true;
            break;
          }
        }
      }
    }

    const busyRe = /Researching\s+\w+|Searching\s+for\b|正在搜索|正在联网搜索|正在检索|正在读取|正在解析|正在加载|正在生成|正在处理|努力思考中|努力工作中/gi;
    const curBodyBusyCount = ((document.body && document.body.innerText) ? (document.body.innerText.match(busyRe) || []).length : 0);
    if (curBodyBusyCount > (job.baselineBodyBusyCount || 0)) {
      domGenerating = true;
    }

    const extraWords = Array.isArray(adapter.transientStatusWords)
      ? adapter.transientStatusWords.map((w) => String(w || '').trim()).filter(Boolean)
      : [];
    const extraIdleRe = /^(?:空闲中|已连接|已阅读|消息|Idle|Connected|Hark|Muse)$/i;
    let statusBusy = false;
    let progressText = '';
    const isIdleOnlyStatus = (s) => /^(?:(?:Muse|Hark|空闲中|已连接|已阅读|消息|Idle|Connected)[….\s]*)+$/i.test((s || '').trim());
    const isTransientStatusText = (s) => {
      const t = (s || '').trim();
      if (!t) return false;
      if (extraWords.some((w) => t === w || t === ('Muse\n' + w) || t === ('Hark\n' + w))) {
        return true;
      }
      return /^(?:(?:Muse|Hark)[ \t\r\n]+(?:正在[\u4e00-\u9fa5]{1,10}|[\u4e00-\u9fa5]{1,8}中|已连接|已阅读|消息|Thinking|Warming\s*up|Sending|Working|Searching|Researching|Replying|Retrieving|Loading|Idle|Connected)[….\s]*|(?:Muse|Hark)[ \t\r\n]*|(?:正在预热|预热中|空闲中|已连接|连接中|正在连接|仍在发送中|发送中|努力工作中|工作中|努力思考中|正在思考|思考中|正在搜索|正在搜索网页|正在联网搜索|搜索中|正在检索|检索中|正在回复|回复中|正在分析|分析中|正在查询|查询中|正在处理|处理中|正在生成|生成中|正在准备|准备中|加载中|待命中|已阅读|消息|Thinking|Warming\s*up|Sending|Working|Searching|Researching|Replying|Retrieving|Loading|Idle|Connected)[….\s]*)+$/i.test(t);
    };
    const isTransientProgressOnly = (txt) => {
      const s = (txt || '').trim();
      if (!s) return false;
      if (isTransientStatusText(s)) return !isIdleOnlyStatus(s);
      const lines = s.split(/\r?\n/).map((l) => l.trim().replace(/[。….\s]+$/g, '')).filter(Boolean);
      if (lines.length === 0 || lines.length > 3 || s.length > 100) return false;
      return lines.every((l) =>
        /^(?:Muse|Hark|ChatGPT|Claude|Gemini|DeepSeek)$/i.test(l)
        || /^(?:正在|努力|准备|开始)[^\n。！？!?]{1,32}$/.test(l)
        || /^[\u4e00-\u9fa5A-Za-z0-9 _\-]{1,18}中$/.test(l)
        || /^(?:正在|努力|开始)?(?:搜索|联网搜索|检索|查询|思考|深度思考|分析|阅读|读取|读入|解析|打开|下载|提取|整合|合并|修订|完善|优化|确认|等待|接收|获取|拉取|同步|写入|保存|浏览|整理|生成|处理|准备|加载|连接|预热|调用|执行|工作|研究|检查|审阅|评审|核对|评估|起草|撰写|编写|规划|制定|综合|汇总|梳理|对比|查看|构思|草拟|校验|复核|总结)[^\n。！？!?]{0,30}$/.test(l)
        || /^(?:已(?:搜索|阅读|读取|检索|解析|接收|获取|加载|打开|提取|合并|整合|浏览|思考|检查|审阅|核对)[^\n。！？!?]{0,24})$/.test(l)
        || /^(?:Searching|Researching|Thinking|Analyzing|Reading|Browsing|Generating|Loading|Fetching|Parsing|Opening|Merging|Synthesizing|Processing|Preparing|Running|Calling|Executing|Working|Warming\s*up|Sending|Retrieving|Checking|Reviewing|Drafting|Writing|Planning|Summarizing|Verifying|Evaluating)\b[^\n.!?]{0,70}$/i.test(l)
      );
    };
    const escRe = (v) => v.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const cleanChrome = (txt) => {
      let s = (txt || '')
        .replace(/(\r?\n\s*(复制|Copy|Copy code|Regenerate|重新生成|重试|Retry|Share|分享|Edit|编辑|Like|Dislike)\s*)+$/gi, '')
        .trim();
      if (isTransientProgressOnly(s)) {
        statusBusy = true;
        if (!progressText) progressText = s;
        return '';
      }
      if (isTransientStatusText(s)) {
        if (!isIdleOnlyStatus(s) && !extraIdleRe.test(s)) statusBusy = true;
        return '';
      }
      const leadStatusRe = /^(?:(?:Muse|Hark)\s*\r?\n+|(?:正在预热|预热中|空闲中|仍在发送中|发送中|正在搜索网页|正在联网搜索|正在搜索|搜索中|正在思考|思考中|读取中|解析中|处理中|生成中|正在[^\r\n。！？!?]{1,28}[。….]?|已(?:搜索|阅读|读取|检索|解析)[^\r\n]{0,20}[。….]?|Thinking[.\s]*|Searching[.\s]*|Reading[.\s]*|Thought for \d+s?)\s*\r?\n+)+/i;
      s = s.replace(leadStatusRe, '').trim();
      const leadSpeakerRe = /^(?:(?:Gemini|ChatGPT|Claude|DeepSeek|Grok|Kimi)\s*(?:说|said|says)\s*[:：]?\s*\r?\n+|(?:Gemini|ChatGPT|Claude|DeepSeek|Grok|Kimi)\s*(?:说|said|says)\s*[:：]\s*)+/i;
      s = s.replace(leadSpeakerRe, '').trim();
      if (/^(?:Gemini|ChatGPT|Claude|DeepSeek|Grok|Kimi)\s*(?:说|said|says)\s*[:：]?$/i.test(s)) {
        return '';
      }
      const tailRe = /(?:(?:Muse|Hark)[ \t\r\n]+(?:正在[\u4e00-\u9fa5]{1,10}|[\u4e00-\u9fa5]{1,8}中|已连接|已阅读|消息|Thinking|Warming\s*up|Sending|Working|Searching|Researching|Replying|Retrieving|Loading|Idle|Connected)(?:[ \t\r\n]+消息)?)+$|(?:\r?\n|\s|^)(?:Muse|Hark|已连接|连接中|正在连接|正在预热|预热中|空闲中|仍在发送中|发送中|努力工作中|工作中|努力思考中|正在思考|思考中|正在搜索|正在搜索网页|搜索中|正在检索|检索中|正在回复|回复中|正在分析|分析中|正在查询|查询中|正在处理|处理中|正在生成|生成中|正在准备|准备中|加载中|待命中|消息|停止|发送|Sending|已阅读|给\s*Hark\s*发消息|附加文件|开始听写|添加附件|发送消息|留下心情|共享页面|更多选项|Go to Ideas|Customize your Feed|\d{1,2}:\d{2})$/i;
      for (let k = 0; k < 10; k++) {
        let next = s.replace(tailRe, '').trim();
        for (const ew of extraWords) {
          const ewRe = new RegExp('(?:\\r?\\n|\\s|^)(?:(?:Muse|Hark)\\s+)?' + escRe(ew) + '$', 'i');
          next = next.replace(ewRe, '').trim();
        }
        if (next === s) break;
        s = next;
      }
      if (isTransientProgressOnly(s)) {
        statusBusy = true;
        if (!progressText) progressText = s;
        return '';
      }
      if (isTransientStatusText(s)) {
        if (!isIdleOnlyStatus(s) && !extraIdleRe.test(s)) statusBusy = true;
        return '';
      }
      return s;
    };

    const normWs = (s) => (s || '').replace(/\s+/g, ' ').trim();
    const promptNorm = (job.promptText || '').trim();
    const promptNormWs = normWs(promptNorm);
    const isSameAsPrompt = (t) => {
      const s = (t || '').trim();
      if (!s) return true;
      if (s === promptNorm) return true;
      const nt = normWs(s);
      if (nt.length >= 12 && promptNormWs.includes(nt)) return true;
      if (job.promptHead && s.includes(job.promptHead) && s.length <= promptNorm.length + 120) return true;
      if (promptNorm.startsWith('[阶段要求]') && s.startsWith('[阶段要求]') && s.length <= promptNorm.length + 120) return true;
      if (job.promptNeedle && s === job.promptNeedle && s.length <= promptNorm.length + 12) return true;
      if ((s.includes('不要复述上方方括号标题') || s.includes('不要重复上方协议标签') || s.includes('不要重复以上协议标签')) && (s.includes('[当前问题]') || s.includes('[用户问题]') || s.includes('[当前群聊发言]'))) return true;
      return false;
    };

    let scopedList = null;
    const adapterListSels = Array.isArray(adapter.messageListSelectors) ? adapter.messageListSelectors : [];
    for (const sel of adapterListSels) {
      if (!sel || sel === 'main') continue;
      try {
        const el = chatCol.querySelector(sel);
        if (el && el.getBoundingClientRect().height > 10) { scopedList = el; break; }
      } catch (_) {}
    }
    if (!scopedList) {
      scopedList = chatCol.querySelector('[data-testid="message-list"], [data-testid*="chat-messages" i], [data-testid*="message-feed" i]');
    }
    const searchRoot = scopedList || chatCol;
    const isOutOfColBounds = (el) => {
      if (!colRect || colRect.width < 120 || !el || !el.getBoundingClientRect) return false;
      const r = el.getBoundingClientRect();
      if (r.width === 0 && r.height === 0) return false;
      return r.right < colRect.left - 16 || r.left > colRect.right + 16;
    };
    const isExcluded = (el) => Boolean(
      (composerBar && composerBar.contains(el))
      || (el.closest && el.closest('nav, aside, header, footer, form'))
      || isOutOfColBounds(el)
    );
    const isMessageWrapper = (el) => {
      const tid = (el.getAttribute('data-testid') || '').toLowerCase();
      if (/list|container|scroll|wrapper|feed|history|timestamp|time|date|header|footer|avatar/.test(tid)) return true;
      const nested = el.querySelectorAll('[data-message-author-role], [data-role], [data-testid="message-bubble"], [data-testid="message-text"], article').length;
      return nested > 1;
    };
    const extractCleanText = (rootNode, skipChrome) => {
      if (!rootNode) return '';
      const skipSel = skipChrome
        ? 'nav, aside, header, footer, form, button, script, style, time, svg, noscript, template, [hidden], [aria-hidden="true"]'
        : 'script, style, time, svg, noscript, template, [hidden], [aria-hidden="true"]';
      const blockSel = 'p, div, li, tr, h1, h2, h3, h4, h5, h6, pre, blockquote, article, section';
      const parts = [];
      let lastBlk = null;
      let lastRow = null;
      let lastCell = null;
      try {
        const walker = document.createTreeWalker(rootNode, 4);
        let n;
        while ((n = walker.nextNode())) {
          const v = n.nodeValue;
          if (!v || !v.trim()) continue;
          const p = n.parentElement;
          if (!p || (skipChrome && composerBar && composerBar.contains(p)) || (p.closest && p.closest(skipSel))) continue;
          if (skipChrome && isOutOfColBounds(p)) continue;
          const row = p.closest ? p.closest('tr') : null;
          const cell = p.closest ? p.closest('td, th') : null;
          const blk = (p.closest && p.closest(blockSel)) || p;
          if (parts.length > 0) {
            if (row && row === lastRow && cell && cell !== lastCell) parts.push(' | ');
            else if (blk !== lastBlk) parts.push('\n');
          }
          parts.push(v.trim());
          lastBlk = blk;
          lastRow = row;
          lastCell = cell;
        }
      } catch (_) {}
      return parts.join('').replace(/\n{3,}/g, '\n\n').trim();
    };
    const getNodeText = (el) => {
      if (!el) return '';
      if (el.querySelector && el.querySelector('table, tr')) {
        return extractCleanText(el, false);
      }
      const t = (el.innerText || '').trim();
      if (t) return t;
      return extractCleanText(el, false);
    };
    const stripTailLite = (raw) => {
      let s = (raw || '').trim();
      for (let k = 0; k < 6; k++) {
        const next = s.replace(/(?:\r?\n|\s|^)(?:\d{1,2}:\d{2}|Muse|Hark|已阅读|已发送|已连接|空闲中|复制|Copy)$/i, '').trim();
        if (next === s) break;
        s = next;
      }
      return s;
    };
    const isOldBaselineText = (t) => {
      const raw = (t || '').trim();
      if (!raw) return true;
      const stripped = stripTailLite(raw);
      const nt = normWs(stripped);
      if (!nt) return true;
      if (job.baselineExact && (job.baselineExact.has(raw) || job.baselineExact.has(stripped))) return true;
      if (job.baselineCleanSet && (job.baselineCleanSet.has(raw) || job.baselineCleanSet.has(stripped))) return true;
      if (job.baselineLines && (job.baselineLines.has(raw) || job.baselineLines.has(stripped))) return true;
      if (nt.length >= 4 && job.baselineMainNorm && job.baselineMainNorm.includes(nt)) return true;
      if (nt.length >= 60 && job.baselineMainNorm) {
        const head48 = nt.slice(0, 48);
        const tail48 = nt.slice(-48);
        if (job.baselineMainNorm.includes(head48) && job.baselineMainNorm.includes(tail48)) return true;
        const tLines = stripped.split(/\r?\n/).map((l) => normWs(l)).filter((l) => l.length >= 12);
        if (tLines.length >= 3) {
          let hits = 0;
          for (const ln of tLines) {
            if (job.baselineMainNorm.includes(ln)) hits++;
          }
          if (hits / tLines.length >= 0.7) return true;
        }
      }
      if (nt.length >= 24 && Array.isArray(job.baselineBlockNorms)) {
        for (const b of job.baselineBlockNorms) {
          if (b.includes(nt) || (nt.includes(b) && b.length >= nt.length * 0.75)) return true;
        }
      }
      return false;
    };
    const isBaselineNode = (el) => Boolean(el && job.baselineDomEls && job.baselineDomEls.has(el));
    const isAfterLastBaseline = (el) => Boolean(
      el
      && job.lastBaselineEl
      && job.lastBaselineEl.isConnected
      && el !== job.lastBaselineEl
      && !el.contains(job.lastBaselineEl)
      && !job.lastBaselineEl.contains(el)
      && (job.lastBaselineEl.compareDocumentPosition(el) & 4)
    );

    const adapterAsst = Array.isArray(adapter.assistantMessageSelectors) ? adapter.assistantMessageSelectors.filter(Boolean) : [];
    const baseMsgSelector = '[data-message-author-role], [data-role], [data-testid="message-bubble"], [data-testid="message-text"], [data-testid*="assistant" i], [data-testid*="response" i], [data-testid*="turn" i], article, [class*="markdown" i], [class*="prose" i]';
    const allNodesSelector = adapterAsst.length ? adapterAsst.join(', ') + ', ' + baseMsgSelector : baseMsgSelector;
    let rawNodeList = [];
    try { rawNodeList = Array.from(searchRoot.querySelectorAll(allNodesSelector)); } catch (_) { rawNodeList = Array.from(searchRoot.querySelectorAll(baseMsgSelector)); }
    const allNodes = rawNodeList.filter((el) => !isExcluded(el) && !isMessageWrapper(el) && isVisibleEl(el, 20, 12));

    const fullMain = extractCleanText(searchRoot, true);
    const endMarker = '（请直接回答「当前问题」，不要复述上方方括号标题）';
    const markerIdx = fullMain.lastIndexOf(endMarker);
    const needleIdx = job.promptNeedle ? fullMain.lastIndexOf(job.promptNeedle) : -1;
    const curMarkerCount = fullMain ? (fullMain.split(endMarker).length - 1) : 0;
    const curNeedleCount = (fullMain && job.promptNeedle) ? (fullMain.split(job.promptNeedle).length - 1) : 0;
    const curHeadCount = (fullMain && job.promptHead) ? (fullMain.split(job.promptHead).length - 1) : 0;
    const hasNewMarker = curMarkerCount > (job.baselineMarkerCount || 0);
    const hasNewNeedle = curNeedleCount > (job.baselineNeedleCount || 0);
    const hasNewHead = curHeadCount > (job.baselineHeadCount || 0);
    const hasNewUserTurn = hasNewMarker || hasNewNeedle || hasNewHead;

    let userIdx = -1;
    const needleFull = job.promptNeedle || '';
    const needlePrefix = needleFull.length > 24 ? needleFull.slice(0, 24) : needleFull;
    const headFull = job.promptHead || '';
    const headPrefix = headFull.length > 24 ? headFull.slice(0, 24) : headFull;
    const matchesPromptSig = (txt) => Boolean(
      txt && (
        (needleFull && txt.includes(needleFull))
        || (needlePrefix.length >= 6 && txt.includes(needlePrefix))
        || (headFull && txt.includes(headFull))
        || (headPrefix.length >= 8 && txt.includes(headPrefix))
      )
    );
    if (needlePrefix || headPrefix || hasNewMarker) {
      for (let i = allNodes.length - 1; i >= 0; i--) {
        const el = allNodes[i];
        const rawT = getNodeText(el);
        if (!rawT) continue;
        if (isBaselineNode(el) && (!hasNewUserTurn || isOldBaselineText(rawT))) continue;
        if (matchesPromptSig(rawT) || (hasNewMarker && !isBaselineNode(el) && rawT.includes(endMarker))) {
          userIdx = i;
          break;
        }
      }
    }
    let userAnchorEl = userIdx >= 0 ? allNodes[userIdx] : null;
    if (!userAnchorEl && (needlePrefix || headPrefix || hasNewMarker)) {
      try {
        const walker = document.createTreeWalker(searchRoot, 4);
        let tNode;
        while ((tNode = walker.nextNode())) {
          const p = tNode.parentElement;
          if (!p || isExcluded(p)) continue;
          const v = (tNode.nodeValue || '').trim();
          if (!v) continue;
          if (isBaselineNode(p) && (!hasNewUserTurn || isOldBaselineText(v))) continue;
          if (matchesPromptSig(v) || (hasNewMarker && !isBaselineNode(p) && v.includes(endMarker))) {
            userAnchorEl = p;
          }
        }
      } catch (_) {}
    }

    let userTurnBox = userAnchorEl;
    if (userTurnBox) {
      const trivialSiblingRe = /^(?:\d{1,2}:\d{2}|👍|❤️|😂|🎉|复制|编辑|重试|Copy|Edit|Retry|已发送|已读)$/i;
      let cur = userTurnBox.parentElement;
      while (cur && cur !== searchRoot && cur !== chatCol && cur !== document.body) {
        if (composerBar && cur.contains(composerBar)) break;
        if (job.lastBaselineEl && job.lastBaselineEl.isConnected && cur.contains(job.lastBaselineEl)) break;
        const ct = (cur.innerText || '').trim();
        if (ct.length > promptNorm.length + 120) break;
        let hasAssistantSibling = false;
        let seenBranch = false;
        for (const ch of cur.children) {
          if (ch === userTurnBox || ch.contains(userTurnBox)) {
            seenBranch = true;
            continue;
          }
          if (seenBranch) {
            const st = (ch.innerText || '').trim();
            if (st && !trivialSiblingRe.test(st)) {
              hasAssistantSibling = true;
              break;
            }
          }
        }
        if (hasAssistantSibling) break;
        userTurnBox = cur;
        cur = cur.parentElement;
      }
    }

    const extractAfterUserTurn = (rootNode, turnBox) => {
      if (!rootNode || !turnBox) return { text: '', hasFreshDom: false };
      const skipSel = 'nav, aside, header, footer, form, button, script, style, time, svg, noscript, template, [hidden], [aria-hidden="true"]';
      const blockSel = 'p, div, li, tr, h1, h2, h3, h4, h5, h6, pre, blockquote, article, section';
      const parts = [];
      let hasFreshDom = false;
      let lastBlk = null;
      let lastRow = null;
      let lastCell = null;
      try {
        const walker = document.createTreeWalker(rootNode, 4);
        let n;
        while ((n = walker.nextNode())) {
          const v = n.nodeValue;
          if (!v || !v.trim()) continue;
          const p = n.parentElement;
          if (!p || (composerBar && composerBar.contains(p)) || (p.closest && p.closest(skipSel))) continue;
          if (turnBox === p || turnBox.contains(p) || (turnBox.compareDocumentPosition(p) & 4) === 0) continue;
          if (isBaselineNode(p) && job.lastBaselineEl && job.lastBaselineEl.isConnected && (job.lastBaselineEl.compareDocumentPosition(p) & 4) === 0) continue;
          if (isOutOfColBounds(p) || !isVisibleEl(p, 1, 1)) continue;
          if (!isBaselineNode(p)) hasFreshDom = true;
          const row = p.closest ? p.closest('tr') : null;
          const cell = p.closest ? p.closest('td, th') : null;
          const blk = (p.closest && p.closest(blockSel)) || p;
          if (parts.length > 0) {
            if (row && row === lastRow && cell && cell !== lastCell) parts.push(' | ');
            else if (blk !== lastBlk) parts.push('\n');
          }
          parts.push(v.trim());
          lastBlk = blk;
          lastRow = row;
          lastCell = cell;
        }
      } catch (_) {}
      return { text: parts.join('').replace(/\n{3,}/g, '\n\n').trim(), hasFreshDom };
    };

    const isBeforeUserAnchor = (el) => {
      const ref = userTurnBox || userAnchorEl;
      if (!ref || !el) return false;
      if (el === ref || el.contains(ref) || ref.contains(el)) return true;
      return (ref.compareDocumentPosition(el) & 4) === 0;
    };

    const sentIntoDom = Boolean(userTurnBox) || userIdx >= 0 || hasNewUserTurn || allNodes.length > (job.baselineCount || 0);
    const stillUnsent = !sentIntoDom && composerVal.length >= Math.min(20, promptNorm.length) && composerVal === promptNorm;

    const now = Date.now();
    const isAsyncAgent = Boolean(adapter.asyncAgent);
    const netState = window.__maamNet;
    const apiPats = Array.isArray(adapter.chatApiPatterns)
      ? adapter.chatApiPatterns.map((p) => String(p || '').toLowerCase()).filter(Boolean)
      : [];
    const netEntries = (netState && Array.isArray(netState.entries))
      ? netState.entries.filter((e) => e && e.id > (job.netStartSeq || 0))
      : [];
    const relevantNet = netEntries.filter((e) => {
      const u = (e.url || '').toLowerCase();
      const patMatch = apiPats.length ? apiPats.some((p) => u.includes(p)) : false;
      return e.method === 'POST' || e.isStream || patMatch;
    });
    const poolNet = relevantNet.filter((e) => e.method === 'POST' || e.isStream);
    const activePostStream = poolNet.some((e) => (e.method === 'POST' || (e.isStream && !isAsyncAgent)) && !e.done && (now - (e.updatedAt || e.startedAt || 0)) < 12000);
    const activePushStream = !isAsyncAgent && Boolean(netState && ((netState.wsLastAt > (job.netStartAt || 0) && now - netState.wsLastAt < 4000) || (netState.esLastAt > (job.netStartAt || 0) && now - netState.esLastAt < 4000)));
    const netGenerating = activePostStream || activePushStream;

    let candidateText = '';
    if (!stillUnsent) {
      const confirmedNewTurn = Boolean(userTurnBox) && (!isBaselineNode(userTurnBox) || hasNewUserTurn);
      if (confirmedNewTurn) {
        const afterRes = extractAfterUserTurn(searchRoot, userTurnBox);
        const afterDom = cleanChrome(afterRes.text);
        if (afterDom && !isSameAsPrompt(afterDom)) {
          if (!isOldBaselineText(afterDom) || (afterRes.hasFreshDom && afterDom.length <= 80)) {
            candidateText = afterDom;
          }
        }
      }

      if (!candidateText && (confirmedNewTurn || hasNewUserTurn || allNodes.length > (job.baselineCount || 0))) {
        const baseAsstSelector = '[data-message-author-role="assistant"], [data-role="assistant"], [data-testid*="assistant" i]';
        const assistantSelector = adapterAsst.length ? adapterAsst.join(', ') + ', ' + baseAsstSelector : baseAsstSelector;
        let rawAsstNodes = [];
        try { rawAsstNodes = Array.from(searchRoot.querySelectorAll(assistantSelector)); } catch (_) { rawAsstNodes = Array.from(searchRoot.querySelectorAll(baseAsstSelector)); }
        const assistantNodes = rawAsstNodes.filter((el) => !isExcluded(el) && !isMessageWrapper(el) && isVisibleEl(el, 20, 12));
        const collectedAsst = [];
        for (let i = 0; i < assistantNodes.length; i++) {
          const node = assistantNodes[i];
          if (isBeforeUserAnchor(node)) continue;
          if (job.baselineEls && job.baselineEls.has(node)) continue;
          if (isBaselineNode(node) && isOldBaselineText(cleanChrome(getNodeText(node)))) continue;
          if (!userTurnBox && job.lastBaselineEl && job.lastBaselineEl.isConnected && !isAfterLastBaseline(node)) continue;
          if (assistantNodes.some((other) => other !== node && other.contains(node))) continue;
          const t = cleanChrome(getNodeText(node));
          if (!t || isSameAsPrompt(t)) continue;
          const allowShortRepeat = confirmedNewTurn && !isBaselineNode(node) && t.length <= 80;
          if (!allowShortRepeat && isOldBaselineText(t)) continue;
          collectedAsst.push(t);
        }
        if (collectedAsst.length > 0) {
          candidateText = userTurnBox ? collectedAsst.join('\n\n') : collectedAsst[collectedAsst.length - 1];
        }
      }

      if (!candidateText && !userTurnBox && (hasNewUserTurn || allNodes.length > (job.baselineCount || 0))) {
        for (let i = allNodes.length - 1; i >= 0; i--) {
          if (i < (job.baselineCount || 0)) continue;
          const el = allNodes[i];
          if (job.baselineEls && job.baselineEls.has(el)) continue;
          if (isBaselineNode(el)) continue;
          if (job.lastBaselineEl && job.lastBaselineEl.isConnected && !isAfterLastBaseline(el)) continue;
          if (el.getAttribute('data-message-author-role') === 'user' || el.getAttribute('data-role') === 'user') continue;
          const rawElText = getNodeText(el);
          if (!rawElText || matchesPromptSig(rawElText)) continue;
          if (job.baselineSet && job.baselineSet.has(rawElText.slice(0, 220))) continue;
          let t = rawElText;
          if (t.startsWith(promptNorm)) t = t.slice(promptNorm.length).trim();
          t = cleanChrome(t);
          if (!t || isSameAsPrompt(t) || isOldBaselineText(t)) continue;
          candidateText = t;
          break;
        }
      }

      if (!candidateText) {
        let sliceStart = -1;
        if (hasNewMarker && markerIdx >= 0) {
          sliceStart = markerIdx + endMarker.length;
        } else if (hasNewNeedle && needleIdx >= 0) {
          sliceStart = needleIdx + job.promptNeedle.length;
        }
        if (sliceStart >= 0) {
          const after = cleanChrome(fullMain.slice(sliceStart));
          if (after && !isSameAsPrompt(after) && !isOldBaselineText(after)) candidateText = after;
        }
      }

      if (!candidateText) {
        for (let i = poolNet.length - 1; i >= 0; i--) {
          const nt = cleanChrome((poolNet[i].text || '').trim());
          if (nt && !isSameAsPrompt(nt) && !isOldBaselineText(nt)) {
            candidateText = nt;
            break;
          }
        }
        if (!candidateText && netState && netState.esLastAt > (job.netStartAt || 0) && netState.esText) {
          const est = cleanChrome(netState.esText);
          if (est && !isSameAsPrompt(est) && !isOldBaselineText(est)) candidateText = est;
        }
      }
    }

    const cleanupJob = () => {
      if (job && job.observer) {
        try { job.observer.disconnect(); } catch (_) {}
      }
      delete window.__maamGroupJobs[jobId];
    };

    if (statusBusy) {
      domGenerating = true;
    }
    const isGenerating = domGenerating || netGenerating;

    if (candidateText && candidateText !== job.lastText) {
      job.lastText = candidateText;
      job.lastChangedAt = now;
    }

    const stableMs = now - (job.lastChangedAt || job.startedAt);
    const elapsedMs = now - job.startedAt;
    const recentDomMut = Boolean(job.lastDomMutAt && (now - job.lastDomMutAt) < (isAsyncAgent ? 2500 : 1200));
    const effectiveGenerating = isGenerating || recentDomMut;

    if (job.lastText) {
      const trimmedLines = job.lastText.trim().split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
      const lastLine = trimmedLines.length ? trimmedLines[trimmedLines.length - 1] : '';
      const looksIncomplete =
        (job.lastText.length < 24 && !/[。！？.!?]$/.test(job.lastText))
        || /(?:[:：\-—*•]|^(?:#{1,6}\s+|\d+[.、]\s*)[^\n。！？.!?]{1,28})$/.test(lastLine);
      const neededStableMs = isAsyncAgent
        ? (looksIncomplete ? 9000 : (job.lastText.length < 40 ? 6000 : 4500))
        : (looksIncomplete ? 3600 : (job.lastText.length < 24 ? 1800 : 1400));
      const quietFallbackMs = Math.max(neededStableMs + 1500, isAsyncAgent ? 7500 : 4200);
      const noDomFallbackMs = Math.max(neededStableMs + 2000, isAsyncAgent ? 9000 : 4800);
      const hardStableCapMs = isAsyncAgent ? 30000 : 22000;
      if (
        (!effectiveGenerating && stableMs >= neededStableMs)
        || (!isGenerating && stableMs >= quietFallbackMs)
        || (!domGenerating && !recentDomMut && stableMs >= noDomFallbackMs)
        || stableMs >= hardStableCapMs
      ) {
        cleanupJob();
        return JSON.stringify({ state: 'done', text: job.lastText });
      }
      return JSON.stringify({ state: 'streaming', text: job.lastText, transient: false });
    }

    const okPost = [...relevantNet].reverse().find((e) => e.method === 'POST' && e.status >= 200 && e.status < 300);
    const fatalChatNet = [...relevantNet].reverse().find((e) => e.status === 401 || e.status === 403 || e.status === 429 || (e.method === 'POST' && e.status >= 400));
    if (!isGenerating && fatalChatNet && (now - (fatalChatNet.updatedAt || fatalChatNet.startedAt)) >= 2500) {
      cleanupJob();
      const msg = (fatalChatNet.status === 401 || fatalChatNet.status === 403)
        ? ('目标站点鉴权失效（HTTP ' + fatalChatNet.status + '），请前往该账号页面重新登录')
        : fatalChatNet.status === 429
          ? '触发目标站点限流（HTTP 429），请稍后点击重试'
          : ('目标接口报错（HTTP ' + fatalChatNet.status + (fatalChatNet.errorSnippet ? '：' + fatalChatNet.errorSnippet : '') + '）');
      return JSON.stringify({ state: 'error', text: '', error: msg });
    }

    const stuckStillSending = Boolean(progressText && progressText.includes('仍在发送中') && elapsedMs >= 15000);
    if (stuckStillSending) {
      cleanupJob();
      return JSON.stringify({ state: 'stalled_retry', text: '' });
    }

    if (isAsyncAgent && !job.lastText && !progressText && sentIntoDom && elapsedMs >= 7000) {
      progressText = okPost
        ? ('服务端已接收（HTTP ' + okPost.status + '），等待异步回复…')
        : '消息已发出，等待服务端响应…';
    }

    const timedOut = stillUnsent
      ? (elapsedMs >= 20000)
      : isAsyncAgent
        ? (elapsedMs >= 95000)
        : (!isGenerating && elapsedMs >= 65000);
    if (timedOut) {
      cleanupJob();
      const waitedSec = Math.round(elapsedMs / 1000);
      const msg = stillUnsent
        ? '消息已填入但未能自动触发发送，请前往该账号页面检查输入框'
        : okPost
          ? ('服务端已接收消息（HTTP ' + okPost.status + '，已等待 ' + waitedSec + ' 秒），但未返回回复（可能服务端静默跳过或长连接中断），可前往查看或手动重试')
          : (!sentIntoDom && !relevantNet.some((e) => e.method === 'POST'))
            ? '未检测到发送请求发出，请检查该账号登录或网络状态后重试'
            : ('发送后未检测到新回复（已等待 ' + waitedSec + ' 秒），可前往查看或手动重试');
      return JSON.stringify({ state: 'error', text: '', error: msg });
    }

    return JSON.stringify({ state: 'streaming', text: progressText, transient: Boolean(progressText) });
  } catch (err) {
    return JSON.stringify({ state: 'error', text: '', error: String(err) });
  }
})"#
    .to_string()
    + &format!("({adapter_json}, {job_json})")
}

#[cfg(desktop)]
pub(crate) fn build_group_sync_latest_js(
  adapter: &SiteAdapterConfig,
  raw_user_content: &str,
) -> String {
  let adapter_json = serde_json::to_string(adapter).unwrap_or_else(|_| "{}".to_string());
  let query_json = serde_json::to_string(raw_user_content).unwrap_or_else(|_| "\"\"".to_string());
  r#"((adapter, rawUserContent) => {
  try {
    const endMarker = '（请直接回答「当前问题」，不要复述上方方括号标题）';

    const isVisibleEl = (el, minW = 16, minH = 12) => {
      if (!el || !el.isConnected) return false;
      const r = el.getBoundingClientRect();
      if (r.width < minW || r.height < minH) return false;
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

    const resolveActiveComposer = () => {
      const compSels = Array.isArray(adapter.composerSelectors) ? adapter.composerSelectors : [];
      for (const sel of compSels) {
        if (!sel) continue;
        try {
          const list = Array.from(document.querySelectorAll(sel)).filter((el) => isVisibleEl(el, 80, 16));
          if (list.length > 0) {
            list.sort((a, b) => b.getBoundingClientRect().bottom - a.getBoundingClientRect().bottom);
            return list[0];
          }
        } catch (_) {}
      }
      const fallback = Array.from(document.querySelectorAll('textarea, [contenteditable="true"], [role="textbox"]'))
        .filter((el) => isVisibleEl(el, 100, 16));
      fallback.sort((a, b) => b.getBoundingClientRect().bottom - a.getBoundingClientRect().bottom);
      return fallback[0] || null;
    };
    const resolveChatColumn = (comp) => {
      const winH = window.innerHeight || document.documentElement.clientHeight || 800;
      if (comp && comp.isConnected) {
        let cur = comp.parentElement;
        while (cur && cur !== document.body && cur !== document.documentElement) {
          const r = cur.getBoundingClientRect();
          if (r.height >= winH * 0.5 && r.width >= 220) {
            return cur;
          }
          cur = cur.parentElement;
        }
      }
      return document.querySelector('main, [role="main"]') || document.body;
    };
    const resolveComposerBar = (comp, col) => {
      const winH = window.innerHeight || document.documentElement.clientHeight || 800;
      if (!comp || !comp.isConnected) return null;
      let bar = comp;
      let cur = comp.parentElement;
      while (cur && cur !== col && cur !== document.body) {
        const r = cur.getBoundingClientRect();
        if (r.height >= winH * 0.36) break;
        bar = cur;
        cur = cur.parentElement;
      }
      return bar;
    };

    const activeComposer = resolveActiveComposer();
    const chatCol = resolveChatColumn(activeComposer);
    const composerBar = resolveComposerBar(activeComposer, chatCol);
    const colRect = (chatCol && chatCol !== document.body) ? chatCol.getBoundingClientRect() : null;

    let scopedList = null;
    const adapterListSels = Array.isArray(adapter.messageListSelectors) ? adapter.messageListSelectors : [];
    for (const sel of adapterListSels) {
      if (!sel || sel === 'main') continue;
      try {
        const el = chatCol.querySelector(sel);
        if (el && el.getBoundingClientRect().height > 10) { scopedList = el; break; }
      } catch (_) {}
    }
    if (!scopedList) {
      scopedList = chatCol.querySelector('[data-testid="message-list"], [data-testid*="chat-messages" i], [data-testid*="message-feed" i]');
    }
    const searchRoot = scopedList || chatCol;
    const isOutOfColBounds = (el) => {
      if (!colRect || colRect.width < 120 || !el || !el.getBoundingClientRect) return false;
      const r = el.getBoundingClientRect();
      if (r.width === 0 && r.height === 0) return false;
      return r.right < colRect.left - 16 || r.left > colRect.right + 16;
    };
    const isExcluded = (el) => Boolean(
      (composerBar && composerBar.contains(el))
      || (el.closest && el.closest('nav, aside, header, footer, form'))
      || isOutOfColBounds(el)
    );
    const isMessageWrapper = (el) => {
      const tid = (el.getAttribute('data-testid') || '').toLowerCase();
      if (/list|container|scroll|wrapper|feed|history|timestamp|time|date|header|footer|avatar/.test(tid)) return true;
      const nested = el.querySelectorAll('[data-message-author-role], [data-role], [data-testid="message-bubble"], [data-testid="message-text"], article').length;
      return nested > 1;
    };
    const extractCleanText = (rootNode, skipChrome) => {
      if (!rootNode) return '';
      const skipSel = skipChrome
        ? 'nav, aside, header, footer, form, button, script, style, time, svg, noscript, template, [hidden], [aria-hidden="true"]'
        : 'script, style, time, svg, noscript, template, [hidden], [aria-hidden="true"]';
      const blockSel = 'p, div, li, tr, h1, h2, h3, h4, h5, h6, pre, blockquote, article, section';
      const parts = [];
      let lastBlk = null;
      let lastRow = null;
      let lastCell = null;
      try {
        const walker = document.createTreeWalker(rootNode, 4);
        let n;
        while ((n = walker.nextNode())) {
          const v = n.nodeValue;
          if (!v || !v.trim()) continue;
          const p = n.parentElement;
          if (!p || (skipChrome && composerBar && composerBar.contains(p)) || (p.closest && p.closest(skipSel))) continue;
          if (skipChrome && isOutOfColBounds(p)) continue;
          const row = p.closest ? p.closest('tr') : null;
          const cell = p.closest ? p.closest('td, th') : null;
          const blk = (p.closest && p.closest(blockSel)) || p;
          if (parts.length > 0) {
            if (row && row === lastRow && cell && cell !== lastCell) parts.push(' | ');
            else if (blk !== lastBlk) parts.push('\n');
          }
          parts.push(v.trim());
          lastBlk = blk;
          lastRow = row;
          lastCell = cell;
        }
      } catch (_) {}
      return parts.join('').replace(/\n{3,}/g, '\n\n').trim();
    };
    const getNodeText = (el) => {
      if (!el) return '';
      if (el.querySelector && el.querySelector('table, tr')) {
        return extractCleanText(el, false);
      }
      const t = (el.innerText || '').trim();
      if (t) return t;
      return extractCleanText(el, false);
    };

    const extraWords = Array.isArray(adapter.transientStatusWords)
      ? adapter.transientStatusWords.map((w) => String(w || '').trim()).filter(Boolean)
      : [];
    const isIdleOnlyStatus = (s) => /^(?:(?:Muse|Hark|空闲中|已连接|已阅读|消息|Idle|Connected)[….\s]*)+$/i.test((s || '').trim());
    const isTransientStatusText = (s) => {
      const t = (s || '').trim();
      if (!t) return false;
      if (extraWords.some((w) => t === w || t === ('Muse\n' + w) || t === ('Hark\n' + w))) {
        return true;
      }
      return /^(?:(?:Muse|Hark)[ \t\r\n]+(?:正在[\u4e00-\u9fa5]{1,10}|[\u4e00-\u9fa5]{1,8}中|已连接|已阅读|消息|Thinking|Warming\s*up|Sending|Working|Searching|Researching|Replying|Retrieving|Loading|Idle|Connected)[….\s]*|(?:Muse|Hark)[ \t\r\n]*|(?:正在预热|预热中|空闲中|已连接|连接中|正在连接|仍在发送中|发送中|努力工作中|工作中|努力思考中|正在思考|思考中|正在搜索|正在搜索网页|正在联网搜索|搜索中|正在检索|检索中|正在回复|回复中|正在分析|分析中|正在查询|查询中|正在处理|处理中|正在生成|生成中|正在准备|准备中|加载中|待命中|已阅读|消息|Thinking|Warming\s*up|Sending|Working|Searching|Researching|Replying|Retrieving|Loading|Idle|Connected)[….\s]*)+$/i.test(t);
    };
    const isTransientProgressOnly = (txt) => {
      const s = (txt || '').trim();
      if (!s) return false;
      if (isTransientStatusText(s)) return !isIdleOnlyStatus(s);
      const lines = s.split(/\r?\n/).map((l) => l.trim().replace(/[。….\s]+$/g, '')).filter(Boolean);
      if (lines.length === 0 || lines.length > 3 || s.length > 100) return false;
      return lines.every((l) =>
        /^(?:Muse|Hark|ChatGPT|Claude|Gemini|DeepSeek)$/i.test(l)
        || /^(?:正在|努力|准备|开始)[^\n。！？!?]{1,32}$/.test(l)
        || /^[\u4e00-\u9fa5A-Za-z0-9 _\-]{1,18}中$/.test(l)
        || /^(?:正在|努力|开始)?(?:搜索|联网搜索|检索|查询|思考|深度思考|分析|阅读|读取|读入|解析|打开|下载|提取|整合|合并|修订|完善|优化|确认|等待|接收|获取|拉取|同步|写入|保存|浏览|整理|生成|处理|准备|加载|连接|预热|调用|执行|工作|研究|检查|审阅|评审|核对|评估|起草|撰写|编写|规划|制定|综合|汇总|梳理|对比|查看|构思|草拟|校验|复核|总结)[^\n。！？!?]{0,30}$/.test(l)
        || /^(?:已(?:搜索|阅读|读取|检索|解析|接收|获取|加载|打开|提取|合并|整合|浏览|思考|检查|审阅|核对)[^\n。！？!?]{0,24})$/.test(l)
        || /^(?:Searching|Researching|Thinking|Analyzing|Reading|Browsing|Generating|Loading|Fetching|Parsing|Opening|Merging|Synthesizing|Processing|Preparing|Running|Calling|Executing|Working|Warming\s*up|Sending|Retrieving|Checking|Reviewing|Drafting|Writing|Planning|Summarizing|Verifying|Evaluating)\b[^\n.!?]{0,70}$/i.test(l)
      );
    };
    const escRe = (v) => v.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const cleanChrome = (txt) => {
      let s = (txt || '')
        .replace(/(\r?\n\s*(复制|Copy|Copy code|Regenerate|重新生成|重试|Retry|Share|分享|Edit|编辑|Like|Dislike)\s*)+$/gi, '')
        .trim();
      if (isTransientProgressOnly(s) || isTransientStatusText(s)) return '';
      const leadStatusRe = /^(?:(?:Muse|Hark)\s*\r?\n+|(?:正在预热|预热中|空闲中|仍在发送中|发送中|正在搜索网页|正在联网搜索|正在搜索|搜索中|正在思考|思考中|读取中|解析中|处理中|生成中|正在[^\r\n。！？!?]{1,28}[。….]?|已(?:搜索|阅读|读取|检索|解析)[^\r\n]{0,20}[。….]?|Thinking[.\s]*|Searching[.\s]*|Reading[.\s]*|Thought for \d+s?)\s*\r?\n+)+/i;
      s = s.replace(leadStatusRe, '').trim();
      const leadSpeakerRe = /^(?:(?:Gemini|ChatGPT|Claude|DeepSeek|Grok|Kimi)\s*(?:说|said|says)\s*[:：]?\s*\r?\n+|(?:Gemini|ChatGPT|Claude|DeepSeek|Grok|Kimi)\s*(?:说|said|says)\s*[:：]\s*)+/i;
      s = s.replace(leadSpeakerRe, '').trim();
      if (/^(?:Gemini|ChatGPT|Claude|DeepSeek|Grok|Kimi)\s*(?:说|said|says)\s*[:：]?$/i.test(s)) return '';
      const tailRe = /(?:(?:Muse|Hark)[ \t\r\n]+(?:正在[\u4e00-\u9fa5]{1,10}|[\u4e00-\u9fa5]{1,8}中|已连接|已阅读|消息|Thinking|Warming\s*up|Sending|Working|Searching|Researching|Replying|Retrieving|Loading|Idle|Connected)(?:[ \t\r\n]+消息)?)+$|(?:\r?\n|\s|^)(?:Muse|Hark|已连接|连接中|正在连接|正在预热|预热中|空闲中|仍在发送中|发送中|努力工作中|工作中|努力思考中|正在思考|思考中|正在搜索|正在搜索网页|搜索中|正在检索|检索中|正在回复|回复中|正在分析|分析中|正在查询|查询中|正在处理|处理中|正在生成|生成中|正在准备|准备中|加载中|待命中|消息|停止|发送|Sending|已阅读|给\s*Hark\s*发消息|附加文件|开始听写|添加附件|发送消息|留下心情|共享页面|更多选项|Go to Ideas|Customize your Feed|\d{1,2}:\d{2})$/i;
      for (let k = 0; k < 10; k++) {
        let next = s.replace(tailRe, '').trim();
        for (const ew of extraWords) {
          const ewRe = new RegExp('(?:\\r?\\n|\\s|^)(?:(?:Muse|Hark)\\s+)?' + escRe(ew) + '$', 'i');
          next = next.replace(ewRe, '').trim();
        }
        if (next === s) break;
        s = next;
      }
      if (isTransientProgressOnly(s) || isTransientStatusText(s)) return '';
      return s;
    };

    const normWs = (s) => (s || '').replace(/\s+/g, ' ').trim();
    const rawQ = (rawUserContent || '').trim();
    const qLines = rawQ.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
    qLines.sort((a, b) => b.length - a.length);
    const qLongest = qLines[0] || rawQ;
    const qNeedle = qLongest.slice(0, 48);
    const qPrefix = qNeedle.length > 24 ? qNeedle.slice(0, 24) : qNeedle;
    const rawQNorm = normWs(rawQ);
    const isPromptOverlap = (t) => {
      const s = (t || '').trim();
      if (!s) return true;
      if (rawQ && s === rawQ) return true;
      const ns = normWs(s);
      if (rawQNorm && ns.length >= 12 && rawQNorm.includes(ns)) return true;
      if ((s.includes('不要复述上方方括号标题') || s.includes('不要重复上方协议标签') || s.includes('不要重复以上协议标签')) && (s.includes('[当前问题]') || s.includes('[用户问题]') || s.includes('[当前群聊发言]'))) return true;
      if (s.startsWith('[阶段要求]') && s.includes(endMarker)) return true;
      return false;
    };

    const adapterAsst = Array.isArray(adapter.assistantMessageSelectors) ? adapter.assistantMessageSelectors.filter(Boolean) : [];
    const baseMsgSelector = '[data-message-author-role], [data-role], [data-testid="message-bubble"], [data-testid="message-text"], [data-testid*="assistant" i], [data-testid*="response" i], [data-testid*="turn" i], article, [class*="markdown" i], [class*="prose" i]';
    const allNodesSelector = adapterAsst.length ? adapterAsst.join(', ') + ', ' + baseMsgSelector : baseMsgSelector;
    let rawNodeList = [];
    try { rawNodeList = Array.from(searchRoot.querySelectorAll(allNodesSelector)); } catch (_) { rawNodeList = Array.from(searchRoot.querySelectorAll(baseMsgSelector)); }
    const allNodes = rawNodeList.filter((el) => !isExcluded(el) && !isMessageWrapper(el) && isVisibleEl(el, 20, 12));

    let userAnchorEl = null;
    if (qPrefix.length >= 4) {
      for (let i = allNodes.length - 1; i >= 0; i--) {
        const rawT = getNodeText(allNodes[i]);
        if (rawT && ((qNeedle && rawT.includes(qNeedle)) || (qPrefix.length >= 6 && rawT.includes(qPrefix)))) {
          userAnchorEl = allNodes[i];
          break;
        }
      }
      if (!userAnchorEl) {
        try {
          const walker = document.createTreeWalker(searchRoot, 4);
          let tNode;
          while ((tNode = walker.nextNode())) {
            const p = tNode.parentElement;
            if (!p || isExcluded(p)) continue;
            const v = (tNode.nodeValue || '').trim();
            if (v && ((qNeedle && v.includes(qNeedle)) || (qPrefix.length >= 6 && v.includes(qPrefix)))) {
              userAnchorEl = p;
            }
          }
        } catch (_) {}
      }
    }
    if (!userAnchorEl) {
      for (let i = allNodes.length - 1; i >= 0; i--) {
        const rawT = getNodeText(allNodes[i]);
        if (rawT && (rawT.includes(endMarker) || rawT.includes('[当前问题]') || rawT.includes('[阶段要求]'))) {
          userAnchorEl = allNodes[i];
          break;
        }
      }
      if (!userAnchorEl) {
        try {
          const walker = document.createTreeWalker(searchRoot, 4);
          let tNode;
          while ((tNode = walker.nextNode())) {
            const p = tNode.parentElement;
            if (!p || isExcluded(p)) continue;
            const v = (tNode.nodeValue || '').trim();
            if (v && (v.includes(endMarker) || v.includes('[当前问题]') || v.includes('[阶段要求]'))) {
              userAnchorEl = p;
            }
          }
        } catch (_) {}
      }
    }

    let userTurnBox = userAnchorEl;
    if (userTurnBox) {
      const trivialSiblingRe = /^(?:\d{1,2}:\d{2}|👍|❤️|😂|🎉|复制|编辑|重试|Copy|Edit|Retry|已发送|已读)$/i;
      let cur = userTurnBox.parentElement;
      while (cur && cur !== searchRoot && cur !== chatCol && cur !== document.body) {
        if (composerBar && cur.contains(composerBar)) break;
        let hasAssistantSibling = false;
        let seenBranch = false;
        for (const ch of cur.children) {
          if (ch === userTurnBox || ch.contains(userTurnBox)) {
            seenBranch = true;
            continue;
          }
          if (seenBranch) {
            const st = (ch.innerText || '').trim();
            if (st && !trivialSiblingRe.test(st)) {
              hasAssistantSibling = true;
              break;
            }
          }
        }
        if (hasAssistantSibling) break;
        userTurnBox = cur;
        cur = cur.parentElement;
      }
    }

    const extractAfterUserTurn = (rootNode, turnBox) => {
      if (!rootNode || !turnBox) return '';
      const skipSel = 'nav, aside, header, footer, form, button, script, style, time, svg, noscript, template, [hidden], [aria-hidden="true"]';
      const blockSel = 'p, div, li, tr, h1, h2, h3, h4, h5, h6, pre, blockquote, article, section';
      const parts = [];
      let lastBlk = null;
      let lastRow = null;
      let lastCell = null;
      try {
        const walker = document.createTreeWalker(rootNode, 4);
        let n;
        while ((n = walker.nextNode())) {
          const v = n.nodeValue;
          if (!v || !v.trim()) continue;
          const p = n.parentElement;
          if (!p || (composerBar && composerBar.contains(p)) || (p.closest && p.closest(skipSel))) continue;
          if (turnBox === p || turnBox.contains(p) || (turnBox.compareDocumentPosition(p) & 4) === 0) continue;
          if (isOutOfColBounds(p) || !isVisibleEl(p, 1, 1)) continue;
          const row = p.closest ? p.closest('tr') : null;
          const cell = p.closest ? p.closest('td, th') : null;
          const blk = (p.closest && p.closest(blockSel)) || p;
          if (parts.length > 0) {
            if (row && row === lastRow && cell && cell !== lastCell) parts.push(' | ');
            else if (blk !== lastBlk) parts.push('\n');
          }
          parts.push(v.trim());
          lastBlk = blk;
          lastRow = row;
          lastCell = cell;
        }
      } catch (_) {}
      return parts.join('').replace(/\n{3,}/g, '\n\n').trim();
    };

    const isBeforeUserAnchor = (el) => {
      const ref = userTurnBox || userAnchorEl;
      if (!ref || !el) return false;
      if (el === ref || el.contains(ref) || ref.contains(el)) return true;
      return (ref.compareDocumentPosition(el) & 4) === 0;
    };

    let text = '';
    if (userTurnBox) {
      const afterDom = cleanChrome(extractAfterUserTurn(searchRoot, userTurnBox));
      if (afterDom && !isPromptOverlap(afterDom)) {
        text = afterDom;
      }
    }

    if (!text) {
      const baseAsstSelector = '[data-message-author-role="assistant"], [data-role="assistant"], [data-testid*="assistant" i]';
      const assistantSelector = adapterAsst.length ? adapterAsst.join(', ') + ', ' + baseAsstSelector : baseAsstSelector;
      let rawAsstNodes = [];
      try { rawAsstNodes = Array.from(searchRoot.querySelectorAll(assistantSelector)); } catch (_) { rawAsstNodes = Array.from(searchRoot.querySelectorAll(baseAsstSelector)); }
      const assistantNodes = rawAsstNodes.filter((el) => !isExcluded(el) && !isMessageWrapper(el) && isVisibleEl(el, 20, 12));
      const collected = [];
      for (let i = 0; i < assistantNodes.length; i++) {
        const node = assistantNodes[i];
        if (userTurnBox && isBeforeUserAnchor(node)) continue;
        if (assistantNodes.some((other) => other !== node && other.contains(node))) continue;
        const t = cleanChrome(getNodeText(node));
        if (!t || isPromptOverlap(t)) continue;
        collected.push(t);
      }
      if (collected.length > 0) {
        text = userTurnBox ? collected.join('\n\n') : collected[collected.length - 1];
      }
    }

    if (!text) {
      for (let i = allNodes.length - 1; i >= 0; i--) {
        const el = allNodes[i];
        if (userTurnBox && isBeforeUserAnchor(el)) continue;
        if (el.getAttribute('data-message-author-role') === 'user' || el.getAttribute('data-role') === 'user') continue;
        const rawT = getNodeText(el);
        if (!rawT || rawT.includes('[阶段要求]') || rawT.includes(endMarker) || (qNeedle && rawT.includes(qNeedle))) continue;
        const t = cleanChrome(rawT);
        if (!t || isPromptOverlap(t)) continue;
        text = t;
        break;
      }
    }

    return JSON.stringify({
      ok: Boolean(text),
      text: text || '',
      error: text ? '' : '未在当前页面检测到可同步的助手回复',
    });
  } catch (err) {
    return JSON.stringify({ ok: false, text: '', error: String(err) });
  }
})"#
    .to_string()
    + &format!("({adapter_json}, {query_json})")
}

#[cfg(all(test, desktop))]
mod tests {
  use super::*;

  fn test_adapter() -> SiteAdapterConfig {
    serde_json::from_value(serde_json::json!({
      "id": "muse",
      "name": "Muse",
      "newChatSelectors": ["button"],
      "composerSelectors": ["textarea"]
    }))
    .unwrap()
  }

  fn json_str(value: &str) -> String {
    serde_json::to_string(value).unwrap()
  }

  /// 占位符碰撞回归：旧实现用链式 .replace()，值里的占位符会被后一次
  /// replace 展开成 JS（文件名含 `__CONTENT__` + 内容含引号/分号即可
  /// 逃出字符串）。新实现把值当 IIFE 实参传，输出里必须原样包含
  /// 每个值的 JSON 编码，且模板里不再出现任何 `__X__` 占位符。
  #[test]
  fn attach_file_values_with_placeholder_tokens_stay_verbatim() {
    let attachment = GroupAttachment {
      filename: "a__CONTENT__b\"; evil();//".to_string(),
      mime_type: "text/plain__FILENAME__".to_string(),
      content: "x\"; alert(1);//__MIME__\n__ADAPTER__".to_string(),
    };
    let js = build_group_attach_file_js(&test_adapter(), &attachment);
    assert!(js.starts_with("((adapter, fileName, mimeType, content) => {"));
    assert!(js.contains(&json_str("a__CONTENT__b\"; evil();//")));
    assert!(js.contains(&json_str("text/plain__FILENAME__")));
    assert!(js.contains(&json_str("x\"; alert(1);//__MIME__\n__ADAPTER__")));
    // 值以 JSON 实参形式原样出现（而不是被某次 .replace() 展开），
    // 且调用点 `})({...args...})` 存在。
    assert!(js.contains("})({"));
  }

  #[test]
  fn group_send_job_and_prompt_with_placeholder_tokens_stay_verbatim() {
    let js = build_group_send_js(
      &test_adapter(),
      "job__PROMPT__1",
      "hi __JOB_ID__\"; evil();//",
    );
    assert!(js.starts_with("((adapter, jobId, promptText) => {"));
    assert!(js.contains(&json_str("job__PROMPT__1")));
    assert!(js.contains(&json_str("hi __JOB_ID__\"; evil();//")));
    assert!(js.contains("})({"));
  }

  #[test]
  fn new_chat_click_adapter_with_placeholder_selector_stays_verbatim() {
    let mut adapter = test_adapter();
    adapter.new_chat_selectors = vec!["button__FILENAME__".to_string()];
    let js = build_new_chat_click_js(&adapter);
    assert!(js.starts_with("((adapter) => {"));
    assert!(js.contains(&json_str("button__FILENAME__")));
    assert!(js.contains("})({"));
  }

  #[test]
  fn preview_and_query_values_with_placeholder_tokens_stay_verbatim() {
    let preview = build_hark_click_preview_card_js("hello \"__PREVIEW__\" ; evil();//");
    assert!(preview.starts_with("((previewTextArg) => {"));
    assert!(preview.contains(&json_str("hello \"__PREVIEW__\" ; evil();//")));
    assert!(preview.contains("})("));

    let sync = build_group_sync_latest_js(&test_adapter(), "user text __ADAPTER__ \"; evil();//");
    assert!(sync.starts_with("((adapter, rawUserContent) => {"));
    assert!(sync.contains(&json_str("user text __ADAPTER__ \"; evil();//")));
    assert!(sync.contains("})({"));
  }
}
