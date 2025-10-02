//! Simple execution environment implementation.

use strata_ee_acct_types::{
    EnvResult, ExecBlock, ExecBlockOutput, ExecPartialState, ExecPayload, ExecutionEnvironment,
};
use strata_ee_chain_types::{BlockInputs, BlockOutputs};

use crate::types::{SimpleBlock, SimpleHeader, SimplePartialState, SimpleWriteBatch};

/// Simple execution environment for testing.
#[derive(Clone, Copy, Debug)]
pub struct SimpleExecutionEnvironment;

impl ExecutionEnvironment for SimpleExecutionEnvironment {
    type PartialState = SimplePartialState;
    type Block = SimpleBlock;
    type WriteBatch = SimpleWriteBatch;

    fn execute_block_body(
        &self,
        pre_state: &Self::PartialState,
        exec_payload: &ExecPayload<'_, Self::Block>,
        inputs: &BlockInputs,
    ) -> EnvResult<ExecBlockOutput<Self>> {
        let body = exec_payload.body();

        // Start with a copy of the pre-state
        let mut accounts = pre_state.accounts().clone();
        let mut outputs = BlockOutputs::new_empty();

        // 1. Apply deposits from inputs
        for deposit in inputs.subject_deposits() {
            let balance = accounts.entry(deposit.dest()).or_insert(0);
            *balance = balance
                .checked_add(*deposit.value())
                .ok_or(strata_ee_acct_types::EnvError::InvalidBlockTx)?;
        }

        // 2. Apply transactions from the block body
        for tx in body.transactions() {
            tx.apply(&mut accounts, &mut outputs)?;
        }

        // 3. Create write batch with the changes
        let write_batch = SimpleWriteBatch::new(accounts.clone());

        Ok(ExecBlockOutput::new(write_batch, outputs))
    }

    fn complete_header(
        &self,
        exec_payload: &ExecPayload<'_, Self::Block>,
        output: &ExecBlockOutput<Self>,
    ) -> EnvResult<<Self::Block as ExecBlock>::Header> {
        let intrinsics = exec_payload.header_intrinsics();

        // Compute state root by applying the write batch
        let post_state = SimplePartialState::new(output.write_batch().accounts().clone());
        let state_root = post_state.compute_state_root()?;

        Ok(SimpleHeader::new(
            intrinsics.parent_blkid,
            state_root,
            intrinsics.index,
        ))
    }

    fn verify_outputs_against_header(
        &self,
        _header: &<Self::Block as ExecBlock>::Header,
        _outputs: &ExecBlockOutput<Self>,
    ) -> EnvResult<()> {
        // For the simple EE, we don't need additional verification
        Ok(())
    }

