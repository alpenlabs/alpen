//! Minimal checkpoint validator using existing Strata functions

use std::{fs, path::PathBuf};

use anyhow::Result;
use argh::FromArgs;
use bitcoin::Transaction;
use serde_json;
use strata_consensus_logic::checkpoint_verification::verify_checkpoint;
use strata_l1tx::{filter::parse_valid_checkpoint_envelopes, TxFilterConfig};
use strata_primitives::{batch::SignedCheckpoint, params::RollupParams};
use strata_state::client_state::L1Checkpoint;
use tracing::{error, info};

#[derive(FromArgs)]
/// Validate Bitcoin transaction checkpoints against rollup parameters
struct Args {
    /// raw Bitcoin transaction in hex format OR path to hex file
    #[argh(option, short = 't')]
    tx: String,

    /// rollup parameters JSON file
    #[argh(option, short = 'c')]
    config: PathBuf,

    /// previous checkpoint file (optional)
    #[argh(option, short = 'p')]
    prev_checkpoint: Option<PathBuf>,

    /// verbose logging
    #[argh(switch, short = 'v')]
    verbose: bool,
}

fn main() {
    let args: Args = argh::from_env();

    // Simple logging setup
    tracing_subscriber::fmt()
        .with_max_level(if args.verbose {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        })
        .init();

    if let Err(e) = run(args) {
        error!("Error: {e:?}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> Result<()> {
    // Load transaction (from hex string or file)
    let tx = load_transaction(&args.tx)?;
    info!("Transaction ID: {}", tx.compute_txid());

    // Load config
    let params: RollupParams = serde_json::from_str(&fs::read_to_string(&args.config)?)?;
    let filter_config = TxFilterConfig::derive_from(&params)?;

    // Load previous checkpoint if provided
    let prev_checkpoint: Option<L1Checkpoint> = if let Some(p) = args.prev_checkpoint {
        Some(serde_json::from_str(&fs::read_to_string(p)?)?)
    } else {
        None
    };

    // Extract checkpoints using existing Strata function
    let checkpoints: Vec<SignedCheckpoint> = 
        parse_valid_checkpoint_envelopes(&tx, &filter_config).collect();

    info!("Found {} checkpoint(s)", checkpoints.len());

    // Validate each checkpoint using existing Strata function
    let mut success_count = 0;
    for (i, signed_checkpoint) in checkpoints.iter().enumerate() {
        let checkpoint = signed_checkpoint.checkpoint();
        let epoch = checkpoint.batch_info().epoch();
        let proof_size = checkpoint.proof().as_bytes().len();

        info!("Checkpoint {}: epoch {}, proof size {} bytes", i + 1, epoch, proof_size);

        match verify_checkpoint(checkpoint, prev_checkpoint.as_ref(), &params) {
            Ok(_) => {
                info!("✅ Checkpoint {} is VALID", i + 1);
                success_count += 1;
            }
            Err(e) => {
                error!("❌ Checkpoint {} FAILED: {}", i + 1, e);
            }
        }
    }

    // Summary
    println!("\n=== RESULTS ===");
    println!("Checkpoints found: {}", checkpoints.len());
    println!("Checkpoints valid: {}", success_count);
    println!("Checkpoints failed: {}", checkpoints.len() - success_count);

    if success_count == checkpoints.len() && !checkpoints.is_empty() {
        println!("✅ All checkpoints valid");
        Ok(())
    } else {
        println!("❌ Some checkpoints failed");
        std::process::exit(1);
    }
}

fn load_transaction(input: &str) -> Result<Transaction> {
    let hex_data = if input.len() < 200 && std::path::Path::new(input).exists() {
        // Assume it's a file path
        fs::read_to_string(input)?
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect::<String>()
    } else {
        // Assume it's hex data
        input.to_string()
    };

    let tx_bytes = hex::decode(hex_data.trim())?;
    let tx: Transaction = bitcoin::consensus::deserialize(&tx_bytes)?;
    Ok(tx)
}