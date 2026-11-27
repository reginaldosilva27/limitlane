//! API-equivalent pricing. Subscription usage is not billed per token; this
//! answers "what would this have cost on the API".

use serde::{Deserialize, Serialize};

/// Token counts for one API response.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

impl Usage {
    pub fn cache_write(&self) -> u64 {
        self.cache_write_5m + self.cache_write_1h
    }
    /// Tokens that occupy the context window for this request.
    pub fn prompt_tokens(&self) -> u64 {
        self.input + self.cache_read + self.cache_write()
    }
    pub fn total(&self) -> u64 {
        self.prompt_tokens() + self.output
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tier {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongPrompt {
    pub threshold_tokens: u64,
    #[serde(flatten)]
    pub tier: Tier,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rate {
    pub pattern: String,
    #[serde(flatten)]
    pub tier: Tier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_prompt: Option<LongPrompt>,
}

#[derive(Debug, Deserialize)]
struct PricingFile {
    as_of: String,
    rates: Vec<Rate>,
}

#[derive(Debug, Clone)]
pub struct Pricing {
    pub as_of: String,
    rates: Vec<Rate>,
}

const BUILTIN: &str = include_str!("pricing.json");

impl Pricing {
    pub fn builtin() -> Self {
        let file: PricingFile = serde_json::from_str(BUILTIN).expect("bundled pricing.json is valid");
        Self::new(file.as_of, file.rates)
    }

    /// Built-in table with user overrides taking precedence on equal patterns.
    pub fn with_overrides(overrides: &[Rate]) -> Self {
        let mut base = Self::builtin();
        let mut rates: Vec<Rate> = overrides.to_vec();
        for r in base.rates.drain(..) {
            if !rates.iter().any(|o| o.pattern == r.pattern) {
                rates.push(r);
            }
        }
        Self::new(base.as_of, rates)
    }

    fn new(as_of: String, mut rates: Vec<Rate>) -> Self {
        // Stable sort: longest pattern first, so "opus-5-5" wins over "opus-5".
        rates.sort_by(|a, b| b.pattern.len().cmp(&a.pattern.len()));
        Self { as_of, rates }
    }

    pub fn rate_for(&self, model: &str) -> Option<&Rate> {
        let m = model.to_ascii_lowercase();
        self.rates.iter().find(|r| m.contains(&r.pattern))
    }

    /// Cost in USD, or `None` when the model is not in the table.
    pub fn cost(&self, model: &str, u: &Usage) -> Option<f64> {
        let rate = self.rate_for(model)?;
        let tier = match &rate.long_prompt {
            Some(lp) if u.prompt_tokens() > lp.threshold_tokens => &lp.tier,
            _ => &rate.tier,
        };
        let per_m = u.input as f64 * tier.input
            + u.output as f64 * tier.output
            + u.cache_read as f64 * tier.cache_read
            + u.cache_write_5m as f64 * tier.cache_write_5m
            + u.cache_write_1h as f64 * tier.cache_write_1h;
        Some(per_m / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn longest_pattern_wins() {
        let p = Pricing::builtin();
        assert_eq!(p.rate_for("claude-opus-5-5-20261001").unwrap().pattern, "opus-5-5");
        assert_eq!(p.rate_for("claude-opus-5-20260601").unwrap().pattern, "opus-5");
        assert_eq!(p.rate_for("claude-fable-5-2").unwrap().pattern, "fable-5-2");
        assert_eq!(p.rate_for("claude-sonnet-4-6").unwrap().pattern, "sonnet-4");
        assert_eq!(p.rate_for("claude-3-5-sonnet-20241022").unwrap().pattern, "3-5-sonnet");
        assert!(p.rate_for("<synthetic>").is_none());
    }

    #[test]
    fn cost_uses_every_token_class() {
        let p = Pricing::builtin();
        let u = Usage { input: 1_000_000, output: 1_000_000, cache_read: 1_000_000, cache_write_5m: 1_000_000, cache_write_1h: 1_000_000 };
        // opus 5.5: 4 + 20 + 0.2 + 5 + 8
        close(p.cost("claude-opus-5-5", &u).unwrap(), 37.2);
    }

    #[test]
    fn long_prompt_tier_applies_above_threshold() {
        let p = Pricing::builtin();
        let short = Usage { input: 100_000, output: 1_000, ..Default::default() };
        close(p.cost("claude-haiku-5-5", &short).unwrap(), 0.01 + 0.0005);
        let long = Usage { input: 100_001, output: 1_000, ..Default::default() };
        close(p.cost("claude-haiku-5-5", &long).unwrap(), 100_001.0 * 0.5 / 1e6 + 0.0025);
    }

    #[test]
    fn overrides_replace_builtin() {
        let o = Rate {
            pattern: "opus-5-5".into(),
            tier: Tier { input: 1.0, output: 1.0, cache_read: 1.0, cache_write_5m: 1.0, cache_write_1h: 1.0 },
            long_prompt: None,
        };
        let p = Pricing::with_overrides(&[o]);
        let u = Usage { input: 2_000_000, ..Default::default() };
        close(p.cost("claude-opus-5-5", &u).unwrap(), 2.0);
    }
}