    fn merge_write_into_state(
        &self,
        state: &mut Self::PartialState,
        wb: &Self::WriteBatch,
    ) -> EnvResult<()> {
        *state = SimplePartialState::new(wb.accounts().clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use strata_acct_types::{AccountId, BitcoinAmount, SubjectId};
    use strata_ee_acct_types::{EnvError, EnvResult, ExecHeader, ExecPayload};
    use strata_ee_chain_types::BlockInputs;

    use super::*;
    use crate::types::{SimpleBlockBody, SimpleHeaderIntrinsics, SimpleTransaction};

    fn alice() -> SubjectId {
        SubjectId::from([1u8; 32])
    }

    fn bob() -> SubjectId {
        SubjectId::from([2u8; 32])
    }

    fn charlie() -> SubjectId {
        SubjectId::from([3u8; 32])
    }

    fn account_123() -> AccountId {
        AccountId::from([123u8; 32])
    }

    /// Helper to execute a block and return the resulting state
    fn execute_block(
        ee: SimpleExecutionEnvironment,
        pre_state: &SimplePartialState,
        intrinsics: &SimpleHeaderIntrinsics,
        body: SimpleBlockBody,
        inputs: BlockInputs,
    ) -> EnvResult<SimplePartialState> {
        let payload = ExecPayload::new(intrinsics, &body);
        let output = ee.execute_block_body(pre_state, &payload, &inputs)?;

        let mut post_state = pre_state.clone();
        ee.merge_write_into_state(&mut post_state, output.write_batch())?;

        Ok(post_state)
    }

    #[test]
    fn test_apply_simple_transfer() {
        let ee = SimpleExecutionEnvironment;

        // Create initial state with alice having 1000
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 1000);
        let pre_state = SimplePartialState::new(accounts);

        // Transfer 300 from alice to bob
        let tx = SimpleTransaction::Transfer {
            from: alice(),
            to: bob(),
            value: 300,
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let post_state = execute_block(ee, &pre_state, &intrinsics, body, inputs)
            .expect("execution should succeed");

        assert_eq!(post_state.accounts().get(&alice()), Some(&700));
        assert_eq!(post_state.accounts().get(&bob()), Some(&300));
    }

    #[test]
    fn test_apply_multiple_transfers() {
        let ee = SimpleExecutionEnvironment;

        // Initial state: alice=1000, bob=500
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 1000);
        accounts.insert(bob(), 500);
        let pre_state = SimplePartialState::new(accounts);

        // Multiple transfers:
        // 1. alice -> bob: 200
        // 2. bob -> charlie: 300
        // 3. alice -> charlie: 100
        let txs = vec![
            SimpleTransaction::Transfer {
                from: alice(),
                to: bob(),
                value: 200,
            },
            SimpleTransaction::Transfer {
                from: bob(),
                to: charlie(),
                value: 300,
            },
            SimpleTransaction::Transfer {
                from: alice(),
                to: charlie(),
                value: 100,
            },
        ];

        let body = SimpleBlockBody::new(txs);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let post_state = execute_block(ee, &pre_state, &intrinsics, body, inputs)
            .expect("execution should succeed");

        // alice: 1000 - 200 - 100 = 700
        // bob: 500 + 200 - 300 = 400
        // charlie: 0 + 300 + 100 = 400
        assert_eq!(post_state.accounts().get(&alice()), Some(&700));
        assert_eq!(post_state.accounts().get(&bob()), Some(&400));
        assert_eq!(post_state.accounts().get(&charlie()), Some(&400));
    }

    #[test]
    fn test_apply_deposit() {
        let ee = SimpleExecutionEnvironment;

        // Start with empty state
        let pre_state = SimplePartialState::new_empty();

        // Create a deposit of 1000 to alice
        let mut inputs = BlockInputs::new_empty();
        inputs.add_subject_deposit(strata_ee_chain_types::SubjectDepositData::new(
            alice(),
            BitcoinAmount::from(1000u64),
        ));

        let body = SimpleBlockBody::new(vec![]);
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let post_state = execute_block(ee, &pre_state, &intrinsics, body, inputs)
            .expect("execution should succeed");

        assert_eq!(post_state.accounts().get(&alice()), Some(&1000));
    }

    #[test]
    fn test_apply_multiple_deposits() {
        let ee = SimpleExecutionEnvironment;

        // Start with bob having 200
        let mut accounts = BTreeMap::new();
        accounts.insert(bob(), 200);
        let pre_state = SimplePartialState::new(accounts);

        // Create multiple deposits
        let mut inputs = BlockInputs::new_empty();
        inputs.add_subject_deposit(strata_ee_chain_types::SubjectDepositData::new(
            alice(),
            BitcoinAmount::from(500u64),
        ));
        inputs.add_subject_deposit(strata_ee_chain_types::SubjectDepositData::new(
            bob(),
            BitcoinAmount::from(300u64),
        ));
        inputs.add_subject_deposit(strata_ee_chain_types::SubjectDepositData::new(
            charlie(),
            BitcoinAmount::from(1000u64),
        ));

        let body = SimpleBlockBody::new(vec![]);
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let post_state = execute_block(ee, &pre_state, &intrinsics, body, inputs)
            .expect("execution should succeed");

        assert_eq!(post_state.accounts().get(&alice()), Some(&500));
        assert_eq!(post_state.accounts().get(&bob()), Some(&500)); // 200 + 300
        assert_eq!(post_state.accounts().get(&charlie()), Some(&1000));
    }

    #[test]
    fn test_withdrawal_simple() {
        let ee = SimpleExecutionEnvironment;

        // alice has 1000
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 1000);
        let pre_state = SimplePartialState::new(accounts);

        // Withdraw 400 to account_123
        let tx = SimpleTransaction::EmitTransfer {
            from: alice(),
            dest: account_123(),
            value: 400,
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let payload = ExecPayload::new(&intrinsics, &body);
        let output = ee
            .execute_block_body(&pre_state, &payload, &inputs)
            .expect("execution should succeed");

        let mut post_state = pre_state.clone();
        ee.merge_write_into_state(&mut post_state, output.write_batch())
            .expect("merge should succeed");

        // alice: 1000 - 400 = 600
        assert_eq!(post_state.accounts().get(&alice()), Some(&600));

        // Verify the withdrawal output
        assert_eq!(output.outputs().output_transfers().len(), 1);
        let transfer = &output.outputs().output_transfers()[0];
        assert_eq!(transfer.dest(), account_123());
        assert_eq!(transfer.value(), BitcoinAmount::from(400u64));
    }

    #[test]
    fn test_multiple_withdrawals() {
        let ee = SimpleExecutionEnvironment;

        // alice=1500, bob=800
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 1500);
        accounts.insert(bob(), 800);
        let pre_state = SimplePartialState::new(accounts);

        let account_456 = AccountId::from([45u8; 32]);

        // Multiple withdrawals
        let txs = vec![
            SimpleTransaction::EmitTransfer {
                from: alice(),
                dest: account_123(),
                value: 300,
            },
            SimpleTransaction::EmitTransfer {
                from: bob(),
                dest: account_456,
                value: 250,
            },
            SimpleTransaction::EmitTransfer {
                from: alice(),
                dest: account_456,
                value: 200,
            },
        ];

        let body = SimpleBlockBody::new(txs);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let payload = ExecPayload::new(&intrinsics, &body);
        let output = ee
            .execute_block_body(&pre_state, &payload, &inputs)
            .expect("execution should succeed");

        let mut post_state = pre_state.clone();
        ee.merge_write_into_state(&mut post_state, output.write_batch())
            .expect("merge should succeed");

        // alice: 1500 - 300 - 200 = 1000
        // bob: 800 - 250 = 550
        assert_eq!(post_state.accounts().get(&alice()), Some(&1000));
        assert_eq!(post_state.accounts().get(&bob()), Some(&550));

        // Verify all withdrawal outputs
        assert_eq!(output.outputs().output_transfers().len(), 3);
        assert_eq!(output.outputs().output_transfers()[0].dest(), account_123());
        assert_eq!(
            output.outputs().output_transfers()[0].value(),
            BitcoinAmount::from(300u64)
        );
        assert_eq!(output.outputs().output_transfers()[1].dest(), account_456);
        assert_eq!(
            output.outputs().output_transfers()[1].value(),
            BitcoinAmount::from(250u64)
        );
        assert_eq!(output.outputs().output_transfers()[2].dest(), account_456);
        assert_eq!(
            output.outputs().output_transfers()[2].value(),
            BitcoinAmount::from(200u64)
        );
    }

