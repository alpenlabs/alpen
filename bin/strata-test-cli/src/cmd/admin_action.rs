//! Sign admin predicate updates.

use std::{
    env,
    io::{self, Write},
};

use anyhow::{anyhow, Context};
use argh::FromArgs;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use bitcoin::Network;
use strata_asm_proto_admin_txs::test_utils::sign_ecdsa_bip137;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_predicate::PredicateKey;

use super::create_ee_predicate_update::{
    build_admin_action, parse_network, parse_predicate_key, parse_target, PredicateUpdateTarget,
};
use crate::admin::{parse_secret_key, signing_message};

/// Sign an admin predicate update with STRATA_ADMIN_KEY and print a base64 signature.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "sign-admin-action")]
pub struct SignAdminActionArgs {
    /// update kind: ee-stf-vk, ol-stf-vk or asm-stf-vk
    #[argh(option, from_str_fn(parse_target))]
    pub update: PredicateUpdateTarget,
    /// authority sequence number
    #[argh(option)]
    pub seq_no: u64,
    /// full predicate key
    #[argh(option, from_str_fn(parse_predicate_key))]
    pub predicate: PredicateKey,
    /// bitcoin network
    #[argh(option, from_str_fn(parse_network))]
    pub network: Network,
}

pub(crate) fn sign_admin_action(args: SignAdminActionArgs) -> Result<(), DisplayedError> {
    sign(args).user_error("admin action signing failed")
}

fn sign(args: SignAdminActionArgs) -> anyhow::Result<()> {
    let secret = env::var("STRATA_ADMIN_KEY")
        .map_err(|_| anyhow!("set STRATA_ADMIN_KEY to an xpriv or 32-byte hex secret"))?;
    let key = parse_secret_key(&secret).context("invalid STRATA_ADMIN_KEY")?;
    let action = build_admin_action(args.predicate, args.update, None);
    let (message, digest) = signing_message(&action, args.seq_no, args.network);
    writeln!(
        io::stderr().lock(),
        "Message to sign:\n{}\n",
        message.as_str()
    )
    .context("display signing message")?;
    let signature = BASE64.encode(sign_ecdsa_bip137(&digest, &key));
    writeln!(
        io::stderr().lock(),
        "Send the base64 signature printed on stdout to the coordinator."
    )
    .context("display signature instructions")?;
    writeln!(io::stdout().lock(), "{signature}").context("write signature")
}
