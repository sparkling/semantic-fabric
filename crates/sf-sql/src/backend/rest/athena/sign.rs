//! SigV4 request signing and `ClientRequestToken` derivation.
//!
//! Signing goes through the official `aws-sigv4` crate (SigV4, `athena`
//! service, configured region, signature in headers) rather than a hand-rolled
//! HMAC chain. Each attempt — including every retry — is signed afresh with the
//! current wall-clock time over a byte-identical body, because an AWS signature
//! is time-scoped and a stale `X-Amz-Date` is rejected outside a 5-minute skew
//! window.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

use super::config::{AthenaConfig, CONTENT_TYPE, SERVICE_NAME};
use super::credentials::AthenaCredentials;

/// Identifies this signer in `aws-credential-types` provenance metadata.
const CREDENTIAL_PROVIDER_NAME: &str = "sf-sql-athena-static";

/// Headers `aws-sigv4` wants added to a request, plus the raw signature (kept
/// so the known-answer test can pin it without re-parsing the header).
pub(crate) struct SignedHeaders {
    pub(crate) headers: Vec<(String, String)>,
    /// Read only by the known-answer test, which pins it independently of the
    /// `Authorization` header text.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) signature: String,
}

/// SigV4-sign one Athena JSON 1.1 POST.
///
/// `target` is the full `X-Amz-Target` value (`AmazonAthena.{Operation}`), and
/// `body` is the exact byte string that will be sent — signing a different body
/// than is transmitted produces a signature mismatch at AWS.
pub(crate) fn sign_request(
    config: &AthenaConfig,
    credentials: &AthenaCredentials,
    target: &str,
    body: &[u8],
    time: SystemTime,
) -> Result<SignedHeaders> {
    let url = config.request_url();
    let identity = Credentials::new(
        credentials.access_key_id.clone(),
        credentials.secret_access_key.clone(),
        credentials.session_token.clone(),
        None,
        CREDENTIAL_PROVIDER_NAME,
    )
    .into();

    let params: aws_sigv4::http_request::SigningParams<'_> = v4::SigningParams::builder()
        .identity(&identity)
        .region(&config.region)
        .name(SERVICE_NAME)
        .time(time)
        .settings(SigningSettings::default())
        .build()
        .map_err(|e| sign_err(credentials, format!("signing params: {e}")))?
        .into();

    // Only these two headers are ours; `host` and `x-amz-date` are contributed
    // by the signer itself and must NOT be passed in here.
    let presigned_headers = [("content-type", CONTENT_TYPE), ("x-amz-target", target)];
    let signable = SignableRequest::new(
        "POST",
        url.as_str(),
        presigned_headers.iter().copied(),
        SignableBody::Bytes(body),
    )
    .map_err(|e| sign_err(credentials, format!("signable request: {e}")))?;

    let (instructions, signature) = sign(signable, &params)
        .map_err(|e| sign_err(credentials, format!("sigv4: {e}")))?
        .into_parts();

    let headers = instructions
        .headers()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    Ok(SignedHeaders { headers, signature })
}

fn sign_err(credentials: &AthenaCredentials, msg: String) -> Error {
    Error::Marshal(credentials.redact(&format!("athena signing: {msg}")))
}

/// Fresh `ClientRequestToken` for ONE logical query execution.
///
/// Athena requires 32..=128 characters and treats the token as an idempotency
/// key: the same token replays the original execution instead of starting a new
/// one. That is exactly the property retries need, so the token is generated
/// once per logical execution and then reused verbatim for every retry of that
/// `StartQueryExecution` — never for a second logical query.
///
/// Derived by digesting the wall clock, the process id, and a
/// monotonically-increasing in-process counter, so no random-number crate is
/// pulled in. The digest is not a secret and carries no security requirement:
/// it only needs to be distinct per execution.
pub(crate) fn client_request_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut hasher = Sha256::new();
    hasher.update(b"sf-sql/athena/ClientRequestToken/v1");
    hasher.update(nanos.to_be_bytes());
    hasher.update(std::process::id().to_be_bytes());
    hasher.update(counter.to_be_bytes());
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

/// Deterministic known-answer coverage for the signer, plus token-shape tests.
///
/// The expected `Authorization` value is pinned as a literal AND recomputed
/// in-test from the SigV4 specification (canonical request → string to sign →
/// HMAC-SHA256 key chain) using only `sha2`, so the assertion is an independent
/// check of the signature rather than a snapshot of whatever the code emits.
#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::{Duration, UNIX_EPOCH};

    use sha2::{Digest, Sha256};

    use super::super::config::AthenaConfig;
    use super::super::credentials::AthenaCredentials;
    use super::{client_request_token, hex_lower, sign_request};

    const KAT_ACCESS_KEY: &str = "AKIDEXAMPLE";
    const KAT_SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
    const KAT_TARGET: &str = "AmazonAthena.GetQueryExecution";
    const KAT_BODY: &[u8] = br#"{"QueryExecutionId":"11111111-2222-3333-4444-555555555555"}"#;
    /// 2026-01-01T00:00:00Z.
    const KAT_EPOCH_SECS: u64 = 1_767_225_600;
    const KAT_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 \
