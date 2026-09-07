//! Feature-gated evidence recorded by the production PostgreSQL observer.

use super::PostgresSchemaIdentityUnavailableV1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum PostgresObservationStreamV1 {
    LegacyTables,
    LegacyEarlierCollisions,
    LegacyColumns,
    LegacyKeys,
    LegacyForeignKeys,
    LegacyRelationStatistics,
    LegacyColumnStatistics,
    RichRelations,
    RichAttributes,
    RichCatalogConstraints,
}

impl PostgresObservationStreamV1 {
    #[cfg(feature = "postgres-observation-evidence")]
    const COUNT: usize = 10;

    #[cfg(feature = "postgres-observation-evidence")]
    const fn index(self) -> usize {
        self as usize
    }

    #[cfg(feature = "postgres-observation-evidence")]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyTables => "legacy-tables",
            Self::LegacyEarlierCollisions => "legacy-earlier-collisions",
            Self::LegacyColumns => "legacy-columns",
            Self::LegacyKeys => "legacy-keys",
            Self::LegacyForeignKeys => "legacy-foreign-keys",
            Self::LegacyRelationStatistics => "legacy-relation-statistics",
            Self::LegacyColumnStatistics => "legacy-column-statistics",
            Self::RichRelations => "rich-relations",
            Self::RichAttributes => "rich-attributes",
            Self::RichCatalogConstraints => "rich-catalog-constraints",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostgresObservationStreamTerminalV1 {
    #[cfg(feature = "postgres-observation-evidence")]
    NotStarted,
    Complete,
    Overflow,
    RowFailure,
    QueryFailure,
}

#[cfg(feature = "postgres-observation-evidence")]
impl PostgresObservationStreamTerminalV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not-started",
            Self::Complete => "complete",
            Self::Overflow => "overflow",
            Self::RowFailure => "row-failure",
            Self::QueryFailure => "query-failure",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum PostgresObservationPhaseV1 {
    Guard,
    RelationsStream,
    AttributesStream,
    RelationNormalization,
    NotNullDerivation,
    ConstraintBudget,
    CatalogConstraintsStream,
    ConstraintNormalization,
    IdentityBuild,
    LegacyComparison,
}

impl PostgresObservationPhaseV1 {
    #[cfg(feature = "postgres-observation-evidence")]
    const COUNT: usize = 10;

    #[cfg(feature = "postgres-observation-evidence")]
    const fn index(self) -> usize {
        self as usize
    }

    #[cfg(feature = "postgres-observation-evidence")]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Guard => "guard",
            Self::RelationsStream => "relations-stream",
            Self::AttributesStream => "attributes-stream",
            Self::RelationNormalization => "relation-normalization",
            Self::NotNullDerivation => "not-null-derivation",
            Self::ConstraintBudget => "constraint-budget",
            Self::CatalogConstraintsStream => "catalog-constraints-stream",
            Self::ConstraintNormalization => "constraint-normalization",
            Self::IdentityBuild => "identity-build",
            Self::LegacyComparison => "legacy-comparison",
        }
    }
}

#[cfg(feature = "postgres-observation-evidence")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostgresObservationSavepointV1 {
    NotStarted,
    Active,
    Released,
    Recovered,
    Failed,
}

#[cfg(feature = "postgres-observation-evidence")]
impl PostgresObservationSavepointV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not-started",
            Self::Active => "active",
            Self::Released => "released",
            Self::Recovered => "recovered",
            Self::Failed => "failed",
        }
    }
}

#[cfg(feature = "postgres-observation-evidence")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostgresObservationCommitV1 {
    NotStarted,
    Started,
    Complete,
    Failed,
}

#[cfg(feature = "postgres-observation-evidence")]
impl PostgresObservationCommitV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not-started",
            Self::Started => "started",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }
}

#[cfg(feature = "postgres-observation-evidence")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostgresObservationStreamEvidenceV1 {
    started: bool,
    cap: usize,
    polled: usize,
    decoded: usize,
    retained_peak: usize,
    terminal: PostgresObservationStreamTerminalV1,
}

