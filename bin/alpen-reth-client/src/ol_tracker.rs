use std::time::Duration;

use alpen_reth_rpc::sequencer;
use futures::{stream, Stream};
use tracing::{debug, error};

use crate::{
    config::Config,
    traits::{ELSequencerClient, L1Client, OlClient},
    types::{AccountStateCommitment, ConsensusEvent, OlBlockId},
    utils::{ExponentialBackoff, RetryTracker},
};

#[derive(Debug, Clone)]
pub(crate) struct OlTrackerConfig {
    pub poll_interval_secs: u64,
    pub max_retry_delay_secs: u64,
    pub retry_backoff_multiplier: f64,
}

impl Default for OlTrackerConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 5,
            max_retry_delay_secs: 60,
            retry_backoff_multiplier: 1.5,
        }
    }
}

async fn check_confirmed_state(
    olclient: &impl OlClient,
    config: &Config,
    best_blockhash: &OlBlockId,
) -> eyre::Result<AccountStateCommitment> {
    olclient
        .account_state_commitment_at(&config.params.account_id, best_blockhash)
        .await
        .map_err(|e| {
            error!("Failed to get account state commitment: {}", e);
            eyre::eyre!("Failed to get account state commitment: {}", e)
        })
}

async fn check_state_at_height(
    height: u64,
    olclient: &impl OlClient,
    l1client: &impl L1Client,
    config: &Config,
) -> eyre::Result<AccountStateCommitment> {
    let finalized_ol_blockhash =
        l1client
            .get_l1_commitment_by_height(height)
            .await
            .map_err(|e| {
                error!("Failed to get L1 commitment at height {}: {}", height, e);
                e
            })?;

    let (blockhash, ..) = olclient
        .ol_block_for_l1(&finalized_ol_blockhash)
        .await
        .map_err(|e| {
            error!("Failed to get OL block for L1 commitment: {}", e);
            e
        })?
        .into_parts();

    olclient
        .account_state_commitment_at(&config.params.account_id, &blockhash)
        .await
        .map_err(|e| {
            error!("Failed to get finalized account state commitment: {}", e);
            eyre::eyre!("Failed to get account state commitment: {}", e)
        })
}

/// Stream-based equivalent of ol_tracker that yields ConsensusEvent items
pub(crate) fn ol_tracker_stream(
    config: Config,
    olclient: impl OlClient + Clone,
    l1client: impl L1Client + Clone,
) -> impl Stream<Item = ConsensusEvent> {
    ol_tracker_stream_with_config(config, olclient, l1client, OlTrackerConfig::default())
}

#[derive(Debug, Clone)]
struct OlTrackerState {
    last_best_ol_blockhash: OlBlockId,
    last_confirmed: AccountStateCommitment,
    last_finalized: AccountStateCommitment,
}

