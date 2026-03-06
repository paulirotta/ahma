use anyhow::Result;
use std::path::Path;

use super::core::Sandbox;

impl Sandbox {
    pub(super) fn build_macos_sandbox_command(
        &self,
        command: &[String],
        working_dir: &Path,
    ) -> Result<(String, Vec<String>)> {
        let profile = self.generate_seatbelt_profile(working_dir);

        let mut args = vec!["-p".to_string(), profile];
        args.extend(command.iter().cloned());

        Ok(("sandbox-exec".to_string(), args))
    }

    fn generate_seatbelt_profile(&self, working_dir: &Path) -> String {
        let wd_str = working_dir.to_string_lossy();
        let scope_rules = self.get_macos_scope_rules();
        let read_scopes_rules = self.get_macos_read_scopes_rules();
        let system_rules = self.get_macos_system_rules();
        let user_tool_rules = self.get_macos_user_tool_rules();
        let temp_rules = self.get_macos_temp_rules();

        let profile = format!(
            r#"(version 1)
(deny default)
(allow process*)
(allow signal)
(allow sysctl-read)
{system_rules}{user_tool_rules}{scope_rules}{read_scopes_rules}(allow file-read* (subpath "{working_dir}"))
(allow file-write* (subpath "{working_dir}"))
{temp_rules}(allow file-read* (literal "/dev/null"))
(allow file-write* (literal "/dev/null"))
(allow file-read* (literal "/dev/tty"))
(allow file-write* (literal "/dev/tty"))
(allow file-read* (literal "/dev/zero"))
(allow file-write* (literal "/dev/zero"))
(allow network*)
(allow mach-lookup)
(allow ipc-posix-shm*)
"#,
            working_dir = wd_str,
            system_rules = system_rules,
            user_tool_rules = user_tool_rules,
            scope_rules = scope_rules,
            read_scopes_rules = read_scopes_rules,
            temp_rules = temp_rules,
        );

        tracing::debug!("Generated macOS Sandbox (Seatbelt) profile:\n{}", profile);
        profile
    }

    fn get_macos_scope_rules(&self) -> String {
        let mut rules = String::new();
        for scope in self.scopes.read().unwrap().iter() {
            rules.push_str(&format!(
                "(allow file-read* (subpath \"{}\"))\n(allow file-write* (subpath \"{}\"))\n",
                scope.display(),
                scope.display()
            ));
        }
        rules
    }

    fn get_macos_read_scopes_rules(&self) -> String {
        let mut rules = String::new();
        for scope in &self.read_scopes {
            rules.push_str(&format!(
                "(allow file-read* (subpath \"{}\"))\n",
                scope.display()
            ));
        }
        rules
    }

    fn get_macos_system_rules(&self) -> String {
        "(allow file-read* (subpath \"/usr\"))\n\
         (allow file-read* (subpath \"/bin\"))\n\
         (allow file-read* (subpath \"/sbin\"))\n\
         (allow file-read* (subpath \"/etc\"))\n\
         (allow file-read* (subpath \"/lib\"))\n\
         (allow file-read* (subpath \"/System\"))\n\
         (allow file-read* (subpath \"/Library\"))\n\
         (allow file-read* (subpath \"/Applications\"))\n"
            .to_string()
    }

    fn get_macos_user_tool_rules(&self) -> String {
        let home_dir = std::env::var("HOME").unwrap_or_else(|_| "/Users/Shared".to_string());
        let home_path = std::path::Path::new(&home_dir);
        let mut rules = String::new();

        let tool_paths = [".cargo", ".rustup"];
        for tool in &tool_paths {
            let path = home_path.join(tool);
            if path.exists() {
                rules.push_str(&format!(
                    "(allow file-read* (subpath \"{}\"))\n",
                    path.display()
                ));
            }
        }
        rules
    }

    fn get_macos_temp_rules(&self) -> String {
        if self.no_temp_files {
            String::new()
        } else {
            "(allow file-read* (subpath \"/private/tmp\"))\n\
             (allow file-write* (subpath \"/private/tmp\"))\n\
             (allow file-read* (subpath \"/private/var/folders\"))\n\
             (allow file-write* (subpath \"/private/var/folders\"))\n"
                .to_string()
        }
    }
}
