use argh::FromArgs;
use bdk_wallet::bitcoin::Network;

use crate::{
    errors::{DisplayableError, DisplayedError},
    params::{create_default_params_file, CliProtocolParams},
    settings::Settings,
};

/// Generate a default protocol parameters file
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "generate-params")]
pub struct GenerateParamsArgs {
    /// output file path (defaults to ~/.config/alpen/params.json)
    #[argh(option, short = 'o')]
    output: Option<std::path::PathBuf>,

    /// network to generate params for (defaults to signet)
    #[argh(option, short = 'n')]
    network: Option<String>,
}

pub async fn generate_params(args: GenerateParamsArgs, settings: Settings) -> Result<(), DisplayedError> {
    let network = if let Some(network_str) = args.network {
        match network_str.as_str() {
            "mainnet" | "bitcoin" => Network::Bitcoin,
            "testnet" => Network::Testnet,
            "signet" => Network::Signet,
            "regtest" => Network::Regtest,
            _ => return Err(DisplayedError::UserError(
                "Invalid network. Valid options: mainnet, testnet, signet, regtest".to_string(),
                Box::new("Invalid network parameter"),
            )),
        }
    } else {
        settings.network
    };

    let output_path = if let Some(path) = args.output {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .user_error("Failed to create output directory")?;
        }
        
        let mut params = CliProtocolParams::default();
        params.network = network;
        let json = serde_json::to_string_pretty(&params)
            .internal_error("Failed to serialize parameters")?;
        
        std::fs::write(&path, json)
            .user_error("Failed to write parameters file")?;
        
        path
    } else {
create_default_params_file()?
    };

    println!("✅ Generated default protocol parameters for {} at: {}", 
             network_name(network), 
             output_path.display());
    
    println!("\n📝 To use custom parameters:");
    println!("   1. Edit the file to customize values");
    println!("   2. Use --protocol-params flag: alpen --protocol-params {} <command>", output_path.display());
    println!("   3. Or set environment variable: ALPEN_CLI_PARAMS=@{}", output_path.display());
    
    Ok(())
}

fn network_name(network: Network) -> &'static str {
    match network {
        Network::Bitcoin => "mainnet",
        Network::Testnet => "testnet", 
        Network::Signet => "signet",
        Network::Regtest => "regtest",
        _ => "unknown",
    }
}