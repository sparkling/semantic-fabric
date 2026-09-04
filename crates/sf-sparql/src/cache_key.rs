//! Fallible, byte-bounded canonical rendering for a future controlled cache path.

use std::fmt;

use spargebra::Query;

use super::{CompileProfileId, CompileScope, PlanKey};

/// A redacted failure from bounded canonical cache-key construction.
///
/// The variants deliberately carry no query text or partially rendered key.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum BoundedCacheKeyError {
    #[error("canonical cache key exceeds its {maximum_bytes}-byte ceiling")]
    LimitExceeded { maximum_bytes: usize },
    #[error("canonical cache key size accounting overflowed")]
    AccountingOverflow,
    #[error("canonical cache key allocation failed")]
    AllocationFailed,
    #[error("canonical cache key formatting failed")]
    FormattingFailed,
}

/// Construct the same collision-safe key as the uncontrolled path without
/// allowing its canonical rendering to exceed `maximum_bytes`.
///
/// The ceiling is an explicit input because the governed V1 value is not yet
/// calibrated. This primitive neither selects policy nor grants governed cache
/// authority.
pub(crate) fn plan_key_for_profile(
    query: &Query,
    scope: CompileScope,
    profile: CompileProfileId,
    maximum_bytes: usize,
) -> Result<PlanKey, BoundedCacheKeyError> {
    let canonical = render_bounded(query, maximum_bytes)?;
    Ok(PlanKey::from_canonical(scope, profile, canonical))
}

fn render_bounded<T>(value: &T, maximum_bytes: usize) -> Result<String, BoundedCacheKeyError>
where
    T: fmt::Display + ?Sized,
{
    render_bounded_with(value, maximum_bytes, |output, additional| {
        output
            .try_reserve_exact(additional)
            .map_err(|_allocation_error| ())
    })
}

fn render_bounded_with<T, R>(
    value: &T,
    maximum_bytes: usize,
    reserve: R,
) -> Result<String, BoundedCacheKeyError>
where
    T: fmt::Display + ?Sized,
    R: FnMut(&mut String, usize) -> Result<(), ()>,
{
    let mut writer = BoundedWriter::new(maximum_bytes, reserve);
    let format_result = fmt::write(&mut writer, format_args!("{value}"));
    writer.finish(format_result)
}

struct BoundedWriter<R> {
    output: String,
    maximum_bytes: usize,
    failure: Option<BoundedCacheKeyError>,
    reserve: R,
}

impl<R> BoundedWriter<R> {
    const MIN_GROWTH_BYTES: usize = 64;

    fn new(maximum_bytes: usize, reserve: R) -> Self {
        Self {
            output: String::new(),
            maximum_bytes,
            failure: None,
            reserve,
        }
    }

    fn finish(self, format_result: fmt::Result) -> Result<String, BoundedCacheKeyError> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        match format_result {
            Ok(()) => Ok(self.output),
            Err(_) => Err(BoundedCacheKeyError::FormattingFailed),
        }
    }

    fn fail(&mut self, failure: BoundedCacheKeyError) -> fmt::Result {
        self.failure = Some(failure);
        Err(fmt::Error)
    }

    fn reserve_for(&mut self, required_bytes: usize) -> fmt::Result
    where
        R: FnMut(&mut String, usize) -> Result<(), ()>,
    {
        if required_bytes <= self.output.capacity() {
            return Ok(());
        }

        // Grow geometrically, but never request retained capacity beyond the
        // caller's content ceiling. Calling `try_reserve_exact(fragment.len())`
        // for every formatter fragment can otherwise turn a one-byte-at-a-time
        // `Display` implementation into linearly many reallocations.
        let doubled = self
            .output
            .capacity()
            .checked_mul(2)
            .unwrap_or(self.maximum_bytes);
        let target_capacity = required_bytes
            .max(doubled)
            .max(Self::MIN_GROWTH_BYTES.min(self.maximum_bytes))
            .min(self.maximum_bytes);
        let Some(additional) = target_capacity.checked_sub(self.output.len()) else {
            return self.fail(BoundedCacheKeyError::AccountingOverflow);
        };
        if (self.reserve)(&mut self.output, additional).is_err() {
            return self.fail(BoundedCacheKeyError::AllocationFailed);
        }
        Ok(())
    }
}