pub(crate) fn ol_tracker_stream_with_config(
    config: Config,
    olclient: impl OlClient + Clone,
    l1client: impl L1Client + Clone,
    tracker_config: OlTrackerConfig,
) -> impl Stream<Item = ConsensusEvent> {
    let tracker_state = OlTrackerState {
        last_best_ol_blockhash: OlBlockId::zero(),
        last_confirmed: AccountStateCommitment::zero(),
        last_finalized: AccountStateCommitment::zero(),
    };
    let retry = RetryTracker::new(
        tracker_config.poll_interval_secs,
        tracker_config.max_retry_delay_secs,
        ExponentialBackoff::new(tracker_config.retry_backoff_multiplier),
    );

    stream::unfold(
        (tracker_state, retry, config, olclient, l1client),
        move |(mut stream_state, mut retry, config, olclient, l1client)| async move {
            loop {
                tokio::time::sleep(Duration::from_secs(retry.delay())).await;

                let result: eyre::Result<Option<ConsensusEvent>> = async {
                    debug!("Polling for OL block updates");

                    let (best_blockhash, _, latest_l1_commitment) = olclient
                        .best_ol_block()
                        .await
                        .map_err(|e| {
                            error!("Failed to get best OL block: {}", e);
                            e
                        })?
                        .into_parts();

                    if best_blockhash == stream_state.last_best_ol_blockhash {
                        debug!("No new OL block, continuing");
                        return Ok(None);
                    }

                    debug!("New OL block detected: {:?}", best_blockhash);
                    stream_state.last_best_ol_blockhash = best_blockhash.clone();

                    let confirmed_state =
                        check_confirmed_state(&olclient, &config, &best_blockhash).await?;

                    let finalized_state = {
                        let safe_l1_height = latest_l1_commitment
                            .height()
                            .saturating_sub(config.finality_depth as u64);

                        let finalized_ol_blockhash = l1client
                            .get_l1_commitment_by_height(safe_l1_height)
                            .await
                            .map_err(|e| {
                                error!(
                                    "Failed to get L1 commitment at height {}: {}",
                                    safe_l1_height, e
                                );
                                e
                            })?;

                        let (blockhash, ..) = olclient
                            .ol_block_for_l1(&finalized_ol_blockhash)
                            .await
                            .map_err(|e| {
                                error!("Failed to get OL block for L1 commitment: {}", e);
                                e
                            })?
                            .into_parts();

                        olclient
                            .account_state_commitment_at(&config.params.account_id, &blockhash)
                            .await
                            .map_err(|e| {
                                error!("Failed to get finalized account state commitment: {}", e);
                                e
                            })?
                    };

                    let res = if (&confirmed_state, &finalized_state)
                        != (&stream_state.last_confirmed, &stream_state.last_finalized)
                    {
                        debug!("New state: {:?}; {:?}", confirmed_state, finalized_state);

                        Ok(Some(ConsensusEvent::OlUpdated {
                            confirmed: confirmed_state.clone(),
                            finalized: finalized_state.clone(),
                        }))
                    } else {
                        Ok(None)
                    };

                    stream_state.last_confirmed = confirmed_state;
                    stream_state.last_finalized = finalized_state;

                    res
                }
                .await;

                match result {
                    Ok(Some(event)) => {
                        retry.reset();
                        debug!("Yielding consensus event: {:?}", event);
                        return Some((event, (stream_state, retry, config, olclient, l1client)));
                    }
                    Ok(None) => {
                        retry.reset();
                        continue;
                    }
                    Err(e) => {
                        retry.increment();
                        let retry_delay = retry.delay();

                        error!(
                            "ol_tracker error (retry #{}, next retry in {:?}): {}",
                            retry.count(),
                            retry_delay,
                            e
                        );
                        continue;
                    }
                }
            }
        },
    )
}

pub(crate) fn seq_head_tracker_stream(
    sequencer_client: impl ELSequencerClient + Clone,
) -> impl Stream<Item = ConsensusEvent> {
    seq_head_tracker_stream_with_config(sequencer_client, OlTrackerConfig::default())
}

pub(crate) fn seq_head_tracker_stream_with_config(
    sequencer_client: impl ELSequencerClient + Clone,
    tracker_config: OlTrackerConfig,
) -> impl Stream<Item = ConsensusEvent> {
    let latest_head = AccountStateCommitment::zero();
    let retry = RetryTracker::new(
        tracker_config.poll_interval_secs,
        tracker_config.max_retry_delay_secs,
        ExponentialBackoff::new(tracker_config.retry_backoff_multiplier),
    );

    stream::unfold(
        (latest_head, retry, sequencer_client),
        move |(last_head, mut retry, sequencer_client)| async move {
            loop {
                tokio::time::sleep(Duration::from_secs(retry.delay())).await;

                let result: eyre::Result<Option<AccountStateCommitment>> = async {
                    debug!("Polling for OL block updates");

                    let new_head = sequencer_client
                        .get_latest_state_commitment()
                        .await
                        .map_err(|e| {
                            error!("Failed to get best OL block: {}", e);
                            e
                        })?;

                    if new_head == last_head {
                        debug!("No new OL block, continuing");
                        return Ok(None);
                    }

                    Ok(Some(new_head))
                }
                .await;

                match result {
                    Ok(Some(new_head)) => {
                        retry.reset();
                        let event = ConsensusEvent::Head(new_head.clone());

                        debug!("Yielding consensus event: {:?}", event);
                        return Some((event, (new_head, retry, sequencer_client)));
                    }
                    Ok(None) => {
                        retry.reset();
                        continue;
                    }
                    Err(e) => {
                        retry.increment();
                        let retry_delay = retry.delay();

                        error!(
                            "ol_tracker error (retry #{}, next retry in {:?}): {}",
                            retry.count(),
                            retry_delay,
                            e
                        );
                        continue;
                    }
                }
            }
        },
    )
}
