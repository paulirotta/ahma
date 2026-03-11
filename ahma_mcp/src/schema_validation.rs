//! # Schema Validation Module
//!
//! This module provides schema validation functionality for tool configurations
//! using the MCP Tool Definition Format (MTDF) schema.

use serde_json::Value;
use std::fmt;
use std::path::Path;

// ─────────────────────────────────────────────────────────────────────────────
// Validation helper functions
// ─────────────────────────────────────────────────────────────────────────────

fn push_error(errors: &mut Vec<SchemaValidationError>, error_type: ValidationErrorType, field_path: String, message: String) {
    errors.push(SchemaValidationError {
        error_type,
        field_path,
        message,
        suggestion: None,
    });
}

fn validate_non_empty_field(
    value: &str,
    field_name: &str,
    path: &str,
    errors: &mut Vec<SchemaValidationError>,
) {
    if value.is_empty() {
        push_error(
            errors,
            ValidationErrorType::MissingRequiredField,
            format!("{}.{}", path, field_name),
            format!("{} cannot be empty", capitalize_first(field_name)),
        );
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

const ASYNC_KEYWORDS: &[&str] = &[
    r"\bnotification\b",
    r"\basynchronously\b",
    r"\basync\b",
    r"\bbackground\b",
];

fn check_async_keywords_in_sync_command(description: &str) -> bool {
    let desc_lower = description.to_lowercase();
    ASYNC_KEYWORDS.iter().any(|kw| {
        regex::Regex::new(kw)
            .map(|re| re.is_match(&desc_lower))
            .unwrap_or(false)
    })
}

/// Error types for schema validation
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationErrorType {
    /// A required field is missing
    MissingRequiredField,
    /// A field has an invalid type
    InvalidType,
    /// A field has an invalid format
    InvalidFormat,
    /// An unknown field is present
    UnknownField,
    /// A field value is invalid
    InvalidValue,
    /// Schema violation
    SchemaViolation,
    /// Constraint violation (e.g., min/max values)
    ConstraintViolation,
    /// Logical inconsistency in configuration
    LogicalInconsistency,
}

impl fmt::Display for ValidationErrorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationErrorType::MissingRequiredField => write!(f, "Missing required field"),
            ValidationErrorType::InvalidType => write!(f, "Invalid type"),
            ValidationErrorType::InvalidFormat => write!(f, "Invalid format"),
            ValidationErrorType::UnknownField => write!(f, "Unknown field"),
            ValidationErrorType::InvalidValue => write!(f, "Invalid value"),
            ValidationErrorType::SchemaViolation => write!(f, "Schema violation"),
            ValidationErrorType::ConstraintViolation => write!(f, "Constraint violation"),
            ValidationErrorType::LogicalInconsistency => write!(f, "Logical inconsistency"),
        }
    }
}

/// Represents a schema validation error
#[derive(Debug, Clone)]
pub struct SchemaValidationError {
    /// The type of validation error
    pub error_type: ValidationErrorType,
    /// Path to the field that caused the error
    pub field_path: String,
    /// Human-readable error message
    pub message: String,
    /// Optional suggestion for fixing the error
    pub suggestion: Option<String>,
}

impl fmt::Display for SchemaValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} - {}",
            self.error_type, self.field_path, self.message
        )
    }
}

impl std::error::Error for SchemaValidationError {}

/// Validator for MCP Tool Definition Format (MTDF)
#[derive(Debug, Clone)]
pub struct MtdfValidator {
    /// Enable strict validation mode
    pub strict_mode: bool,
    /// Allow unknown fields in the configuration
    pub allow_unknown_fields: bool,
}

impl Default for MtdfValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl MtdfValidator {
    /// Create a new validator with default settings
    pub fn new() -> Self {
        Self {
            strict_mode: true,
            allow_unknown_fields: false,
        }
    }

    /// Enable or disable strict validation mode
    pub fn with_strict_mode(mut self, strict: bool) -> Self {
        self.strict_mode = strict;
        self
    }

    /// Allow or disallow unknown fields
    pub fn with_unknown_fields_allowed(mut self, allow: bool) -> Self {
        self.allow_unknown_fields = allow;
        self
    }

