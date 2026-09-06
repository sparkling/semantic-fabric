//! Payload-free compiler-stage spans for the product translation path.

use sf_core::TELEMETRY_TARGET;

const SCHEMA: &str = "semantic-fabric.telemetry.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompilerStage {
    Parse,
    Rewrite,
    Saturate,
    Unfold,
    Build,
    Resolve,
    Normalize,
    Lower,
    Cascade,
}

impl CompilerStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Rewrite => "rewrite",
            Self::Saturate => "saturate",
            Self::Unfold => "unfold",
            Self::Build => "build",
            Self::Resolve => "resolve",
            Self::Normalize => "normalize",
            Self::Lower => "lower",
            Self::Cascade => "cascade",
        }
    }
}

pub(crate) fn in_stage<T>(stage: CompilerStage, operation: impl FnOnce() -> T) -> T {
    tracing::info_span!(
        target: TELEMETRY_TARGET,
        "sf.compiler.stage",
        schema = SCHEMA,
        stage = stage.as_str(),
    )
    .in_scope(operation)
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use sf_core::ir::{
        LogicalSource, ObjectMap, PredicateObjectMap, SubjectMap, Template, TermMap, TermSpec,
        TriplesMap,
    };
    use sf_core::NamedNode;
    use sf_sql::Dialect;
    use tracing::Dispatch;
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    struct CaptureWriter(Capture);

    impl Write for CaptureWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 .0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for Capture {
        type Writer = CaptureWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CaptureWriter(self.clone())
        }
    }

    #[test]
    fn compiler_stage_vocabulary_is_closed_and_static() {
        let stages = [
            CompilerStage::Parse,
            CompilerStage::Rewrite,
            CompilerStage::Saturate,
            CompilerStage::Unfold,
            CompilerStage::Build,
            CompilerStage::Resolve,
            CompilerStage::Normalize,
            CompilerStage::Lower,
            CompilerStage::Cascade,
        ];
        assert_eq!(
            stages.map(CompilerStage::as_str),
            [
                "parse",
                "rewrite",
                "saturate",
                "unfold",
                "build",
                "resolve",
                "normalize",
                "lower",
                "cascade",
            ]
        );
    }

    #[test]
    fn cascade_span_count_does_not_scale_with_union_branches() {
        let query = format!(
            "SELECT ?s WHERE {{ {} }}",
            std::iter::repeat_n("{ ?s <http://example.com/p> ?o }", 16)
                .collect::<Vec<_>>()
                .join(" UNION ")
        );
        let (plan, output) =
            capture(|| crate::parse_and_translate(&query, &[mapping()], Dialect::Sqlite).unwrap());
        assert!(plan.branches.len() > 1, "test must exercise branch fan-out");
        assert_eq!(
            output.matches("\"stage\":\"cascade\"").count(),
            1,
            "output={output}"
        );
    }

    #[test]
    fn flat_compiler_reports_its_actual_stages_without_tree_stage_claims() {
        let (_plan, output) = capture(|| {
            crate::parse_and_translate_flat_with(
                "SELECT ?s WHERE { ?s <http://example.com/p> ?o }",
                &[mapping()],
                Dialect::Sqlite,
                &crate::Tbox::default(),
                &[],
            )
            .unwrap()
        });
        for stage in ["parse", "rewrite", "unfold", "cascade"] {
            assert_eq!(
                output.matches(&format!("\"stage\":\"{stage}\"")).count(),
                1,
                "stage={stage}, output={output}"
            );
        }
        for tree_only in ["build", "resolve", "normalize", "lower"] {
            assert!(!output.contains(&format!("\"stage\":\"{tree_only}\"")));
        }
    }

    fn capture<T>(operation: impl FnOnce() -> T) -> (T, String) {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .without_time()
            .with_span_events(FmtSpan::NEW)
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::INFO)
            .finish();
        let value = tracing::dispatcher::with_default(&Dispatch::new(subscriber), operation);
        let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        (value, output)
    }

    fn mapping() -> TriplesMap {
        TriplesMap {
            id: "telemetry-map".to_owned(),
            source: LogicalSource::Table("items".to_owned()),
            subject: SubjectMap {
                term: TermMap::Template(
                    Template::parse("http://example.com/item/{id}").unwrap(),
                    TermSpec::iri(),
                ),
                classes: Vec::new(),
                graphs: Vec::new(),
            },
            predicate_object_maps: vec![PredicateObjectMap {
                predicates: vec![TermMap::Constant(
                    NamedNode::new_unchecked("http://example.com/p").into(),
                )],
                objects: vec![ObjectMap::Term(TermMap::Column(
                    "value".into(),
                    TermSpec::plain_literal(),
                ))],
                graphs: Vec::new(),
            }],
        }
    }
}
