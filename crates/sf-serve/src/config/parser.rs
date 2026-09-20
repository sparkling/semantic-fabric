//! Explicit parser ownership; missing configuration never selects raw parsing.
use super::*;

pub(super) enum ParserSetup {
    Missing,
    Isolated(sf_sparql::ParserRuntime),
    #[cfg(test)]
    InProcessUnitFixture,
}

impl ServeConfig {
    /// Install a verified, held parser executable before sharing this config.
    /// Without it readiness and query compilation fail closed.
    pub fn set_parser_runtime(&mut self, parser: sf_sparql::ParserRuntime) {
        self.parser = ParserSetup::Isolated(parser);
    }

    /// The exact `ParserRuntime` this config snapshot currently holds, cheap
    /// to clone (an `Arc` inside). Read once per request at admission
    /// (`request_budget_for`) so a concurrent reload's fresh config cannot
    /// retarget an in-flight request's isolated SQL emission, and a request
    /// admitted under an older config keeps using the executable it started
    /// with.
    pub(crate) fn parser_runtime(&self) -> Option<sf_sparql::ParserRuntime> {
        match &self.parser {
            ParserSetup::Isolated(parser) => Some(parser.clone()),
            ParserSetup::Missing => None,
            #[cfg(test)]
            ParserSetup::InProcessUnitFixture => None,
        }
    }

    pub(crate) fn with_parser<T>(
        &self,
        control: &RequestBudget,
        work: impl FnOnce() -> sf_sparql::Result<T>,
    ) -> sf_sparql::Result<T> {
        match &self.parser {
            ParserSetup::Isolated(parser) => parser.with_request(Arc::new(control.clone()), work),
            ParserSetup::Missing => Err(sf_sparql::Error::Mapping(
                "parser runtime is unavailable".into(),
            )),
            #[cfg(test)]
            ParserSetup::InProcessUnitFixture => work(),
        }
    }

    /// An explicit mock for unit tests of non-parser seams. This API and variant
    /// do not exist in integration-test libraries or deployable artifacts.
    #[cfg(test)]
    pub(crate) fn use_in_process_test_parser(&mut self) {
        self.parser = ParserSetup::InProcessUnitFixture;
    }
}