    #[test]
    fn test_multi_block_deposits_transfers_withdrawals() {
        let ee = SimpleExecutionEnvironment;

        // Block 0 (genesis): empty state
        let mut state = SimplePartialState::new_empty();
        let mut parent_blkid = [0u8; 32];
        let mut index = 0u64;

        // Block 1: Deposit 2000 to alice, 1500 to bob
        {
            let mut inputs = BlockInputs::new_empty();
            inputs.add_subject_deposit(strata_ee_chain_types::SubjectDepositData::new(
                alice(),
                BitcoinAmount::from(2000u64),
            ));
            inputs.add_subject_deposit(strata_ee_chain_types::SubjectDepositData::new(
                bob(),
                BitcoinAmount::from(1500u64),
            ));

            let body = SimpleBlockBody::new(vec![]);
            index += 1;
            let intrinsics = SimpleHeaderIntrinsics {
                parent_blkid,
                index,
            };

            state = execute_block(ee, &state, &intrinsics, body, inputs)
                .expect("block 1 should succeed");

            assert_eq!(state.accounts().get(&alice()), Some(&2000));
            assert_eq!(state.accounts().get(&bob()), Some(&1500));

            // Compute parent_blkid for next block
            let empty_body = SimpleBlockBody::new(vec![]);
            let payload = ExecPayload::new(&intrinsics, &empty_body);
            let empty_inputs = BlockInputs::new_empty();
            let output = ee
                .execute_block_body(&state, &payload, &empty_inputs)
                .unwrap();
            let header = ee.complete_header(&payload, &output).unwrap();
            parent_blkid = header.compute_block_id();
        }

        // Block 2: alice -> bob: 400, bob -> charlie: 300
        {
            let txs = vec![
                SimpleTransaction::Transfer {
                    from: alice(),
                    to: bob(),
                    value: 400,
                },
                SimpleTransaction::Transfer {
                    from: bob(),
                    to: charlie(),
                    value: 300,
                },
            ];

            let body = SimpleBlockBody::new(txs);
            let inputs = BlockInputs::new_empty();
            index += 1;
            let intrinsics = SimpleHeaderIntrinsics {
                parent_blkid,
                index,
            };

            state = execute_block(ee, &state, &intrinsics, body, inputs)
                .expect("block 2 should succeed");

            // alice: 2000 - 400 = 1600
            // bob: 1500 + 400 - 300 = 1600
            // charlie: 0 + 300 = 300
            assert_eq!(state.accounts().get(&alice()), Some(&1600));
            assert_eq!(state.accounts().get(&bob()), Some(&1600));
            assert_eq!(state.accounts().get(&charlie()), Some(&300));

            let empty_body = SimpleBlockBody::new(vec![]);
            let payload = ExecPayload::new(&intrinsics, &empty_body);
            let empty_inputs = BlockInputs::new_empty();
            let output = ee
                .execute_block_body(&state, &payload, &empty_inputs)
                .unwrap();
            let header = ee.complete_header(&payload, &output).unwrap();
            parent_blkid = header.compute_block_id();
        }

        // Block 3: Deposit 500 to charlie, alice withdraws 600, charlie -> bob: 200
        {
            let mut inputs = BlockInputs::new_empty();
            inputs.add_subject_deposit(strata_ee_chain_types::SubjectDepositData::new(
                charlie(),
                BitcoinAmount::from(500u64),
            ));

            let txs = vec![
                SimpleTransaction::EmitTransfer {
                    from: alice(),
                    dest: account_123(),
                    value: 600,
                },
                SimpleTransaction::Transfer {
                    from: charlie(),
                    to: bob(),
                    value: 200,
                },
            ];

            let body = SimpleBlockBody::new(txs);
            index += 1;
            let intrinsics = SimpleHeaderIntrinsics {
                parent_blkid,
                index,
            };

            let payload = ExecPayload::new(&intrinsics, &body);
            let output = ee
                .execute_block_body(&state, &payload, &inputs)
                .expect("block 3 should succeed");

            ee.merge_write_into_state(&mut state, output.write_batch())
                .expect("merge should succeed");

            // alice: 1600 - 600 = 1000
            // bob: 1600 + 200 = 1800
            // charlie: 300 + 500 - 200 = 600
            assert_eq!(state.accounts().get(&alice()), Some(&1000));
            assert_eq!(state.accounts().get(&bob()), Some(&1800));
            assert_eq!(state.accounts().get(&charlie()), Some(&600));

            // Verify withdrawal
            assert_eq!(output.outputs().output_transfers().len(), 1);
            assert_eq!(output.outputs().output_transfers()[0].dest(), account_123());
            assert_eq!(
                output.outputs().output_transfers()[0].value(),
                BitcoinAmount::from(600u64)
            );

            let header = ee.complete_header(&payload, &output).unwrap();
            parent_blkid = header.compute_block_id();
        }

        // Block 4: bob withdraws 800, charlie withdraws 400, deposit 1000 to alice
        {
            let mut inputs = BlockInputs::new_empty();
            inputs.add_subject_deposit(strata_ee_chain_types::SubjectDepositData::new(
                alice(),
                BitcoinAmount::from(1000u64),
            ));

            let account_789 = AccountId::from([78u8; 32]);
            let txs = vec![
                SimpleTransaction::EmitTransfer {
                    from: bob(),
                    dest: account_123(),
                    value: 800,
                },
                SimpleTransaction::EmitTransfer {
                    from: charlie(),
                    dest: account_789,
                    value: 400,
                },
            ];

            let body = SimpleBlockBody::new(txs);
            index += 1;
            let intrinsics = SimpleHeaderIntrinsics {
                parent_blkid,
                index,
            };

            let payload = ExecPayload::new(&intrinsics, &body);
            let output = ee
                .execute_block_body(&state, &payload, &inputs)
                .expect("block 4 should succeed");

            ee.merge_write_into_state(&mut state, output.write_batch())
                .expect("merge should succeed");

            // alice: 1000 + 1000 = 2000
            // bob: 1800 - 800 = 1000
            // charlie: 600 - 400 = 200
            assert_eq!(state.accounts().get(&alice()), Some(&2000));
            assert_eq!(state.accounts().get(&bob()), Some(&1000));
            assert_eq!(state.accounts().get(&charlie()), Some(&200));

            // Verify withdrawals
            assert_eq!(output.outputs().output_transfers().len(), 2);
            assert_eq!(output.outputs().output_transfers()[0].dest(), account_123());
            assert_eq!(
                output.outputs().output_transfers()[0].value(),
                BitcoinAmount::from(800u64)
            );
            assert_eq!(output.outputs().output_transfers()[1].dest(), account_789);
            assert_eq!(
                output.outputs().output_transfers()[1].value(),
                BitcoinAmount::from(400u64)
            );
        }
    }

