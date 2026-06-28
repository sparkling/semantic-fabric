//! `sf-core` — the shared core of semantic-fabric (ADR-0003): the R2RML mapping
//! intermediate representation (IR), RDF term generation, and the R2RML §10
//! natural SQL→XSD datatype mapping. The virtualiser (`sf-sparql`) consumes these
//! types; nothing re-parses mappings (ADR-0003 R1). RDF terms are `oxrdf` types
//! (RDF 1.2) end to end (ADR-0003 R2). The datatype mapping lives here exactly
//! once (ADR-0003 R3, ADR-0015). **No I/O.**
//!
//! Scope is R2RML-only: the IR models exactly what R2RML needs — no RML-Core /
//! reference-formulation / heterogeneous-source generality (ADR-0002).
//!
//! ## Allocation discipline (ADR-0006)
//!
//! Term generation runs once per result row, so it is written to avoid per-row
//! heap traffic:
//!
//! * Constant predicate / `rdf:type` / datatype IRIs are pre-built once into the
//!   IR (`oxrdf::NamedNode`) and emitted **by reference** (`NamedNodeRef`,
//!   zero-copy) via [`term::generate_into`].
//! * `rr:template` is pre-compiled to a [`ir::Segment`] list (no per-row
//!   placeholder scan); template IRIs use `new_unchecked` (the template already
//!   fixes the form — per-row RFC-3987 re-validation is waste).
//! * Derived lexical forms are written through a caller-owned scratch `String`
//!   buffer rather than returned as an owned `Term` per row.
//! * Mapping-IR symbols are interned (with `lasso`) only at **parse** time, in
//!   `sf-mapping`; term generation **never** interns or allocates per-row data
//!   values — row values are borrowed straight through.

/// RDF term types come from Oxigraph's `oxrdf` (ADR-0003 R2, ADR-0004).
pub use oxrdf::{
    vocab, BlankNode, BlankNodeRef, GraphName, GraphNameRef, Literal, LiteralRef, NamedNode,
    NamedNodeRef, NamedOrBlankNode, Quad, Term, TermRef, Triple,
};

pub mod datatype;
pub mod ir;
pub mod term;

/// Errors raised by the shared core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A malformed mapping document / IR (e.g. an ill-formed `rr:template`).
    #[error("mapping error: {0}")]
    Mapping(String),
    /// Term generation failed (e.g. a constant carrying an unsupported term).
    #[error("term generation error: {0}")]
    Term(String),
    /// A value could not be cast to its target XSD datatype (R2RML §10).
    #[error("datatype error: {0}")]
    Datatype(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A single source row, addressed by column name (R2RML term maps reference
/// columns by name, not position).
///
/// `value` returns the column's value, or `None` when the column is SQL `NULL`
/// **or** absent — both mean "no value", and a referenced column with no value
/// yields no RDF term (R2RML §11; enforced in [`term`], not via SQL).
///
/// The returned `&str` borrows from the row, so term generation never copies a
/// per-row value. Implementors (e.g. `sf-sql`) resolve names to result-set
/// columns once and make `value` an O(1) lookup; the slice impl below is the
/// simple in-memory form used by tests.
pub trait Row {
    /// The value of `column`, or `None` if it is SQL `NULL` or not present.
    fn value(&self, column: &str) -> Option<&str>;
}

/// An ordered `(name, value)` row. `None` = SQL `NULL`.
impl Row for [(&str, Option<&str>)] {
    fn value(&self, column: &str) -> Option<&str> {
        self.iter()
            .find(|(name, _)| *name == column)
            .and_then(|(_, value)| *value)
    }
}

#[cfg(test)]
mod alloc_probe {
    //! A thread-local counting global allocator, active only under `cfg(test)`,
    //! used to prove the CONSTRUCT term-gen path does not allocate per row
    //! (ADR-0006 *Confirmation*). Thread-local + const-initialised so it never
    //! itself allocates and is immune to other tests running in parallel.
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static ALLOCS: Cell<u64> = const { Cell::new(0) };
        static ARMED: Cell<bool> = const { Cell::new(false) };
    }

    pub struct Counting;

    // SAFETY: every method forwards to the System allocator unchanged; the only
    // added work is a non-allocating thread-local counter bump.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            bump();
            System.alloc(layout)
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout);
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            bump();
            System.realloc(ptr, layout, new_size)
        }
    }

    fn bump() {
        if ARMED.with(|a| a.get()) {
            ALLOCS.with(|a| a.set(a.get() + 1));
        }
    }

    /// Reset the counter and begin counting allocations on this thread.
    pub fn arm() {
        ALLOCS.with(|a| a.set(0));
        ARMED.with(|a| a.set(true));
    }

    /// Stop counting; returns the number of (re)allocations since [`arm`].
    pub fn disarm() -> u64 {
        ARMED.with(|a| a.set(false));
        ALLOCS.with(|a| a.get())
    }
}