#[cfg(feature = "postgres-observation-evidence")]
impl PostgresObservationStreamEvidenceV1 {
    const fn empty() -> Self {
        Self {
            started: false,
            cap: 0,
            polled: 0,
            decoded: 0,
            retained_peak: 0,
            terminal: PostgresObservationStreamTerminalV1::NotStarted,
        }
    }

    pub const fn started(self) -> bool {
        self.started
    }

    pub const fn cap(self) -> usize {
        self.cap
    }

    pub const fn polled(self) -> usize {
        self.polled
    }

    pub const fn decoded(self) -> usize {
        self.decoded
    }

    pub const fn retained_peak(self) -> usize {
        self.retained_peak
    }

    pub const fn overflow(self) -> bool {
        matches!(self.terminal, PostgresObservationStreamTerminalV1::Overflow)
    }

    pub const fn terminal(self) -> PostgresObservationStreamTerminalV1 {
        self.terminal
    }
}

pub(super) trait PostgresObservationObserverV1 {
    fn stream_started(&mut self, _stream: PostgresObservationStreamV1, _cap: usize) {}
    fn row_polled(&mut self, _stream: PostgresObservationStreamV1) {}
    fn row_decoded(&mut self, _stream: PostgresObservationStreamV1, _retained: usize) {}
    fn stream_terminal(
        &mut self,
        _stream: PostgresObservationStreamV1,
        _terminal: PostgresObservationStreamTerminalV1,
    ) {
    }
    fn phase_started(&mut self, _phase: PostgresObservationPhaseV1) {}
    fn phase_completed(&mut self, _phase: PostgresObservationPhaseV1) {}
    fn phase_failed(
        &mut self,
        _phase: PostgresObservationPhaseV1,
        _error: PostgresSchemaIdentityUnavailableV1,
    ) {
    }
    fn guard_query_started(&mut self) {}
    fn guard_passed(&mut self, _server_version_num: i32) {}
    fn savepoint_active(&mut self) {}
    fn savepoint_released(&mut self) {}
    fn savepoint_recovered(&mut self) {}
    fn savepoint_failed(&mut self) {}
    fn commit_started(&mut self) {}
    fn commit_completed(&mut self) {}
    fn commit_failed(&mut self) {}
    fn constraint_counts(&mut self, _not_null: usize, _catalog: usize, _combined: usize) {}
}

#[derive(Default)]
pub(super) struct NoopPostgresObservationObserverV1;

impl PostgresObservationObserverV1 for NoopPostgresObservationObserverV1 {}

#[cfg(feature = "postgres-observation-evidence")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresObservationEvidenceV1 {
    streams: [PostgresObservationStreamEvidenceV1; PostgresObservationStreamV1::COUNT],
    phases_started: [bool; PostgresObservationPhaseV1::COUNT],
    phases_completed: [bool; PostgresObservationPhaseV1::COUNT],
    guard_passed: bool,
    guard_queries_started: usize,
    server_version_num: Option<i32>,
    not_null_constraints: usize,
    catalog_constraints: usize,
    combined_constraints: usize,
    failure: Option<(
        PostgresObservationPhaseV1,
        PostgresSchemaIdentityUnavailableV1,
    )>,
    savepoint: PostgresObservationSavepointV1,
    commit: PostgresObservationCommitV1,
}

#[cfg(feature = "postgres-observation-evidence")]
impl Default for PostgresObservationEvidenceV1 {
    fn default() -> Self {
        Self {
            streams: [PostgresObservationStreamEvidenceV1::empty();
                PostgresObservationStreamV1::COUNT],
            phases_started: [false; PostgresObservationPhaseV1::COUNT],
            phases_completed: [false; PostgresObservationPhaseV1::COUNT],
            guard_passed: false,
            guard_queries_started: 0,
            server_version_num: None,
            not_null_constraints: 0,
            catalog_constraints: 0,
            combined_constraints: 0,
            failure: None,
            savepoint: PostgresObservationSavepointV1::NotStarted,
            commit: PostgresObservationCommitV1::NotStarted,
        }
    }
}

