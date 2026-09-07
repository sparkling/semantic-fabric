use std::fmt;
use std::ops::Range;

use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{Key, Tag, XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, Zeroizing};

use super::{SpillError, SpillIdentity};

const MAGIC: &[u8; 8] = b"SFSPILL\0";
const VERSION: u8 = 1;
const RUN_ID_LEN: usize = 16;
const DIGEST_LEN: usize = 32;
const NONCE_LEN: usize = 24;
pub(super) const TAG_LEN: usize = 16;

pub(super) const VERSION_OFFSET: usize = MAGIC.len();
const RUN_ID_OFFSET: usize = VERSION_OFFSET + 1;
const QUERY_OFFSET: usize = RUN_ID_OFFSET + RUN_ID_LEN;
const OPERATOR_OFFSET: usize = QUERY_OFFSET + DIGEST_LEN;
const SCHEMA_OFFSET: usize = OPERATOR_OFFSET + DIGEST_LEN;
const SEQUENCE_OFFSET: usize = SCHEMA_OFFSET + DIGEST_LEN;
pub(super) const DECLARED_LEN_OFFSET: usize = SEQUENCE_OFFSET + 8;
const NONCE_OFFSET: usize = DECLARED_LEN_OFFSET + 4;
pub(super) const HEADER_LEN: usize = NONCE_OFFSET + NONCE_LEN;

#[derive(Clone, Copy)]
pub(super) struct FrameBinding {
    pub(super) run: [u8; RUN_ID_LEN],
    pub(super) identity: SpillIdentity,
}

/// Stable heap owner for the per-query secret. The allocation is established
/// while still all-zero and entropy is written directly into that allocation,
/// so moving `EphemeralKey` moves only the `Box` pointer.
pub(super) struct EphemeralKey(Box<Zeroizing<[u8; 32]>>);

impl EphemeralKey {
    pub(super) fn generate() -> Result<Self, SpillError> {
        Self::generate_with(getrandom::fill)
    }

    fn generate_with<E>(fill: impl FnOnce(&mut [u8]) -> Result<(), E>) -> Result<Self, SpillError> {
        // Only zeroes are moved while constructing the allocation. Secret
        // material is subsequently filled in place at its stable address.
        let mut key = Self(Box::new(Zeroizing::new([0_u8; 32])));
        key.fill_with(fill)?;
        Ok(key)
    }

    fn fill_with<E>(
        &mut self,
        fill: impl FnOnce(&mut [u8]) -> Result<(), E>,
    ) -> Result<(), SpillError> {
        if fill(&mut **self.0).is_err() {
            // Do not rely solely on Drop: make partial entropy writes disappear
            // before the error is constructed and returned.
            self.erase();
            return Err(SpillError::EntropyUnavailable);
        }
        Ok(())
    }

    fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub(super) fn erase(&mut self) {
        (**self.0).zeroize();
    }

    #[cfg(test)]
    pub(super) fn from_test_byte(byte: u8) -> Self {
        Self::generate_with(|output| {
            output.fill(byte);
            Ok::<(), ()>(())
        })
        .expect("infallible test fill")
    }

    #[cfg(test)]
    fn is_erased(&self) -> bool {
        self.bytes() == &[0; 32]
    }
}

impl fmt::Debug for EphemeralKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EphemeralKey([REDACTED])")
    }
}

impl Drop for EphemeralKey {
    fn drop(&mut self) {
        self.erase();
    }
}

pub(super) fn encoded_len(plaintext_len: usize) -> Result<usize, SpillError> {
    HEADER_LEN
        .checked_add(plaintext_len)
        .and_then(|value| value.checked_add(TAG_LEN))
        .ok_or(SpillError::InvalidConfiguration)
}

pub(super) fn seal(
    scratch: &mut Vec<u8>,
    key: &EphemeralKey,
    binding: FrameBinding,
    sequence: u64,
    plaintext: &[u8],
) -> Result<(), SpillError> {
    let declared = u32::try_from(plaintext.len()).map_err(|_| SpillError::QuotaExceeded)?;
    let total = encoded_len(plaintext.len()).map_err(|_| SpillError::QuotaExceeded)?;
    if total > scratch.capacity() {
        return Err(SpillError::QuotaExceeded);
    }

    // `Vec::zeroize` covers the full capacity, not only the current length, so
    // a smaller next frame cannot leave prior plaintext in spare capacity.
    scratch.zeroize();
    scratch.resize(total, 0);
    let nonce = nonce(binding.run, sequence);
    encode_header(
        &mut scratch[..HEADER_LEN],
        binding,
        sequence,
        declared,
        nonce,
    );
    scratch[HEADER_LEN..HEADER_LEN + plaintext.len()].copy_from_slice(plaintext);

    let (header, body_and_tag) = scratch.split_at_mut(HEADER_LEN);
    let (body, tag_output) = body_and_tag.split_at_mut(plaintext.len());
    // RustCrypto's `ChaChaPoly1305` implements `ZeroizeOnDrop` for its stored
    // cipher state. This claim is limited to that owned working-key state;
    // library-internal derived temporaries are outside this prototype's proof.
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key.bytes()));
    let tag = cipher
        .encrypt_in_place_detached(XNonce::from_slice(&nonce), header, body)
        .map_err(|_| SpillError::EncryptionFailed)?;
    tag_output.copy_from_slice(tag.as_slice());
    Ok(())
}

