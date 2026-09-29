//! Borrowed execution bindings and scalar branch modifiers.
use super::*;

/// Borrowed, ordered execution bindings; shared names retain the base recipe.
pub(crate) enum BindingView<'a> {
    Direct(&'a std::collections::BTreeMap<String, TermDef>),
    Merged(Vec<(&'a str, &'a TermDef)>),
}

impl<'a> BindingView<'a> {
    /// Caller has proved equality for shared names. Preserve base ownership and
    /// BTreeMap order while admitting only borrowed pointer-vector storage.
    pub(crate) fn merged(
        base: &'a std::collections::BTreeMap<String, TermDef>,
        overlay: Option<&'a std::collections::BTreeMap<String, TermDef>>,
        work: sf_sql::source_work::SourceWork<'_>,
    ) -> Result<Self> {
        use source_control::validation_error as error;
        work.checkpoint().map_err(error)?;
        let Some(overlay) = overlay.filter(|map| !map.is_empty()) else {
            return Ok(Self::Direct(base));
        };
        let (mut left, mut right) = (base.iter().peekable(), overlay.iter().peekable());
        let mut entries = sf_sql::source_work::SourceVec::default();
        while left.peek().is_some() || right.peek().is_some() {
            work.charge(1).map_err(error)?;
            let ordering = match (left.peek(), right.peek()) {
                (Some((a, _)), Some((b, _))) => {
                    work.charge(a.len().min(b.len())).map_err(error)?;
                    a.cmp(b)
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Greater,
            };
            let (name, definition) = match ordering {
                std::cmp::Ordering::Less => left.next().expect("left entry"),
                std::cmp::Ordering::Greater => right.next().expect("right entry"),
                std::cmp::Ordering::Equal => {
                    right.next();
                    left.next().expect("equal base entry")
                }
            };
            entries
                .push((name.as_str(), definition), work)
                .map_err(error)?;
        }
        work.checkpoint().map_err(error)?;
        Ok(Self::Merged(entries.into_vec()))
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Direct(map) => map.len(),
            Self::Merged(entries) => entries.len(),
        }
    }

    pub(super) fn get(
        &self,
        name: &str,
        work: sf_sql::source_work::SourceWork<'_>,
    ) -> Result<Option<&'a TermDef>> {
        for (candidate, definition) in self.iter() {
            work.charge(1).map_err(source_control::validation_error)?;
            work.charge(candidate.len().min(name.len()))
                .map_err(source_control::validation_error)?;
            match candidate.cmp(name) {
                std::cmp::Ordering::Equal => return Ok(Some(definition)),
                std::cmp::Ordering::Greater => break,
                std::cmp::Ordering::Less => {}
            }
        }
        work.checkpoint()
            .map_err(source_control::validation_error)?;
        Ok(None)
    }

    pub(crate) fn iter(&self) -> BindingIter<'_, 'a> {
        match self {
            Self::Direct(map) => BindingIter::Direct(map.iter()),
            Self::Merged(entries) => BindingIter::Merged(entries.iter()),
        }
    }
}

pub(crate) enum BindingIter<'view, 'a> {
    Direct(std::collections::btree_map::Iter<'a, String, TermDef>),
    Merged(std::slice::Iter<'view, (&'a str, &'a TermDef)>),
}

impl<'a> Iterator for BindingIter<'_, 'a> {
    type Item = (&'a str, &'a TermDef);
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Direct(iter) => iter
                .next()
                .map(|(name, definition)| (name.as_str(), definition)),
            Self::Merged(iter) => iter.next().copied(),
        }
    }
}

/// Scalar execution overlay; preparing a root never copies its branch forest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BranchModifiers {
    pub(crate) distinct: bool,
    pub(super) limit: Option<usize>,
    pub(super) offset: usize,
}

impl BranchModifiers {
    pub(crate) fn stored(b: &Branch) -> Self {
        Self {
            distinct: b.distinct,
            limit: b.limit,
            offset: b.offset,
        }
    }

    pub(crate) fn prepared(
        b: &Branch,
        single: bool,
        distinct: bool,
        unordered: bool,
        limit: Option<usize>,
        offset: usize,
    ) -> Self {
        let mut m = Self::stored(b);
        if single {
            m.distinct = distinct;
            if unordered {
                m.limit = limit;
                m.offset = offset;
            }
        }
        m
    }
}
