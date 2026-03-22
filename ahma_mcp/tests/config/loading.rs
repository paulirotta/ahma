#[cfg(test)]
mod config_tests {
    use ahma_mcp::{config::load_tool_configs_sync, utils::logging::init_test_logging};
    use std::path::Path;

    #[test]
    fn test_config_loading() {
        init_test_logging();
        println!("Testing configuration loading...");
        let tools_dir = Path::new(".ahma");
        match load_tool_configs_sync(&ahma_mcp::shell::cli::AppConfig::default(), Some(tools_dir)) {
            Ok(configs) => {
                println!("Successfully loaded {} configurations:", configs.len());
                for (name, config) in configs.iter() {
                    println!(
                        "  - {}: {} subcommands",
                        name,
                        config.subcommand.as_deref().unwrap_or_default().len()
                    );
                }
            }
            Err(e) => {
                eprintln!("Failed to load configurations: {}", e);
                eprintln!("Error chain:");
                let mut source = e.source();
                while let Some(err) = source {
                    eprintln!("  caused by: {}", err);
                    source = err.source();
                }
                panic!("Configuration loading failed: {}", e);
            }
        }
    }
}
