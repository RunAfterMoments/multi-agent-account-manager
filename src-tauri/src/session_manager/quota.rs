//! Quota evaluation (P2-11 split).
//!
//! Moved verbatim from `session_manager.rs`: the pure `evaluate_account_quota`
//! function and its helpers, which classify an account's quota state from the
//! stored record (plan usage, extra caps, remaining text). No behavior change:
//! the parent module re-exports `AccountQuotaEval` / `evaluate_account_quota`
//! so call sites are untouched. The struct fields are `pub(crate)` because the
//! group auto-routing code in the parent constructs a default eval literal.

use super::AUTO_ROUTE_STICKY_MAX_USED_PCT;
use crate::storage::AccountRecord;

#[cfg_attr(not(desktop), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AccountQuotaEval {
  pub(crate) depleted: bool,
  pub(crate) sticky_ok: bool,
  pub(crate) tier: u8,
  pub(crate) effective_used_pct: u8,
}

#[cfg_attr(not(desktop), allow(dead_code))]
pub(crate) fn is_quota_value_depleted(val: &str) -> bool {
  val.contains("用尽")
    || val.contains("耗尽")
    || val.contains("DEPLETED")
    || val.contains("剩余 0")
    || val.contains("剩余0")
}

#[cfg_attr(not(desktop), allow(dead_code))]
pub(crate) fn is_quota_value_low(val: &str) -> bool {
  val.contains("偏低") || val.contains("LOW")
}

#[cfg_attr(not(desktop), allow(dead_code))]
pub(crate) fn evaluate_account_quota(acc: &AccountRecord) -> AccountQuotaEval {
  if let Some(d) = &acc.quota_detail {
    let plan_used = d.plan_used_percent.map(|p| p.min(100));
    let plan_val = d.plan_value.as_str();
    let plan_depleted = plan_used.is_some_and(|p| p >= 100) || is_quota_value_depleted(plan_val);
    let plan_low = !plan_depleted
      && (plan_used.is_some_and(|p| p >= AUTO_ROUTE_STICKY_MAX_USED_PCT)
        || is_quota_value_low(plan_val));

    let has_extra =
      d.extra_label.is_some() || d.extra_value.is_some() || d.extra_used_percent.is_some();
    let extra_used = d.extra_used_percent.map(|p| p.min(100));
    let extra_val = d.extra_value.as_deref().unwrap_or("");
    let extra_depleted =
      has_extra && (extra_used.is_some_and(|p| p >= 100) || is_quota_value_depleted(extra_val));
    let extra_low = has_extra
      && !extra_depleted
      && (extra_used.is_some_and(|p| p >= AUTO_ROUTE_STICKY_MAX_USED_PCT)
        || is_quota_value_low(extra_val));
    let extra_is_cap = d.extra_label.as_deref().is_some_and(|lbl| {
      let lower = lbl.to_ascii_lowercase();
      lbl.contains('月') || lower.contains("monthly")
    });

    if extra_is_cap {
      if plan_depleted || extra_depleted {
        return AccountQuotaEval {
          depleted: true,
          sticky_ok: false,
          tier: 4,
          effective_used_pct: 100,
        };
      }
      let effective_used_pct = plan_used.unwrap_or(0).max(extra_used.unwrap_or(0));
      let low = plan_low || extra_low;
      return AccountQuotaEval {
        depleted: false,
        sticky_ok: !low,
        tier: if low { 1 } else { 0 },
        effective_used_pct,
      };
    }

    if plan_depleted {
      if !has_extra || extra_depleted {
        return AccountQuotaEval {
          depleted: true,
          sticky_ok: false,
          tier: 4,
          effective_used_pct: 100,
        };
      }
      return AccountQuotaEval {
        depleted: false,
        sticky_ok: !extra_low,
        tier: if extra_low { 2 } else { 1 },
        effective_used_pct: extra_used.unwrap_or(if extra_low { 90 } else { 50 }),
      };
    }

    let sticky_ok = !plan_low;
    let effective_used_pct = plan_used.unwrap_or(if plan_low { 90 } else { 50 });
    return AccountQuotaEval {
      depleted: false,
      sticky_ok,
      tier: if plan_low { 1 } else { 0 },
      effective_used_pct,
    };
  }

  if let Some(rem) = acc.quota_percent {
    let used = 100u8.saturating_sub(rem.min(100));
    if used >= 100
      || acc
        .quota_text
        .as_deref()
        .is_some_and(is_quota_value_depleted)
    {
      return AccountQuotaEval {
        depleted: true,
        sticky_ok: false,
        tier: 4,
        effective_used_pct: 100,
      };
    }
    let low = used >= AUTO_ROUTE_STICKY_MAX_USED_PCT
      || acc.quota_text.as_deref().is_some_and(is_quota_value_low);
    return AccountQuotaEval {
      depleted: false,
      sticky_ok: !low,
      tier: if low { 1 } else { 0 },
      effective_used_pct: used,
    };
  }

  if let Some(txt) = acc.quota_text.as_deref() {
    if is_quota_value_depleted(txt) {
      return AccountQuotaEval {
        depleted: true,
        sticky_ok: false,
        tier: 4,
        effective_used_pct: 100,
      };
    }
    if is_quota_value_low(txt) {
      return AccountQuotaEval {
        depleted: false,
        sticky_ok: false,
        tier: 1,
        effective_used_pct: 90,
      };
    }
  }

  AccountQuotaEval {
    depleted: false,
    sticky_ok: true,
    tier: 3,
    effective_used_pct: 50,
  }
}
