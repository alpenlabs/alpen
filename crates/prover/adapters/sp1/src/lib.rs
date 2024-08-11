use anyhow::Ok;

use serde::{de::DeserializeOwned, Serialize};
use serde_json::to_vec;

use sp1_sdk::{ProverClient, SP1ProofWithPublicValues, SP1Stdin, SP1VerifyingKey};
use zkvm::{Proof, ProverOptions, VerifcationKey, ZKVMHost, ZKVMVerifier};

pub struct SP1Host {
    elf: Vec<u8>,
    _inputs: Vec<u8>,
    prover_options: ProverOptions,
}

impl ZKVMHost for SP1Host {
    fn init(guest_code: Vec<u8>, prover_options: zkvm::ProverOptions) -> Self {
        SP1Host {
            elf: guest_code,
            _inputs: Vec::new(),
            prover_options,
        }
    }

    fn prove<T: serde::Serialize>(&self, input: T) -> anyhow::Result<(Proof, VerifcationKey)> {
        // Init the prover
        let client = ProverClient::new();
        let (pk, vk) = client.setup(&self.elf);

        // Setup the I/O
        let mut stdin = SP1Stdin::new();
        stdin.write(&input);

        // Start proving
        let mut prover = client.prove(&pk, stdin);
        if self.prover_options.stark_to_snark_conversion {
            prover = prover.plonk();
        }
        let proof = prover.run()?;

        // Proof seralization
        let serialized_proof = bincode::serialize(&proof)?;
        let verification_key = bincode::serialize(&vk)?;
        Ok((Proof(serialized_proof), VerifcationKey(verification_key)))
    }
}

pub struct SP1Verifier;

impl ZKVMVerifier for SP1Verifier {
    fn verify(verification_key: &VerifcationKey, proof: &Proof) -> anyhow::Result<()> {
        let proof: SP1ProofWithPublicValues = bincode::deserialize(&proof.0)?;
        let vkey: SP1VerifyingKey = bincode::deserialize(&verification_key.0)?;

        let client = ProverClient::new();
        client.verify(&proof, &vkey)?;

        Ok(())
    }

    fn verify_with_public_params<T: DeserializeOwned + serde::Serialize>(
        verification_key: &VerifcationKey,
        public_params: T,
        proof: &Proof,
    ) -> anyhow::Result<()> {
        let mut proof: SP1ProofWithPublicValues = bincode::deserialize(&proof.0)?;
        let vkey: SP1VerifyingKey = bincode::deserialize(&verification_key.0)?;

        let client = ProverClient::new();
        client.verify(&proof, &vkey)?;

        let actual_public_parameter: T = proof.public_values.read();

        // TODO: use custom ZKVM error
        anyhow::ensure!(
            to_vec(&actual_public_parameter)? == to_vec(&public_params)?,
            "Failed to verify proof given the public param"
        );

        Ok(())
    }

    fn extract_public_output<T: Serialize + DeserializeOwned>(proof: &Proof) -> anyhow::Result<T> {
        let mut proof: SP1ProofWithPublicValues = bincode::deserialize(&proof.0)?;
        let public_params: T = proof.public_values.read();
        Ok(public_params)
    }
}

// NOTE: SP1 prover runs in release mode only; therefore run the tests on release mode only
#[cfg(test)]
mod tests {
    use zkvm::ProverOptions;

    use super::*;

    // Adding compiled guest code `TEST_ELF` to save the build time
    // #![no_main]
    // sp1_zkvm::entrypoint!(main);
    // fn main() {
    //     let n = sp1_zkvm::io::read::<u32>();
    //     sp1_zkvm::io::commit(&n);
    // }
    const TEST_ELF: &[u8] = include_bytes!("../elf/riscv32im-succinct-zkvm-elf");

    #[test]
    fn test_mock_prover() {
        let input: u32 = 1;
        let zkvm = SP1Host::init(TEST_ELF.to_vec(), ProverOptions::default());

        // assert proof generation works
        let (proof, vk) = zkvm.prove(input).expect("Failed to generate proof");

        // assert proof verification works
        SP1Verifier::verify(&vk, &proof).expect("Proof verification failed");

        // assert public outputs extraction from proof  works
        let out: u32 =
            SP1Verifier::extract_public_output(&proof).expect("Failed to extract public outputs");
        assert_eq!(input, out)
    }

    #[test]
    fn test_mock_prover_with_public_param() {
        let input: u32 = 1;
        let zkvm = SP1Host::init(TEST_ELF.to_vec(), ProverOptions::default());

        // assert proof generation works
        let (proof, vk) = zkvm.prove(input).expect("Failed to generate proof");

        // assert proof verification works
        SP1Verifier::verify_with_public_params(&vk, input, &proof)
            .expect("Proof verification failed");
    }
}