impl<R> fmt::Write for BoundedWriter<R>
where
    R: FnMut(&mut String, usize) -> Result<(), ()>,
{
    fn write_str(&mut self, fragment: &str) -> fmt::Result {
        if self.failure.is_some() {
            return Err(fmt::Error);
        }

        let required_bytes =
            match checked_required_bytes(self.output.len(), fragment.len(), self.maximum_bytes) {
                Ok(required_bytes) => required_bytes,
                Err(error) => return self.fail(error),
            };
        self.reserve_for(required_bytes)?;
        self.output.push_str(fragment);
        Ok(())
    }
}

fn checked_required_bytes(
    current_bytes: usize,
    additional_bytes: usize,
    maximum_bytes: usize,
) -> Result<usize, BoundedCacheKeyError> {
    let required_bytes = current_bytes
        .checked_add(additional_bytes)
        .ok_or(BoundedCacheKeyError::AccountingOverflow)?;
    if required_bytes > maximum_bytes {
        return Err(BoundedCacheKeyError::LimitExceeded { maximum_bytes });
    }
    Ok(required_bytes)
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use spargebra::SparqlParser;

    use super::*;
    use crate::cache::{
        plan_key_for_profile as unbounded_plan_key, CompileBindingId, Epoch, PlanCache,
    };
    use crate::{ColumnTypeAuthority, ConstraintAuthority};
    use sf_sql::Dialect;

    fn parse(source: &str) -> Query {
        SparqlParser::new().parse_query(source).unwrap()
    }

    fn scope() -> CompileScope {
        CompileScope::new(
            CompileBindingId::mint(),
            Dialect::Sqlite,
            Epoch(0),
            ConstraintAuthority::Unverified,
            ColumnTypeAuthority::Unverified,
        )
    }

    #[test]
    fn matches_current_canonical_keys_across_query_forms_and_profiles() {
        let queries = [
            "PREFIX ex: <http://example.test/> SELECT DISTINCT ?s ?o WHERE { \
             ?s ex:p ?o OPTIONAL { ?s ex:q ?q } FILTER(?o != ?q) } ORDER BY ?s LIMIT 5",
            "ASK FROM <http://example.test/graph> WHERE { ?s ?p \"café 東京\" }",
            "CONSTRUCT { ?s <http://example.test/p> ?o } \
             WHERE { ?s <http://example.test/p> ?o }",
            "DESCRIBE ?s WHERE { ?s a <http://example.test/Thing> }",
        ];

        for source in queries {
            let query = parse(source);
            let canonical = query.to_string();
            let scope = scope();
            for profile in [CompileProfileId::Uncontrolled, CompileProfileId::GovernedV1] {
                let expected = unbounded_plan_key(&query, scope, profile);
                let actual = plan_key_for_profile(&query, scope, profile, canonical.len()).unwrap();
                assert_eq!(actual, expected, "canonical key drift for {source}");
                assert_eq!(actual.canonical.as_bytes(), canonical.as_bytes());
            }
        }
    }

    #[test]
    fn accepts_exact_utf8_byte_ceiling_and_rejects_n_plus_one() {
        let query = parse("SELECT ?s WHERE { ?s <http://example.test/label> \"café 東京\" }");
        let canonical = query.to_string();
        assert!(canonical.len() > canonical.chars().count());
        let exact = canonical.len();

        let key =
            plan_key_for_profile(&query, scope(), CompileProfileId::GovernedV1, exact).unwrap();
        assert_eq!(key.canonical.as_bytes(), canonical.as_bytes());

        assert_eq!(
            plan_key_for_profile(&query, scope(), CompileProfileId::GovernedV1, exact - 1,)
                .unwrap_err(),
            BoundedCacheKeyError::LimitExceeded {
                maximum_bytes: exact - 1,
            }
        );
    }

    #[test]
    fn distinguishes_allocation_and_formatter_failures_without_content() {
        let allocation =
            render_bounded_with(&"private-query", usize::MAX, |_, _| Err(())).unwrap_err();
        assert_eq!(allocation, BoundedCacheKeyError::AllocationFailed);
        assert_eq!(
            allocation.to_string(),
            "canonical cache key allocation failed"
        );

        struct BrokenDisplay;
        impl fmt::Display for BrokenDisplay {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                Err(fmt::Error)
            }
        }

        let formatting = render_bounded(&BrokenDisplay, 64).unwrap_err();
        assert_eq!(formatting, BoundedCacheKeyError::FormattingFailed);
        assert_eq!(
            formatting.to_string(),
            "canonical cache key formatting failed"
        );
    }

    #[test]
    fn distinguishes_accounting_overflow_from_the_content_limit() {
        assert_eq!(
            checked_required_bytes(1, usize::MAX, usize::MAX),
            Err(BoundedCacheKeyError::AccountingOverflow)
        );
        assert_eq!(
            checked_required_bytes(2, 3, 4),
            Err(BoundedCacheKeyError::LimitExceeded { maximum_bytes: 4 })
        );
        assert_eq!(
            BoundedCacheKeyError::AccountingOverflow.to_string(),
            "canonical cache key size accounting overflowed"
        );
    }

    #[test]
    fn tiny_formatter_fragments_use_geometric_fallible_growth() {
        use std::cell::Cell;

        struct TinyFragments(usize);
        impl fmt::Display for TinyFragments {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                for _ in 0..self.0 {
                    formatter.write_str("x")?;
                }
                Ok(())
            }
        }

        let reservations = Cell::new(0_usize);
        let rendered = render_bounded_with(&TinyFragments(1024), 1024, |output, additional| {
            reservations.set(reservations.get() + 1);
            output.try_reserve_exact(additional).map_err(|_| ())
        })
        .unwrap();

        assert_eq!(rendered.len(), 1024);
        assert!(
            reservations.get() <= 6,
            "geometric growth made {} reservations",
            reservations.get()
        );
    }

    #[test]
    fn failure_yields_no_partial_key_or_cache_mutation() {
        let query = parse("SELECT * WHERE { ?s ?p ?o }");
        let canonical_bytes = query.to_string().len();
        let scope = scope();
        let cache = PlanCache::new(2);
        let baseline =
            plan_key_for_profile(&query, scope, CompileProfileId::GovernedV1, canonical_bytes)
                .unwrap();
        cache.put(baseline.clone(), 7_u8);

        let failure = plan_key_for_profile(
            &query,
            scope,
            CompileProfileId::GovernedV1,
            canonical_bytes - 1,
        );
        assert!(matches!(
            failure,
            Err(BoundedCacheKeyError::LimitExceeded { .. })
        ));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&baseline), Some(7));
    }

    #[test]
    fn canonical_content_disambiguates_a_forced_hash_collision() {
        let scope = scope();
        let a_query = parse("SELECT ?x WHERE { ?x <http://example.test/a> ?y }");
        let b_query = parse("SELECT ?x WHERE { ?x <http://example.test/b> ?y }");
        let mut a = plan_key_for_profile(
            &a_query,
            scope,
            CompileProfileId::GovernedV1,
            a_query.to_string().len(),
        )
        .unwrap();
        let mut b = plan_key_for_profile(
            &b_query,
            scope,
            CompileProfileId::GovernedV1,
            b_query.to_string().len(),
        )
        .unwrap();
        a.structural_hash = 42;
        b.structural_hash = 42;
        assert_ne!(a.canonical, b.canonical);
        assert_ne!(a, b);

        let cache = PlanCache::new(2);
        cache.put(a.clone(), 1_u8);
        assert_eq!(cache.get(&a), Some(1));
        assert_eq!(cache.get(&b), None);
    }
}
