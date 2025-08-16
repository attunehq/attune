use attune_macros::workspace_root;

pub const WORKSPACE_ROOT: &str = workspace_root!();

pub const TEST_PACKAGE_AMD64: &[u8] =
    include_bytes!("../../../../scripts/fixtures/attune-test-package_2.0.0_linux_amd64.deb");
pub const TEST_PACKAGE_ARM64: &[u8] =
    include_bytes!("../../../../scripts/fixtures/attune-test-package_2.0.0_linux_arm64.deb");
