//! Endpoint configuration for the AWS Athena backend.
//!
//! [`AthenaConfig`] is validated on construction and on every fluent setter, so
//! an [`AthenaBackend`](super::AthenaBackend) can never be built around an
//! unusable endpoint, an unsigned-scheme URL, or an out-of-range Athena API
//! bound. Credentials live in
//! [`AthenaCredentials`](super::credentials::AthenaCredentials).

use std::net::IpAddr;
use std::time::Duration;

use crate::error::{Error, Result};

/// SigV4 credential-scope service name for Athena.
pub(crate) const SERVICE_NAME: &str = "athena";
/// AWS JSON 1.1 protocol content type (Athena is a JSON 1.1 service).
pub(crate) const CONTENT_TYPE: &str = "application/x-amz-json-1.1";
/// `X-Amz-Target` prefix; the operation name is appended verbatim.
pub(crate) const TARGET_PREFIX: &str = "AmazonAthena.";

const DEFAULT_CATALOG: &str = "AwsDataCatalog";
const DEFAULT_WORKGROUP: &str = "primary";

/// Athena caps `GetQueryResults` `MaxResults` at 1000.
pub(crate) const MAX_PAGE_SIZE: u32 = 1000;
const MAX_REGION_LEN: usize = 64;
const MAX_ENDPOINT_LEN: usize = 2048;
const MAX_RETRIES_LIMIT: u32 = 10;
const MAX_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_TOTAL_DEADLINE: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_RETRY_BACKOFF: Duration = Duration::from_secs(60);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_RESPONSE_BYTES_LIMIT: usize = 256 * 1024 * 1024;

pub(super) fn cfg_err(msg: impl Into<String>) -> Error {
    Error::Marshal(format!("athena config: {}", msg.into()))
}

/// Endpoint, query context, and client-side bounds for the Athena backend.
#[derive(Clone, Debug)]
pub struct AthenaConfig {
    pub(crate) region: String,
    pub(crate) endpoint: String,
    pub(crate) catalog: String,
    pub(crate) database: Option<String>,
    pub(crate) workgroup: String,
    pub(crate) output_location: Option<String>,
    pub(crate) request_timeout: Duration,
    pub(crate) total_deadline: Duration,
    pub(crate) poll_interval: Duration,
    pub(crate) max_retries: u32,
    pub(crate) retry_backoff: Duration,
    pub(crate) page_size: u32,
    pub(crate) max_response_bytes: usize,
}

impl AthenaConfig {
    /// Config for `region`, pointed at the official regional Athena endpoint
    /// (`https://athena.{region}.amazonaws.com`).
    pub fn new(region: impl Into<String>) -> Result<Self> {
        let region = validate_region(&region.into())?;
        let endpoint = format!("https://{SERVICE_NAME}.{region}.amazonaws.com");
        Ok(Self {
            region,
            endpoint,
            catalog: DEFAULT_CATALOG.to_owned(),
            database: None,
            workgroup: DEFAULT_WORKGROUP.to_owned(),
            output_location: None,
            request_timeout: Duration::from_secs(30),
            total_deadline: Duration::from_secs(300),
            poll_interval: Duration::from_millis(250),
            max_retries: 3,
            retry_backoff: Duration::from_millis(100),
            page_size: MAX_PAGE_SIZE,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        })
    }

    /// Build from `SF_ATHENA_REGION` plus the optional
    /// `SF_ATHENA_ENDPOINT`, `SF_ATHENA_DATABASE`, `SF_ATHENA_CATALOG`,
    /// `SF_ATHENA_WORKGROUP`, and `SF_ATHENA_OUTPUT_LOCATION`.
    pub fn from_env() -> Result<Self> {
        Self::from_env_with(|name| std::env::var(name).ok())
    }

