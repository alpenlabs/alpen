use strata_db_types::types::L1TxEntry;

/// Input messages consumed by the broadcaster service.
#[derive(Debug)]
pub(crate) enum BroadcasterInputMessage {
    /// Notify the service about a newly persisted entry.
    NotifyNewEntry { idx: u64, txentry: L1TxEntry },
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{traits::DatabaseBackend, types::L1TxStatus};
    use strata_l1_txfmt::MagicBytes;
    use strata_service::{AsyncServiceInput, TickMsg, TickingInput, TokioMpscInput};
    use strata_storage::ops::l1tx_broadcast::Context;
    use strata_tasks::TaskManager;
    use tokio::{
        runtime::Handle,
        sync::mpsc,
        time::{timeout, Duration},
    };

    use super::BroadcasterInputMessage;
    use crate::{
        broadcaster::BroadcasterBuilder,
        test_utils::{gen_l1_tx_entry_with_status, TestBitcoinClient},
        BtcioParams,
    };

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

    #[tokio::test]
    async fn broadcaster_service_exits_when_command_channel_closes() {
        let task_manager = TaskManager::new(Handle::current());
        let executor = task_manager.create_executor();

        let pool = threadpool::Builder::new().num_threads(2).build();
        let broadcast_db = get_test_sled_backend().broadcast_db();
        let ops = Arc::new(Context::new(broadcast_db).into_ops(pool));

        let btcio_params = BtcioParams::new(6, MagicBytes::new(*b"ALPN"), 0);
        let handle =
            BroadcasterBuilder::new(Arc::new(TestBitcoinClient::new(0)), ops, btcio_params)
                .with_broadcast_poll_interval_ms(1)
                .launch(&executor)
                .await
                .expect("launch broadcaster service");

        let monitor = handle
            .monitor()
            .expect("builder launch should always attach a service monitor")
            .clone();
        let mut listener_input = monitor.create_listener_input(&executor);

        // Dropping the last command sender closes input and should terminate the worker.
        drop(handle);

        let recv_result = timeout(Duration::from_secs(1), listener_input.recv_next())
            .await
            .expect("timed out waiting for service input closure")
            .expect("listener input should not error");

        assert!(
            recv_result.is_none(),
            "broadcaster service should exit once command channel is closed"
        );
    }
}