#[cfg(feature = "postgres-observation-evidence")]
impl PostgresObservationEvidenceV1 {
    pub const fn stream(
        &self,
        stream: PostgresObservationStreamV1,
    ) -> PostgresObservationStreamEvidenceV1 {
        self.streams[stream.index()]
    }

    pub const fn phase_was_started(&self, phase: PostgresObservationPhaseV1) -> bool {
        self.phases_started[phase.index()]
    }

    pub const fn phase_was_completed(&self, phase: PostgresObservationPhaseV1) -> bool {
        self.phases_completed[phase.index()]
    }

    pub const fn guard_passed(&self) -> bool {
        self.guard_passed
    }

    pub const fn guard_queries_started(&self) -> usize {
        self.guard_queries_started
    }

    pub const fn server_version_num(&self) -> Option<i32> {
        self.server_version_num
    }

    pub const fn recorded_constraint_counts(&self) -> (usize, usize, usize) {
        (
            self.not_null_constraints,
            self.catalog_constraints,
            self.combined_constraints,
        )
    }

    pub const fn failure(
        &self,
    ) -> Option<(
        PostgresObservationPhaseV1,
        PostgresSchemaIdentityUnavailableV1,
    )> {
        self.failure
    }

    pub const fn savepoint(&self) -> PostgresObservationSavepointV1 {
        self.savepoint
    }

    pub const fn commit(&self) -> PostgresObservationCommitV1 {
        self.commit
    }
}

#[cfg(feature = "postgres-observation-evidence")]
impl PostgresSchemaIdentityUnavailableV1 {
    /// Closed identifier used by the qualification JSON protocol.
    pub const fn evidence_code(self) -> &'static str {
        use super::observation::{
            PostgresSchemaIdentityGuardCodeV1 as Guard, PostgresSchemaIdentityLimitCodeV1 as Limit,
        };
        use PostgresSchemaIdentityUnavailableV1 as Error;

        match self {
            Error::ProfileNotImplemented => "ProfileNotImplemented",
            Error::UnqualifiedEnginePatch => "UnqualifiedEnginePatch",
            Error::GuardUnsupported(Guard::ServerEncoding) => "GuardUnsupported:ServerEncoding",
            Error::GuardUnsupported(Guard::ClientEncoding) => "GuardUnsupported:ClientEncoding",
            Error::GuardUnsupported(Guard::IdentifierLength) => "GuardUnsupported:IdentifierLength",
            Error::GuardUnsupported(Guard::IndexKeyLimit) => "GuardUnsupported:IndexKeyLimit",
            Error::GuardUnsupported(Guard::IntegerDatetimes) => "GuardUnsupported:IntegerDatetimes",
            Error::GuardUnsupported(Guard::ReplicationRole) => "GuardUnsupported:ReplicationRole",
            Error::GuardUnsupported(Guard::SearchPath) => "GuardUnsupported:SearchPath",
            Error::GuardUnsupported(Guard::PublicNamespace) => "GuardUnsupported:PublicNamespace",
            Error::GuardUnsupported(Guard::CurrentDatabase) => "GuardUnsupported:CurrentDatabase",
            Error::LegacyCoordinateMismatch => "LegacyCoordinateMismatch",
            Error::CatalogQuery => "CatalogQuery",
            Error::CatalogDecode => "CatalogDecode",
            Error::LimitExceeded(Limit::RichRelations) => "LimitExceeded:RichRelations",
            Error::LimitExceeded(Limit::PhysicalAttributes) => "LimitExceeded:PhysicalAttributes",
            Error::LimitExceeded(Limit::LiveColumns) => "LimitExceeded:LiveColumns",
            Error::LimitExceeded(Limit::RawConstraints) => "LimitExceeded:RawConstraints",
            Error::LimitExceeded(Limit::KeyMembers) => "LimitExceeded:KeyMembers",
            Error::LimitExceeded(Limit::Facets) => "LimitExceeded:Facets",
            Error::LimitExceeded(Limit::TextBytes) => "LimitExceeded:TextBytes",
            Error::LimitExceeded(Limit::CanonicalBody) => "LimitExceeded:CanonicalBody",
            Error::UnsupportedRelation => "UnsupportedRelation",
            Error::UnsupportedType => "UnsupportedType",
            Error::UnsupportedCollation => "UnsupportedCollation",
            Error::UnsupportedConstraint => "UnsupportedConstraint",
            Error::IdentityRejected => "IdentityRejected",
        }
    }
}

