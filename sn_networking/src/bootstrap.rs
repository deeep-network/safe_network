// Copyright 2023 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::SwarmDriver;
use std::time::{Duration, Instant};
use tokio::time::Interval;

/// The interval in which kad.bootstrap is called
pub(crate) const BOOTSTRAP_INTERVAL: Duration = Duration::from_secs(5);

/// Every BOOTSTRAP_CONNECTED_PEERS_STEP connected peer, we step up the BOOTSTRAP_INTERVAL to slow down bootstrapping
/// process
const BOOTSTRAP_CONNECTED_PEERS_STEP: u32 = 50;

/// If the previously added peer has been before LAST_PEER_ADDED_TIME_LIMIT, then we should slowdown the bootstrapping
/// process. This is to make sure we don't flood the network with `FindNode` msgs.
const LAST_PEER_ADDED_TIME_LIMIT: Duration = Duration::from_secs(180);

/// The bootstrap interval to use if we haven't added any new peers in a while.
const NO_PEER_ADDED_SLOWDOWN_INTERVAL: Duration = Duration::from_secs(300);

impl SwarmDriver {
    pub(crate) async fn run_bootstrap_continuously(
        &mut self,
        current_bootstrap_interval: Duration,
    ) -> Option<Interval> {
        let peers_in_rt = self.swarm.connected_peers().count() as u32;

        let (should_bootstrap, new_interval) = self
            .bootstrap
            .should_we_bootstrap(peers_in_rt, current_bootstrap_interval)
            .await;
        if should_bootstrap {
            debug!("Trying to initiate bootstrap. Current bootstrap_interval {current_bootstrap_interval:?}");
            match self.swarm.behaviour_mut().kademlia.bootstrap() {
                Ok(query_id) => {
                    debug!("Initiated kad bootstrap process with query id {query_id:?}");
                    self.bootstrap.initiated();
                }
                Err(err) => {
                    error!("Failed to initiate kad bootstrap with error: {err:?}");
                }
            };
        }
        if let Some(new_interval) = &new_interval {
            debug!(
                "The new bootstrap_interval has been updated to {:?}",
                new_interval.period()
            );
        }
        new_interval
    }
}

/// Tracks and helps with the continuous kad::bootstrapping process
pub(crate) struct ContinuousBootstrap {
    is_ongoing: bool,
    last_peer_added_instant: Instant,
}

impl ContinuousBootstrap {
    pub(crate) fn new() -> Self {
        Self {
            is_ongoing: false,
            last_peer_added_instant: Instant::now(),
        }
    }

    /// The Kademlia Bootstrap request has been sent successfully.
    pub(crate) fn initiated(&mut self) {
        self.is_ongoing = true;
    }

    /// Notify about a newly added peer to the RT. This will help with slowing down the bootstrap process.
    pub(crate) fn notify_new_peer(&mut self) {
        self.last_peer_added_instant = Instant::now();
    }

    /// A previous Kademlia Bootstrap process has been completed. Now a new bootstrap process can start.
    pub(crate) fn completed(&mut self) {
        self.is_ongoing = false;
    }

    /// Returns `true` if we should carry out the Kademlia Bootstrap process immediately.
    /// Also optionally returns the new interval to re-bootstrap.
    pub(crate) async fn should_we_bootstrap(
        &mut self,
        peers_in_rt: u32,
        current_interval: Duration,
    ) -> (bool, Option<Interval>) {
        // kad bootstrap process needs at least one peer in the RT be carried out.
        let should_bootstrap = !self.is_ongoing && peers_in_rt >= 1;

        // if it has been a while (LAST_PEER_ADDED_TIME_LIMIT) since we have added a new peer to our RT, then, slowdown
        // the bootstrapping process.
        // Don't slow down if we haven't even added one peer to our RT.
        if self.last_peer_added_instant.elapsed() > LAST_PEER_ADDED_TIME_LIMIT && peers_in_rt != 0 {
            info!(
                "It has been {LAST_PEER_ADDED_TIME_LIMIT:?} since we last added a peer to RT.
                Slowing down the continuous bootstrapping process"
            );

            let mut new_interval = tokio::time::interval(NO_PEER_ADDED_SLOWDOWN_INTERVAL);
            new_interval.tick().await; // the first tick completes immediately
            return (should_bootstrap, Some(new_interval));
        }

        // increment bootstrap_interval in steps of BOOTSTRAP_INTERVAL every BOOTSTRAP_CONNECTED_PEERS_STEP
        let step = peers_in_rt / BOOTSTRAP_CONNECTED_PEERS_STEP;
        let step = std::cmp::max(1, step);
        let new_interval = BOOTSTRAP_INTERVAL * step;
        let new_interval = if new_interval > current_interval {
            info!("More peers have been added to our RT!. Slowing down the continuous bootstrapping process");
            let mut interval = tokio::time::interval(new_interval);
            interval.tick().await; // the first tick completes immediately
            Some(interval)
        } else {
            None
        };
        (should_bootstrap, new_interval)
    }
}