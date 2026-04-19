use rust_code_analysis::{
    FuncSpace, SpaceKind, get_function_spaces, get_language_for_file, read_file_with_eol,
};
use std::path::Path;

use crate::simplify::models::{
    Cognitive, Cyclomatic, Loc, Metrics, MetricsResults, Mi, SpaceEntry,
};

// ---------------------------------------------------------------------------
// Conversion helpers: rust-code-analysis native types → our MetricsResults
// ---------------------------------------------------------------------------

fn code_metrics_to_metrics(cm: &rust_code_analysis::CodeMetrics) -> Metrics {
    Metrics {
        cognitive: Cognitive {
            sum: cm.cognitive.cognitive_sum(),
        },
        cyclomatic: Cyclomatic {
            sum: cm.cyclomatic.cyclomatic_sum(),
        },
        mi: Mi {
            mi_visual_studio: cm.mi.mi_visual_studio(),
        },
        loc: Loc {
            sloc: cm.loc.sloc(),
        },
    }
}

fn space_kind_str(kind: SpaceKind) -> &'static str {
    match kind {
        SpaceKind::Function => "function",
        SpaceKind::Class => "class",
        SpaceKind::Struct => "struct",
        SpaceKind::Trait => "trait",
        SpaceKind::Impl => "impl",
        SpaceKind::Unit => "unit",
        SpaceKind::Namespace => "namespace",
        SpaceKind::Interface => "interface",
        SpaceKind::Unknown => "unknown",
    }
}

fn func_space_to_space_entry(space: &FuncSpace) -> SpaceEntry {
    let kind_str = space_kind_str(space.kind).to_string();

    SpaceEntry {
        name: space.name.clone().unwrap_or_default(),
        start_line: space.start_line as u32,
        end_line: space.end_line as u32,
        kind: kind_str,
        metrics: code_metrics_to_metrics(&space.metrics),
        spaces: space.spaces.iter().map(func_space_to_space_entry).collect(),
    }
}

fn func_space_to_metrics_results(space: FuncSpace) -> MetricsResults {
    MetricsResults {
        name: space.name.clone().unwrap_or_default(),
        metrics: code_metrics_to_metrics(&space.metrics),
        spaces: space.spaces.iter().map(func_space_to_space_entry).collect(),
    }
}

// ---------------------------------------------------------------------------
// Per-file analysis using the library
// ---------------------------------------------------------------------------

pub(crate) fn analyze_file(path: &Path) -> Option<MetricsResults> {
    let lang = get_language_for_file(path)?;
    let source = read_file_with_eol(path).ok().flatten()?;
    let func_space = get_function_spaces(&lang, source, path, None)?;
    Some(func_space_to_metrics_results(func_space))
}
