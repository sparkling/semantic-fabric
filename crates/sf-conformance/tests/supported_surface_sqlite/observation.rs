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
        let kind = problem["type"]
            .as_str()
            .ok_or_else(|| "problem JSON omitted its type".to_owned())?;
        let title = problem["title"]
            .as_str()
            .ok_or_else(|| "problem JSON omitted its title".to_owned())?;
        let detail = problem["detail"]
            .as_str()
            .ok_or_else(|| "problem JSON omitted its detail".to_owned())?;
        let correlation_id = problem["correlationId"]
            .as_str()
            .ok_or_else(|| "problem JSON omitted its correlation identity".to_owned())?;
        let expected_instance = format!("urn:semantic-fabric:problem-instance:{correlation_id}");
        if status != response.status
            || !is_generated_correlation_id(correlation_id)
            || problem["instance"].as_str() != Some(expected_instance.as_str())
            || response.correlation_id.as_deref() != Some(correlation_id)
        {
            return Err("problem JSON contradicts the HTTP response".to_owned());
        }
        let expected_status = if status == 501 {
            ExpectedStatus::Unsupported
        } else if (400..500).contains(&status) {
            ExpectedStatus::Rejected
        } else {
            return Err("problem status is outside the supported-surface outcome space".to_owned());
        };
        let normalized = format!(
            "problem:type={kind};title={title};status={status};detail={detail};code={cause};allow={};correlation=linked",
            response.allow.as_deref().unwrap_or("-")
        );
        Ok(Self {
            status: expected_status,
            http_status: status,
            media_type: response.media_type.clone(),
            cause: cause.to_owned(),
            result_sha256: manifest_sha256(normalized.as_bytes()),
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

fn is_generated_correlation_id(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().get(..3) == Some(b"sf-")
        && value.as_bytes().get(19) == Some(&b'-')
        && value.bytes().enumerate().all(|(index, byte)| {
            index < 3 || index == 19 || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
        })
}

pub fn finish_replay(failures: Vec<String>) -> Result<(), String> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
