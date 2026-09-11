//! Prospective OPTIONAL alias/term analysis, without cloned column inventories.
use std::collections::BTreeMap;

use crate::build::control::{BuildVec, BuildWork};
use crate::iq::{Branch, R2rmlGraphScope, TermDef};
use crate::{CompilerWorkMode, Result};
use sf_core::ir::{Segment, TermMap};

#[derive(Clone, Copy)]
struct Alias {
    id: usize,
    nullable: bool,
    subplan: bool,
}

pub(crate) struct Preparation<'a> {
    aliases: Vec<Alias>,
    has_subplan: bool,
    work: BuildWork<'a>,
}

impl<'a> Preparation<'a> {
    pub(crate) fn new(left: &Branch, mode: CompilerWorkMode<'a>) -> Result<Self> {
        // Fixed, payload-free boundary also lets public acceptance observe the
        // actual helper, instead of stopping earlier at the enclosing LOWER.
        tracing::debug_span!("sf.compiler.optional", operation = "preparation").in_scope(|| {
            let work = BuildWork::new(mode);
            work.charge(1)?;
            let mut aliases = BuildVec::new(Vec::new());
            for opt in &left.opts {
                work.charge(1)?;
                work.push(
                    &mut aliases,
                    Alias {
                        id: opt.scan.alias,
                        nullable: true,
                        subplan: false,
                    },
                )?;
            }
            for sp in &left.subplan_joins {
                work.charge(1)?;
                work.push(
                    &mut aliases,
                    Alias {
                        id: sp.alias,
                        nullable: sp.left,
                        subplan: true,
                    },
                )?;
            }
            Ok(Self {
                aliases: aliases.into_inner(),
                has_subplan: !left.subplan_joins.is_empty(),
                work,
            })
        })
    }

    /// Charge comparisons before examining user-controlled variable bytes. A
    /// borrowed ordered scan avoids an opaque/unaccounted BTree search schedule.
    pub(crate) fn lookup<'b>(
        &self,
        bindings: &'b BTreeMap<String, TermDef>,
        name: &str,
    ) -> Result<Option<&'b TermDef>> {
        self.work.charge(1)?;
        for (key, value) in bindings {
            self.work.charge(1)?;
            self.work.charge(key.len().min(name.len()))?;
            match key.as_str().cmp(name) {
                std::cmp::Ordering::Equal => return Ok(Some(value)),
                std::cmp::Ordering::Greater => break,
                std::cmp::Ordering::Less => (),
            }
        }
        Ok(None)
    }

    pub(crate) fn nullable(&self, def: &TermDef) -> Result<bool> {
        self.term(def, false, self.work)
    }

    pub(crate) fn shared_reads_left_subplan(&self, left: &Branch, right: &Branch) -> Result<bool> {
        self.work.checkpoint()?;
        if !self.has_subplan {
            return Ok(false);
        }
        for name in right.bindings.keys() {
            if let Some(def) = self.lookup(&left.bindings, name)? {
                if self.term(def, true, self.work)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn contains(&self, id: usize, subplan: bool, work: BuildWork<'_>) -> Result<bool> {
        for alias in &self.aliases {
            work.charge(1)?;
            if alias.id == id
                && if subplan {
                    alias.subplan
                } else {
                    alias.nullable
                }
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn map(&self, map: &TermMap, alias: usize, subplan: bool, work: BuildWork<'_>) -> Result<bool> {
        work.charge(1)?;
        match map {
            TermMap::Constant(_) => Ok(false),
            TermMap::Column(_, _) => self.contains(alias, subplan, work),
            TermMap::Template(template, _) => {
                for segment in template.segments() {
                    work.charge(1)?;
                    if matches!(segment, Segment::Column(_))
                        && self.contains(alias, subplan, work)?
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    fn term(&self, def: &TermDef, subplan: bool, work: BuildWork<'_>) -> Result<bool> {
        let work = work.enter()?;
        match def {
            TermDef::Const(_) => Ok(false),
            // Preserve the original nullable detector's alias semantics, even
            // for a manually constructed Derived with a constant term map.
            TermDef::Derived { alias, term_map } => {
                if subplan {
                    self.map(term_map, *alias, subplan, work)
                } else {
                    self.contains(*alias, subplan, work)
                }
            }
            TermDef::R2rmlBlank {
                term_map,
                alias,
                graph,
            } => {
                if self.map(term_map, *alias, subplan, work)? {
                    return Ok(true);
                }
                work.charge(1)?;
                match graph {
                    R2rmlGraphScope::Default => Ok(false),
                    R2rmlGraphScope::Mapped { term_map, alias } => {
                        self.map(term_map, *alias, subplan, work)
                    }
                }
            }
            TermDef::Coalesce(left, right) => {
                Ok(self.term(left, subplan, work)? || self.term(right, subplan, work)?)
            }
            TermDef::Concat(parts) => {
                for part in parts {
                    if self.term(part, subplan, work)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            TermDef::Agg { col, .. } => {
                if subplan {
                    self.contains(col.alias, subplan, work)
                } else {
                    Ok(false)
                }
            }
            TermDef::ComposedTriple {
                subject,
                predicate,
                object,
            } => Ok(self.term(subject, subplan, work)?
                || self.term(predicate, subplan, work)?
                || self.term(object, subplan, work)?),
        }
    }
}
