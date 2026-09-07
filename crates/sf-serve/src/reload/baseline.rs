//! Observation equality detects drift; it never grants backend DDL authority.
use std::collections::BTreeMap;
use std::sync::Mutex;

use super::*;
use sf_core::{SourceId, TableSchema};

type Observations = BTreeMap<SourceId, Vec<TableSchema>>;

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
        observations.insert(id, schema);
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
        if next.get(&id) != self.baseline.observations.get(&id) {
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
