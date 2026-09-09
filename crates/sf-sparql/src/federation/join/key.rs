//! Mapping-defined inversion; no schema observation authorizes a type shortcut.
use crate::iq::{ColRef, TermDef};
use crate::Result;
use sf_core::ir::{Segment, TermMap, TermType};
use sf_core::Term;

pub(super) fn column(def: &TermDef) -> Option<ColRef> {
    let TermDef::Derived { term_map, alias } = def else {
        return None;
    };
    match term_map {
        TermMap::Column(column, spec)
            if spec.term_type == TermType::Literal
                && (spec.datatype.is_some() || spec.language.is_some()) =>
        {
            Some(ColRef::new(*alias, column.clone()))
        }
        TermMap::Template(template, spec)
            if spec.term_type == TermType::Iri && spec.base.is_none() =>
        {
            let columns: Vec<_> = template
                .segments()
                .iter()
                .filter_map(|s| match s {
                    Segment::Column(c) => Some(c),
                    _ => None,
                })
                .collect();
            (columns.len() == 1).then(|| ColRef::new(*alias, columns[0].clone()))
        }
        _ => None,
    }
}
pub(super) fn supported(def: &TermDef) -> bool {
    column(def).is_some()
}

pub(super) fn inverse(def: &TermDef, term: &Term) -> Result<Option<String>> {
    let TermDef::Derived { term_map, .. } = def else {
        return Ok(None);
    };
    let Some(col) = column(def) else {
        return Ok(None);
    };
    let value = match (term_map, term) {
        (TermMap::Column(..), Term::Literal(lit)) => lit.value().to_owned(),
        (TermMap::Template(template, _), Term::NamedNode(node)) => {
            let mut prefix = String::new();
            let mut suffix = String::new();
            let mut after = false;
            for part in template.segments() {
                match part {
                    Segment::Column(_) => after = true,
                    Segment::Literal(text) if after => suffix.push_str(text),
                    Segment::Literal(text) => prefix.push_str(text),
                }
            }
            let Some(encoded) = node
                .as_str()
                .strip_prefix(&prefix)
                .and_then(|s| s.strip_suffix(&suffix))
            else {
                return Ok(None);
            };
            let Some(decoded) = decode(encoded) else {
                return Ok(None);
            };
            decoded
        }
        _ => return Ok(None),
    };
    let row = [(col.column.as_ref(), Some(value.as_str()))];
    // Re-expansion rejects noncanonical escapes and a mismatched RDF literal
    // datatype/language/direction; SQL is only a conservative prefilter.
    Ok(sf_core::term::generate(term_map, row.as_slice())
        .ok()
        .flatten()
        .is_some_and(|candidate| super::same_term(&candidate, term))
        .then_some(value))
}

fn decode(value: &str) -> Option<String> {
    let mut out = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            out.push((high * 16 + low) as u8);
        } else {
            out.push(byte);
        }
    }
    String::from_utf8(out).ok()
}
