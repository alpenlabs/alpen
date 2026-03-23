use strata_db_types::types::L1TxEntry;

/// Input messages consumed by the broadcaster service.
#[derive(Debug)]
pub(crate) enum BroadcasterInputMessage {
    /// Notify the service about a newly persisted entry.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "NotifyNewEntry is produced only after handle switchover"
        )
    )]
    NotifyNewEntry { idx: u64, txentry: L1TxEntry },
}

#[cfg(test)]
mod tests {
    use strata_db_types::types::L1TxStatus;
    use strata_service::{AsyncServiceInput, TickMsg, TickingInput, TokioMpscInput};
    use tokio::{sync::mpsc, time::Duration};

    use super::BroadcasterInputMessage;
    use crate::test_utils::gen_l1_tx_entry_with_status;

    #[tokio::test]
    async fn ticking_input_returns_none_after_channel_close() {
        let (tx, rx) = mpsc::channel::<BroadcasterInputMessage>(1);
        drop(tx);

        let mut input = TickingInput::new(Duration::from_millis(1), TokioMpscInput::new(rx));

        let closed = input.recv_next().await.unwrap();
        assert!(closed.is_none());
    }

    #[tokio::test]
    async fn ticking_input_forwards_messages() {
        let (tx, rx) = mpsc::channel::<BroadcasterInputMessage>(1);
        let txentry = gen_l1_tx_entry_with_status(L1TxStatus::Unpublished);

        tx.send(BroadcasterInputMessage::NotifyNewEntry {
            idx: 7,
            txentry: txentry.clone(),
        })
        .await
        .unwrap();

        let mut input = TickingInput::new(Duration::from_secs(60), TokioMpscInput::new(rx));

        let msg = input.recv_next().await.unwrap();
        match msg {
            Some(TickMsg::Msg(BroadcasterInputMessage::NotifyNewEntry { idx, txentry: got })) => {
                assert_eq!(idx, 7);
                assert_eq!(got, txentry);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }
}
