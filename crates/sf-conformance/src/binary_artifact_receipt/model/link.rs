#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkInputOrigin {
    Workspace,
    CargoRegistry,
    RustSysroot,
    BuildOutput,
    HostSystem,
}

impl LinkInputOrigin {
    pub fn name(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::CargoRegistry => "cargo-registry",
            Self::RustSysroot => "rust-sysroot",
            Self::BuildOutput => "build-output",
            Self::HostSystem => "host-system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "workspace" => Some(Self::Workspace),
            "cargo-registry" => Some(Self::CargoRegistry),
            "rust-sysroot" => Some(Self::RustSysroot),
            "build-output" => Some(Self::BuildOutput),
            "host-system" => Some(Self::HostSystem),
            _ => None,
        }
    }

    pub(super) fn path_prefix(self) -> &'static str {
        match self {
            Self::Workspace => "workspace/",
            Self::CargoRegistry => "cargo-registry/",
            Self::RustSysroot => "rust-sysroot/",
            Self::BuildOutput => "build-output/",
            Self::HostSystem => "host-system/",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LinkInput {
    pub origin: LinkInputOrigin,
    pub logical_path: String,
    pub byte_length: u64,
    pub sha256: String,
}

/// One observed final-leaf HostSystem alias and its canonical terminal input.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LinkInputAlias {
    pub alias_logical_path: String,
    pub terminal_logical_path: String,
    pub hop_count: u8,
    pub resolution_sha256: String,
}
