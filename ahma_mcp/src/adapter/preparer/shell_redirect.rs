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
