use anyhow::Result;
use std::path::PathBuf;

/// Apply Landlock sandbox restrictions to the current process.
#[cfg(target_os = "linux")]
pub fn enforce_landlock_sandbox(
    scopes: &[PathBuf],
    read_scopes: &[PathBuf],
    no_temp_files: bool,
) -> Result<()> {
    use anyhow::Context;
    use landlock::{
        ABI, Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
    };

    let abi = ABI::V1.best_supported();
    let access_all = AccessFs::from_all(abi);
    let access_read = AccessFs::from_read(abi);

    tracing::info!("Enforcing Landlock sandbox using ABI: {:?}", abi);

    let mut ruleset = Ruleset::default()
        .handle_access(access_all)
        .context("Failed to create Landlock ruleset")?
        .create()
        .context("Failed to create Landlock ruleset instance")?;

    // Add sandbox scopes
    for scope in scopes {
        ruleset = ruleset
            .add_rule(PathBeneath::new(
                PathFd::new(scope).context("Failed to open sandbox scope for Landlock")?,
                access_all,
            ))
            .context("Failed to add Landlock rule for sandbox scope")?;
    }

    // Add explicit read_scopes target files (for --livelog)
    for read_scope in read_scopes {
        if let Ok(fd) = PathFd::new(read_scope) {
            ruleset = ruleset
                .add_rule(PathBeneath::new(fd, access_read))
                .context("Failed to add Landlock rule for read-only scope")?;
        }
    }

    add_landlock_system_rules(&mut ruleset, access_read)?;
    add_landlock_home_tool_rules(&mut ruleset, access_read)?;

    if !no_temp_files {
        add_landlock_temp_rules(&mut ruleset, access_all)?;
    }

    let status = ruleset
        .restrict_self()
        .context("Failed to apply Landlock restrictions")?;

    if status.ruleset == landlock::RestrictionStatus::No {
        return Err(anyhow::anyhow!(
            "Failed to enforce Landlock sandbox: enforcement was refused by kernel (status: {:?}). \
             Ensure your kernel supports Landlock and the process has sufficient privileges.",
            status
        ));
    }

    tracing::info!(
        "Landlock sandbox enforced for scopes: {:?} (status: {:?})",
        scopes,
        status
    );

    Ok(())
}

#[cfg(target_os = "linux")]
fn add_landlock_system_rules(
    ruleset: &mut landlock::RulesetCreated,
    access_read: landlock::BitFlags<landlock::AccessFs>,
) -> Result<()> {
    use landlock::{AccessFs, PathBeneath, PathFd, RulesetCreatedAttr};
    let system_paths = [
        "/usr", "/bin", "/sbin", "/etc", "/lib", "/lib64", "/proc", "/dev", "/sys",
    ];
    let access_read_execute = access_read | AccessFs::Execute;
    for path in &system_paths {
        let path_obj = std::path::Path::new(path);
        if path_obj.exists()
            && let Ok(fd) = PathFd::new(path_obj)
        {
            let _ = ruleset.add_rule(PathBeneath::new(fd, access_read_execute));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn add_landlock_home_tool_rules(
    ruleset: &mut landlock::RulesetCreated,
    access_read: landlock::BitFlags<landlock::AccessFs>,
) -> Result<()> {
    use landlock::{PathBeneath, PathFd, RulesetCreatedAttr};
    if let Ok(home) = std::env::var("HOME") {
        let home_path = std::path::Path::new(&home);
        let tool_paths = [".cargo", ".rustup", ".nvm", ".npm", ".go", ".cache"];
        for tool in &tool_paths {
            let path = home_path.join(tool);
            if path.exists()
                && let Ok(fd) = PathFd::new(&path)
            {
                let _ = ruleset.add_rule(PathBeneath::new(fd, access_read));
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn add_landlock_temp_rules(
    ruleset: &mut landlock::RulesetCreated,
    access_all: landlock::BitFlags<landlock::AccessFs>,
) -> Result<()> {
    use landlock::{PathBeneath, PathFd, RulesetCreatedAttr};
    let tmp_path = std::path::Path::new("/tmp");
    if tmp_path.exists()
        && let Ok(fd) = PathFd::new(tmp_path)
    {
        let _ = ruleset.add_rule(PathBeneath::new(fd, access_all));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn enforce_landlock_sandbox(
    _scopes: &[PathBuf],
    _read_scopes: &[PathBuf],
    _no_temp_files: bool,
) -> Result<()> {
    Ok(())
}