pub(super) fn inspect_header(
    header: &[u8; HEADER_LEN],
    binding: FrameBinding,
    expected_sequence: u64,
    max_plaintext: usize,
) -> Result<usize, SpillError> {
    if &header[..MAGIC.len()] != MAGIC {
        return Err(SpillError::MalformedBlock);
    }
    if header[VERSION_OFFSET] != VERSION {
        return Err(SpillError::UnsupportedVersion);
    }
    if header[RUN_ID_OFFSET..QUERY_OFFSET] != binding.run
        || header[QUERY_OFFSET..OPERATOR_OFFSET] != binding.identity.query
        || header[OPERATOR_OFFSET..SCHEMA_OFFSET] != binding.identity.operator
        || header[SCHEMA_OFFSET..SEQUENCE_OFFSET] != binding.identity.schema
    {
        return Err(SpillError::IdentityMismatch);
    }

    let sequence = read_u64(&header[SEQUENCE_OFFSET..DECLARED_LEN_OFFSET]);
    if sequence != expected_sequence {
        return Err(SpillError::SequenceMismatch);
    }
    let declared = read_u32(&header[DECLARED_LEN_OFFSET..NONCE_OFFSET]) as usize;
    if declared > max_plaintext {
        return Err(SpillError::LengthMismatch);
    }
    if header[NONCE_OFFSET..] != nonce(binding.run, sequence) {
        return Err(SpillError::SequenceMismatch);
    }
    encoded_len(declared).map_err(|_| SpillError::LengthMismatch)
}

pub(super) fn open(
    scratch: &mut [u8],
    key: &EphemeralKey,
    binding: FrameBinding,
    expected_sequence: u64,
    max_plaintext: usize,
) -> Result<Range<usize>, SpillError> {
    if scratch.len() < HEADER_LEN + TAG_LEN {
        return Err(SpillError::TruncatedBlock);
    }
    let mut header_copy = [0_u8; HEADER_LEN];
    header_copy.copy_from_slice(&scratch[..HEADER_LEN]);
    let expected_len = inspect_header(&header_copy, binding, expected_sequence, max_plaintext)?;
    if scratch.len() < expected_len {
        return Err(SpillError::TruncatedBlock);
    }
    if scratch.len() != expected_len {
        return Err(SpillError::LengthMismatch);
    }

    let payload_len = expected_len - HEADER_LEN - TAG_LEN;
    let tag_offset = HEADER_LEN + payload_len;
    let mut tag = [0_u8; TAG_LEN];
    tag.copy_from_slice(&scratch[tag_offset..]);
    let (header, body_and_tag) = scratch.split_at_mut(HEADER_LEN);
    let body = &mut body_and_tag[..payload_len];
    // The RustCrypto cipher's stored working-key state implements
    // `ZeroizeOnDrop`; see the compile-time trait assertion below. Derived
    // library/compiler temporaries remain outside the erasure claim.
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key.bytes()));
    cipher
        .decrypt_in_place_detached(
            XNonce::from_slice(&nonce(binding.run, expected_sequence)),
            header,
            body,
            Tag::from_slice(&tag),
        )
        .map_err(|_| SpillError::AuthenticationFailed)?;
    Ok(HEADER_LEN..tag_offset)
}

