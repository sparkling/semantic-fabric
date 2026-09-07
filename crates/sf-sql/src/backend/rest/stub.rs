//! Feature-disabled HTTP backends. Each provider family retains an unsupported
//! stub when its owning feature is disabled.

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

macro_rules! stub_backend {
    ($name:ident, $stream:ident, $msg:expr) => {
        /// Feature-disabled backend stub.
        pub struct $name;
        /// Stub stream — never yields rows.
        pub struct $stream;

        impl BranchStream for $stream {
            async fn next_row(&mut self) -> Result<Option<RawTuple>> {
                Err(Error::Unsupported($msg.to_owned()))
            }
        }

        impl SqlBackend for $name {
            type Stream<'s>
                = $stream
            where
                Self: 's;

            async fn column_names(&mut self, _probe_sql: &str) -> Result<Vec<String>> {
                Err(Error::Unsupported($msg.to_owned()))
            }

            async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> Result<$stream> {
                Err(Error::Unsupported($msg.to_owned()))
            }
        }
    };
}

#[cfg(not(feature = "rest-backends"))]
stub_backend!(
    SnowflakeBackend,
    SnowflakeStream,
    "SnowflakeBackend: enable the `rest-backends` feature"
);
#[cfg(not(feature = "rest-backends"))]
stub_backend!(
    BigQueryBackend,
    BigQueryStream,
    "BigQueryBackend: enable the `rest-backends` feature"
);
#[cfg(not(feature = "athena-backend"))]
stub_backend!(
    AthenaBackend,
    AthenaStream,
    "AthenaBackend: enable the `athena-backend` feature"
);
#[cfg(not(feature = "rest-backends"))]
stub_backend!(
    DatabricksBackend,
    DatabricksStream,
    "DatabricksBackend: enable the `rest-backends` feature"
);
#[cfg(not(feature = "rest-backends"))]
stub_backend!(
    TrinoBackend,
    TrinoStream,
    "TrinoBackend: enable the `rest-backends` feature"
);
#[cfg(not(feature = "rest-backends"))]
stub_backend!(
    PrestoDbBackend,
    PrestoDbStream,
    "PrestoDbBackend: enable the `rest-backends` feature"
);

#[cfg(all(test, not(feature = "rest-backends")))]
mod tests {
    #[tokio::test]
    async fn stub_returns_unsupported() {
        use crate::backend::rest::SnowflakeBackend;
        use crate::backend::SqlBackend;
        let mut b = SnowflakeBackend;
        let r = b.column_names("SELECT 1").await;
        assert!(matches!(r, Err(crate::error::Error::Unsupported(_))));
    }
}
