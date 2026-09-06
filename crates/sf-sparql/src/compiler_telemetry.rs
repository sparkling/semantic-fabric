//! Payload-free compiler-stage spans for the product translation path.

const SCHEMA: &str = "semantic-fabric.telemetry.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompilerStage {
    Parse,
    Rewrite,
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
        target: "semantic_fabric::telemetry",
        "sf.compiler.stage",
        schema = SCHEMA,
        stage = stage.as_str(),
    )
    .in_scope(operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_stage_vocabulary_is_closed_and_static() {
        let stages = [
            CompilerStage::Parse,
            CompilerStage::Rewrite,
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
                "build",
                "resolve",
                "normalize",
                "lower",
                "cascade",
            ]
        );
    }
}
