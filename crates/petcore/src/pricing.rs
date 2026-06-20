//! Approximate USD pricing per MILLION tokens, ported from sessions.py `PRICING`.
//! cache-write = 1.25x input (5-min TTL) / 2x input (1-hour TTL); cache-read = 0.1x input.

use crate::providers::TokenBreakdown;

#[derive(Clone, Copy, Debug)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
}

/// Per-family rates. Order of the family check matters (fable before others is
/// irrelevant since names are distinct, but we keep the sensible default = opus).
pub fn rates(family: &str) -> Rates {
    match family {
        "opus" => Rates { input: 5.0, output: 25.0, cache_write_5m: 6.25, cache_write_1h: 10.0, cache_read: 0.5 },
        "sonnet" => Rates { input: 3.0, output: 15.0, cache_write_5m: 3.75, cache_write_1h: 6.0, cache_read: 0.3 },
        "haiku" => Rates { input: 1.0, output: 5.0, cache_write_5m: 1.25, cache_write_1h: 2.0, cache_read: 0.1 },
        "fable" => Rates { input: 10.0, output: 50.0, cache_write_5m: 12.5, cache_write_1h: 20.0, cache_read: 1.0 },
        _ => rates("opus"), // sensible default for unknown / missing model ids
    }
}

/// Map a raw model id string to a family. Mirrors `_model_family`.
pub fn model_family(model: Option<&str>) -> String {
    let m = model.unwrap_or("").to_lowercase();
    for fam in ["fable", "opus", "sonnet", "haiku"] {
        if m.contains(fam) {
            return fam.to_string();
        }
    }
    "opus".to_string()
}

/// USD cost estimate for a token breakdown under a model family.
pub fn cost(tok: &TokenBreakdown, family: &str) -> f64 {
    let r = rates(family);
    (tok.input as f64 * r.input
        + tok.output as f64 * r.output
        + tok.cache_read as f64 * r.cache_read
        + tok.cache_write_5m as f64 * r.cache_write_5m
        + tok.cache_write_1h as f64 * r.cache_write_1h)
        / 1_000_000.0
}
