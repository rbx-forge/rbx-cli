/// Conversions between toml::Value and serde_json::Value,
/// and CLI string → typed value parsing.
use serde_json::Value as Json;
use toml::Value as Toml;

/// Convert a TOML value to JSON for the Roblox API.
pub fn toml_to_json(v: Toml) -> Json {
    match v {
        Toml::Boolean(b) => Json::Bool(b),
        Toml::Integer(i) => Json::Number(i.into()),
        Toml::Float(f) => serde_json::Number::from_f64(f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Toml::String(s) => Json::String(s),
        Toml::Array(arr) => Json::Array(arr.into_iter().map(toml_to_json).collect()),
        Toml::Table(t) => Json::Object(t.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect()),
        Toml::Datetime(dt) => Json::String(dt.to_string()),
    }
}

/// Convert a JSON value (from Roblox API) back to TOML for rbxconfig.toml.
///
/// JSON numbers that came in as floats but have no fractional part (e.g. 12345.0)
/// are demoted to integers — the Roblox API tends to round-trip ints through floats,
/// and writing `12345.0` for what the user typed as `12345` is surprising.
pub fn json_to_toml(v: Json) -> Toml {
    match v {
        Json::Bool(b) => Toml::Boolean(b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Toml::Integer(i)
            } else if let Some(f) = n.as_f64() {
                if f.is_finite()
                    && f.fract() == 0.0
                    && (i64::MIN as f64..=i64::MAX as f64).contains(&f)
                {
                    Toml::Integer(f as i64)
                } else {
                    Toml::Float(f)
                }
            } else {
                Toml::Float(0.0)
            }
        }
        Json::String(s) => Toml::String(s),
        Json::Array(arr) => Toml::Array(arr.into_iter().map(json_to_toml).collect()),
        Json::Object(obj) => {
            Toml::Table(obj.into_iter().map(|(k, v)| (k, json_to_toml(v))).collect())
        }
        Json::Null => Toml::String("null".to_string()),
    }
}

/// Compact display of a JSON value (truncated for terminal output).
pub fn compact(v: &Json) -> String {
    match v {
        Json::String(s) => {
            if s.len() > 50 {
                // Truncate by characters, not bytes: a byte slice can split a
                // multibyte UTF-8 codepoint (emoji, accents) and panic.
                let head: String = s.chars().take(47).collect();
                format!("\"{}...\"", head)
            } else {
                format!("\"{}\"", s)
            }
        }
        other => {
            let encoded = other.to_string();
            if encoded.len() > 80 {
                let head: String = encoded.chars().take(77).collect();
                format!("{}...", head)
            } else {
                encoded
            }
        }
    }
}

/// Human-readable type label (matching the Lune list command).
pub fn type_label(v: &Json) -> &'static str {
    match v {
        Json::Bool(_) => "bool",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
        Json::Null => "null",
    }
}

/// Canonical JSON string for value comparison.
///
/// Whole-number floats are normalized to integers (recursively, including
/// inside arrays/objects) so that `12345` and `12345.0` — which the Roblox
/// API tends to return as floats even when the user stored an int — compare
/// as equal. Without this, every sync would re-publish numeric fields as
/// phantom updates.
pub fn canonical(v: &Json) -> String {
    normalize_numbers(v).to_string()
}

fn normalize_numbers(v: &Json) -> Json {
    match v {
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Json::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                if f.is_finite()
                    && f.fract() == 0.0
                    && (i64::MIN as f64..=i64::MAX as f64).contains(&f)
                {
                    Json::Number((f as i64).into())
                } else {
                    Json::Number(n.clone())
                }
            } else {
                Json::Number(n.clone())
            }
        }
        Json::Array(arr) => Json::Array(arr.iter().map(normalize_numbers).collect()),
        Json::Object(obj) => Json::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), normalize_numbers(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}