#[cfg(feature = "postgres-observation-evidence")]
impl PostgresObservationObserverV1 for PostgresObservationEvidenceV1 {
    fn stream_started(&mut self, stream: PostgresObservationStreamV1, cap: usize) {
        self.streams[stream.index()] = PostgresObservationStreamEvidenceV1 {
            started: true,
            cap,
            ..PostgresObservationStreamEvidenceV1::empty()
        };
    }

    fn row_polled(&mut self, stream: PostgresObservationStreamV1) {
        let evidence = &mut self.streams[stream.index()];
        evidence.polled = evidence.polled.saturating_add(1);
    }

    fn row_decoded(&mut self, stream: PostgresObservationStreamV1, retained: usize) {
        let evidence = &mut self.streams[stream.index()];
        evidence.decoded = evidence.decoded.saturating_add(1);
        evidence.retained_peak = evidence.retained_peak.max(retained);
        if stream == PostgresObservationStreamV1::RichCatalogConstraints {
            self.catalog_constraints = evidence.decoded;
            self.combined_constraints = self
                .not_null_constraints
                .saturating_add(self.catalog_constraints);
        }
    }

    fn stream_terminal(
        &mut self,
        stream: PostgresObservationStreamV1,
        terminal: PostgresObservationStreamTerminalV1,
    ) {
        self.streams[stream.index()].terminal = terminal;
    }

    fn phase_started(&mut self, phase: PostgresObservationPhaseV1) {
        self.phases_started[phase.index()] = true;
    }

    fn phase_completed(&mut self, phase: PostgresObservationPhaseV1) {
        self.phases_completed[phase.index()] = true;
    }

    fn phase_failed(
        &mut self,
        phase: PostgresObservationPhaseV1,
        error: PostgresSchemaIdentityUnavailableV1,
    ) {
        if phase == PostgresObservationPhaseV1::Guard {
            self.guard_passed = false;
        }
        if self.failure.is_none() {
            self.failure = Some((phase, error));
        }
    }

    fn guard_query_started(&mut self) {
        self.guard_queries_started = self.guard_queries_started.saturating_add(1);
    }

    fn guard_passed(&mut self, server_version_num: i32) {
        self.guard_passed = true;
        self.server_version_num = Some(server_version_num);
    }

    fn savepoint_active(&mut self) {
        self.savepoint = PostgresObservationSavepointV1::Active;
    }

    fn savepoint_released(&mut self) {
        self.savepoint = PostgresObservationSavepointV1::Released;
    }

    fn savepoint_recovered(&mut self) {
        self.savepoint = PostgresObservationSavepointV1::Recovered;
    }

    fn savepoint_failed(&mut self) {
        self.savepoint = PostgresObservationSavepointV1::Failed;
    }

    fn commit_started(&mut self) {
        self.commit = PostgresObservationCommitV1::Started;
    }

    fn commit_completed(&mut self) {
        self.commit = PostgresObservationCommitV1::Complete;
    }

    fn commit_failed(&mut self) {
        self.commit = PostgresObservationCommitV1::Failed;
    }

    fn constraint_counts(&mut self, not_null: usize, catalog: usize, combined: usize) {
        self.not_null_constraints = not_null;
        self.catalog_constraints = catalog;
        self.combined_constraints = combined;
    }
}

#[cfg(test)]
mod tests;
