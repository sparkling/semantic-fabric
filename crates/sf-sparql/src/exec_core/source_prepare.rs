//! Borrowed source recipe proof; never copy recursive recipes for comparison.

/// After scope shape/alignment validation, check root recipe collisions too:
/// wrapper validation alone does not compare the wrapper's existing bindings.
pub(in crate::exec_core) fn requires_key_overlay(
    branches: &[crate::iq::Branch],
    scopes: &[Option<crate::DedupScope>],
    work: sf_sql::source_work::SourceWork<'_>,
) -> crate::Result<bool> {
    let mut missing = false;
    work.checkpoint()
        .map_err(super::super::sql_error::map_sql_err)?;
    for (branch, scope) in branches.iter().zip(scopes) {
        work.charge(1)
            .map_err(super::super::sql_error::map_sql_err)?;
        let Some(scope) = scope else { continue };
        for (name, expected) in &scope.key_bindings {
            work.charge(1)
                .map_err(super::super::sql_error::map_sql_err)?;
            match super::super::dedup_scope_runtime::find_binding(&branch.bindings, name, work)? {
                Some(actual) if !recipes_same(actual, expected, work)? => {
                    let message =
                        "shared term-dedup key collides with a different branch binding -> 501";
                    work.charge(message.len())
                        .map_err(super::super::sql_error::map_sql_err)?;
                    return Err(crate::Error::Unsupported(message.into()));
                }
                Some(_) => {}
                None => missing = true,
            }
        }
    }
    work.checkpoint()
        .map_err(super::super::sql_error::map_sql_err)?;
    Ok(missing)
}

/// Field-complete recipe equality without Debug strings or recursive equality.
/// The stack borrows recipes and RDF terms; collection cursors avoid width-sized
/// task allocation. No compiler envelope or fixed depth cap is imposed here.
pub(in crate::exec_core) fn recipes_same(
    left: &crate::iq::TermDef,
    right: &crate::iq::TermDef,
    work: sf_sql::source_work::SourceWork<'_>,
) -> crate::Result<bool> {
    compare_recipes(left, right, None, work)
}