    /// Validate a tool configuration file
    ///
    /// # Arguments
    /// * `file_path` - Path to the configuration file
    /// * `content` - JSON content as a string
    ///
    /// # Returns
    /// * `Ok(ToolConfig)` if validation succeeds, returning the parsed configuration
    /// * `Err(Vec<SchemaValidationError>)` if validation fails
    pub fn validate_tool_config(
        &self,
        file_path: &Path,
        content: &str,
    ) -> Result<crate::config::ToolConfig, Vec<SchemaValidationError>> {
        use crate::config::ToolConfig;

        let mut errors = Vec::new();

        // Parse as JSON first
        let json_value: Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(e) => {
                errors.push(SchemaValidationError {
                    error_type: ValidationErrorType::InvalidFormat,
                    field_path: file_path.to_string_lossy().to_string(),
                    message: format!("Invalid JSON: {}", e),
                    suggestion: None,
                });
                return Err(errors);
            }
        };

        // Try to deserialize into ToolConfig
        let config = match serde_json::from_value::<ToolConfig>(json_value.clone()) {
            Ok(config) => config,
            Err(e) => {
                errors.push(SchemaValidationError {
                    error_type: ValidationErrorType::SchemaViolation,
                    field_path: file_path.to_string_lossy().to_string(),
                    message: format!("Failed to deserialize: {}", e),
                    suggestion: None,
                });
                return Err(errors);
            }
        };

        // Perform additional validation on the config
        self.validate_tool_config_struct(&config, &mut errors);

