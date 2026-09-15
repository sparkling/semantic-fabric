//! Controlled D1 inventories; native comparison and RDF identity remain separate.
use super::*;
use crate::build::control::BuildWork;
use crate::Result;

#[derive(Clone, Copy)]
pub(super) enum InventoryWork<'a> {
    Compiler(BuildWork<'a>),
    Source(sf_sql::source_work::SourceWork<'a>),
}

fn source_result<T>(result: sf_sql::Result<T>) -> Result<T> {
    result.map_err(|error| match error {
        sf_sql::Error::QueryControl(reason) => crate::Error::QueryControl(reason),
        other => crate::Error::Sql(other.to_string()),
    })
}

impl InventoryWork<'_> {
    pub(super) fn charge(self, units: usize) -> Result<()> {
        match self {
            Self::Compiler(work) => work.charge(units),
            Self::Source(work) => source_result(work.charge(units)),
        }
    }
    pub(super) fn checkpoint(self) -> Result<()> {
        match self {
            Self::Compiler(work) => work.checkpoint(),
            Self::Source(work) => source_result(work.checkpoint()),
        }
    }
    pub(super) fn variable(self, name: &str) -> Result<Box<str>> {
        match self {
            Self::Compiler(work) => work.variable(name),
            Self::Source(work) => {
                source_result(work.charge(name.len()))?;
                Ok(source_result(work.string(name))?.into_boxed_str())
            }
        }
    }
}

enum KeyBuffer {
    Compiler(crate::build::control::BuildVec<crate::iq::LexicalKey>),
    Source(sf_sql::source_work::SourceVec<crate::iq::LexicalKey>),
}
impl KeyBuffer {
    fn new(work: InventoryWork<'_>) -> Self {
        match work {
            InventoryWork::Compiler(_) => {
                Self::Compiler(crate::build::control::BuildVec::new(Vec::new()))
            }
            InventoryWork::Source(_) => Self::Source(Default::default()),
        }
    }
    fn push(&mut self, key: crate::iq::LexicalKey, work: InventoryWork<'_>) -> Result<()> {
        match (self, work) {
            (Self::Compiler(out), InventoryWork::Compiler(work)) => work.push(out, key),
            (Self::Source(out), InventoryWork::Source(work)) => source_result(out.push(key, work)),
            _ => unreachable!("lexical buffer retains its work domain"),
        }
    }
    fn finish(self) -> Vec<crate::iq::LexicalKey> {
        match self {
            Self::Compiler(out) => out.into_inner(),
            Self::Source(out) => out.into_vec(),
        }
    }
}

fn consumer_payload(mode: &Consumer, work: InventoryWork<'_>) -> Result<()> {
    work.charge(1)?;
    match mode {
        Consumer::Lexical(LexicalMode::Iri { base: Some(base) }) => work.charge(base.len())?,
        Consumer::Lexical(LexicalMode::TypedLiteral { datatype }) => {
            work.charge(datatype.as_str().len())?
        }
        _ => (),
    }
    Ok(())
}

fn insert_consumer(
    set: &mut BTreeSet<Consumer>,
    mode: &Consumer,
    work: InventoryWork<'_>,
) -> Result<()> {
    for existing in set.iter() {
        consumer_payload(existing, work)?;
        consumer_payload(mode, work)?;
    }
    work.charge(std::mem::size_of::<Consumer>())?;
    consumer_payload(mode, work)?;
    set.insert(mode.clone());
    work.checkpoint()
}

pub(super) fn record(
    name: &str,
    mode: Option<&Consumer>,
    modes: &mut Modes,
    work: InventoryWork<'_>,
) -> Result<()> {
    use std::collections::btree_map::Entry;
    for key in modes.keys() {
        work.charge(1)?;
        work.charge(key.len().min(name.len()))?;
    }
    work.charge(std::mem::size_of::<(Box<str>, Option<BTreeSet<Consumer>>)>())?;
    match modes.entry(work.variable(name)?) {
        Entry::Occupied(mut entry) => match (entry.get_mut(), mode) {
            (Some(set), Some(mode)) => insert_consumer(set, mode, work)?,
            (value, _) => *value = None,
        },
        Entry::Vacant(entry) => {
            let value = if let Some(mode) = mode {
                let mut set = BTreeSet::new();
                insert_consumer(&mut set, mode, work)?;
                Some(set)
            } else {
                None
            };
            entry.insert(value);
        }
    }
    work.checkpoint()
}

