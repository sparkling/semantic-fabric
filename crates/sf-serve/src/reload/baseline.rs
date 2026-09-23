//! Observation equality detects drift; it never grants backend DDL authority.
use std::collections::BTreeMap;
use std::sync::Mutex;

use super::*;
use sf_core::{SourceId, TableSchema};

#[derive(PartialEq)]
pub(crate) struct Observation {
    tables: Vec<TableSchema>,
    verified: Option<crate::generation::GenerationObservation>,
}

type Observations = BTreeMap<SourceId, Observation>;

pub(crate) struct Baseline {
    inputs: SemanticInputs,
    observations: Observations,
}

impl Baseline {
    pub(crate) fn new(inputs: SemanticInputs, observations: Observations) -> Self {
        Self {
            inputs,
            observations,
        }
    }

    pub(crate) fn record(
        observations: &mut Observations,
        id: SourceId,
        source: &crate::IntrospectedSource,
    ) {
        let mut schema = source.observed_schema().to_vec();
        // Statistics change with ordinary writes and are not schema drift.
        for table in &mut schema {
            table.row_estimate = None;
            for column in &mut table.columns {
                column.distinct_estimate = None;
            }
            table.columns.sort_by(|a, b| a.name.cmp(&b.name));
        }
        schema.sort_by(|a, b| a.name.cmp(&b.name));
        observations.insert(
            id,
            Observation {
                tables: schema,
                verified: source.verified_identity(),
            },
        );
    }
}

#[cfg(test)]
impl Baseline {
    pub(super) fn is_verified(&self, id: SourceId) -> bool {
        self.observations
            .get(&id)
            .is_some_and(|observation| observation.verified.is_some())
    }
}

pub(super) struct Attempt {
    runtime: Arc<RuntimeManager>,
    baseline: Arc<Baseline>,
    expected: Mutex<RuntimeReadiness>,
}

impl Attempt {
    pub(super) fn new(
        runtime: Arc<RuntimeManager>,
        baseline: Arc<Baseline>,
        expected: RuntimeReadiness,
    ) -> Self {
        Self {
            runtime,
            baseline,
            expected: Mutex::new(expected),
        }
    }

    pub(super) fn expected(&self) -> Result<RuntimeReadiness, ServeError> {
        self.expected
            .lock()
            .map(|state| *state)
            .map_err(|_| transition_error())
    }

    pub(super) fn fence(&self, cause: ReadinessCause) -> Result<(), ServeError> {
        let mut expected = self.expected.lock().map_err(|_| transition_error())?;
        *expected = self
            .runtime
            .fence_authored(&ReloadAuthority(()), *expected, cause)
            .map_err(|_| transition_error())?;
        Ok(())
    }

    pub(super) fn capture(&self, opts: &ServeOptions) -> Result<SemanticInputs, ServeError> {
        let inputs = SemanticInputs::capture(opts)?;
        if inputs != self.baseline.inputs {
            // This precedes parsing, source opening and all expensive validation.
            self.fence(ReadinessCause::SchemaDrift)?;
        }
        Ok(inputs)
    }

    pub(super) fn observe(
        &self,
        next: &mut Observations,
        id: SourceId,
        source: &crate::IntrospectedSource,
    ) -> Result<(), ServeError> {
        Baseline::record(next, id, source);
        let previous = self.baseline.observations.get(&id);
        if previous.is_some_and(|prior| prior.verified.is_some())
            && next.get(&id).is_some_and(|now| now.verified.is_none())
        {
            // A verified source never downgrades on reload: refuse the
            // unverified fallback so readiness stays fenced until a verified
            // candidate or restart (G3, user decision 2026-09-23).
            self.fence(ReadinessCause::CapabilityDrift)?;
            return Err(crate::pg_generation::authored::generation_error(
                crate::pg_generation::PgGenerationError::CapabilityDrift,
            ));
        }
        if next.get(&id) != previous {
            // This precedes mapping validation and observation of the next source.
            self.fence(ReadinessCause::SchemaDrift)?;
        }
        Ok(())
    }
}

fn transition_error() -> ServeError {
    ServeError::new(StartupCause::Runtime {
        error: "reload state changed".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::schema_identity::{
        ObservedSchemaIdentityV1, ProfileIdV1, SchemaObservationInputV1, SchemaProfilesV1,
    };

    #[test]
    fn equal_compiler_tables_do_not_hide_changed_verified_facts() {
        let identity = |tag| {
            ObservedSchemaIdentityV1::build(SchemaObservationInputV1 {
                profiles: SchemaProfilesV1 {
                    structural: ProfileIdV1::new(tag).unwrap(),
                    types: ProfileIdV1::new("test-types-v1").unwrap(),
                    constraints: ProfileIdV1::new("test-constraints-v1").unwrap(),
                },
                relations: vec![],
                constraints: vec![],
            })
            .unwrap()
        };
        let first = Observation {
            tables: vec![TableSchema::new("items")],
            verified: Some(crate::generation::GenerationObservation::Postgres(
                identity("test-one-v1"),
            )),
        };
        let mut next = Observation {
            tables: first.tables.clone(),
            verified: first.verified.clone(),
        };
        assert!(first == next);
        next.verified = Some(crate::generation::GenerationObservation::Postgres(
            identity("test-two-v1"),
        ));
        assert!(first != next);
        next.verified = None;
        assert!(
            first != next,
            "an observation must not inherit verified authority"
        );
    }
}
