use argh::FromArgs;
use bip39::Language;

use crate::{errors::DisplayedError, seed::Seed};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "backup")]
/// Prints a BIP39 mnemonic encoding the internal wallet's seed bytes
pub struct BackupArgs {
    /// select a language for the BIP39 mnemonic. defaults to English.
    /// options:
    /// en, cn, cn-trad,
    /// cz, fr, it, jp, kr or es
    #[argh(option)]
    language: Option<String>,
}

/// Unsupported mnemonic language error
#[derive(Clone, Copy, Debug)]
pub struct UnsupportedMnemonicLanguage;

pub async fn backup(args: BackupArgs, seed: Seed) -> Result<(), DisplayedError> {
    let language = match args.language.unwrap_or_else(|| "en".to_owned()).as_str() {
        "en" => Ok(Language::English),
        "cn" => Ok(Language::SimplifiedChinese),
        "cn-trad" => Ok(Language::TraditionalChinese),
        "cz" => Ok(Language::Czech),
        "fr" => Ok(Language::French),
        "it" => Ok(Language::Italian),
        "jp" => Ok(Language::Japanese),
        "kr" => Ok(Language::Korean),
        "es" => Ok(Language::Spanish),
        other => Err(DisplayedError::UserError(
            format!("The mnemonic language provided '{other}' is not supported"),
            Box::new(UnsupportedMnemonicLanguage),
        )),
    }?;

    seed.print_mnemonic(language);
    Ok(())
}
