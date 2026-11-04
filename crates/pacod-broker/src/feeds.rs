//! Price feeds for gateways (docs/pacod-gw.md, R2a).
//!
//! pacod-gw's price tracker reads feeds in its own `pacod` format: `{"currency", "models": {"<model
//! id>": {rates…, "variants", "tiers", "windows"}}}`, token rates per million tokens. The broker
//! serves the prices in force in that format, so a gateway keeps its model repository's prices
//! current from the broker without new code on its side.
//!
//! What the format cannot say is left out, never approximated: overrides (one-off spot prices),
//! commitments, compute rates (per second, hour or GPU hour) and tiers by other units than input
//! tokens. Windows become variants named after the window, with the hours they apply in UTC for
//! the current week; a window in a zone with daylight saving therefore moves by an hour in UTC
//! across a change, as the feed is read again.

use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use pacod_broker_model::pricing::{PriceWindow, TierUnit};
use pacod_broker_model::schedule::Weekday;
use pacod_broker_model::{Pricing, Rates};
use schemars::JsonSchema;
use serde::Serialize;

/// The rates gw's tracker reads; each optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, JsonSchema)]
pub struct FeedRates {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    /// The broker's `cache_write`: gw names the five-minute write its default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_5m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_request: Option<f64>,
    /// The broker's `per_call` (a tool or MCP call).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_server_tool_call: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_image: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_audio_second: Option<f64>,
}

