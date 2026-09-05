use std::collections::BTreeSet;
use std::str::FromStr;

use cargo_platform::{Cfg, Platform};

const QUALIFICATION_CFG: &str = concat!(
    "debug_assertions\n",
    "panic=\"unwind\"\n",
    "target_abi=\"\"\n",
    "target_arch=\"x86_64\"\n",
    "target_endian=\"little\"\n",
    "target_env=\"gnu\"\n",
    "target_family=\"unix\"\n",
    "target_feature=\"fxsr\"\n",
    "target_feature=\"sse\"\n",
    "target_feature=\"sse2\"\n",
    "target_has_atomic=\"16\"\n",
    "target_has_atomic=\"32\"\n",
    "target_has_atomic=\"64\"\n",
    "target_has_atomic=\"8\"\n",
    "target_has_atomic=\"ptr\"\n",
    "target_os=\"linux\"\n",
    "target_pointer_width=\"64\"\n",
    "target_vendor=\"unknown\"\n",
    "unix\n",
);

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

pub(super) fn canonical_qualification_cfg(raw: &str) -> Result<String, String> {
    if raw != QUALIFICATION_CFG {
        return Err(
            "qualification target cfg is not the exact pinned x86_64 GNU/Linux fact set".to_owned(),
        );
    }
    Ok(raw.to_owned())
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

    #[test]
    fn qualification_cfg_requires_canonical_gnu_x86_64_facts() {
        let valid = QUALIFICATION_CFG;
        assert_eq!(canonical_qualification_cfg(valid).unwrap(), valid);
        assert!(canonical_qualification_cfg(&valid.replace("gnu", "musl")).is_err());
        assert!(canonical_qualification_cfg(&valid.replace(
            "target_arch=\"x86_64\"\ntarget_endian=\"little\"",
            "target_endian=\"little\"\ntarget_arch=\"x86_64\""
        ))
        .is_err());
        assert!(canonical_qualification_cfg(&valid.replace(
            "target_os=\"linux\"\n",
            "target_os=\"linux\"\ntarget_os=\"windows\"\n"
        ))
        .is_err());
        assert!(canonical_qualification_cfg(&format!("{valid}windows\n")).is_err());
    }
}