fn encode_header(
    header: &mut [u8],
    binding: FrameBinding,
    sequence: u64,
    declared: u32,
    nonce: [u8; NONCE_LEN],
) {
    header[..MAGIC.len()].copy_from_slice(MAGIC);
    header[VERSION_OFFSET] = VERSION;
    header[RUN_ID_OFFSET..QUERY_OFFSET].copy_from_slice(&binding.run);
    header[QUERY_OFFSET..OPERATOR_OFFSET].copy_from_slice(&binding.identity.query);
    header[OPERATOR_OFFSET..SCHEMA_OFFSET].copy_from_slice(&binding.identity.operator);
    header[SCHEMA_OFFSET..SEQUENCE_OFFSET].copy_from_slice(&binding.identity.schema);
    header[SEQUENCE_OFFSET..DECLARED_LEN_OFFSET].copy_from_slice(&sequence.to_be_bytes());
    header[DECLARED_LEN_OFFSET..NONCE_OFFSET].copy_from_slice(&declared.to_be_bytes());
    header[NONCE_OFFSET..].copy_from_slice(&nonce);
}

fn nonce(run: [u8; RUN_ID_LEN], sequence: u64) -> [u8; NONCE_LEN] {
    let mut nonce = [0_u8; NONCE_LEN];
    nonce[..RUN_ID_LEN].copy_from_slice(&run);
    nonce[RUN_ID_LEN..].copy_from_slice(&sequence.to_be_bytes());
    nonce
}

fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(bytes.try_into().expect("fixed header slice"))
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().expect("fixed header slice"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_frame_binds_every_identity_field() {
        let binding = FrameBinding {
            run: [7; 16],
            identity: SpillIdentity::new([8; 32], [9; 32], [10; 32]),
        };
        let key = EphemeralKey::from_test_byte(11);
        let mut scratch = Vec::with_capacity(encoded_len(3).unwrap());
        seal(&mut scratch, &key, binding, 12, b"rdf").unwrap();
        assert_eq!(scratch.len(), HEADER_LEN + 3 + TAG_LEN);
        assert_eq!(scratch[VERSION_OFFSET], 1);
        assert_eq!(
            inspect_header((&scratch[..HEADER_LEN]).try_into().unwrap(), binding, 12, 3),
            Ok(scratch.len())
        );
        assert_eq!(
            open(&mut scratch, &key, binding, 12, 3).unwrap(),
            HEADER_LEN..HEADER_LEN + 3
        );
        assert_eq!(&scratch[HEADER_LEN..HEADER_LEN + 3], b"rdf");
    }

    #[test]
    fn key_erasure_primitive_overwrites_all_bytes() {
        let mut key = EphemeralKey::from_test_byte(0xa5);
        key.erase();
        assert!(key.is_erased());
    }

    #[test]
    fn partial_entropy_failure_is_erased_before_error_return() {
        let mut key = EphemeralKey::from_test_byte(0xa5);
        let result = key.fill_with(|output| {
            output[..7].fill(0x5a);
            Err::<(), ()>(())
        });
        assert_eq!(result, Err(SpillError::EntropyUnavailable));
        assert!(key.is_erased());
    }

    #[test]
    fn entropy_is_filled_at_the_stable_owner_address() {
        let mut fill_address = std::ptr::null();
        let key = EphemeralKey::generate_with(|output| {
            fill_address = output.as_ptr();
            output.fill(0x5a);
            Ok::<(), ()>(())
        })
        .unwrap();
        assert_eq!(fill_address, key.bytes().as_ptr());

        let moved_owner = (key,);
        assert_eq!(fill_address, moved_owner.0.bytes().as_ptr());
    }

    #[test]
    fn rustcrypto_cipher_working_key_is_zeroized_on_drop() {
        fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<XChaCha20Poly1305>();
    }

    #[test]
    fn aad_authenticates_run_query_operator_and_schema_fields() {
        let binding = FrameBinding {
            run: [1; 16],
            identity: SpillIdentity::new([2; 32], [3; 32], [4; 32]),
        };
        let key = EphemeralKey::from_test_byte(5);
        let capacity = encoded_len(7).unwrap();
        let mut original = Vec::with_capacity(capacity);
        seal(&mut original, &key, binding, 0, b"binding").unwrap();

        for field in [RUN_ID_OFFSET, QUERY_OFFSET, OPERATOR_OFFSET, SCHEMA_OFFSET] {
            let mut changed = original.clone();
            changed[field] ^= 1;
            let mut expected = binding;
            match field {
                RUN_ID_OFFSET => {
                    expected.run[0] ^= 1;
                    changed[NONCE_OFFSET] ^= 1;
                }
                QUERY_OFFSET => expected.identity.query[0] ^= 1,
                OPERATOR_OFFSET => expected.identity.operator[0] ^= 1,
                SCHEMA_OFFSET => expected.identity.schema[0] ^= 1,
                _ => unreachable!(),
            }
            assert_eq!(
                open(&mut changed, &key, expected, 0, 7),
                Err(SpillError::AuthenticationFailed),
                "header field at offset {field} must be AEAD-associated data"
            );
        }
    }
}
