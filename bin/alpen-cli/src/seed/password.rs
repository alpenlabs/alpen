use argon2::{Algorithm, Argon2, Params, Version};
use dialoguer::Password as InputPassword;
use zeroize::{ZeroizeOnDrop, Zeroizing};
use zxcvbn::{zxcvbn, Entropy};

use super::PW_SALT_LEN;

#[derive(Debug, ZeroizeOnDrop)]
pub struct Password {
    inner: String,
}

#[derive(Debug, Copy, Clone)]
pub enum HashVersion {
    V0,
}

impl HashVersion {
    const fn params(&self) -> (Algorithm, Version, Result<Params, argon2::Error>) {
        match self {
            HashVersion::V0 => (
                Algorithm::Argon2id,
                Version::V0x13,
                // NOTE: This is the OWASP recommended params for Argon2id
                //       see https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html#argon2id
                Params::new(19_456, 2, 1, Some(32)),
            ),
        }
    }
}

impl Password {
    /// Constructs a password from a string. (The complexity of the password is not checked.)
    pub const fn new(password: String) -> Self {
        Self { inner: password }
    }

    /// Constructs a password from user interaction. (The complexity of the password is not
    /// checked.)
    pub fn read(new: bool) -> Result<Self, dialoguer::Error> {
        let mut input = InputPassword::new().allow_empty_password(true);
        if new {
            input = input
                .with_prompt("Create a new password (leave empty for no password, dangerous!)")
                .with_confirmation(
                    "Confirm password (leave empty for no password, dangerous!)",
                    "Passwords didn't match",
                );
        } else {
            input = input.with_prompt("Enter your password");
        }

        let password = input.interact()?;

        Ok(Self::new(password))
    }

    /// Returns the password entropy.
    pub fn entropy(&self) -> Entropy {
        zxcvbn(self.inner.as_str(), &[])
    }

    /// Derives a seed encryption key from a password.
    pub fn seed_encryption_key(
        &mut self,
        salt: &[u8; PW_SALT_LEN],
        version: HashVersion,
    ) -> Result<Zeroizing<[u8; 32]>, argon2::Error> {
        let mut sek = Zeroizing::new([0u8; 32]);
        let (algo, ver, params) = version.params();
        Argon2::new(algo, ver, params.expect("valid params")).hash_password_into(
            self.inner.as_bytes(),
            salt,
            sek.as_mut(),
        )?;
        Ok(sek)
    }

    /// Validates the password strength and returns feedback if it's weak.
    pub fn validate(&self) -> Result<(), String> {
        let entropy = self.entropy();
        if entropy.score() <= zxcvbn::Score::Two {
            let feedback = entropy.feedback();
            let feedback_message = if let Some(feedback) = feedback {
                feedback
                    .warning()
                    .map(|w| w.to_string())
                    .unwrap_or_default()
            } else {
                "".to_string() // empty string is fine
            };
            let mut message = String::new();

            message.push_str(feedback_message.as_str());

            if let Some(feedback) = feedback {
                message.push_str("Suggestions:\n");
                for suggestion in feedback.suggestions() {
                    message.push_str(&format!("- {suggestion}\n"));
                }
            }

            return Err(message);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct IncorrectPassword;
