use ahma_mcp::sandbox::{Sandbox, SandboxMode};
use ahma_mcp::test_utils::path_helpers::{test_out_of_scope_path, test_temp_path};

#[test]
fn test_high_security_mode_enforcement() {
    // Use the crate directory as a non-temp scope so that paths inside it
    // are not rejected by the no_temp_files policy.
    let scope = dunce::canonicalize(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap();

    let sandbox = Sandbox::new(vec![scope.clone()], SandboxMode::Strict, true, false).unwrap();
    assert!(sandbox.is_no_temp_files());

    // 1. Path inside the (non-temp) scope should be allowed
    let valid_path = scope.join("Cargo.toml");
    let result = sandbox.validate_path(&valid_path);
    assert!(
        result.is_ok(),
        "Valid path in non-temp scope should be allowed: {:?}",
        result.err()
    );

    // 2. Path in the system temp dir should be blocked (out of scope AND temp)
    let tmp_path = test_temp_path("high_security_test.txt");
    let result = sandbox.validate_path(&tmp_path);
    assert!(
        result.is_err(),
        "Path in system temp dir should be blocked in high security mode"
    );

    // 3. Path completely outside scope should be blocked
    let outside_path = test_out_of_scope_path();
    let result = sandbox.validate_path(&outside_path);
    assert!(
        result.is_err(),
        "Path outside sandbox scope should be blocked"
    );
}
