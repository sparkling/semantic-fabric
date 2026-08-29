use super::{PreparedRuntimeObservation, PREPARED_LOADER_POLICY};
use crate::binary_artifact_receipt::runtime_elf::RuntimeElfRole;
use crate::binary_artifact_receipt::runtime_linkage::prepared_receipt::{
    PreparedRuntimeReceipt, RecordedBindingIdentity, RecordedObjectIdentity,
    RecordedPreparedRuntimeObservation, RecordedRuntimeRole,
};

impl PreparedRuntimeObservation {
    pub(in super::super) fn to_non_admission_receipt(
        &self,
    ) -> Result<PreparedRuntimeReceipt, String> {
        if self.loader_policy != PREPARED_LOADER_POLICY {
            return Err("prepared observation loader policy drift".to_owned());
        }
        let bwrap_path = self
            .bwrap_path
            .to_str()
            .ok_or_else(|| "prepared observation bubblewrap path is not UTF-8".to_owned())?;
        let bindings = self
            .bindings
            .iter()
            .map(|binding| RecordedBindingIdentity {
                object: RecordedObjectIdentity {
                    logical_path: binding.object.logical_path.clone(),
                    role: recorded_role(binding.object.role),
                    soname: binding.object.soname.clone(),
                    device: binding.object.device,
                    inode: binding.object.inode,
                    byte_length: binding.object.byte_length,
                    sha256: binding.object.sha256.clone(),
                },
                destination: binding.destination.clone(),
                mode: binding.mode,
            })
            .collect();
        PreparedRuntimeReceipt::from_recorded(RecordedPreparedRuntimeObservation {
            view: self.view.clone(),
            bindings,
            bwrap_sha256: self.bwrap_sha256.clone(),
            bwrap_byte_length: self.bwrap_byte_length,
            bwrap_path: bwrap_path.to_owned(),
            bwrap_executable_policy: self.bwrap_executable_policy.clone(),
            stdout: self.stdout.clone(),
            stdout_sha256: self.stdout_sha256.clone(),
        })
    }
}

fn recorded_role(role: RuntimeElfRole) -> RecordedRuntimeRole {
    match role {
        RuntimeElfRole::RootPie => RecordedRuntimeRole::RootPie,
        RuntimeElfRole::Loader => RecordedRuntimeRole::Loader,
        RuntimeElfRole::Libc => RecordedRuntimeRole::Libc,
        RuntimeElfRole::SharedObject => RecordedRuntimeRole::SharedObject,
    }
}
