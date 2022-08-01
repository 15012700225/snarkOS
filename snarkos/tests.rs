// Copyright (C) 2019-2022 Aleo Systems Inc.
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

use crate::Node;
use snarkos_consensus::account::Account;
use snarkos_environment::TestEnvironment;
use snarkos_network::message::DisconnectReason;

use clap::Parser;
use std::net::SocketAddr;

/// A type alias for the current version of the network.
pub type CurrentNetwork = snarkvm::prelude::Testnet3;

/// A snarkOS node used for local testing.
pub struct TestNode {
    pub node: Node<CurrentNetwork, TestEnvironment<CurrentNetwork>>,
}

impl TestNode {
    /// Returns the local listening address of the node.
    pub fn local_ip(&self) -> SocketAddr {
        *self.node.local_ip()
    }

    /// Returns the list of connected peers of the node.
    pub async fn connected_peers(&self) -> Vec<SocketAddr> {
        self.node.peers().connected_peers().await
    }

    /// Returns the number of connected peers of the node.
    pub async fn number_of_connected_peers(&self) -> usize {
        self.node.peers().number_of_connected_peers().await
    }

    /// Resets the node's known peers. This is practical, as it makes the node not reconnect
    /// to known peers in test cases where it's undesirable.
    pub async fn reset_known_peers(&self) {
        self.node.peers().reset_known_peers().await
    }

    /// Attempts to connect the node to the given address.
    pub async fn connect(&self, addr: SocketAddr) -> anyhow::Result<()> {
        self.node.connect_to(addr).await
    }

    /// Disconnects the node from the given address.
    pub async fn disconnect(&self, addr: SocketAddr) {
        self.node.disconnect_from(addr, DisconnectReason::NoReasonGiven).await
    }

    /// Starts a snarkOS node with all the default characteristics from `TestNode::with_args`.
    pub async fn default() -> Self {
        TestNode::with_args(&["--node", "127.0.0.1:0"]).await
    }

    /// Starts a snarkOS node with a manually specified ip and port.
    pub(crate) async fn new_with_custom_ip(ip: &str, port: u16) -> Self {
        TestNode::with_args(&["--node", &format!("{ip}:{port}")]).await
    }

    /// Starts a snarkOS node with a local address and the RPC server disabled;
    /// extra arguments may be passed via `extra_args`.
    pub async fn with_args(extra_args: &[&str]) -> Self {
        let permanent_args = &["snarkos", "--norpc"];
        let combined_args = permanent_args.iter().chain(extra_args.iter());
        let config = crate::CLI::parse_from(combined_args);
        let node = Node::<CurrentNetwork, TestEnvironment<CurrentNetwork>>::new(&config, Account::sample().unwrap())
            .await
            .unwrap();

        TestNode { node }
    }

    pub async fn shut_down(&self) {
        self.node.shut_down().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snarkos_environment::Environment;

    macro_rules! wait_until {
        ($limit_secs: expr, $condition: expr) => {
            let now = std::time::Instant::now();
            loop {
                if $condition {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                println!("lsdjflsdjf");
                assert!(now.elapsed() <= std::time::Duration::from_secs($limit_secs), "timed out!");
            }
        };
    }

    #[tokio::test]
    async fn test_node_connection() {
        // Start 2 snarkOS nodes.
        let test_node1 = TestNode::new_with_custom_ip("127.0.0.1", 3000).await;
        let test_node2 = TestNode::new_with_custom_ip("127.0.0.1", 4000).await;

        // Connect one to the other.
        test_node1.connect(test_node2.local_ip()).await.unwrap();

        assert_eq!(test_node1.number_of_connected_peers().await, 1);
        assert!(test_node1.connected_peers().await.contains(&test_node2.local_ip()));

        assert_eq!(test_node2.number_of_connected_peers().await, 1);
        assert!(test_node2.connected_peers().await.contains(&test_node1.local_ip()));
    }

    #[tokio::test]
    async fn test_node_cant_connect_twice() {
        // Start 2 snarkOS nodes.
        let test_node1 = TestNode::new_with_custom_ip("127.0.0.1", 5000).await;
        let test_node2 = TestNode::new_with_custom_ip("127.0.0.1", 6000).await;

        // Connect the snarkOS node to the test node.
        test_node1.connect(test_node2.local_ip()).await.unwrap();

        // The second connection attempt should fail.
        assert!(test_node1.connect(test_node2.local_ip()).await.is_err());
    }

    // TODO (raychu86): Implement disconnect.
    // #[tokio::test]
    // async fn test_node_disconnect() {
    //     // Start 2 snarkOS nodes.
    //     let test_node1 = TestNode::new_with_custom_ip("127.0.0.1", 7000).await;
    //     let test_node2 = TestNode::new_with_custom_ip("127.0.0.1", 8000).await;
    //
    //     // Connect the snarkOS node to the test node.
    //     test_node1.connect(test_node2.local_addr()).await.unwrap();
    //
    //     // Disconnect the snarkOS nodes.
    //     test_node1.disconnect(test_node2.local_addr()).await;
    //
    //     assert_eq!(test_node1.number_of_connected_peers().await, 0);
    //     assert!(!test_node1.connected_peers().await.contains(&test_node2.local_addr()));
    //
    //     assert_eq!(test_node2.number_of_connected_peers().await, 0);
    //     assert!(!test_node2.connected_peers().await.contains(&test_node1.local_addr()));
    // }

    #[tokio::test]
    async fn test_node_maximum_peers() {
        const MAXIMUM_NUMBER_OF_PEERS: usize = TestEnvironment::<CurrentNetwork>::MAXIMUM_NUMBER_OF_PEERS as usize;

        // Start a snarkOS node.
        let main_test_node = TestNode::new_with_custom_ip("127.0.0.1", 3333).await;

        // Start the maximum number of test nodes the snarkOS node is permitted to connect to at once.
        let mut test_nodes = Vec::with_capacity(MAXIMUM_NUMBER_OF_PEERS);
        for i in 0..MAXIMUM_NUMBER_OF_PEERS {
            test_nodes.push(TestNode::new_with_custom_ip("127.0.0.1", (3000 + i) as u16).await);
        }

        // Create one additional test node.
        let extra_test_node = TestNode::new_with_custom_ip("127.0.0.1", 3334).await;

        // All the test nodes should be able to connect to the snarkOS node.
        for test_node in &test_nodes {
            test_node.connect(main_test_node.local_ip()).await.unwrap();
        }

        // A short sleep to ensure all the connections are ready.
        wait_until!(1, main_test_node.number_of_connected_peers().await == MAXIMUM_NUMBER_OF_PEERS);

        // Assert that snarkOS node can't connect to the extra node.
        assert!(main_test_node.connect(extra_test_node.local_ip()).await.is_err());
    }
}
