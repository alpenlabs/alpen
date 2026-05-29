use std::time::Duration;

use num_format::{Locale, ToFormattedString};
use zkaleido::ExecutionSummary;

use crate::args::{EvalArgs, PerfMode};

#[derive(Debug, Clone)]
pub struct ProofSummary {
    pub prepare_duration: Duration,
    pub prove_duration: Duration,
    pub total_duration: Duration,
    pub proof_bytes: usize,
    pub proof_type: String,
}

/// Returns a formatted header for the performance report with basic PR data.
pub fn format_header(args: &EvalArgs) -> String {
    let mut detail_text = String::new();

    if args.post_to_gh {
        detail_text.push_str(&format!("*Commit*: {}\n", &args.commit_hash[..8]));
    } else {
        detail_text.push_str("*Local execution*\n");
    }

    detail_text.push_str(&format!("*Mode*: {}\n", args.mode));

    detail_text
}

/// Returns formatted results for the [`ExecutionSummary`]s shaped in a table.
pub fn format_execute_results(results: &[(String, ExecutionSummary)], host_name: String) -> String {
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

pub fn format_prove_results(results: &[(String, ProofSummary)], host_name: String) -> String {
    let mut table_text = String::new();
    table_text.push('\n');
    table_text.push_str(
        "| program                | proof type | proof bytes | prepare ms | prove ms | total ms |\n",
    );
    table_text.push_str(
        "|------------------------|------------|-------------|------------|----------|----------|",
    );

    for (name, summary) in results.iter() {
        table_text.push_str(&format!(
            "\n| {:<22} | {:<10} | {:>11} | {:>10} | {:>8} | {:>8} |",
            name,
            summary.proof_type,
            summary.proof_bytes.to_formatted_string(&Locale::en),
            summary
                .prepare_duration
                .as_millis()
                .to_formatted_string(&Locale::en),
            summary
                .prove_duration
                .as_millis()
                .to_formatted_string(&Locale::en),
            summary
                .total_duration
                .as_millis()
                .to_formatted_string(&Locale::en)
        ));
    }
    table_text.push('\n');

    format!("*{host_name} Proving Results*\n {table_text}")
}

pub fn format_results_for_mode(
    mode: PerfMode,
    execute_results: &[(String, ExecutionSummary)],
    prove_results: &[(String, ProofSummary)],
    host_name: String,
) -> String {
    match mode {
        PerfMode::Execute => format_execute_results(execute_results, host_name),
        PerfMode::Prove => format_prove_results(prove_results, host_name),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn format_prove_results_includes_timing_breakdown_columns() {
        let text = format_prove_results(
            &[(
                "Checkpoint".to_string(),
                ProofSummary {
                    prepare_duration: Duration::from_millis(12),
                    prove_duration: Duration::from_millis(34),
                    total_duration: Duration::from_millis(46),
                    proof_bytes: 356,
                    proof_type: "Groth16".to_string(),
                },
            )],
            "SP1".to_string(),
        );

        assert!(text.contains("prepare ms"));
        assert!(text.contains("prove ms"));
        assert!(text.contains("total ms"));
        assert!(text.contains("Checkpoint"));
        assert!(text.contains("Groth16"));
        assert!(text.contains("356"));
    }
}
