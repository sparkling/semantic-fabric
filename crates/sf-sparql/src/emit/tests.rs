use super::*;
use crate::iq::{Scan, StrMatchOp};
use sf_core::ir::{LogicalSource, TermSpec};

#[path = "root_encoding_tests.rs"]
mod root_encoding_tests;
#[path = "root_rendering_tests.rs"]
mod root_rendering_tests;

fn branch_with(cond: SqlCond) -> Branch {
    let mut b = Branch::single(Scan {
        alias: 0,
        source: (LogicalSource::Table("emp".to_owned())).into(),
    });
    b.where_conds.push(cond);
    b
}

/// Compare SQL with the actual shared term recipe, not a second encoder.
pub(super) fn reference_encode(value: &str) -> String {
    let mut out = String::new();
    sf_core::ir::Template::parse("{v}")
        .unwrap()
        .expand(&[("v", Some(value))][..], true, &mut out);
    out
}