#[cfg(test)]
#[global_allocator]
static GLOBAL: alloc_probe::Counting = alloc_probe::Counting;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Template, TermMap, TermSpec};

    /// Guard against a vacuous zero-alloc result: prove the probe actually
    /// observes a real heap allocation, so the assertion below has teeth.
    #[test]
    fn alloc_probe_actually_counts() {
        alloc_probe::arm();
        let v = std::hint::black_box(vec![0u8; 1024]);
        let observed = alloc_probe::disarm();
        drop(v);
        assert!(
            observed >= 1,
            "counting allocator must observe a real allocation (got {observed})"
        );
    }

    #[test]
    fn row_distinguishes_null_from_value() {
        let row: &[(&str, Option<&str>)] = &[("id", Some("42")), ("name", None)];
        assert_eq!(row.value("id"), Some("42"));
        assert_eq!(row.value("name"), None); // SQL NULL
        assert_eq!(row.value("missing"), None); // absent
    }

    /// ADR-0006 *Confirmation*: "an allocation-count test over a fixed result
    /// size shows no per-row owned `Term` on the CONSTRUCT path." We generate a
    /// full `(s, p, o)` triple per row — subject from a template IRI (written
    /// through a reused buffer), predicate a by-reference constant IRI, object a
    /// column literal borrowed straight from the row — and assert zero
    /// allocations once the buffers reach steady state.
    #[test]
    fn construct_path_does_not_allocate_per_row() {
        let subject = TermMap::Template(
            Template::parse("http://ex.org/emp/{id}").unwrap(),
            TermSpec::iri(),
        );
        let predicate = TermMap::Constant(Term::NamedNode(NamedNode::new_unchecked(
            "http://ex.org/name",
        )));
        let object = TermMap::Column("name".into(), TermSpec::plain_literal());

        let rows: [&[(&str, Option<&str>)]; 2] = [
            &[("id", Some("1")), ("name", Some("Ada"))],
            &[("id", Some("2")), ("name", Some("Grace"))],
        ];

        let mut bs = String::with_capacity(128);
        let mut bp = String::with_capacity(128);
        let mut bo = String::with_capacity(128);

        // Warm up: let every buffer grow to its steady-state capacity first.
        for _ in 0..16 {
            for row in rows {
                let s = term::generate_into(&subject, row, &mut bs).unwrap().unwrap();
                let p = term::generate_into(&predicate, row, &mut bp).unwrap().unwrap();
                let o = term::generate_into(&object, row, &mut bo).unwrap().unwrap();
                std::hint::black_box((s, p, o));
                bs.clear();
                bp.clear();
                bo.clear();
            }
        }

        alloc_probe::arm();
        for _ in 0..1000 {
            for row in rows {
                let s = term::generate_into(&subject, row, &mut bs).unwrap().unwrap();
                let p = term::generate_into(&predicate, row, &mut bp).unwrap().unwrap();
                let o = term::generate_into(&object, row, &mut bo).unwrap().unwrap();
                std::hint::black_box((s, p, o));
                bs.clear();
                bp.clear();
                bo.clear();
            }
        }
        let allocations = alloc_probe::disarm();

        assert_eq!(
            allocations, 0,
            "CONSTRUCT term-gen emitted {allocations} per-row allocations; the \
             constant/template/column path must be alloc-free after warmup"
        );
    }

    /// The convenience owned path ([`term::generate`]) does of course allocate;
    /// the discipline is about the write-through path above.
    #[test]
    fn owned_path_is_available_for_select() {
        let object = TermMap::Column("name".into(), TermSpec::plain_literal());
        let row: &[(&str, Option<&str>)] = &[("name", Some("Ada"))];
        let term = term::generate(&object, row).unwrap().unwrap();
        assert_eq!(term, Term::Literal(Literal::new_simple_literal("Ada")));
    }
}