    fn from_env_with(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let region = get("SF_ATHENA_REGION").ok_or_else(|| cfg_err("SF_ATHENA_REGION not set"))?;
        let mut cfg = Self::new(region)?;
        if let Some(endpoint) = get("SF_ATHENA_ENDPOINT") {
            cfg = cfg.with_endpoint(endpoint)?;
        }
        if let Some(database) = get("SF_ATHENA_DATABASE") {
            cfg = cfg.with_database(database)?;
        }
        if let Some(catalog) = get("SF_ATHENA_CATALOG") {
            cfg = cfg.with_catalog(catalog)?;
        }
        if let Some(workgroup) = get("SF_ATHENA_WORKGROUP") {
            cfg = cfg.with_workgroup(workgroup)?;
        }
        if let Some(output) = get("SF_ATHENA_OUTPUT_LOCATION") {
            cfg = cfg.with_output_location(output)?;
        }
        Ok(cfg)
    }

    /// Override the endpoint — for VPC/private Athena endpoints and for
    /// pointing the backend at a local mock server in tests.
    ///
    /// HTTPS is required except for loopback hosts, which may use plain HTTP so
    /// a test server needs no certificate. URL userinfo is rejected outright:
    /// credentials belong in the SigV4 signature, never in a URL.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Result<Self> {
        self.endpoint = validate_endpoint(&endpoint.into())?;
        Ok(self)
    }

    /// Data catalog for `QueryExecutionContext` (default `AwsDataCatalog`).
    pub fn with_catalog(mut self, catalog: impl Into<String>) -> Result<Self> {
        self.catalog = non_empty(catalog.into(), "catalog")?;
        Ok(self)
    }

    /// Default database for `QueryExecutionContext`.
    pub fn with_database(mut self, database: impl Into<String>) -> Result<Self> {
        self.database = Some(non_empty(database.into(), "database")?);
        Ok(self)
    }

    /// Athena workgroup (default `primary`).
    pub fn with_workgroup(mut self, workgroup: impl Into<String>) -> Result<Self> {
        self.workgroup = non_empty(workgroup.into(), "workgroup")?;
        Ok(self)
    }

    /// `ResultConfiguration.OutputLocation` S3 URI. Optional: a workgroup with
    /// enforced result configuration supplies it server-side.
    pub fn with_output_location(mut self, location: impl Into<String>) -> Result<Self> {
        self.output_location = Some(non_empty(location.into(), "output location")?);
        Ok(self)
    }

    /// Per-HTTP-request timeout (each attempt, including each retry).
    pub fn with_request_timeout(mut self, timeout: Duration) -> Result<Self> {
        if timeout.is_zero() || timeout > MAX_REQUEST_TIMEOUT {
            return Err(cfg_err(format!(
                "request timeout must be > 0 and <= {}s",
                MAX_REQUEST_TIMEOUT.as_secs()
            )));
        }
        self.request_timeout = timeout;
        Ok(self)
    }

    /// Wall-clock budget for one logical execution (start + poll + paging).
    pub fn with_total_deadline(mut self, deadline: Duration) -> Result<Self> {
        if deadline.is_zero() || deadline > MAX_TOTAL_DEADLINE {
            return Err(cfg_err(format!(
                "total deadline must be > 0 and <= {}s",
                MAX_TOTAL_DEADLINE.as_secs()
            )));
        }
        self.total_deadline = deadline;
        Ok(self)
    }

    /// Delay between `GetQueryExecution` polls.
    pub fn with_poll_interval(mut self, interval: Duration) -> Result<Self> {
        if interval.is_zero() || interval > MAX_TOTAL_DEADLINE {
            return Err(cfg_err("poll interval must be > 0"));
        }
        self.poll_interval = interval;
        Ok(self)
    }

    /// Bound on retries per HTTP request (0 disables retrying).
    pub fn with_max_retries(mut self, max_retries: u32) -> Result<Self> {
        if max_retries > MAX_RETRIES_LIMIT {
            return Err(cfg_err(format!(
                "max retries must be <= {MAX_RETRIES_LIMIT}"
            )));
        }
        self.max_retries = max_retries;
        Ok(self)
    }

    /// Base delay for the exponential retry backoff.
    pub fn with_retry_backoff(mut self, backoff: Duration) -> Result<Self> {
        if backoff > MAX_RETRY_BACKOFF {
            return Err(cfg_err(format!(
                "retry backoff must be <= {}s",
                MAX_RETRY_BACKOFF.as_secs()
            )));
        }
        self.retry_backoff = backoff;
        Ok(self)
    }

    /// `GetQueryResults` page size, 1..=1000 (the Athena API bound).
    pub fn with_page_size(mut self, page_size: u32) -> Result<Self> {
        if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
            return Err(cfg_err(format!(
                "page size must be in 1..={MAX_PAGE_SIZE}, got {page_size}"
            )));
        }
        self.page_size = page_size;
        Ok(self)
    }

    /// Maximum bytes accepted for one AWS JSON response body.
    ///
    /// The default is 64 MiB, large enough for Athena's maximum-size row plus
    /// JSON framing while preventing an endpoint from forcing unbounded
    /// buffering. Values may be configured up to 256 MiB.
    pub fn with_max_response_bytes(mut self, max_response_bytes: usize) -> Result<Self> {
        if max_response_bytes == 0 || max_response_bytes > MAX_RESPONSE_BYTES_LIMIT {
            return Err(cfg_err(format!(
                "max response bytes must be in 1..={MAX_RESPONSE_BYTES_LIMIT}, \
                 got {max_response_bytes}"
            )));
        }
        self.max_response_bytes = max_response_bytes;
        Ok(self)
    }

    /// The signing region.
    pub fn region(&self) -> &str {
        &self.region
    }

    /// The fixed endpoint every operation is POSTed to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Full request URL. Athena's JSON 1.1 API has a single `/` resource.
    pub(crate) fn request_url(&self) -> String {
        format!("{}/", self.endpoint)
    }

    /// Cross-field validation the individual setters cannot do alone.
    pub(crate) fn validate(&self) -> Result<()> {
        if self.request_timeout > self.total_deadline {
            return Err(cfg_err(
                "request timeout must not exceed the total deadline",
            ));
        }
        if self.poll_interval > self.total_deadline {
            return Err(cfg_err("poll interval must not exceed the total deadline"));
        }
        Ok(())
    }
}

