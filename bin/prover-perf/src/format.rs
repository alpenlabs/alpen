use num_format::{Locale, ToFormattedString};
use zkaleido::ExecutionSummary;

use crate::args::EvalArgs;

/// Returns a formatted header for the performance report with basic PR data.
pub fn format_header(args: &EvalArgs) -> String {
    let mut detail_text = String::new();

    if args.post_to_gh {
        detail_text.push_str(&format!("*Commit*: {}\n", args.commit_hash));
    } else {
        detail_text.push_str("*Local execution*\n");
    }

    detail_text
}

/// Formats the checkpoint guest's cycles and gas in a table.
pub fn format_checkpoint_result(summary: &ExecutionSummary) -> String {
    format!(
        "*SP1 Checkpoint Guest Execution Results*\n\n\
         | cycles | gas |\n\
         |-------:|----:|\n\
         | {} | {} |\n",
        summary.cycles().to_formatted_string(&Locale::en),
        summary.gas().unwrap_or(0).to_formatted_string(&Locale::en),
    )
}