    #[test]
    fn test_transfer_insufficient_balance_fails() {
        let ee = SimpleExecutionEnvironment;

        // alice has only 100
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 100);
        let pre_state = SimplePartialState::new(accounts);

        // Try to transfer 200 (more than she has)
        let tx = SimpleTransaction::Transfer {
            from: alice(),
            to: bob(),
            value: 200,
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let result = execute_block(ee, &pre_state, &intrinsics, body, inputs);
        assert!(matches!(result, Err(EnvError::InvalidBlockTx)));
    }

    #[test]
    fn test_transfer_from_nonexistent_account_fails() {
        let ee = SimpleExecutionEnvironment;

        // Empty state (no accounts exist)
        let pre_state = SimplePartialState::new_empty();

        // Try to transfer from alice who doesn't exist
        let tx = SimpleTransaction::Transfer {
            from: alice(),
            to: bob(),
            value: 100,
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let result = execute_block(ee, &pre_state, &intrinsics, body, inputs);
        assert!(matches!(result, Err(EnvError::InvalidBlockTx)));
    }

    #[test]
    fn test_withdrawal_insufficient_balance_fails() {
        let ee = SimpleExecutionEnvironment;

        // bob has only 50
        let mut accounts = BTreeMap::new();
        accounts.insert(bob(), 50);
        let pre_state = SimplePartialState::new(accounts);

        // Try to withdraw 100 (more than he has)
        let tx = SimpleTransaction::EmitTransfer {
            from: bob(),
            dest: account_123(),
            value: 100,
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let result = execute_block(ee, &pre_state, &intrinsics, body, inputs);
        assert!(matches!(result, Err(EnvError::InvalidBlockTx)));
    }

    #[test]
    fn test_withdrawal_from_nonexistent_account_fails() {
        let ee = SimpleExecutionEnvironment;

        // Empty state (no accounts exist)
        let pre_state = SimplePartialState::new_empty();

        // Try to withdraw from alice who doesn't exist
        let tx = SimpleTransaction::EmitTransfer {
            from: alice(),
            dest: account_123(),
            value: 100,
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let result = execute_block(ee, &pre_state, &intrinsics, body, inputs);
        assert!(matches!(result, Err(EnvError::InvalidBlockTx)));
    }

    #[test]
    fn test_exact_balance_withdrawal_succeeds() {
        let ee = SimpleExecutionEnvironment;

        // charlie has exactly 750
        let mut accounts = BTreeMap::new();
        accounts.insert(charlie(), 750);
        let pre_state = SimplePartialState::new(accounts);

        // Withdraw exactly 750 (entire balance)
        let tx = SimpleTransaction::EmitTransfer {
            from: charlie(),
            dest: account_123(),
            value: 750,
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let payload = ExecPayload::new(&intrinsics, &body);
        let output = ee
            .execute_block_body(&pre_state, &payload, &inputs)
            .expect("exact balance withdrawal should succeed");

        let mut post_state = pre_state.clone();
        ee.merge_write_into_state(&mut post_state, output.write_batch())
            .expect("merge should succeed");

        // charlie: 750 - 750 = 0
        assert_eq!(post_state.accounts().get(&charlie()), Some(&0));

        // Verify the withdrawal
        assert_eq!(output.outputs().output_transfers().len(), 1);
        assert_eq!(output.outputs().output_transfers()[0].dest(), account_123());
        assert_eq!(
            output.outputs().output_transfers()[0].value(),
            BitcoinAmount::from(750u64)
        );
    }

    #[test]
    fn test_multiple_transfers_one_underflows() {
        let ee = SimpleExecutionEnvironment;

        // alice=500, bob=200
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 500);
        accounts.insert(bob(), 200);
        let pre_state = SimplePartialState::new(accounts);

        // First two transfers succeed, third one would underflow alice's balance
        let txs = vec![
            SimpleTransaction::Transfer {
                from: alice(),
                to: bob(),
                value: 200,
            },
            SimpleTransaction::Transfer {
                from: alice(),
                to: charlie(),
                value: 200,
            },
            // This would bring alice to 500 - 200 - 200 - 200 = -100, which underflows
            SimpleTransaction::Transfer {
                from: alice(),
                to: bob(),
                value: 200,
            },
        ];

        let body = SimpleBlockBody::new(txs);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let result = execute_block(ee, &pre_state, &intrinsics, body, inputs);
        assert!(
            matches!(result, Err(EnvError::InvalidBlockTx)),
            "Block should fail when any transaction underflows"
        );
    }

    #[test]
    fn test_exact_balance_transfer_succeeds() {
        let ee = SimpleExecutionEnvironment;

        // alice has exactly 1000
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 1000);
        let pre_state = SimplePartialState::new(accounts);

        // Transfer exactly 1000 (her entire balance)
        let tx = SimpleTransaction::Transfer {
            from: alice(),
            to: bob(),
            value: 1000,
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let post_state = execute_block(ee, &pre_state, &intrinsics, body, inputs)
            .expect("exact balance transfer should succeed");

        assert_eq!(post_state.accounts().get(&alice()), Some(&0));
        assert_eq!(post_state.accounts().get(&bob()), Some(&1000));
    }

    #[test]
    fn test_emit_message_simple() {
        let ee = SimpleExecutionEnvironment;

        // alice has 1000
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 1000);
        let pre_state = SimplePartialState::new(accounts);

        // Send message with 300 value and some data to bob in another EE
        let msg_data = vec![1, 2, 3, 4, 5];
        let tx = SimpleTransaction::EmitMessage {
            from: alice(),
            dest_account: account_123(),
            dest_subject: bob(),
            value: 300,
            data: msg_data.clone(),
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let payload = ExecPayload::new(&intrinsics, &body);
        let output = ee
            .execute_block_body(&pre_state, &payload, &inputs)
            .expect("execution should succeed");

        let mut post_state = pre_state.clone();
        ee.merge_write_into_state(&mut post_state, output.write_batch())
            .expect("merge should succeed");

        // alice: 1000 - 300 = 700
        assert_eq!(post_state.accounts().get(&alice()), Some(&700));

        // Verify the message output
        assert_eq!(output.outputs().output_messages().len(), 1);
        let message = &output.outputs().output_messages()[0];
        assert_eq!(message.dest(), account_123());
        assert_eq!(message.payload().value(), BitcoinAmount::from(300u64));

        // Message data should contain: dest_subject (32 bytes) + user data
        let msg_payload_data = message.payload().data();
        assert_eq!(msg_payload_data.len(), 32 + msg_data.len());
        assert_eq!(&msg_payload_data[0..32], bob().inner());
        assert_eq!(&msg_payload_data[32..], &msg_data[..]);
    }

    #[test]
    fn test_emit_multiple_messages() {
        let ee = SimpleExecutionEnvironment;

        // alice=2000, bob=1500
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 2000);
        accounts.insert(bob(), 1500);
        let pre_state = SimplePartialState::new(accounts);

        let account_456 = AccountId::from([45u8; 32]);
        let charlie_remote = SubjectId::from([99u8; 32]);

        // Multiple messages
        let txs = vec![
            SimpleTransaction::EmitMessage {
                from: alice(),
                dest_account: account_123(),
                dest_subject: bob(),
                value: 400,
                data: vec![10, 20, 30],
            },
            SimpleTransaction::EmitMessage {
                from: bob(),
                dest_account: account_456,
                dest_subject: charlie_remote,
                value: 250,
                data: vec![],
            },
            SimpleTransaction::EmitMessage {
                from: alice(),
                dest_account: account_456,
                dest_subject: charlie(),
                value: 600,
                data: vec![99, 88, 77, 66],
            },
        ];

        let body = SimpleBlockBody::new(txs);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let payload = ExecPayload::new(&intrinsics, &body);
        let output = ee
            .execute_block_body(&pre_state, &payload, &inputs)
            .expect("execution should succeed");

        let mut post_state = pre_state.clone();
        ee.merge_write_into_state(&mut post_state, output.write_batch())
            .expect("merge should succeed");

        // alice: 2000 - 400 - 600 = 1000
        // bob: 1500 - 250 = 1250
        assert_eq!(post_state.accounts().get(&alice()), Some(&1000));
        assert_eq!(post_state.accounts().get(&bob()), Some(&1250));

        // Verify all message outputs
        assert_eq!(output.outputs().output_messages().len(), 3);

        let msg1 = &output.outputs().output_messages()[0];
        assert_eq!(msg1.dest(), account_123());
        assert_eq!(msg1.payload().value(), BitcoinAmount::from(400u64));
        assert_eq!(&msg1.payload().data()[0..32], bob().inner());
        assert_eq!(&msg1.payload().data()[32..], &[10, 20, 30]);

        let msg2 = &output.outputs().output_messages()[1];
        assert_eq!(msg2.dest(), account_456);
        assert_eq!(msg2.payload().value(), BitcoinAmount::from(250u64));
        assert_eq!(&msg2.payload().data()[0..32], charlie_remote.inner());
        assert_eq!(msg2.payload().data().len(), 32); // no user data

        let msg3 = &output.outputs().output_messages()[2];
        assert_eq!(msg3.dest(), account_456);
        assert_eq!(msg3.payload().value(), BitcoinAmount::from(600u64));
        assert_eq!(&msg3.payload().data()[0..32], charlie().inner());
        assert_eq!(&msg3.payload().data()[32..], &[99, 88, 77, 66]);
    }

    #[test]
    fn test_emit_message_insufficient_balance_fails() {
        let ee = SimpleExecutionEnvironment;

        // alice has only 100
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 100);
        let pre_state = SimplePartialState::new(accounts);

        // Try to send message with 200 value (more than she has)
        let tx = SimpleTransaction::EmitMessage {
            from: alice(),
            dest_account: account_123(),
            dest_subject: bob(),
            value: 200,
            data: vec![1, 2, 3],
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let result = execute_block(ee, &pre_state, &intrinsics, body, inputs);
        assert!(matches!(result, Err(EnvError::InvalidBlockTx)));
    }

    #[test]
    fn test_emit_message_from_nonexistent_account_fails() {
        let ee = SimpleExecutionEnvironment;

        // Empty state (no accounts exist)
        let pre_state = SimplePartialState::new_empty();

        // Try to send message from alice who doesn't exist
        let tx = SimpleTransaction::EmitMessage {
            from: alice(),
            dest_account: account_123(),
            dest_subject: bob(),
            value: 100,
            data: vec![],
        };
        let body = SimpleBlockBody::new(vec![tx]);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let result = execute_block(ee, &pre_state, &intrinsics, body, inputs);
        assert!(matches!(result, Err(EnvError::InvalidBlockTx)));
    }

    #[test]
    fn test_mixed_transfers_and_messages() {
        let ee = SimpleExecutionEnvironment;

        // alice=3000, bob=2000
        let mut accounts = BTreeMap::new();
        accounts.insert(alice(), 3000);
        accounts.insert(bob(), 2000);
        let pre_state = SimplePartialState::new(accounts);

        // Mix of transfers, withdrawals, and messages
        let txs = vec![
            // Internal transfer
            SimpleTransaction::Transfer {
                from: alice(),
                to: charlie(),
                value: 500,
            },
            // Message to another EE
            SimpleTransaction::EmitMessage {
                from: alice(),
                dest_account: account_123(),
                dest_subject: bob(),
                value: 300,
                data: vec![1, 2, 3],
            },
            // Withdrawal to OL
            SimpleTransaction::EmitTransfer {
                from: bob(),
                dest: account_123(),
                value: 400,
            },
            // Another internal transfer
            SimpleTransaction::Transfer {
                from: charlie(),
                to: bob(),
                value: 200,
            },
            // Another message
            SimpleTransaction::EmitMessage {
                from: bob(),
                dest_account: account_123(),
                dest_subject: charlie(),
                value: 100,
                data: vec![],
            },
        ];

        let body = SimpleBlockBody::new(txs);
        let inputs = BlockInputs::new_empty();
        let intrinsics = SimpleHeaderIntrinsics {
            parent_blkid: [0; 32],
            index: 1,
        };

        let payload = ExecPayload::new(&intrinsics, &body);
        let output = ee
            .execute_block_body(&pre_state, &payload, &inputs)
            .expect("execution should succeed");

        let mut post_state = pre_state.clone();
        ee.merge_write_into_state(&mut post_state, output.write_batch())
            .expect("merge should succeed");

        // alice: 3000 - 500 - 300 = 2200
        // bob: 2000 - 400 + 200 - 100 = 1700
        // charlie: 0 + 500 - 200 = 300
        assert_eq!(post_state.accounts().get(&alice()), Some(&2200));
        assert_eq!(post_state.accounts().get(&bob()), Some(&1700));
        assert_eq!(post_state.accounts().get(&charlie()), Some(&300));

        // Verify outputs: 1 transfer + 2 messages
        assert_eq!(output.outputs().output_transfers().len(), 1);
        assert_eq!(output.outputs().output_messages().len(), 2);

        // Check transfer output
        assert_eq!(output.outputs().output_transfers()[0].dest(), account_123());
        assert_eq!(
            output.outputs().output_transfers()[0].value(),
            BitcoinAmount::from(400u64)
        );

        // Check message outputs
        let msg1 = &output.outputs().output_messages()[0];
        assert_eq!(msg1.dest(), account_123());
        assert_eq!(msg1.payload().value(), BitcoinAmount::from(300u64));

        let msg2 = &output.outputs().output_messages()[1];
        assert_eq!(msg2.dest(), account_123());
        assert_eq!(msg2.payload().value(), BitcoinAmount::from(100u64));
    }
}
