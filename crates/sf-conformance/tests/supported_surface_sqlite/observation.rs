use sf_conformance::supported_surface::{manifest_sha256, Case, ExpectedStatus};

use super::fixture::ResponseSnapshot;

#[derive(Debug)]
pub struct Observation {
    status: ExpectedStatus,
    http_status: u16,
    media_type: String,
    cause: String,
    result_sha256: String,
}

impl Observation {
    pub fn supported(
        response: &ResponseSnapshot,
        cause: &str,
        normalized_result: &str,
    ) -> Result<Self, String> {
        if response.status != 200 {
            return Err("supported scenario did not return HTTP 200".to_owned());
        }
        Ok(Self {
            status: ExpectedStatus::Supported,
            http_status: response.status,
            media_type: response.media_type.clone(),
            cause: cause.to_owned(),
            result_sha256: manifest_sha256(normalized_result.as_bytes()),
        })
    }

    pub fn problem(response: &ResponseSnapshot) -> Result<Self, String> {
        if response.media_type != "application/problem+json" {
            return Err("error scenario did not return application/problem+json".to_owned());
        }
        let problem: serde_json::Value = serde_json::from_slice(&response.body)
            .map_err(|_| "error scenario returned malformed problem JSON".to_owned())?;
        let status = problem["status"]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| "problem JSON omitted a bounded status".to_owned())?;
        let cause = problem["code"]
            .as_str()
            .ok_or_else(|| "problem JSON omitted its typed code".to_owned())?;
        if status != response.status || problem["detail"].as_str().is_none() {
            return Err("problem JSON contradicts the HTTP response".to_owned());
        }
        let expected_status = if status == 501 {
            ExpectedStatus::Unsupported
        } else if (400..500).contains(&status) {
            ExpectedStatus::Rejected
        } else {
            return Err("problem status is outside the supported-surface outcome space".to_owned());
        };
        Ok(Self {
            status: expected_status,
            http_status: status,
            media_type: response.media_type.clone(),
            cause: cause.to_owned(),
            result_sha256: manifest_sha256(format!("problem:{status}:{cause}").as_bytes()),
        })
    }

    pub fn mismatch(&self, expected: &Case) -> Option<String> {
        let matches = self.status == expected.expected_status
            && self.http_status == expected.http_status
            && self.media_type == expected.response_media_type
            && self.cause == expected.cause
            && self.result_sha256 == expected.result_sha256;
        (!matches).then(|| {
            format!(
                "{} expected={}/{}/{}/{}/{} observed={}/{}/{}/{}/{}",
                expected.id,
                expected.expected_status.name(),
                expected.http_status,
                expected.response_media_type,
                expected.cause,
                expected.result_sha256,
                self.status.name(),
                self.http_status,
                self.media_type,
                self.cause,
                self.result_sha256
            )
        })
    }
}

pub fn finish_replay(failures: Vec<String>) -> Result<(), String> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
