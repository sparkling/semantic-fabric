//! Bounded ELF GNU build-ID observation shared by supervisor and worker.

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::os::unix::fs::MetadataExt;

use sha2::{Digest, Sha256};

use super::protocol::BuildIdentityDigest;

const ELF_HEADER_LEN: usize = 64;
const ELF64_PROGRAM_HEADER_LEN: usize = 56;
const MAX_PROGRAM_HEADERS: usize = 128;
const MAX_NOTE_SEGMENT_BYTES: usize = 64 * 1024;
const PT_NOTE: u32 = 4;
const NT_GNU_BUILD_ID: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    len: u64,
    mode: u32,
    inode: u64,
    device: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

/// Observe one canonical GNU build ID without hashing the full executable.
///
/// This identifier correlates the descriptor-launched image with the worker's
/// loaded image. It is not a signature, provenance proof, or dynamic-closure
/// attestation.
pub(super) fn observe(file: &File) -> Option<BuildIdentityDigest> {
    let metadata_before = metadata_identity(file)?;
    let file_len = metadata_before.len;
    if file_len == 0
        || metadata_before.mode & libc::S_IFMT != libc::S_IFREG
        || metadata_before.mode & 0o111 == 0
    {
        return None;
    }
    let mut header = [0_u8; ELF_HEADER_LEN];
    file.read_exact_at(&mut header, 0).ok()?;
    if &header[..4] != b"\x7fELF"
        || header[4] != 2
        || header[5] != 1
        || header[6] != 1
        || !matches!(read_u16(&header, 16), 2 | 3)
        || read_u16(&header, 18) != 62
        || read_u32(&header, 20) != 1
        || usize::from(read_u16(&header, 52)) != ELF_HEADER_LEN
        || usize::from(read_u16(&header, 54)) != ELF64_PROGRAM_HEADER_LEN
    {
        return None;
    }

    let table_offset = read_u64(&header, 32);
    let header_count = usize::from(read_u16(&header, 56));
    if header_count == 0 || header_count > MAX_PROGRAM_HEADERS {
        return None;
    }
    let table_len = header_count.checked_mul(ELF64_PROGRAM_HEADER_LEN)?;
    let table_end = table_offset.checked_add(u64::try_from(table_len).ok()?)?;
    if table_end > file_len {
        return None;
    }
    let mut table = vec![0_u8; table_len];
    file.read_exact_at(&mut table, table_offset).ok()?;

    let mut observed = None;
    for entry in table.chunks_exact(ELF64_PROGRAM_HEADER_LEN) {
        if read_u32(entry, 0) != PT_NOTE {
            continue;
        }
        let note_offset = read_u64(entry, 8);
        let note_len = usize::try_from(read_u64(entry, 32)).ok()?;
        if note_len == 0 || note_len > MAX_NOTE_SEGMENT_BYTES {
            return None;
        }
        let note_end = note_offset.checked_add(u64::try_from(note_len).ok()?)?;
        if note_end > file_len {
            return None;
        }
        let mut notes = vec![0_u8; note_len];
        file.read_exact_at(&mut notes, note_offset).ok()?;
        parse_notes(&notes, &mut observed)?;
    }
    if metadata_identity(file)? != metadata_before {
        return None;
    }
    observed
}

fn metadata_identity(file: &File) -> Option<FileIdentity> {
    let metadata = file.metadata().ok()?;
    Some(FileIdentity {
        len: metadata.len(),
        mode: metadata.mode(),
        inode: metadata.ino(),
        device: metadata.dev(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
}

fn parse_notes(notes: &[u8], observed: &mut Option<BuildIdentityDigest>) -> Option<()> {
    let mut offset = 0;
    while offset < notes.len() {
        if notes.len() - offset < 12 {
            return None;
        }
        let name_len = usize::try_from(read_u32(notes, offset)).ok()?;
        let description_len = usize::try_from(read_u32(notes, offset + 4)).ok()?;
        let note_type = read_u32(notes, offset + 8);
        let name_offset = offset.checked_add(12)?;
        let description_offset = name_offset.checked_add(align_four(name_len)?)?;
        let next = description_offset.checked_add(align_four(description_len)?)?;
        if next > notes.len()
            || name_offset.checked_add(name_len)? > notes.len()
            || description_offset.checked_add(description_len)? > notes.len()
        {
            return None;
        }
        if note_type == NT_GNU_BUILD_ID && &notes[name_offset..name_offset + name_len] == b"GNU\0" {
            if !(16..=64).contains(&description_len) || observed.is_some() {
                return None;
            }
            let mut digest = Sha256::new();
            digest.update(b"semantic-fabric/elf-gnu-build-id/v1\0");
            digest.update(&notes[description_offset..description_offset + description_len]);
            *observed = Some(BuildIdentityDigest::new(digest.finalize().into()));
        }
        offset = next;
    }
    Some(())
}

fn align_four(value: usize) -> Option<usize> {
    value.checked_add(3).map(|aligned| aligned & !3)
}

fn read_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        input[offset..offset + 2]
            .try_into()
            .expect("bounded ELF u16"),
    )
}

fn read_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        input[offset..offset + 4]
            .try_into()
            .expect("bounded ELF u32"),
    )
}

fn read_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        input[offset..offset + 8]
            .try_into()
            .expect("bounded ELF u64"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_test_image_has_one_bounded_build_identity() {
        let executable = File::open("/proc/self/exe").expect("open test image");
        let first = observe(&executable).expect("observe current GNU build ID");
        assert_eq!(observe(&executable), Some(first));
        assert_eq!(format!("{first:?}"), "BuildIdentityDigest(<redacted>)");
    }

    #[test]
    fn malformed_note_lengths_fail_closed() {
        let mut observed = None;
        let mut note = [0_u8; 12];
        note[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse_notes(&note, &mut observed), None);
        assert_eq!(observed, None);
    }
}