Credential=AKIDEXAMPLE/20260101/us-east-1/athena/aws4_request, \
SignedHeaders=content-type;host;x-amz-date;x-amz-target, \
Signature=c6957812c1a01ef137e54b4c2bcfd94ddc9834f7efb648b41aeccc18ba5aa63e";

    fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
        let mut block = [0u8; 64];
        if key.len() > 64 {
            block[..32].copy_from_slice(&Sha256::digest(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let mut inner_pad = [0x36u8; 64];
        let mut outer_pad = [0x5cu8; 64];
        for i in 0..64 {
            inner_pad[i] ^= block[i];
            outer_pad[i] ^= block[i];
        }
        let mut inner = Sha256::new();
        inner.update(inner_pad);
        inner.update(msg);
        let inner = inner.finalize();
        let mut outer = Sha256::new();
        outer.update(outer_pad);
        outer.update(inner);
        outer.finalize().into()
    }

    /// SigV4 from first principles, per the AWS signing specification.
    fn expected_signature() -> String {
        let payload_hash = hex_lower(&Sha256::digest(KAT_BODY));
        let canonical_request = format!(
            "POST\n/\n\n\
content-type:application/x-amz-json-1.1\n\
host:athena.us-east-1.amazonaws.com\n\
x-amz-date:20260101T000000Z\n\
x-amz-target:{KAT_TARGET}\n\
\n\
content-type;host;x-amz-date;x-amz-target\n\
{payload_hash}"
        );
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n20260101T000000Z\n20260101/us-east-1/athena/aws4_request\n{}",
            hex_lower(&Sha256::digest(canonical_request.as_bytes()))
        );
        let k_date = hmac_sha256(format!("AWS4{KAT_SECRET_KEY}").as_bytes(), b"20260101");
        let k_region = hmac_sha256(&k_date, b"us-east-1");
        let k_service = hmac_sha256(&k_region, b"athena");
        let k_signing = hmac_sha256(&k_service, b"aws4_request");
        hex_lower(&hmac_sha256(&k_signing, string_to_sign.as_bytes()))
    }

    #[test]
    fn hmac_sha256_matches_rfc4231_vector() {
        // RFC 4231 test case 2, so a bug in the in-test HMAC cannot silently
        // agree with a bug in the signer.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex_lower(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn sigv4_known_answer_pins_the_authorization_header() {
        let config = AthenaConfig::new("us-east-1").unwrap();
        let credentials = AthenaCredentials::new(KAT_ACCESS_KEY, KAT_SECRET_KEY).unwrap();
        let time = UNIX_EPOCH + Duration::from_secs(KAT_EPOCH_SECS);

        let signed = sign_request(&config, &credentials, KAT_TARGET, KAT_BODY, time).unwrap();

        let header = |name: &str| {
            signed
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        assert_eq!(header("x-amz-date").as_deref(), Some("20260101T000000Z"));
        assert_eq!(header("x-amz-security-token"), None);
        assert_eq!(signed.signature, expected_signature());
        assert_eq!(
            header("authorization").as_deref(),
            Some(KAT_AUTHORIZATION),
            "signing output drifted from the pinned known answer"
        );
    }

    #[test]
    fn session_token_is_signed_and_sent() {
        let config = AthenaConfig::new("us-east-1").unwrap();
        let credentials = AthenaCredentials::new(KAT_ACCESS_KEY, KAT_SECRET_KEY)
            .unwrap()
            .with_session_token("SESSIONTOKEN")
            .unwrap();
        let time = UNIX_EPOCH + Duration::from_secs(KAT_EPOCH_SECS);

        let signed = sign_request(&config, &credentials, KAT_TARGET, KAT_BODY, time).unwrap();
        let authorization = signed
            .headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert!(
            authorization.contains("x-amz-security-token"),
            "session token must be part of SignedHeaders: {authorization}"
        );
        assert!(signed
            .headers
            .iter()
            .any(|(k, v)| k == "x-amz-security-token" && v == "SESSIONTOKEN"));
        // A different credential set must produce a different signature.
        assert_ne!(signed.signature, expected_signature());
    }

    #[test]
    fn client_request_tokens_are_unique_and_within_the_athena_length_bounds() {
        let tokens: HashSet<String> = (0..256).map(|_| client_request_token()).collect();
        assert_eq!(tokens.len(), 256, "ClientRequestToken collision");
        for token in &tokens {
            assert!(
                (32..=128).contains(&token.len()),
                "token length {} outside Athena's 32..=128",
                token.len()
            );
            assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }
}