fn non_empty(value: String, what: &str) -> Result<String> {
    if value.trim().is_empty() {
        return Err(cfg_err(format!("{what} must not be empty")));
    }
    Ok(value)
}

fn validate_region(raw: &str) -> Result<String> {
    let region = raw.trim();
    if region.is_empty() || region.len() > MAX_REGION_LEN {
        return Err(cfg_err(format!(
            "region must be 1..={MAX_REGION_LEN} characters"
        )));
    }
    if !region
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(cfg_err(
            "region must contain only lowercase ASCII letters, digits, and '-'",
        ));
    }
    if !region.starts_with(|c: char| c.is_ascii_lowercase()) || region.ends_with('-') {
        return Err(cfg_err(
            "region must start with a letter and must not end with '-'",
        ));
    }
    Ok(region.to_owned())
}

fn validate_endpoint(raw: &str) -> Result<String> {
    let endpoint = raw.trim();
    if endpoint.is_empty() || endpoint.len() > MAX_ENDPOINT_LEN {
        return Err(cfg_err(format!(
            "endpoint must be 1..={MAX_ENDPOINT_LEN} characters"
        )));
    }
    if endpoint
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(cfg_err("endpoint must not contain whitespace"));
    }
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|e| cfg_err(format!("endpoint is not a valid URL: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(cfg_err("endpoint must start with http:// or https://"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(cfg_err("endpoint must not carry a query or fragment"));
    }
    if url.path() != "/" {
        return Err(cfg_err("endpoint must not carry a path"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(cfg_err("endpoint must not carry URL userinfo"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| cfg_err("endpoint must have a host"))?;
    if url.scheme() == "http" && !is_loopback(host) {
        return Err(cfg_err(
            "plain http endpoints are only allowed for loopback hosts",
        ));
    }
    // `Url` canonicalizes the scheme, host casing, IPv6 notation, and port.
    // Store that exact authority so SigV4 and reqwest cannot disagree.
    url.set_path("");
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::AthenaConfig;

    #[test]
    fn default_endpoint_is_the_official_regional_host() {
        let cfg = AthenaConfig::new("us-east-1").unwrap();
        assert_eq!(cfg.endpoint(), "https://athena.us-east-1.amazonaws.com");
        assert_eq!(cfg.workgroup, "primary");
        assert_eq!(cfg.catalog, "AwsDataCatalog");
        assert_eq!(cfg.request_url(), "https://athena.us-east-1.amazonaws.com/");
    }

    #[test]
    fn environment_config_is_scoped_to_sf_athena_variables() {
        let cfg = AthenaConfig::from_env_with(|name| match name {
            "SF_ATHENA_REGION" => Some("eu-west-1".to_owned()),
            "SF_ATHENA_DATABASE" => Some("analytics".to_owned()),
            _ => None,
        })
        .unwrap();
        assert_eq!(cfg.region, "eu-west-1");
        assert_eq!(cfg.database.as_deref(), Some("analytics"));

        let aws_only = AthenaConfig::from_env_with(|name| {
            (name == "AWS_REGION").then(|| "us-east-1".to_owned())
        });
        assert!(aws_only.is_err());
    }

    #[test]
    fn region_charset_is_enforced() {
        for bad in ["", "US-EAST-1", "us east 1", "-us-east-1", "us-east-1-"] {
            assert!(AthenaConfig::new(bad).is_err(), "accepted region {bad:?}");
        }
        assert!(AthenaConfig::new("eu-central-1").is_ok());
    }

    #[test]
    fn endpoint_rejects_userinfo_and_plain_http_off_loopback() {
        let cfg = AthenaConfig::new("us-east-1").unwrap();
        for bad in [
            "https://user:pass@athena.us-east-1.amazonaws.com",
            "http://athena.us-east-1.amazonaws.com",
            "ftp://athena.us-east-1.amazonaws.com",
            "https://athena.us-east-1.amazonaws.com/some/path",
            "https://athena.us-east-1.amazonaws.com?x=1",
            "https://",
        ] {
            assert!(
                cfg.clone().with_endpoint(bad).is_err(),
                "accepted endpoint {bad:?}"
            );
        }
        assert!(cfg.clone().with_endpoint("http://127.0.0.1:9999").is_ok());
        assert!(cfg.clone().with_endpoint("http://localhost:9999").is_ok());
        assert!(cfg.clone().with_endpoint("http://[::1]:9999").is_ok());
        assert!(cfg
            .with_endpoint("https://vpce-abc.athena.eu-west-1.vpce.amazonaws.com")
            .is_ok());
    }

    #[test]
    fn endpoint_is_normalized_once_for_signing_and_transport() {
        let cfg = AthenaConfig::new("us-east-1")
            .unwrap()
            .with_endpoint("HTTPS://ATHENA.US-EAST-1.AMAZONAWS.COM:443/")
            .unwrap();
        assert_eq!(cfg.endpoint(), "https://athena.us-east-1.amazonaws.com");
        assert_eq!(cfg.request_url(), "https://athena.us-east-1.amazonaws.com/");
    }

    #[test]
    fn bounds_are_validated() {
        let cfg = AthenaConfig::new("us-east-1").unwrap();
        assert!(cfg.clone().with_page_size(0).is_err());
        assert!(cfg.clone().with_page_size(1001).is_err());
        assert!(cfg.clone().with_max_response_bytes(0).is_err());
        assert!(cfg
            .clone()
            .with_max_response_bytes(256 * 1024 * 1024 + 1)
            .is_err());
        assert!(cfg.clone().with_max_response_bytes(1024).is_ok());
        assert!(cfg.clone().with_page_size(1).is_ok());
        assert!(cfg.clone().with_page_size(1000).is_ok());
        assert!(cfg.clone().with_max_retries(11).is_err());
        assert!(cfg.clone().with_request_timeout(Duration::ZERO).is_err());
        assert!(cfg.clone().with_total_deadline(Duration::ZERO).is_err());
        assert!(cfg.clone().with_poll_interval(Duration::ZERO).is_err());
        assert!(cfg
            .clone()
            .with_retry_backoff(Duration::from_secs(61))
            .is_err());
        assert!(cfg.clone().with_catalog(" ").is_err());
        assert!(cfg.with_database("").is_err());
    }

    #[test]
    fn request_timeout_may_not_exceed_the_total_deadline() {
        let cfg = AthenaConfig::new("us-east-1")
            .unwrap()
            .with_total_deadline(Duration::from_secs(1))
            .unwrap()
            .with_request_timeout(Duration::from_secs(30))
            .unwrap();
        assert!(cfg.validate().is_err());
    }
}
