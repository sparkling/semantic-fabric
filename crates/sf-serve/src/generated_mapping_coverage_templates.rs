//! Charged, borrowed matching of one IRI text against a static IRI template.

use sf_core::ir::encoding::UCSCHAR_RANGES;
use sf_core::ir::{Segment, Template, TermSpec, TermType};
use sf_core::query_control::QueryControlError;

use super::terms::Scan;

/// Recipe-level possibility only; Match never implies a source row exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Fit {
    Match,
    Disjoint,
    Unknown,
}

type Outcome<T> = Result<T, QueryControlError>;

#[derive(Default)]
struct Shape {
    slots: usize,
    first: usize,
    last: usize,
    total: usize,
    lead: usize,
    trail: usize,
}

impl Shape {
    fn add(&mut self, index: usize, segment: &Segment) {
        match segment {
            Segment::Literal(text) => {
                self.total = self.total.saturating_add(text.len());
                if self.slots == 0 {
                    self.lead = self.lead.saturating_add(text.len());
                } else {
                    self.trail = self.trail.saturating_add(text.len());
                }
            }
            Segment::Column(_) => {
                if self.slots == 0 {
                    self.first = index;
                }
                self.slots += 1;
                self.last = index;
                self.trail = 0;
            }
        }
    }
}

pub(super) fn fit(
    scan: &Scan<'_>,
    template: &Template,
    spec: &TermSpec,
    want: &str,
) -> Outcome<Fit> {
    scan.charge(1)?;
    if spec.term_type != TermType::Iri {
        return Ok(Fit::Disjoint);
    }
    if spec.base.is_some() {
        return Ok(Fit::Unknown);
    }
    let segments = template.segments();
    let mut shape = Shape::default();
    for (index, segment) in segments.iter().enumerate() {
        scan.charge(1)?;
        shape.add(index, segment);
    }
    let length = want.len();
    let impossible = if shape.slots == 0 {
        shape.total != length
    } else {
        shape.total > length
    };
    if impossible {
        return Ok(Fit::Disjoint);
    }
    if shape.slots == 0 {
        if literals_at(scan, segments, want, 0)? {
            return Ok(Fit::Match);
        }
        return Ok(Fit::Disjoint);
    }
    let tail_start = length - shape.trail;
    let head = &segments[..shape.first];
    let tail = &segments[shape.last + 1..];
    if !literals_at(scan, head, want, 0)? || !literals_at(scan, tail, want, tail_start)? {
        return Ok(Fit::Disjoint);
    }
    if shape.slots > 1 {
        return Ok(Fit::Unknown);
    }
    let Some(middle) = want.get(shape.lead..tail_start) else {
        return Ok(Fit::Unknown);
    };
    scan.charge(1 + middle.len())?;
    if canonical(middle) {
        return Ok(Fit::Match);
    }
    Ok(Fit::Unknown)
}

fn literals_at(scan: &Scan<'_>, segments: &[Segment], want: &str, start: usize) -> Outcome<bool> {
    let bytes = want.as_bytes();
    let mut at = start;
    for segment in segments {
        let Segment::Literal(text) = segment else {
            continue;
        };
        scan.charge(1 + text.len())?;
        let end = at.saturating_add(text.len());
        if bytes.get(at..end) != Some(text.as_bytes()) {
            return Ok(false);
        }
        at = end;
    }
    Ok(true)
}

/// Mirrors the `percent_encode_iri` pass-through set; a test compares every scalar.
pub(super) fn passes_through(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(ch, '-' | '.' | '_' | '~')
        || UCSCHAR_RANGES
            .iter()
            .any(|&(low, high)| (low..=high).contains(&(ch as u32)))
}

/// True only for text the encoder can emit for some value.
fn canonical(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let Some((ch, used)) = escaped(&bytes[at..]) else {
                return false;
            };
            if passes_through(ch) {
                return false;
            }
            at += used;
        } else {
            let Some(ch) = text.get(at..).and_then(|rest| rest.chars().next()) else {
                return false;
            };
            if !passes_through(ch) {
                return false;
            }
            at += ch.len_utf8();
        }
    }
    true
}

fn escaped(bytes: &[u8]) -> Option<(char, usize)> {
    let width = match escaped_byte(bytes)? {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    let mut buf = [0_u8; 4];
    for (slot, out) in buf.iter_mut().enumerate().take(width) {
        *out = escaped_byte(bytes.get(slot * 3..)?)?;
    }
    let text = std::str::from_utf8(&buf[..width]).ok()?;
    Some((text.chars().next()?, width * 3))
}

fn escaped_byte(bytes: &[u8]) -> Option<u8> {
    let [b'%', high, low, ..] = bytes else {
        return None;
    };
    Some((nibble(*high)? << 4) | nibble(*low)?)
}

const fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
