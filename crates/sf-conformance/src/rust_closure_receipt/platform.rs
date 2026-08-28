use std::collections::BTreeSet;
use std::str::FromStr;

use cargo_platform::{Cfg, Platform};

#[derive(Debug, Clone)]
pub(super) struct TargetContext {
    name: String,
    cfg: Vec<Cfg>,
}

impl TargetContext {
    pub(super) fn parse(name: &str, raw_cfg: &str) -> Result<Self, String> {
        super::validate_text("target name", name)?;
        let mut cfg = BTreeSet::new();
        for (index, line) in raw_cfg.lines().enumerate() {
            if line.is_empty() {
                return Err(format!("rustc target cfg line {} is empty", index + 1));
            }
            let value = Cfg::from_str(line)
                .map_err(|error| format!("parse rustc target cfg line {}: {error}", index + 1))?;
            if !cfg.insert(value) {
                return Err(format!("duplicate rustc target cfg line {}", index + 1));
            }
        }
        if cfg.is_empty() {
            return Err("rustc target cfg is empty".to_owned());
        }
        Ok(Self {
            name: name.to_owned(),
            cfg: cfg.into_iter().collect(),
        })
    }

    pub(super) fn matches(&self, target: Option<&str>) -> Result<bool, String> {
        let Some(target) = target else {
            return Ok(true);
        };
        let platform = Platform::from_str(target)
            .map_err(|error| format!("parse Cargo dependency target {target:?}: {error}"))?;
        Ok(platform.matches(&self.name, &self.cfg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_named_and_cfg_platforms() {
        let target = TargetContext::parse(
            "x86_64-unknown-linux-gnu",
            "target_arch=\"x86_64\"\ntarget_os=\"linux\"\nunix\n",
        )
        .unwrap();

        assert!(target.matches(None).unwrap());
        assert!(target.matches(Some("x86_64-unknown-linux-gnu")).unwrap());
        assert!(target.matches(Some("cfg(unix)")).unwrap());
        assert!(target.matches(Some("cfg(target_os = \"linux\")")).unwrap());
        assert!(!target.matches(Some("cfg(windows)")).unwrap());
        assert!(!target
            .matches(Some("cfg(target_arch = \"wasm32\")"))
            .unwrap());
    }

    #[test]
    fn rejects_invalid_or_duplicate_cfg_lines() {
        assert!(TargetContext::parse("target", "").is_err());
        assert!(TargetContext::parse("target", "unix\nunix\n").is_err());
        assert!(TargetContext::parse("target", "cfg(unix)\n").is_err());
    }
}
