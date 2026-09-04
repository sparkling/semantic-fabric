use super::*;

use sf_core::ir::{Template, TermMap, TermSpec};
use sf_sql::Dialect;

use crate::iq::SubPlanJoin;
use crate::PlanForm;

const TARGET: &str = "a_quote";
const SUBJECT: &str = "z_subject";
const PREDICATE: &str = "z_predicate";
const OBJECT: &str = "z_object";

fn env() -> StarEnv {
    let mut env = StarEnv::new();
    env.insert(
        Variable::new_unchecked(TARGET),
        ComposedInfo {
            s_var: Variable::new_unchecked(SUBJECT),
            p_var: Variable::new_unchecked(PREDICATE),
            o_var: Variable::new_unchecked(OBJECT),
        },
    );
    env
}

fn column(alias: usize, name: &str) -> TermDef {
    TermDef::Derived {
        term_map: TermMap::Column(name.into(), TermSpec::iri()),
        alias,
    }
}

fn template(alias: usize, value: &str) -> TermDef {
    TermDef::Derived {
        term_map: TermMap::Template(
            Template::parse(value).expect("valid template"),
            TermSpec::iri(),
        ),
        alias,
    }
}

fn branch(component_alias: usize, raw_alias: usize, raw_template: &str) -> Branch {
    let mut branch = Branch::empty();
    // Reverse insertion order is deliberate: projection order comes from the
    // BTreeMap's sorted keys, not insertion order.
    branch
        .bindings
        .insert(SUBJECT.to_owned(), column(component_alias, "s"));
    branch.bindings.insert(
        PREDICATE.to_owned(),
        TermDef::Const(sf_core::Term::NamedNode(sf_core::NamedNode::new_unchecked(
            "http://example.com/p",
        ))),
    );
    branch
        .bindings
        .insert(OBJECT.to_owned(), column(component_alias, "o"));
    branch
        .bindings
        .insert(TARGET.to_owned(), template(raw_alias, raw_template));
    branch
}

fn wrapped_child(child: Branch, distinct: bool) -> SubPlanJoin {
    SubPlanJoin {
        alias: 91,
        plan: Box::new(Plan {
            branches: vec![child],
            form: PlanForm::Select {
                vars: vec![TARGET.to_owned()],
            },
            distinct,
            limit: None,
            offset: 0,
            order: Vec::new(),
            rust_group: None,
            dialect: Dialect::Sqlite,
            dedup_scopes: Vec::new(),
            construct_drops_some_branch_var: false,
        }),
        on: Vec::new(),
        left: false,
    }
}

fn target_is_composed(branch: &Branch) -> bool {
    matches!(
        branch.bindings.get(TARGET),
        Some(TermDef::ComposedTriple { .. })
    )
}

#[test]
fn virtual_overlay_matches_a_committed_sorted_projection() {
    let env = env();
    let branch = branch(7, 7, "urn:quote:{s}:{o}");
    let replacement = composed_term_def(&Variable::new_unchecked(TARGET), &env, &branch.bindings)
        .expect("components are bound");
    let updates = vec![(TARGET.to_owned(), replacement)];

    let mut committed = branch.clone();
    for (name, definition) in &updates {
        committed.bindings.insert(name.clone(), definition.clone());
    }

    assert_eq!(
        projection_with_binding_updates(&branch, &updates),
        committed.projection()
    );
}

#[test]
fn accepted_root_updates_an_accepted_descendant_after_distinct_propagation() {
    let env = env();
    let mut root = branch(7, 7, "urn:quote:{s}:{o}");
    root.subplan_joins
        .push(wrapped_child(branch(12, 12, "urn:quote:{s}:{o}"), true));

    apply_composed_bindings_checked(&mut root, &env);

    let descendant = &root.subplan_joins[0].plan.branches[0];
    assert!(target_is_composed(&root));
    assert!(descendant.distinct);
    assert!(target_is_composed(descendant));
}

#[test]
fn rejected_root_leaves_itself_and_its_descendant_unmodified() {
    let env = env();
    let mut root = branch(7, 7, "urn:quote:{o}:{s}");
    root.subplan_joins
        .push(wrapped_child(branch(12, 12, "urn:quote:{s}:{o}"), true));
    let before = format!("{root:#?}");

    apply_composed_bindings_checked(&mut root, &env);

    assert_eq!(format!("{root:#?}"), before);
}

#[test]
fn accepted_root_keeps_a_rejected_descendant_but_preserves_distinct_ordering() {
    let env = env();
    let mut root = branch(7, 7, "urn:quote:{s}:{o}");
    root.subplan_joins
        .push(wrapped_child(branch(12, 12, "urn:quote:{o}:{s}"), true));

    apply_composed_bindings_checked(&mut root, &env);

    let descendant = &root.subplan_joins[0].plan.branches[0];
    assert!(target_is_composed(&root));
    assert!(descendant.distinct);
    assert!(!target_is_composed(descendant));
}

#[test]
fn alias_identity_is_part_of_the_guarded_projection_footprint() {
    let env = env();
    let mut root = branch(12, 7, "urn:quote:{s}:{o}");
    let before = format!("{root:#?}");

    apply_composed_bindings_checked(&mut root, &env);

    assert_eq!(format!("{root:#?}"), before);
}