/// Compare an outer recipe with the inner recipe's positional remapping without
/// constructing a copied TermDef, template, RDF term, or formatted column name.
pub(in crate::exec_core) fn recipes_remapped_same(
    outer: &crate::iq::TermDef,
    inner: &crate::iq::TermDef,
    projection: &[crate::iq::ColRef],
    alias: usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> crate::Result<bool> {
    validate_remap(inner, projection, work)?;
    compare_recipes(outer, inner, Some((projection, alias)), work)
}

fn validate_remap(
    root: &crate::iq::TermDef,
    projection: &[crate::iq::ColRef],
    work: sf_sql::source_work::SourceWork<'_>,
) -> crate::Result<()> {
    use crate::exec_core::sql_error::map_sql_err as error;
    use crate::iq::{R2rmlGraphScope, TermDef};
    use sf_core::ir::{Segment, TermMap};
    use sf_sql::source_work::SourceVec;
    enum Visit<'a> {
        Def(&'a TermDef),
        Defs(&'a [TermDef]),
        Map(&'a TermMap, usize),
        Segments(&'a [Segment], usize),
        Column(usize, &'a str, u8),
    }
    let mut stack = SourceVec::default();
    stack.push(Visit::Def(root), work).map_err(error)?;
    while let Some(item) = stack.pop() {
        work.charge(1).map_err(error)?;
        match item {
            Visit::Def(def) => match def {
                TermDef::Const(_) => {}
                TermDef::Derived { term_map, alias } => stack
                    .push(Visit::Map(term_map, *alias), work)
                    .map_err(error)?,
                TermDef::R2rmlBlank {
                    term_map,
                    alias,
                    graph,
                } => {
                    if let R2rmlGraphScope::Mapped { term_map, alias } = graph {
                        stack
                            .push(Visit::Map(term_map, *alias), work)
                            .map_err(error)?;
                    }
                    stack
                        .push(Visit::Map(term_map, *alias), work)
                        .map_err(error)?;
                }
                TermDef::Coalesce(a, b) => {
                    stack.push(Visit::Def(b), work).map_err(error)?;
                    stack.push(Visit::Def(a), work).map_err(error)?;
                }
                TermDef::Concat(parts) => stack.push(Visit::Defs(parts), work).map_err(error)?,
                TermDef::ComposedTriple {
                    subject,
                    predicate,
                    object,
                } => {
                    for def in [object, predicate, subject] {
                        stack.push(Visit::Def(def), work).map_err(error)?;
                    }
                }
                TermDef::Agg { col, operand, .. } => {
                    if let Some(c) = operand {
                        stack
                            .push(Visit::Column(c.alias, &c.column, 2), work)
                            .map_err(error)?;
                    }
                    stack
                        .push(Visit::Column(col.alias, &col.column, 2), work)
                        .map_err(error)?;
                }
            },
            Visit::Defs(parts) => {
                if let Some((first, rest)) = parts.split_first() {
                    stack.push(Visit::Defs(rest), work).map_err(error)?;
                    stack.push(Visit::Def(first), work).map_err(error)?;
                }
            }
            Visit::Map(map, alias) => match map {
                TermMap::Constant(_) => {}
                TermMap::Column(name, _) => stack
                    .push(Visit::Column(alias, name, 0), work)
                    .map_err(error)?,
                TermMap::Template(template, _) => {
                    stack
                        .push(Visit::Segments(template.segments(), alias), work)
                        .map_err(error)?;
                }
            },
            Visit::Segments(segments, alias) => {
                if let Some((first, rest)) = segments.split_first() {
                    stack
                        .push(Visit::Segments(rest, alias), work)
                        .map_err(error)?;
                    if let Segment::Column(name) = first {
                        stack
                            .push(Visit::Column(alias, name, 1), work)
                            .map_err(error)?;
                    }
                }
            }
            Visit::Column(alias, name, kind) => {
                let mut found = false;
                for col in projection {
                    work.charge(1).map_err(error)?;
                    if col.alias == alias {
                        work.charge(col.column.len().min(name.len()))
                            .map_err(error)?;
                        if col.column.as_ref() == name {
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    // Prepay the input scan and a proven upper bound for Debug
                    // escaping (each UTF-8 byte expands to at most six bytes),
                    // plus the fixed diagnostic and decimal usize envelope.
                    work.product(name.len(), 7).map_err(error)?;
                    work.charge(160).map_err(error)?;
                    let message=match kind {
                        0=>format!("SubPlan remap: column '{}' on alias {} not in inner projection → 501",name,alias),
                        1=>format!("SubPlan remap: template column '{}' on alias {} not in projection → 501",name,alias),
                        _=>format!("SubPlan remap: ColRef ColRef {{ alias: {alias}, column: {name:?} }} not in inner projection → 501"),
                    };
                    return Err(crate::Error::Unsupported(message));
                }
            }
        }
    }
    work.checkpoint().map_err(error)
}

fn compare_recipes(
    left: &crate::iq::TermDef,
    right: &crate::iq::TermDef,
    remap: Option<(&[crate::iq::ColRef], usize)>,
    work: sf_sql::source_work::SourceWork<'_>,
) -> crate::Result<bool> {
    use crate::exec_core::sql_error::map_sql_err as error;
    use crate::iq::{R2rmlGraphScope, TermDef};
    use sf_core::ir::{Segment, TermMap, TermSpec};
    use sf_core::{NamedOrBlankNode, Term};
    use sf_sql::source_work::{SourceVec, SourceWork};
    fn text(a: &str, b: &str, work: SourceWork<'_>) -> crate::Result<bool> {
        work.charge(1).map_err(error)?;
        work.charge(a.len().min(b.len())).map_err(error)?;
        Ok(a == b)
    }
    fn optional(a: Option<&str>, b: Option<&str>, work: SourceWork<'_>) -> crate::Result<bool> {
        work.charge(1).map_err(error)?;
        match (a, b) {
            (Some(a), Some(b)) => text(a, b, work),
            (None, None) => Ok(true),
            _ => Ok(false),
        }
    }
    fn spec(a: &TermSpec, b: &TermSpec, work: SourceWork<'_>) -> crate::Result<bool> {
        work.charge(1).map_err(error)?;
        Ok(a.term_type == b.term_type
            && optional(
                a.datatype.as_ref().map(|x| x.as_str()),
                b.datatype.as_ref().map(|x| x.as_str()),
                work,
            )?
            && optional(a.language.as_deref(), b.language.as_deref(), work)?
            && optional(a.base.as_deref(), b.base.as_deref(), work)?)
    }
    fn column(
        a: &crate::iq::ColRef,
        b: &crate::iq::ColRef,
        remap: Option<(&[crate::iq::ColRef], usize)>,
        work: SourceWork<'_>,
    ) -> crate::Result<bool> {
        work.charge(1).map_err(error)?;
        Ok(a.alias == remap.map_or(b.alias, |(_, alias)| alias)
            && column_name(&a.column, &b.column, b.alias, remap, work)?)
    }
    fn column_name(
        outer: &str,
        inner: &str,
        inner_alias: usize,
        remap: Option<(&[crate::iq::ColRef], usize)>,
        work: SourceWork<'_>,
    ) -> crate::Result<bool> {
        let Some((projection, _)) = remap else {
            return text(outer, inner, work);
        };
        let mut position = None;
        for (index, col) in projection.iter().enumerate() {
            work.charge(1).map_err(error)?;
            if col.alias == inner_alias && text(&col.column, inner, work)? {
                position = Some(index);
                break;
            }
        }
        let Some(position) = position else {
            return Ok(false);
        };
        work.product(outer.len(), 2).map_err(error)?;
        let Some(digits) = outer.strip_prefix('c') else {
            return Ok(false);
        };
        Ok(!digits.is_empty()
            && (digits.len() == 1 || !digits.starts_with('0'))
            && digits.bytes().all(|b| b.is_ascii_digit())
            && digits.parse::<usize>().ok() == Some(position))
    }
    enum Pair<'a> {
        Def(&'a TermDef, &'a TermDef),
        Defs(&'a [TermDef], &'a [TermDef]),
        Graph(&'a R2rmlGraphScope, &'a R2rmlGraphScope),
        Map(&'a TermMap, &'a TermMap, usize),
        Segments(&'a [Segment], &'a [Segment], usize),
        Term(&'a Term, &'a Term),
    }
    let mut pending = SourceVec::default();
    pending.push(Pair::Def(left, right), work).map_err(error)?;
    while let Some(pair) = pending.pop() {
        work.charge(1).map_err(error)?;
        match pair {
            Pair::Def(a, b) => match (a, b) {
                (TermDef::Const(a), TermDef::Const(b)) => {
                    pending.push(Pair::Term(a, b), work).map_err(error)?
                }
                (
                    TermDef::Derived {
                        term_map: a,
                        alias: x,
                    },
                    TermDef::Derived {
                        term_map: b,
                        alias: y,
                    },
                ) => {
                    if *x != remap.map_or(*y, |(_, alias)| alias) {
                        return Ok(false);
                    }
                    pending.push(Pair::Map(a, b, *y), work).map_err(error)?;
                }
                (
                    TermDef::R2rmlBlank {
                        term_map: a,
                        alias: x,
                        graph: ga,
                    },
                    TermDef::R2rmlBlank {
                        term_map: b,
                        alias: y,
                        graph: gb,
                    },
                ) => {
                    if *x != remap.map_or(*y, |(_, alias)| alias) {
                        return Ok(false);
                    }
                    pending.push(Pair::Graph(ga, gb), work).map_err(error)?;
                    pending.push(Pair::Map(a, b, *y), work).map_err(error)?;
                }
                (TermDef::Coalesce(a, b), TermDef::Coalesce(x, y)) => {
                    pending.push(Pair::Def(b, y), work).map_err(error)?;
                    pending.push(Pair::Def(a, x), work).map_err(error)?;
                }
                (TermDef::Concat(a), TermDef::Concat(b)) => {
                    pending.push(Pair::Defs(a, b), work).map_err(error)?
                }
                (
                    TermDef::Agg {
                        col: a,
                        kind: ak,
                        operand: ao,
                        fixed_type: at,
                    },
                    TermDef::Agg {
                        col: b,
                        kind: bk,
                        operand: bo,
                        fixed_type: bt,
                    },
                ) => {
                    if ak != bk || at != bt || !column(a, b, remap, work)? {
                        return Ok(false);
                    }
                    match (ao, bo) {
                        (Some(a), Some(b)) => {
                            if !column(a, b, remap, work)? {
                                return Ok(false);
                            }
                        }
                        (None, None) => {}
                        _ => return Ok(false),
                    }
                }
                (
                    TermDef::ComposedTriple {
                        subject: a,
                        predicate: b,
                        object: c,
                    },
                    TermDef::ComposedTriple {
                        subject: x,
                        predicate: y,
                        object: z,
                    },
                ) => {
                    pending.push(Pair::Def(c, z), work).map_err(error)?;
                    pending.push(Pair::Def(b, y), work).map_err(error)?;
                    pending.push(Pair::Def(a, x), work).map_err(error)?;
                }
                _ => return Ok(false),
            },
            Pair::Defs(a, b) => {
                if a.len() != b.len() {
                    return Ok(false);
                }
                if let (Some((a, ar)), Some((b, br))) = (a.split_first(), b.split_first()) {
                    pending.push(Pair::Defs(ar, br), work).map_err(error)?;
                    pending.push(Pair::Def(a, b), work).map_err(error)?;
                }
            }
            Pair::Graph(a, b) => match (a, b) {
                (R2rmlGraphScope::Default, R2rmlGraphScope::Default) => {}
                (
                    R2rmlGraphScope::Mapped {
                        term_map: a,
                        alias: x,
                    },
                    R2rmlGraphScope::Mapped {
                        term_map: b,
                        alias: y,
                    },
                ) => {
                    if *x != remap.map_or(*y, |(_, alias)| alias) {
                        return Ok(false);
                    }
                    pending.push(Pair::Map(a, b, *y), work).map_err(error)?;
                }
                _ => return Ok(false),
            },
            Pair::Map(a, b, alias) => match (a, b) {
                (TermMap::Constant(a), TermMap::Constant(b)) => {
                    pending.push(Pair::Term(a, b), work).map_err(error)?
                }
                (TermMap::Column(a, sa), TermMap::Column(b, sb)) => {
                    if !column_name(a, b, alias, remap, work)? || !spec(sa, sb, work)? {
                        return Ok(false);
                    }
                }
                (TermMap::Template(a, sa), TermMap::Template(b, sb)) => {
                    if !spec(sa, sb, work)? {
                        return Ok(false);
                    }
                    pending
                        .push(Pair::Segments(a.segments(), b.segments(), alias), work)
                        .map_err(error)?;
                }
                _ => return Ok(false),
            },
            Pair::Segments(a, b, alias) => {
                if a.len() != b.len() {
                    return Ok(false);
                }
                if let (Some((a, ar)), Some((b, br))) = (a.split_first(), b.split_first()) {
                    match (a, b) {
                        (Segment::Column(a), Segment::Column(b)) => {
                            if !column_name(a, b, alias, remap, work)? {
                                return Ok(false);
                            }
                        }
                        (Segment::Literal(a), Segment::Literal(b)) => {
                            if !text(a, b, work)? {
                                return Ok(false);
                            }
                        }
                        _ => return Ok(false),
                    }
                    pending
                        .push(Pair::Segments(ar, br, alias), work)
                        .map_err(error)?;
                }
            }
            Pair::Term(a, b) => match (a, b) {
                (Term::NamedNode(a), Term::NamedNode(b)) => {
                    if !text(a.as_str(), b.as_str(), work)? {
                        return Ok(false);
                    }
                }
                (Term::BlankNode(a), Term::BlankNode(b)) => {
                    if !text(a.as_str(), b.as_str(), work)? {
                        return Ok(false);
                    }
                }
                (Term::Literal(a), Term::Literal(b)) => {
                    if !text(a.value(), b.value(), work)?
                        || !optional(a.language(), b.language(), work)?
                        || !text(a.datatype().as_str(), b.datatype().as_str(), work)?
                        || a.direction() != b.direction()
                    {
                        return Ok(false);
                    }
                }
                (Term::Triple(a), Term::Triple(b)) => {
                    let same_subject = match (&a.subject, &b.subject) {
                        (NamedOrBlankNode::NamedNode(a), NamedOrBlankNode::NamedNode(b)) => {
                            text(a.as_str(), b.as_str(), work)?
                        }
                        (NamedOrBlankNode::BlankNode(a), NamedOrBlankNode::BlankNode(b)) => {
                            text(a.as_str(), b.as_str(), work)?
                        }
                        _ => false,
                    };
                    if !same_subject || !text(a.predicate.as_str(), b.predicate.as_str(), work)? {
                        return Ok(false);
                    }
                    pending
                        .push(Pair::Term(&a.object, &b.object), work)
                        .map_err(error)?;
                }
                _ => return Ok(false),
            },
        }
    }
    work.checkpoint().map_err(error)?;
    Ok(true)
}

#[cfg(test)]
#[path = "source_prepare_tests.rs"]
mod tests;
