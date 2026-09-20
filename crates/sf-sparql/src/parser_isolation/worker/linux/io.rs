//! Allocation-free worker pipe primitives.

use super::WorkerFailure;

pub(super) fn read_exact(descriptor: libc::c_int, output: &mut [u8]) -> Result<(), WorkerFailure> {
    let mut offset = 0;
    while offset < output.len() {
        let count = unsafe {
            libc::read(
                descriptor,
                output[offset..].as_mut_ptr().cast(),
                output.len() - offset,
            )
        };
        if count > 0 {
            offset += count as usize;
        } else if count < 0
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
        {
            continue;
        } else {
            return Err(WorkerFailure);
        }
    }
    Ok(())
}

pub(super) fn write_all(descriptor: libc::c_int, input: &[u8]) -> Result<(), WorkerFailure> {
    let mut offset = 0;
    while offset < input.len() {
        let count = unsafe {
            libc::write(
                descriptor,
                input[offset..].as_ptr().cast(),
                input.len() - offset,
            )
        };
        if count > 0 {
            offset += count as usize;
        } else if count < 0
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
        {
            continue;
        } else {
            return Err(WorkerFailure);
        }
    }
    Ok(())
}

pub(super) fn require_parent_eof() -> Result<(), WorkerFailure> {
    let mut unexpected = [0_u8; 1];
    loop {
        let count = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                unexpected.as_mut_ptr().cast(),
                unexpected.len(),
            )
        };
        if count == 0 {
            return Ok(());
        }
        if count > 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(WorkerFailure);
        }
    }
}