impl FeedRates {
    /// The rates gw understands; `None` when there are none (a compute resource priced by the hour).
    pub fn of(r: &Rates) -> Option<Self> {
        let f = Self {
            input: r.input,
            output: r.output,
            cache_read: r.cache_read,
            cache_write_5m: r.cache_write,
            reasoning: r.reasoning,
            per_request: r.per_request,
            per_server_tool_call: r.per_call,
            per_image: r.per_image,
            per_audio_second: r.per_audio_second,
        };
        (f != Self::default()).then_some(f)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct FeedTier {
    pub above_input_tokens: u64,
    #[serde(flatten)]
    pub rates: FeedRates,
}

/// Hours in which a variant applies: ISO weekdays and `HH:MM-HH:MM` spans in UTC.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct FeedWindow {
    pub variant: String,
    pub weekdays: Vec<u8>,
    pub hours_utc: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct FeedEntry {
    #[serde(flatten)]
    pub rates: FeedRates,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, FeedRates>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<FeedTier>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<FeedWindow>,
}

/// A feed in pacod-gw's `pacod` format.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PriceFeed {
    pub currency: String,
    pub models: BTreeMap<String, FeedEntry>,
}

/// The feed entry of one resource's pricing at `at`: the period in force, without what the format
/// cannot carry. `None` for a free resource, one without a period now, or one priced only in
/// units gw does not know.
pub fn entry(p: &Pricing, at: Timestamp) -> Option<FeedEntry> {
    if p.free {
        return None;
    }
    let period = p.period_in_force(at)?;
    let rates = FeedRates::of(&period.rates)?;
    let tiers = period
        .tiers
        .iter()
        .filter(|t| t.unit == TierUnit::InputTokens && t.above >= 0.0)
        .filter_map(|t| Some(FeedTier { above_input_tokens: t.above as u64, rates: FeedRates::of(&t.rates)? }))
        .collect();
    let mut variants = BTreeMap::new();
    let mut windows = Vec::new();
    for w in &period.windows {
        let Some(r) = FeedRates::of(&w.rates) else { continue };
        let name = variant_name(&w.name);
        variants.insert(name.clone(), r);
        windows.extend(utc_windows(&name, w, p.timezone.tz(), at));
    }
    Some(FeedEntry { rates, variants, tiers, windows })
}

/// gw's variant names are `[a-z0-9_]+`; the broker's window names are slugs with hyphens.
fn variant_name(window: &str) -> String {
    let s: String =
        window.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if s.is_empty() { "window".into() } else { s }
}

/// A window's hours in UTC over the week (Monday to Sunday, in the window's zone) that holds
/// `at`: one entry per distinct span, with the UTC weekdays it falls on.
fn utc_windows(variant: &str, w: &PriceWindow, tz: &TimeZone, at: Timestamp) -> Vec<FeedWindow> {
    let today = at.to_zoned(tz.clone()).date();
    let Ok(monday) = today.checked_sub(jiff::Span::new().days(i64::from(Weekday::of(today).iso()) - 1)) else {
        return Vec::new();
    };
    let crosses = w.when.to_time <= w.when.from_time;
    let mut spans: BTreeMap<String, BTreeSet<u8>> = BTreeMap::new();
    for i in 0..7 {
        let Ok(day) = monday.checked_add(jiff::Span::new().days(i)) else { continue };
        if !(w.when.weekdays.is_empty() || w.when.weekdays.contains(&Weekday::of(day))) {
            continue;
        }
        let end_day = if crosses { day.tomorrow().ok() } else { Some(day) };
        let (Some(start), Some(end)) = (w.when.from_time.on(day, tz), end_day.and_then(|d| w.when.to_time.on(d, tz)))
        else {
            continue;
        };
        for (weekday, span) in split_by_utc_day(start, end) {
            spans.entry(span).or_default().insert(weekday);
        }
    }
    spans
        .into_iter()
        .map(|(span, days)| FeedWindow {
            variant: variant.into(),
            weekdays: days.into_iter().collect(),
            hours_utc: vec![span],
        })
        .collect()
}

/// `[start, end)` cut at UTC midnights: (ISO weekday, `HH:MM-HH:MM`) per UTC day, `24:00` for a
/// span that runs to midnight.
fn split_by_utc_day(mut start: Timestamp, end: Timestamp) -> Vec<(u8, String)> {
    let mut out = Vec::new();
    while start < end {
        let z = start.to_zoned(TimeZone::UTC);
        let date: Date = z.date();
        let Some(next) = date.tomorrow().ok().and_then(|d| d.to_zoned(TimeZone::UTC).ok()).map(|z| z.timestamp())
        else {
            break;
        };
        let piece_end = end.min(next);
        let to = if piece_end == next {
            "24:00".to_string()
        } else {
            let e = piece_end.to_zoned(TimeZone::UTC);
            format!("{:02}:{:02}", e.hour(), e.minute())
        };
        out.push((Weekday::of(date).iso(), format!("{:02}:{:02}-{to}", z.hour(), z.minute())));
        start = piece_end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pricing(v: serde_json::Value) -> Pricing {
        serde_json::from_value(v).unwrap()
    }

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn rates_tiers_and_windows_take_gws_names() {
        let p = pricing(json!({
            "currency": "EUR", "timezone": "UTC",
            "periods": [{
                "valid_from": "2026-01-01T00:00:00Z",
                "rates": {"input": 0.6, "output": 0.9, "cache_write": 0.75, "per_call": 0.01},
                "tiers": [{"above": 200000, "unit": "input_tokens", "rates": {"input": 1.2}},
                          {"above": 1000, "unit": "requests", "rates": {"input": 0.1}}],
                "windows": [{"name": "off-peak", "from_time": "22:00", "to_time": "06:00", "rates": {"input": 0.3, "output": 0.45}}]
            }]
        }));
        let e = entry(&p, at("2026-11-04T12:00:00Z")).unwrap();
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["input"], 0.6);
        assert_eq!(v["cache_write_5m"], 0.75);
        assert_eq!(v["per_server_tool_call"], 0.01);
        assert_eq!(v["tiers"], json!([{"above_input_tokens": 200000, "input": 1.2}]), "only input-token tiers");
        assert_eq!(v["variants"]["off_peak"], json!({"input": 0.3, "output": 0.45}));
        // 22:00 to 06:00 every day: 22:00–24:00 and 00:00–06:00 on all seven days.
        let w: Vec<(Vec<u8>, String)> =
            e.windows.iter().map(|w| (w.weekdays.clone(), w.hours_utc[0].clone())).collect();
        assert_eq!(
            w,
            vec![(vec![1, 2, 3, 4, 5, 6, 7], "00:00-06:00".into()), (vec![1, 2, 3, 4, 5, 6, 7], "22:00-24:00".into())]
        );
        assert!(e.windows.iter().all(|w| w.variant == "off_peak"));
    }

    #[test]
    fn windows_in_another_zone_become_utc_hours() {
        // Sofia is UTC+2 in November: 08:00–20:00 on weekdays is 06:00–18:00 UTC.
        let p = pricing(json!({
            "currency": "EUR", "timezone": "Europe/Sofia",
            "periods": [{"valid_from": "2026-01-01T00:00:00Z", "rates": {"input": 1.0},
                "windows": [{"name": "day", "weekdays": ["mon","tue","wed","thu","fri"], "from_time": "08:00", "to_time": "20:00", "rates": {"input": 2.0}}]}]
        }));
        let e = entry(&p, at("2026-11-04T12:00:00Z")).unwrap();
        assert_eq!(e.windows.len(), 1);
        assert_eq!(e.windows[0].weekdays, vec![1, 2, 3, 4, 5]);
        assert_eq!(e.windows[0].hours_utc, vec!["06:00-18:00".to_string()]);
    }

    #[test]
    fn what_gw_cannot_read_is_left_out() {
        let free = pricing(json!({"free": true}));
        assert!(entry(&free, at("2026-11-04T12:00:00Z")).is_none());
        let gpu = pricing(
            json!({"currency": "EUR", "periods": [{"valid_from": "2026-01-01T00:00:00Z", "rates": {"per_gpu_hour": 2.5}}]}),
        );
        assert!(entry(&gpu, at("2026-11-04T12:00:00Z")).is_none(), "compute rates have no gw field");
        let later = pricing(
            json!({"currency": "EUR", "periods": [{"valid_from": "2027-01-01T00:00:00Z", "rates": {"input": 1}}]}),
        );
        assert!(entry(&later, at("2026-11-04T12:00:00Z")).is_none(), "nothing in force yet");
    }

    #[test]
    fn variant_names_are_gw_slugs() {
        assert_eq!(variant_name("Off-Peak"), "off_peak");
        assert_eq!(variant_name("weekend"), "weekend");
    }
}
