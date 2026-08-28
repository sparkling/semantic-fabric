use std::collections::{BTreeMap, BTreeSet};

pub(super) fn active_optional_aliases(
    declared_features: &BTreeMap<String, Vec<String>>,
    active_features: &[String],
) -> BTreeSet<String> {
    let mut active = BTreeSet::new();
    for feature in active_features {
        let Some(members) = declared_features.get(feature) else {
            continue;
        };
        for member in members {
            if let Some(alias) = member.strip_prefix("dep:") {
                active.insert(normalize_alias(alias));
                continue;
            }
            let Some((dependency, _)) = member.split_once('/') else {
                continue;
            };
            // Cargo's weak `dep?/feature` syntax never activates the optional
            // dependency. If another active feature enables it, that feature's
            // own `dep:` or non-weak member is processed independently.
            if !dependency.ends_with('?') {
                active.insert(normalize_alias(dependency));
            }
        }
    }
    active
}

pub(super) fn dependency_alias(name: &str, rename: Option<&str>) -> String {
    normalize_alias(rename.unwrap_or(name))
}

fn normalize_alias(value: &str) -> String {
    value.replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_members_do_not_activate_optional_dependencies() {
        let declared = BTreeMap::from([(
            "a-weak".to_owned(),
            vec!["z-dependency?/feature".to_owned()],
        )]);

        assert!(active_optional_aliases(&declared, &["a-weak".to_owned()]).is_empty());
    }

    #[test]
    fn a_later_feature_can_activate_a_dependency_referenced_weakly_earlier() {
        let declared = BTreeMap::from([
            (
                "a-weak".to_owned(),
                vec!["z-dependency?/feature".to_owned()],
            ),
            ("z-enable".to_owned(), vec!["dep:z-dependency".to_owned()]),
        ]);

        assert_eq!(
            active_optional_aliases(&declared, &["a-weak".to_owned(), "z-enable".to_owned()]),
            BTreeSet::from(["z_dependency".to_owned()])
        );
    }
}
