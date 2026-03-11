use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    Strict,
    Test,
}

/// A guard that holds a read lock on the sandbox scopes.
pub struct ScopesGuard<'a>(pub(super) std::sync::RwLockReadGuard<'a, Vec<PathBuf>>);

impl std::ops::Deref for ScopesGuard<'_> {
    type Target = [PathBuf];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Debug for ScopesGuard<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_mode_equality() {
        assert_eq!(SandboxMode::Strict, SandboxMode::Strict);
        assert_eq!(SandboxMode::Test, SandboxMode::Test);
        assert_ne!(SandboxMode::Strict, SandboxMode::Test);
    }

    #[test]
    fn test_sandbox_mode_copy() {
        let m = SandboxMode::Strict;
        let m2 = m; // copy
        assert_eq!(m, m2);
    }

    #[test]
    fn test_sandbox_mode_clone() {
        let m = SandboxMode::Test;
        #[allow(clippy::clone_on_copy)]
        let m2 = m.clone();
        assert_eq!(m, m2);
    }

    #[test]
    fn test_sandbox_mode_debug() {
        let strict = format!("{:?}", SandboxMode::Strict);
        let test = format!("{:?}", SandboxMode::Test);
        assert!(strict.contains("Strict"));
        assert!(test.contains("Test"));
    }
}
