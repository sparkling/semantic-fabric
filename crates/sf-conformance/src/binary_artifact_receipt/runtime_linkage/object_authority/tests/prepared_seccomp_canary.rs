use super::prepared_seccomp::{native_bwrap, native_seccomp};

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};

use crate::binary_artifact_receipt::runtime_linkage::object_authority::prepared_probe::execute_seccomp_canary_for_test;

const ELF_CODE_OFFSET: usize = 120;
const ALLOWED_CONTROL_SYSCALL: u32 = 5; // fstat; bad pointer returns EFAULT
const FORBIDDEN_SOCKET_SYSCALL: u32 = 41;
const EXIT_GROUP_SYSCALL: u32 = 231;

pub(super) fn run_native_canary() {
    // This positive control uses the identical ELF and bwrap/FD/root shape.
    // It proves PID-1 survives the fixed descriptor topology before the sole
    // syscall-number delta below is expected to trigger SIGSYS.
    let control = sealed_memfd(&syscall_canary_elf(ALLOWED_CONTROL_SYSCALL));
    let output =
        execute_seccomp_canary_for_test(native_bwrap(), native_seccomp(), control).unwrap();
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());

    let forbidden = sealed_memfd(&syscall_canary_elf(FORBIDDEN_SOCKET_SYSCALL));
    let error =
        execute_seccomp_canary_for_test(native_bwrap(), native_seccomp(), forbidden).unwrap_err();
    assert!(error.contains("exit status: 159"), "{error}");
}

#[test]
fn control_and_socket_canaries_differ_only_by_the_tested_syscall() {
    assert_eq!(
        i64::from(ALLOWED_CONTROL_SYSCALL),
        libc::SYS_fstat,
        "control must remain the independently named allowed fstat syscall"
    );
    assert_eq!(
        i64::from(FORBIDDEN_SOCKET_SYSCALL),
        libc::SYS_socket,
        "negative canary must remain the independently named socket syscall"
    );
    assert_eq!(i64::from(EXIT_GROUP_SYSCALL), libc::SYS_exit_group);
    let control = syscall_canary_elf(ALLOWED_CONTROL_SYSCALL);
    let socket = syscall_canary_elf(FORBIDDEN_SOCKET_SYSCALL);
    let mut expected = control.clone();
    expected[ELF_CODE_OFFSET + 1..ELF_CODE_OFFSET + 5]
        .copy_from_slice(&FORBIDDEN_SOCKET_SYSCALL.to_le_bytes());
    assert_eq!(socket, expected);
    assert_eq!(
        &socket[ELF_CODE_OFFSET + 21..ELF_CODE_OFFSET + 26],
        &[0xb8, 0xe7, 0, 0, 0]
    );
}

fn sealed_memfd(bytes: &[u8]) -> File {
    // SAFETY: the static name is NUL terminated and the returned descriptor is owned.
    let descriptor = unsafe {
        libc::memfd_create(
            c"semantic-fabric-seccomp-canary".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    assert!(descriptor >= 0);
    // SAFETY: ownership of the new descriptor transfers to File.
    let mut file = unsafe { File::from_raw_fd(descriptor) };
    file.write_all(bytes).unwrap();
    let seals = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    // SAFETY: fcntl applies seals to the live owned memfd.
    assert_eq!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) },
        0
    );
    file
}

fn syscall_canary_elf(syscall: u32) -> Vec<u8> {
    let mut code = [
        0xb8, 0, 0, 0, 0, 0xbf, 2, 0, 0, 0, 0xbe, 1, 0, 0, 0, 0x31, 0xd2, 0x0f, 0x05, 0x31, 0xff,
        0xb8, 0, 0, 0, 0, 0x0f, 0x05,
    ];
    code[1..5].copy_from_slice(&syscall.to_le_bytes());
    code[22..26].copy_from_slice(&EXIT_GROUP_SYSCALL.to_le_bytes());
    let file_size = ELF_CODE_OFFSET + code.len();
    let mut elf = vec![0; ELF_CODE_OFFSET];
    elf[..8].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0]);
    put_u16(&mut elf, 16, 2);
    put_u16(&mut elf, 18, 62);
    put_u32(&mut elf, 20, 1);
    put_u64(&mut elf, 24, 0x400078);
    put_u64(&mut elf, 32, 64);
    put_u16(&mut elf, 52, 64);
    put_u16(&mut elf, 54, 56);
    put_u16(&mut elf, 56, 1);
    put_u32(&mut elf, 64, 1);
    put_u32(&mut elf, 68, 5);
    put_u64(&mut elf, 80, 0x400000);
    put_u64(&mut elf, 88, 0x400000);
    put_u64(&mut elf, 96, file_size as u64);
    put_u64(&mut elf, 104, file_size as u64);
    put_u64(&mut elf, 112, 0x1000);
    elf.extend_from_slice(&code);
    elf
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
