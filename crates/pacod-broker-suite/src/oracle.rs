//! The scenario's own answer to a query: an independent filter of what the providers announced,
//! written plainly so that it can be checked against the broker's index.
//!
//! It understands the filters that need no time evaluation and no text search: kind, tags,
//! capabilities (flags and MCP tools), protocols, regions, model limits, residency, training use,
//! currency and maximum prices on a period's base rates. Scenarios that use the oracle keep to
//! these, and to resources without schedules (always available) and with public visibility in
//! open mode.

use std::collections::HashSet;

use serde_json::Value;

use crate::run::Run;

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn capability(r: &Value, name: &str) -> bool {
    let c = &r["capabilities"];
    if let Some(tool) = name.strip_prefix("mcp_tool:") {
        return c["mcp_tools"].as_array().into_iter().flatten().any(|t| t["name"] == tool);
    }
    if let Some(key) = name.strip_prefix("extra:") {
        return !matches!(c["extra"][key], Value::Null | Value::Bool(false));
    }
    c[name] == true
}

/// The ids of the resources `q` should find, from the scenario's announcements.
pub(crate) fn expected(run: &Run<'_>, q: &Value) -> anyhow::Result<HashSet<String>> {
    for unsupported in ["q", "at", "during", "availability", "hardware", "software", "min_reputation", "provider"] {
        anyhow::ensure!(q.get(unsupported).is_none(), "the oracle does not evaluate `{unsupported}`");
    }
    let mut out = HashSet::new();
    for (name, a) in run.live_announcements() {
        for r in a["resources"].as_array().into_iter().flatten() {
            let key = r["key"].as_str().unwrap_or_default();
            if let Some(k) = q["kind"].as_str()
                && r["kind"] != k
            {
                continue;
            }
            let tags = strs(&r["tags"]);
            if !strs(&q["tags"]).iter().all(|t| tags.contains(t)) {
                continue;
            }
            if !strs(&q["capabilities"]).iter().all(|c| capability(r, c)) {
                continue;
            }
            let protocols: Vec<String> = r["interfaces"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["protocol"].as_str().map(String::from))
                .collect();
            let wanted = strs(&q["protocols"]);
            if !wanted.is_empty() && !wanted.iter().any(|p| protocols.contains(p)) {
                continue;
            }
            let regions = strs(&r["availability"]["regions"]);
            let wanted = strs(&q["regions"]);
            if !wanted.is_empty() && !wanted.iter().any(|g| regions.contains(g)) {
                continue;
            }
            let residency = strs(&r["policy"]["residency"]);
            let allowed = strs(&q["residency"]);
            if !allowed.is_empty() && (residency.is_empty() || !residency.iter().all(|x| allowed.contains(x))) {
                continue;
            }
            if q["no_training_use"] == true && r["policy"]["training_use"] != false {
                continue;
            }
            let m = &q["model"];
            if !m.is_null() {
                let info = &r["model"];
                if info.is_null() {
                    continue;
                }
                if let Some(min) = m["min_context_window"].as_u64()
                    && info["context_window"].as_u64().is_none_or(|c| c < min)
                {
                    continue;
                }
                if let Some(f) = m["family"].as_str()
                    && info["family"] != f
                {
                    continue;
                }
            }
            let pricing = &r["pricing"];
            let free = pricing["free"] == true;
            if let Some(cur) = q["currency"].as_str()
                && !(free || pricing["currency"].as_str().is_some_and(|c| c.eq_ignore_ascii_case(cur)))
            {
                continue;
            }
            let rates = &pricing["periods"][0]["rates"];
            let priced_ok = q["max_price"].as_object().into_iter().flatten().all(|(rate, max)| {
                let v = if free { Some(0.0) } else { rates[rate].as_f64() };
                match v {
                    Some(v) => v <= max.as_f64().unwrap_or(f64::INFINITY) + 1e-12,
                    None => q["include_unpriced"] == true,
                }
            });
            if !priced_ok {
                continue;
            }
            out.insert(run.rid_of(name, key));
        }
    }
    Ok(out)
}
