// Copyright (C) 2019-2021 Aleo Systems Inc.
// This file is part of the snarkOS library.

// The snarkOS library is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// The snarkOS library is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with the snarkOS library. If not, see <https://www.gnu.org/licenses/>.

use crate::{helpers::Tasks, Environment, NodeType};
use snarkos_ledger::{ledger::Ledger, storage::rocksdb::RocksDB};
use snarkvm::dpc::{Address, Block, Network};

use anyhow::Result;
use once_cell::sync::OnceCell;
use rand::thread_rng;
use std::{
    marker::PhantomData,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{net::TcpListener, task};

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[repr(u8)]
pub enum Status {
    Idle = 0,
    Mining,
    Syncing,
    ShuttingDown,
}

/// A node server implementation.
#[derive(Clone, Debug)]
pub struct Node<N: Network, E: Environment<N>> {
    /// The current status of the node.
    status: Arc<AtomicU8>,
    /// The local address of this node.
    local_ip: OnceCell<SocketAddr>,
    /// The ledger state of the node.
    ledger: Ledger<N>,
    /// The list of tasks spawned by the node.
    tasks: Tasks<task::JoinHandle<()>>,
    /// A terminator bit for the miner.
    terminator: Arc<AtomicBool>,
    /// Phantom data.
    _phantom: PhantomData<E>,
}

impl<N: Network, E: Environment<N>> Node<N, E> {
    pub fn new() -> Result<Self> {
        // Initialize the node.
        let node = Self {
            status: Arc::new(AtomicU8::new(0)),
            local_ip: OnceCell::new(),
            ledger: Ledger::<N>::open::<RocksDB, _>(".ledger")?,
            tasks: Tasks::new(),
            terminator: Arc::new(AtomicBool::new(false)),
            _phantom: PhantomData,
        };
        Ok(node)
    }

    /// Returns the current status of the node.
    #[inline]
    pub fn status(&self) -> Status {
        match self.status.load(Ordering::SeqCst) {
            0 => Status::Idle,
            1 => Status::Mining,
            2 => Status::Syncing,
            3 => Status::ShuttingDown,
            _ => unreachable!("Invalid status code"),
        }
    }

    /// Updates the node to the given status.
    #[inline]
    pub fn set_status(&self, state: Status) {
        self.status.store(state as u8, Ordering::SeqCst);

        match state {
            Status::ShuttingDown => {
                // debug!("Shutting down");
                self.terminator.store(true, Ordering::SeqCst);
                self.tasks.flush();
            }
            _ => (),
        }
    }

    /// Updates the local IP address of the node to the given address.
    #[inline]
    pub fn set_local_ip(&self, ip_address: SocketAddr) {
        self.local_ip.set(ip_address).expect("The local IP address was set more than once!");
    }

    // /// Initializes a listener for connections.
    // pub async fn start_listener(&self) -> Result<()> {
    //     let listener = TcpListener::bind(&self.config.desired_address).await?;
    //
    //     // Update the local IP address of the node.
    //     let discovered_local_ip = listener.local_addr()?;
    //     self.set_local_ip(discovered_local_ip);
    //
    //     info!("Initializing the listener...");
    //     let node = self.clone();
    //     self.add_task(task::spawn(async move {
    //         info!("Listening for peers at {}", discovered_local_ip);
    //         loop {
    //             match listener.accept().await {
    //                 Ok((stream, remote_address)) => {
    //                     if !node.can_connect() {
    //                         continue;
    //                     }
    //                     let node_clone = node.clone();
    //                     tokio::spawn(async move {
    //                         if let Err(error) = node_clone.peer_book.receive_connection(node_clone.clone(), remote_address, stream) {
    //                             error!("Failed to receive a connection: {}", error);
    //                         }
    //                     });
    //                     // Adds a small delay to avoid connecting above the limit.
    //                     tokio::time::sleep(Duration::from_millis(1)).await;
    //                 }
    //                 Err(error) => error!("Failed to accept a connection: {}", error),
    //             }
    //             // metrics::increment_counter!(connections::ALL_ACCEPTED);
    //         }
    //     }));
    //     Ok(())
    // }

    /// Initializes a miner.
    #[inline]
    pub fn start_miner(&self, miner_address: Address<N>) {
        // If the node is a mining node, initialize a miner.
        if E::NODE_TYPE == NodeType::Miner {
            let node = self.clone();
            self.add_task(task::spawn(async move {
                let rng = &mut thread_rng();
                let mut ledger = node.ledger.clone();
                loop {
                    // Retrieve the status of the node.
                    let status = node.status();
                    // Ensure the node is not syncing or shutting down.
                    if status != Status::Syncing && status != Status::ShuttingDown {
                        // Set the status of the node to mining.
                        node.set_status(Status::Mining);
                        // Start the mining process.
                        let miner = ledger.mine_next_block(miner_address, &node.terminator, rng);
                        // Ensure the miner did not error.
                        if let Err(error) = miner {
                            error!("{}", error);
                        }
                    }
                }
            }));
        }
    }

    /// Adds the given task handle to the node.
    #[inline]
    pub fn add_task(&self, handle: task::JoinHandle<()>) {
        self.tasks.append(handle);
    }

    /// Disconnects from peers and proceeds to shut down the node.
    #[inline]
    pub async fn shut_down(&self) {
        self.set_status(Status::ShuttingDown);
        // for address in self.connected_peers() {
        //     self.disconnect_from_peer(address).await;
        // }
    }
}
