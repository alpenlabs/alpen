use argh::FromArgs;
use rand_core::OsRng;

use crate::{
    errors::{DisplayableError, DisplayedError},
    seed::{password::Password, EncryptedSeedPersister, Seed},
};

/// Changes the seed's encryption password
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "change-password")]
pub struct ChangePwdArgs {}

pub async fn change_pwd(
    _args: ChangePwdArgs,
    seed: Seed,
    persister: impl EncryptedSeedPersister,
) -> Result<(), DisplayedError> {
    let mut new_pw =
        Password::read(true).internal_error("Failed to read the password entered by user")?;
    if let Err(feedback) = new_pw.validate() {
        println!("Password is weak. {feedback}");
    }
    let encrypted_seed = seed
        .encrypt(&mut new_pw, &mut OsRng)
        .internal_error("Failed to encrypt seed")?;
    persister
        .save(&encrypted_seed)
        .internal_error("Failed to save encrypted seed")?;

    println!("Password changed successfully");
    Ok(())
}
