pub(super) fn maybe_append_shell_redirect(program: &str, args: &mut Vec<String>) {
    if let Some(idx) = shell_script_index(program, args.as_slice())
        && let Some(script) = args.get_mut(idx)
    {
        ensure_shell_redirect(script);
    }
}

fn shell_script_index(program: &str, args: &[String]) -> Option<usize> {
    if !is_shell_program(program) {
        return None;
    }
    // Unix shells use `-c <script>`; PowerShell uses `-Command <script>`.
    let command_idx = args
        .iter()
        .position(|a| a == "-c" || a.eq_ignore_ascii_case("-command"))?;
    let script_idx = command_idx + 1;
    if script_idx < args.len() {
        Some(script_idx)
    } else {
        None
    }
}

fn ensure_shell_redirect(script: &mut String) {
    if script.trim_end().ends_with("2>&1") {
        return;
    }

    let needs_space = script
        .chars()
        .last()
        .map(|c| !c.is_whitespace())
        .unwrap_or(false);

    if needs_space {
        script.push(' ');
    }
    script.push_str("2>&1");
}

fn is_shell_program(program: &str) -> bool {
    // Strip optional `.exe` suffix (Windows) for comparison.
    let base = program
        .rsplit_once('.')
        .filter(|(_, ext)| ext.eq_ignore_ascii_case("exe"))
        .map(|(stem, _)| stem)
        .unwrap_or(program);
    // Last path component only (handles `/bin/bash`, `C:\Windows\pwsh`, etc.)
    let name = base
        .rsplit_once(['/', '\\'])
        .map(|(_, n)| n)
        .unwrap_or(base);
    matches!(
        name,
        "sh" | "bash" | "zsh" | "fish" | "ksh" | "pwsh" | "powershell" | "cmd"
    )
}

#[cfg(test)]
mod tests {
    use super::maybe_append_shell_redirect;

    fn run_maybe_append(program: &str, args: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        maybe_append_shell_redirect(program, &mut v);
        v
    }

    #[test]
    fn non_shell_program_unchanged() {
        let args = run_maybe_append("git", &["status"]);
        assert_eq!(args, vec!["status"]);
    }

    #[test]
    fn shell_sh_adds_redirect() {
        let args = run_maybe_append("/bin/sh", &["-c", "echo hi"]);
        assert_eq!(args, vec!["-c", "echo hi 2>&1"]);
    }

    #[test]
    fn shell_bash_adds_redirect() {
        let args = run_maybe_append("/usr/bin/bash", &["-c", "ls"]);
        assert_eq!(args, vec!["-c", "ls 2>&1"]);
    }

    #[test]
    fn shell_zsh_adds_redirect() {
        let args = run_maybe_append("zsh", &["-c", "echo test"]);
        assert_eq!(args, vec!["-c", "echo test 2>&1"]);
    }

    #[test]
    fn shell_fish_adds_redirect() {
        let args = run_maybe_append("fish", &["-c", "pwd"]);
        assert_eq!(args, vec!["-c", "pwd 2>&1"]);
    }

    #[test]
    fn shell_ksh_adds_redirect() {
        let args = run_maybe_append("ksh", &["-c", "echo ksh"]);
        assert_eq!(args, vec!["-c", "echo ksh 2>&1"]);
    }

    #[test]
    fn shell_script_already_has_redirect_no_duplicate() {
        let args = run_maybe_append("sh", &["-c", "echo hi 2>&1"]);
        assert_eq!(args, vec!["-c", "echo hi 2>&1"]);
    }

    #[test]
    fn shell_script_with_trailing_space_gets_redirect() {
        let args = run_maybe_append("sh", &["-c", "echo hi "]);
        assert_eq!(args, vec!["-c", "echo hi 2>&1"]);
    }

    #[test]
    fn powershell_command_flag_adds_redirect() {
        let args = run_maybe_append("pwsh", &["-Command", "Write-Output x"]);
        assert_eq!(args, vec!["-Command", "Write-Output x 2>&1"]);
    }

    #[test]
    fn powershell_command_case_insensitive() {
        let args = run_maybe_append("powershell", &["-command", "Write-Host y"]);
        assert_eq!(args, vec!["-command", "Write-Host y 2>&1"]);
    }

    #[test]
    fn windows_exe_suffix_still_recognized() {
        let args = run_maybe_append("bash.exe", &["-c", "echo win"]);
        assert_eq!(args, vec!["-c", "echo win 2>&1"]);
    }

    #[test]
    fn windows_path_with_backslash() {
        let args = run_maybe_append("C:\\Windows\\pwsh.exe", &["-Command", "echo z"]);
        assert_eq!(args, vec!["-Command", "echo z 2>&1"]);
    }

    #[test]
    fn cmd_exe_recognized() {
        let args = run_maybe_append("cmd.exe", &["/c", "echo cmd"]);
        assert_eq!(args, vec!["/c", "echo cmd"]);
    }

    #[test]
    fn no_script_after_minus_c_unchanged() {
        let args = run_maybe_append("sh", &["-c"]);
        assert_eq!(args, vec!["-c"]);
    }

    #[test]
    fn no_minus_c_unchanged() {
        let args = run_maybe_append("sh", &["-e", "script"]);
        assert_eq!(args, vec!["-e", "script"]);
    }
}