pub(super) fn column_mode(
    spec: &sf_core::ir::TermSpec,
    work: InventoryWork<'_>,
) -> Result<Option<Consumer>> {
    use sf_core::ir::TermType;
    work.charge(1)?;
    let mode = if spec.term_type == TermType::Iri {
        Consumer::Lexical(LexicalMode::Iri {
            base: spec
                .base
                .as_deref()
                .map(|value| work.variable(value))
                .transpose()?,
        })
    } else if spec.term_type == TermType::BlankNode || spec.language.is_some() {
        Consumer::Lexical(LexicalMode::Decoded)
    } else if let Some(datatype) = &spec.datatype {
        work.charge(std::mem::size_of::<sf_core::NamedNode>())?;
        work.charge(datatype.as_str().len())?;
        Consumer::Lexical(LexicalMode::TypedLiteral {
            datatype: datatype.clone(),
        })
    } else {
        Consumer::Natural
    };
    work.checkpoint()?;
    Ok(Some(mode))
}

pub(super) fn term(
    map: &TermMap,
    owner: usize,
    alias: usize,
    modes: &mut Modes,
    work: InventoryWork<'_>,
) -> Result<()> {
    work.charge(1)?;
    if owner != alias {
        return Ok(());
    }
    match map {
        TermMap::Constant(_) => (),
        TermMap::Column(name, spec) => {
            let mode = column_mode(spec, work)?;
            record(name, mode.as_ref(), modes, work)?;
        }
        TermMap::Template(template, spec) => {
            let mode = (spec.term_type == sf_core::ir::TermType::Iri)
                .then_some(Consumer::Lexical(LexicalMode::Decoded));
            for segment in template.segments() {
                work.charge(1)?;
                if let sf_core::ir::Segment::Column(name) = segment {
                    record(name, mode.as_ref(), modes, work)?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn finish(modes: Modes, work: InventoryWork<'_>) -> Result<Vec<crate::iq::LexicalKey>> {
    let mut out = KeyBuffer::new(work);
    for (column, modes) in modes {
        work.charge(1)?;
        let modes = modes.unwrap_or_default();
        let (mut natural, mut iri, mut typed) = (false, false, false);
        for mode in &modes {
            work.charge(1)?;
            natural |= matches!(mode, Consumer::Natural);
            iri |= matches!(mode, Consumer::Lexical(LexicalMode::Iri { .. }));
            typed |= matches!(mode, Consumer::Lexical(LexicalMode::TypedLiteral { .. }));
        }
        let natural_only = modes.len() == 1 && natural;
        for consumer in modes {
            work.charge(1)?;
            let mode = match consumer {
                Consumer::Natural if natural_only || typed => LexicalMode::Natural,
                Consumer::Natural => continue,
                Consumer::Lexical(_) if natural && iri => continue,
                Consumer::Lexical(LexicalMode::Decoded) if natural => {
                    LexicalMode::DecodedWithNatural
                }
                Consumer::Lexical(mode) => mode,
            };
            let name = work.variable(&column)?;
            out.push(crate::iq::LexicalKey { column: name, mode }, work)?;
        }
    }
    Ok(out.finish())
}

fn insert(set: &mut BTreeSet<Box<str>>, name: &str, work: BuildWork<'_>) -> Result<()> {
    for existing in set.iter() {
        work.charge(1)?;
        work.charge(existing.len().min(name.len()))?;
    }
    work.charge(std::mem::size_of::<Box<str>>())?;
    set.insert(work.variable(name)?);
    work.checkpoint()
}

pub(crate) fn condition_columns(
    cond: &SqlCond,
    work: BuildWork<'_>,
    f: &mut impl FnMut(usize, &str) -> Result<()>,
) -> Result<()> {
    let work = work.enter()?;
    match cond {
        SqlCond::ExpressionError => (),
        SqlCond::LiteralCmp(cmp) => {
            work.charge(2)?;
            for col in cmp.columns() {
                f(col.alias, &col.column)?;
            }
        }
        SqlCond::IriCmp(cmp) => {
            for operand in [&cmp.left, &cmp.right] {
                work.charge(1)?;
                if let crate::iq::iri_cmp::IriOperand::Template { parts, .. } = operand {
                    work.charge(parts.len())?;
                }
                for col in operand.columns() {
                    f(col.alias, &col.column)?;
                }
            }
        }
        SqlCond::ColEq(a, b) | SqlCond::NativeColEq(a, b) | SqlCond::NullSafeEq(a, b) => {
            f(a.alias, &a.column)?;
            f(b.alias, &b.column)?;
        }
        SqlCond::Cmp(a, _, _)
        | SqlCond::NativeCmp(a, _, _)
        | SqlCond::IsNotNull(a)
        | SqlCond::DecodedIsNotNull(a)
        | SqlCond::IsNull(a)
        | SqlCond::StrMatch { col: a, .. } => f(a.alias, &a.column)?,
        SqlCond::Not(cond) => condition_columns(cond, work, f)?,
        SqlCond::And(conds) | SqlCond::Or(conds) => {
            for cond in conds {
                condition_columns(cond, work, f)?;
            }
        }
        SqlCond::Exists { .. } | SqlCond::NotExists { .. } | SqlCond::PathExists { .. } => (),
        SqlCond::TemplateEq(left, a, right, b, _) => {
            for (segments, alias) in [(left, a), (right, b)] {
                for segment in segments {
                    work.charge(1)?;
                    if let sf_core::ir::Segment::Column(name) = segment {
                        f(*alias, name)?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn native_keys_with_work(
    branch: &Branch,
    alias: usize,
    work: BuildWork<'_>,
) -> Result<Vec<(Box<str>, bool)>> {
    fn visit(
        cond: &SqlCond,
        alias: usize,
        native: &mut BTreeSet<Box<str>>,
        rdf: &mut BTreeSet<Box<str>>,
        work: BuildWork<'_>,
    ) -> Result<()> {
        let nested = work.enter()?;
        match cond {
            SqlCond::And(conds)
            | SqlCond::Or(conds)
            | SqlCond::Exists { conds, .. }
            | SqlCond::NotExists { conds, .. }
            | SqlCond::PathExists { conds, .. } => {
                for cond in conds {
                    visit(cond, alias, native, rdf, nested)?;
                }
            }
            SqlCond::Not(cond) => visit(cond, alias, native, rdf, nested)?,
            SqlCond::IsNull(_) | SqlCond::IsNotNull(_) | SqlCond::DecodedIsNotNull(_) => (),
            other => {
                let target = if matches!(other, SqlCond::NativeColEq(..) | SqlCond::NativeCmp(..)) {
                    native
                } else {
                    rdf
                };
                // Reuse this node's depth, not an extra synthetic recursive level.
                condition_columns(other, work, &mut |owner, name| {
                    work.charge(1)?;
                    if owner == alias {
                        insert(target, name, work)?;
                    }
                    Ok(())
                })?;
            }
        }
        Ok(())
    }
    work.checkpoint()?;
    let (mut native, mut rdf) = (BTreeSet::new(), BTreeSet::new());
    for definition in branch.bindings.values() {
        for (owner, name) in super::super::resolve_work::columns(definition, work)? {
            work.charge(1)?;
            if owner == alias {
                insert(&mut rdf, name, work)?;
            }
        }
    }
    for cond in branch.where_conds.iter().chain(
        branch
            .opts
            .iter()
            .flat_map(|opt| opt.on.iter().chain(&opt.extra)),
    ) {
        visit(cond, alias, &mut native, &mut rdf, work)?;
    }
    let mut out = work.vector(native.len())?;
    for name in native {
        for key in &rdf {
            work.charge(1)?;
            work.charge(key.len().min(name.len()))?;
        }
        let both = rdf.contains(&name);
        out.push((name, both));
    }
    work.checkpoint()?;
    Ok(out)
}

#[cfg(test)]
#[path = "distinct_scan_work_tests.rs"]
mod tests;
