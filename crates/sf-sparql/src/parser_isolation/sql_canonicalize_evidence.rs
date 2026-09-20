//! Closed controls and observations for SQL child lifecycle evidence.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SqlCanonicalizeEvidenceMode {
    HoldAfterRequest = 1,
    MalformedResult = 2,
    TruncatedResult = 3,
    OversizedResult = 4,
}

impl SqlCanonicalizeEvidenceMode {
    pub(crate) const fn encode(self) -> [u8; 1] {
        [self as u8]
    }

    pub(crate) const fn decode(encoded: [u8; 1]) -> Option<Self> {
        match encoded[0] {
            1 => Some(Self::HoldAfterRequest),
            2 => Some(Self::MalformedResult),
            3 => Some(Self::TruncatedResult),
            4 => Some(Self::OversizedResult),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct SqlCanonicalizeEvidenceState {
    request_ready: AtomicBool,
    pidfd: Mutex<Option<OwnedFd>>,
}

impl SqlCanonicalizeEvidenceState {
    pub fn request_ready(&self) -> bool {
        self.request_ready.load(Ordering::Acquire)
    }

    pub fn child_is_live(&self) -> Option<bool> {
        let guard = self.pidfd.lock().ok()?;
        let pidfd = guard.as_ref()?;
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd.as_raw_fd(),
                0,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            Some(true)
        } else {
            Some(std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH))
        }
    }

    pub(crate) fn mark_request_ready(&self, pidfd: OwnedFd) {
        *self.pidfd.lock().expect("SQL evidence state lock") = Some(pidfd);
        self.request_ready.store(true, Ordering::Release);
    }
}
