use num_format::{Locale, ToFormattedString};

use crate::{args::EvalArgs, PerformanceReport};

/// Returns a formatted header for the performance report with basic PR data.
pub fn format_header(args: &EvalArgs) -> String {
    let mut detail_text = String::new();

    if args.post_to_gh {
        detail_text.push_str(&format!("*Commit*: {}\n", &args.commit_hash[..8]));
    } else {
        detail_text.push_str("*Local execution*\n");
    }

    detail_text
}

/// Returns formatted results for the [`PerformanceReport`]s shaped in a table.
pub fn format_results(results: &[PerformanceReport], host_name: String) -> String {
    let mut table_text = String::new();
    table_text.push('\n');
    table_text.push_str("| program                | cycles      | success  |\n");
    table_text.push_str("|------------------------|-------------|----------|");

    for result in results.iter() {
        table_text.push_str(&format!(
            "\n| {:<22} | {:>11} | {:<7} |",
            result.name,
            result.cycles.to_formatted_string(&Locale::en),
            if result.success { "✅" } else { "❌" }
        ));
    }
    table_text.push('\n');

    format!("*{host_name} Execution Results*\n {table_text}")
}
