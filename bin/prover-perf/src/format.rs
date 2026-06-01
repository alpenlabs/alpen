use num_format::{Locale, ToFormattedString};
use zkaleido::ExecutionSummary;

use crate::{args::EvalArgs, programs::ProofReport};

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

/// Returns formatted results for the [`ExecutionSummary`]s shaped in a table.
pub fn format_results(results: &[(String, ExecutionSummary)], host_name: String) -> String {
    let mut table_text = String::new();
    table_text.push('\n');
    table_text.push_str("| program                | cycles      | gas      |\n");
    table_text.push_str("|------------------------|-------------|----------|");

    for (name, summary) in results.iter() {
        table_text.push_str(&format!(
            "\n| {:<22} | {:>11} | {:>8} |",
            name,
            summary.cycles().to_formatted_string(&Locale::en),
            summary.gas().unwrap_or(0).to_formatted_string(&Locale::en)
        ));
    }
    table_text.push('\n');

    format!("*{host_name} Execution Results*\n {table_text}")
}

/// Returns formatted results for completed proof receipts.
pub fn format_proof_results(results: &[(String, ProofReport)], host_name: String) -> String {
    let mut table_text = String::new();
    table_text.push('\n');
    table_text.push_str(
        "| program                | proof bytes | public bytes | proof type | elapsed ms |\n",
    );
    table_text.push_str(
        "|------------------------|-------------|--------------|------------|------------|",
    );

    for (name, report) in results.iter() {
        let receipt = &report.receipt;
        table_text.push_str(&format!(
            "\n| {:<22} | {:>11} | {:>12} | {:<10} | {:>10} |",
            name,
            receipt
                .receipt()
                .proof()
                .as_bytes()
                .len()
                .to_formatted_string(&Locale::en),
            receipt
                .receipt()
                .public_values()
                .as_bytes()
                .len()
                .to_formatted_string(&Locale::en),
            format!("{:?}", receipt.metadata().proof_type()),
            report.elapsed.as_millis().to_formatted_string(&Locale::en)
        ));
    }
    table_text.push('\n');

    format!("*{host_name} Proof Results*\n {table_text}")
}