        if errors.is_empty() {
            Ok(config)
        } else {
            Err(errors)
        }
    }

    /// Validate a ToolConfig struct
    fn validate_tool_config_struct(
        &self,
        config: &crate::config::ToolConfig,
        errors: &mut Vec<SchemaValidationError>,
    ) {
        // Validate required fields
        if config.name.is_empty() {
            errors.push(SchemaValidationError {
                error_type: ValidationErrorType::MissingRequiredField,
                field_path: "name".to_string(),
                message: "Tool name cannot be empty".to_string(),
                suggestion: None,
            });
        }

        if config.command.is_empty() {
            errors.push(SchemaValidationError {
                error_type: ValidationErrorType::ConstraintViolation,
                field_path: "command".to_string(),
                message: "Command cannot be empty".to_string(),
                suggestion: None,
            });
        }

        if config.description.is_empty() {
            errors.push(SchemaValidationError {
                error_type: ValidationErrorType::MissingRequiredField,
                field_path: "description".to_string(),
                message: "Description cannot be empty".to_string(),
                suggestion: None,
            });
        }

        // Validate timeout constraints
        if let Some(timeout) = config.timeout_seconds {
            if timeout > 3600 {
                errors.push(SchemaValidationError {
                    error_type: ValidationErrorType::ConstraintViolation,
                    field_path: "timeout_seconds".to_string(),
                    message: "Timeout should not exceed 3600 seconds (1 hour)".to_string(),
                    suggestion: Some("Consider using a shorter timeout or breaking the operation into smaller steps".to_string()),
                });
            }
            if timeout == 0 {
                errors.push(SchemaValidationError {
                    error_type: ValidationErrorType::ConstraintViolation,
                    field_path: "timeout_seconds".to_string(),
                    message: "Timeout should be at least 1 second".to_string(),
                    suggestion: Some(
                        "Set a minimum timeout of 1 second for reliable operation detection"
                            .to_string(),
                    ),
                });
            }
        }

        // Validate subcommands if present
        if let Some(ref subcommands) = config.subcommand {
            for (i, subcommand) in subcommands.iter().enumerate() {
                self.validate_subcommand(
                    subcommand,
                    &format!("subcommand[{}]", i),
                    config.synchronous,
                    errors,
                );
            }
        }
    }

    /// Format validation errors into a human-readable report
    ///
    /// # Arguments
    /// * `errors` - Vector of validation errors
    /// * `file_path` - Path to the file being validated
    ///
    /// # Returns
    /// * Formatted error report as a string
    pub fn format_errors(&self, errors: &[SchemaValidationError], file_path: &Path) -> String {
        let mut report = format!("Validation errors in {}:\n\n", file_path.display());

        let error_count = errors.len();
        report.push_str(&format!("Found {} error(s):\n\n", error_count));

        for (i, error) in errors.iter().enumerate() {
            report.push_str(&format!(
                "{}. {} at '{}': {}\n",
                i + 1,
                error.error_type,
                error.field_path,
                error.message
            ));

            if let Some(ref suggestion) = error.suggestion {
                report.push_str(&format!("   Suggestion: {}\n", suggestion));
            }
            report.push('\n');
        }

        // Add general help
        report.push_str("Common fixes:\n");
        report.push_str("- Check the tool configuration schema at docs/tool-schema-guide.md\n");
        report.push_str("- Ensure all required fields are present\n");
        report.push_str("- Verify data types match the expected schema\n");
        report.push_str("- Review suggestions above for specific field corrections\n");

        report
    }

    /// Validate a subcommand configuration
    fn validate_subcommand(
        &self,
        subcommand: &crate::config::SubcommandConfig,
        path: &str,
        tool_synchronous: Option<bool>,
        errors: &mut Vec<SchemaValidationError>,
    ) {
        validate_non_empty_field(&subcommand.name, "name", path, errors);
        validate_non_empty_field(&subcommand.description, "description", path, errors);

        let effective_sync = subcommand.synchronous.or(tool_synchronous);
        self.validate_sync_description_consistency(subcommand, path, effective_sync, errors);
        self.validate_nested_subcommands(subcommand, path, effective_sync, errors);
        self.validate_subcommand_options(subcommand, path, errors);
    }

    fn validate_sync_description_consistency(
        &self,
        subcommand: &crate::config::SubcommandConfig,
        path: &str,
        effective_sync: Option<bool>,
        errors: &mut Vec<SchemaValidationError>,
    ) {
        if effective_sync != Some(true) {
            return;
        }

        if check_async_keywords_in_sync_command(&subcommand.description) {
            errors.push(SchemaValidationError {
                error_type: ValidationErrorType::LogicalInconsistency,
                field_path: format!("{}.description", path),
                message: "Description mentions async behavior but subcommand is forced synchronous".to_string(),
                suggestion: Some("Either set synchronous to false or update description".to_string()),
            });
        }
    }

    fn validate_nested_subcommands(
        &self,
        subcommand: &crate::config::SubcommandConfig,
        path: &str,
        effective_sync: Option<bool>,
        errors: &mut Vec<SchemaValidationError>,
    ) {
        let Some(ref nested) = subcommand.subcommand else {
            return;
        };

        for (i, nested_sub) in nested.iter().enumerate() {
            self.validate_subcommand(
                nested_sub,
                &format!("{}.subcommand[{}]", path, i),
                effective_sync,
                errors,
            );
        }
    }

    fn validate_subcommand_options(
        &self,
        subcommand: &crate::config::SubcommandConfig,
        path: &str,
        errors: &mut Vec<SchemaValidationError>,
    ) {
        let Some(ref options) = subcommand.options else {
            return;
        };

        for (i, option) in options.iter().enumerate() {
            self.validate_option(option, &format!("{}.options[{}]", path, i), errors);
        }
    }

    /// Validate a command option
    fn validate_option(
        &self,
        option: &crate::config::CommandOption,
        path: &str,
        errors: &mut Vec<SchemaValidationError>,
    ) {
        if option.name.is_empty() {
            errors.push(SchemaValidationError {
                error_type: ValidationErrorType::MissingRequiredField,
                field_path: format!("{}.name", path),
                message: "Option name cannot be empty".to_string(),
                suggestion: None,
            });
        }

        // Validate type
        let valid_types = ["string", "boolean", "integer", "array"];
        if !valid_types.contains(&option.option_type.as_str()) {
            let suggestion = match option.option_type.as_str() {
                "bool" => Some("Use 'boolean' instead of 'bool'".to_string()),
                "int" => Some("Use 'integer' instead of 'int'".to_string()),
                "str" => Some("Use 'string' instead of 'str'".to_string()),
                _ => None,
            };

            let error_type = if suggestion.is_some() {
                ValidationErrorType::InvalidValue
            } else {
                ValidationErrorType::InvalidType
            };

            errors.push(SchemaValidationError {
                error_type,
                field_path: format!("{}.type", path),
                message: format!(
                    "Invalid option type '{}'. Must be one of: {}",
                    option.option_type,
                    valid_types.join(", ")
                ),
                suggestion,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_creation() {
        let validator = MtdfValidator::new();
        assert!(validator.strict_mode);
        assert!(!validator.allow_unknown_fields);
    }

    #[test]
    fn test_validator_builder() {
        let validator = MtdfValidator::new()
            .with_strict_mode(false)
            .with_unknown_fields_allowed(true);
        assert!(!validator.strict_mode);
        assert!(validator.allow_unknown_fields);
    }
}
