//! Checking answers: values at paths, and comparisons written as text.

use serde_json::Value;

/// The value at `path` (`hits[0].price.rates.input`, `live.status`), if there is one.
pub fn at<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        let (name, rest) = match part.find('[') {
            Some(i) => (&part[..i], &part[i..]),
            None => (part, ""),
        };
        if !name.is_empty() {
            cur = cur.get(name)?;
        }
        let mut r = rest;
        while let Some(stripped) = r.strip_prefix('[') {
            let end = stripped.find(']')?;
            let i: usize = stripped[..end].parse().ok()?;
            cur = cur.get(i)?;
            r = &stripped[end + 1..];
        }
    }
    Some(cur)
}

/// Sets the value at `path`, creating objects on the way; arrays must already be long enough.
pub fn set(v: &mut Value, path: &str, value: Value) -> anyhow::Result<()> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur = v;
    for (n, part) in parts.iter().enumerate() {
        let last = n + 1 == parts.len();
        let (name, rest) = match part.find('[') {
            Some(i) => (&part[..i], &part[i..]),
            None => (*part, ""),
        };
        let mut idx: Vec<usize> = Vec::new();
        let mut r = rest;
        while let Some(s) = r.strip_prefix('[') {
            let end = s.find(']').ok_or_else(|| anyhow::anyhow!("{path}: unclosed ["))?;
            idx.push(s[..end].parse()?);
            r = &s[end + 1..];
        }
        if !name.is_empty() {
            if !cur.is_object() {
                *cur = Value::Object(Default::default());
            }
            let obj = cur.as_object_mut().expect("an object");
            if last && idx.is_empty() {
                obj.insert(name.to_string(), value);
                return Ok(());
            }
            cur = obj.entry(name.to_string()).or_insert(Value::Null);
        }
        for (k, i) in idx.iter().enumerate() {
            let arr = cur.as_array_mut().ok_or_else(|| anyhow::anyhow!("{path}: {name} is not a list"))?;
            let len = arr.len();
            let slot = arr.get_mut(*i).ok_or_else(|| anyhow::anyhow!("{path}: index {i} past {len} items"))?;
            if last && k + 1 == idx.len() {
                *slot = value;
                return Ok(());
            }
            cur = slot;
        }
    }
    Ok(())
}

/// Whether `actual` meets `want`. A string starting with `>`, `>=`, `<`, `<=`, `!=` compares
/// numbers (a unit `ms` or `s` on the right is read as seconds or milliseconds as written);
/// `~text` means "contains"; anything else must be equal.
pub fn meets(actual: Option<&Value>, want: &Value) -> Result<(), String> {
    if let Value::String(w) = want {
        for op in [">=", "<=", "!=", ">", "<"] {
            if let Some(rhs) = w.strip_prefix(op) {
                let rhs = rhs.trim();
                if op == "!=" && rhs.parse::<f64>().is_err() {
                    return match actual.and_then(Value::as_str) {
                        Some(a) if a == rhs => Err(format!("{a:?} must not be {rhs:?}")),
                        _ => Ok(()),
                    };
                }
                let r = number(rhs).ok_or_else(|| format!("{w:?}: not a number on the right"))?;
                let a = actual
                    .and_then(Value::as_f64)
                    .ok_or_else(|| format!("no number to compare with {w}, found {}", show(actual)))?;
                let ok = match op {
                    ">=" => a >= r,
                    "<=" => a <= r,
                    ">" => a > r,
                    "<" => a < r,
                    _ => (a - r).abs() > 1e-12,
                };
                return if ok { Ok(()) } else { Err(format!("{a} is not {w}")) };
            }
        }
        if let Some(part) = w.strip_prefix('~') {
            let text = actual.map(|a| a.to_string()).unwrap_or_default();
            return if text.contains(part) {
                Ok(())
            } else {
                Err(format!("{} does not contain {part:?}", show(actual)))
            };
        }
    }
    match (actual, want) {
        (Some(a), w) if a == w => Ok(()),
        // 0.1 written in YAML and 0.1 computed may differ in the last bit.
        (Some(Value::Number(a)), Value::Number(w))
            if (a.as_f64().unwrap_or(f64::NAN) - w.as_f64().unwrap_or(f64::NAN)).abs() < 1e-9 =>
        {
            Ok(())
        }
        (None, Value::Null) => Ok(()),
        _ => Err(format!("found {}, expected {want}", show(actual))),
    }
}

fn number(s: &str) -> Option<f64> {
    if let Some(v) = s.strip_suffix("ms") {
        return v.trim().parse().ok();
    }
    if let Some(v) = s.strip_suffix('s') {
        return v.trim().parse::<f64>().ok().map(|x| x * 1000.0);
    }
    s.parse().ok()
}

fn show(v: Option<&Value>) -> String {
    match v {
        None => "nothing".into(),
        Some(v) => {
            let s = v.to_string();
            if s.len() > 200 { format!("{}…", &s[..200]) } else { s }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_and_comparisons() {
        let v = json!({"hits": [{"price": {"rates": {"input": 0.25}, "source": "window:night"}}], "total": 3});
        assert_eq!(at(&v, "hits[0].price.rates.input"), Some(&json!(0.25)));
        assert!(meets(at(&v, "total"), &json!(">=3")).is_ok());
        assert!(meets(at(&v, "total"), &json!("<3")).is_err());
        assert!(meets(at(&v, "hits[0].price.source"), &json!("~night")).is_ok());
        assert!(meets(at(&v, "hits[0].price.source"), &json!("!=base")).is_ok());
        assert!(meets(at(&v, "nothing"), &json!(1)).is_err());
        let mut w = json!({"resources": [{"pricing": {"periods": [{"rates": {"input": 1}}]}}]});
        set(&mut w, "resources[0].pricing.periods[0].rates.input", json!(0.2)).unwrap();
        set(&mut w, "provider.name", json!("x")).unwrap();
        assert_eq!(at(&w, "resources[0].pricing.periods[0].rates.input"), Some(&json!(0.2)));
        assert_eq!(at(&w, "provider.name"), Some(&json!("x")));
    }
}
