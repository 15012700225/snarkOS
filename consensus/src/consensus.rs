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

use crate::{error::ConsensusError, ConsensusParameters, MemoryPool, MerkleTreeLedger};
use snarkos_metrics::{
    self as metrics,
    misc::{BLOCK_HEIGHT, *},
};
use snarkos_storage::BlockPath;
use snarkvm::{
    algorithms::CRH,
    dpc::{
        testnet1::{Testnet1DPC, Testnet1Parameters, Testnet1Transaction},
        Account,
        AccountScheme,
        Address,
        AleoAmount,
        DPCScheme,
        Parameters,
        Payload,
        PrivateKey,
        Program,
        Record,
    },
    ledger::{posw::txids_to_roots, Block, LedgerScheme, Storage, StorageError, Transactions},
    utilities::{to_bytes_le, ToBytes},
};

use rand::{CryptoRng, Rng};
use std::sync::Arc;

pub struct Consensus<S: Storage> {
    pub parameters: ConsensusParameters,
    pub dpc: Arc<Testnet1DPC>,
    pub ledger: Arc<MerkleTreeLedger<S>>,
    pub memory_pool: MemoryPool<Testnet1Transaction>,
}

impl<S: Storage> Consensus<S> {
    /// Check if the transaction is valid.
    pub fn verify_transaction(&self, transaction: &Testnet1Transaction) -> Result<bool, ConsensusError> {
        if !self
            .parameters
            .authorized_inner_snark_ids
            .contains(&to_bytes_le![transaction.inner_circuit_id]?)
        {
            return Ok(false);
        }

        Ok(self.dpc.verify(transaction, &*self.ledger))
    }

    /// Check if the transactions are valid.
    pub fn verify_transactions(&self, transactions: &[Testnet1Transaction]) -> Result<bool, ConsensusError> {
        for tx in transactions {
            if !self
                .parameters
                .authorized_inner_snark_ids
                .contains(&to_bytes_le![tx.inner_circuit_id]?)
            {
                return Ok(false);
            }
        }

        Ok(self.dpc.verify_transactions(transactions, &*self.ledger))
    }

    /// Check if the block is valid.
    /// Verify transactions and transaction fees.
    pub fn verify_block(&self, block: &Block<Testnet1Transaction>) -> Result<bool, ConsensusError> {
        let transaction_ids: Vec<_> = block.transactions.to_transaction_ids()?;
        let (merkle_root, pedersen_merkle_root, _) = txids_to_roots(&transaction_ids);

        // Verify the block header
        if !crate::is_genesis(&block.header) {
            let parent_block = self.ledger.latest_block()?;
            if let Err(err) =
                self.parameters
                    .verify_header(&block.header, &parent_block.header, &merkle_root, &pedersen_merkle_root)
            {
                error!("block header failed to verify: {:?}", err);
                return Ok(false);
            }
        }
        // Verify block amounts and check that there is a single coinbase transaction

        let mut coinbase_transaction_count = 0;
        let mut total_value_balance = AleoAmount::ZERO;

        for transaction in block.transactions.iter() {
            let value_balance = transaction.value_balance;

            if value_balance.is_negative() {
                coinbase_transaction_count += 1;
            }

            total_value_balance = total_value_balance.add(value_balance);
        }

        // Check that there is only 1 coinbase transaction
        if coinbase_transaction_count > 1 {
            error!("multiple coinbase transactions");
            return Ok(false);
        }

        // Check that the block value balances are correct
        let expected_block_reward = crate::get_block_reward(self.ledger.block_height()).0;
        if total_value_balance.0 + expected_block_reward != 0 {
            trace!("total_value_balance: {:?}", total_value_balance);
            trace!("expected_block_reward: {:?}", expected_block_reward);

            return Ok(false);
        }

        // Check that all the transaction proofs verify
        self.verify_transactions(&block.transactions.0)
    }

    /// Receive a block from an external source and process it based on ledger state.
    pub async fn receive_block(
        &self,
        block: &Block<Testnet1Transaction>,
        batch_import: bool,
    ) -> Result<(), ConsensusError> {
        // Block is an unknown orphan
        if !self.ledger.previous_block_hash_exists(block) && !self.ledger.is_previous_block_canon(&block.header) {
            debug!("Processing a block that is an unknown orphan");
            println!("ORPHAN");

            // There are two possible cases for an unknown orphan.
            // 1) The block is a genesis block, or
            // 2) The block is unknown and does not correspond with the canon chain.
            if crate::is_genesis(&block.header) && self.ledger.is_empty() {
                self.process_block(block).await?;
            } else {
                metrics::increment_counter!(ORPHAN_BLOCKS);
                self.ledger.insert_only(block)?;
            }
        } else {
            // If the block is not an unknown orphan, find the origin of the block
            match self.ledger.get_block_path(&block.header)? {
                BlockPath::ExistingBlock => {
                    debug!("Received a pre-existing block");
                    return Err(ConsensusError::PreExistingBlock);
                }
                BlockPath::CanonChain(block_height) => {
                    println!("CANON");

                    debug!("Processing a block that is on canon chain. Height {}", block_height);

                    self.process_block(block).await?;

                    if !batch_import {
                        // Attempt to fast forward the block state if the node already stores
                        // the children of the new canon block.
                        let child_path = self.ledger.longest_child_path(block.header.get_hash())?;

                        if child_path.len() > 1 {
                            debug!(
                                "Attempting to canonize the descendants of block at height {}.",
                                block_height
                            );
                        }

                        for child_block_hash in child_path.into_iter().skip(1) {
                            let new_block = self.ledger.get_block(&child_block_hash)?;

                            debug!(
                                "Processing the next known descendant. Height {}",
                                self.ledger.block_height()
                            );
                            self.process_block(&new_block).await?;
                        }
                    }
                }
                BlockPath::SideChain(side_chain_path) => {
                    println!("SIDE");

                    debug!(
                        "Processing a block that is on side chain. Height {}",
                        side_chain_path.new_block_number
                    );

                    // If the side chain is now heavier than the canon chain,
                    // perform a fork to the side chain.
                    let canon_difficulty =
                        self.get_canon_difficulty_from_height(side_chain_path.shared_block_number)?;

                    if side_chain_path.aggregate_difficulty > canon_difficulty {
                        debug!(
                            "Determined side chain is heavier than canon chain by {}%",
                            get_delta_percentage(side_chain_path.aggregate_difficulty, canon_difficulty)
                        );
                        warn!("A valid fork has been detected. Performing a fork to the side chain.");

                        // Fork to superior side chain
                        self.ledger.revert_for_fork(&side_chain_path)?;

                        // Update the current block height metric.
                        metrics::gauge!(BLOCK_HEIGHT, self.ledger.block_height() as f64);

                        if !side_chain_path.path.is_empty() {
                            for block_hash in side_chain_path.path {
                                if block_hash == block.header.get_hash() {
                                    self.process_block(block).await?
                                } else {
                                    let new_block = self.ledger.get_block(&block_hash)?;
                                    self.process_block(&new_block).await?;
                                }
                            }
                        }
                    } else {
                        metrics::increment_counter!(ORPHAN_BLOCKS);

                        // If the sidechain is not longer than the main canon chain, simply store the block
                        self.ledger.insert_only(block)?;
                    }
                }
            };
        }

        Ok(())
    }

    /// Return whether or not the given block is valid and insert it.
    /// 1. Verify that the block header is valid.
    /// 2. Verify that the transactions are valid.
    /// 3. Insert/canonize block.
    pub async fn process_block(&self, block: &Block<Testnet1Transaction>) -> Result<(), ConsensusError> {
        if self.ledger.is_canon(&block.header.get_hash()) {
            return Ok(());
        }

        // 1. Verify that the block valid
        if !self.verify_block(block)? {
            return Err(ConsensusError::InvalidBlock(block.header.get_hash().0.to_vec()));
        }

        // 2. Insert/canonize block
        self.ledger.insert_and_commit(block)?;

        // Increment the current block height metric
        metrics::increment_gauge!(BLOCK_HEIGHT, 1.0);

        // 3. Remove transactions from the mempool
        for transaction_id in block.transactions.to_transaction_ids()? {
            self.memory_pool.remove_by_hash(&transaction_id).await?;
        }

        Ok(())
    }

    /// Generate a transaction by spending old records and specifying new record attributes
    #[allow(clippy::too_many_arguments)]
    pub fn create_transaction<R: Rng + CryptoRng>(
        &self,
        old_records: Vec<Record<Testnet1Parameters>>,
        private_keys: Vec<PrivateKey<Testnet1Parameters>>,
        new_records: Vec<Record<Testnet1Parameters>>,
        memo: Option<[u8; 64]>,
        rng: &mut R,
    ) -> Result<Testnet1Transaction, ConsensusError> {
        // Offline execution to generate a transaction authorization.
        let authorization = self
            .dpc
            .authorize::<R>(&private_keys, old_records, new_records, memo, rng)?;

        // Generate the local data.
        let local_data = authorization.to_local_data(rng)?;

        // Construct the program proofs.
        let program_proofs = ConsensusParameters::generate_program_proofs::<S>(&self.dpc, &local_data)?;

        // Online execution to generate a transaction.
        let transaction = self.dpc.execute(
            &private_keys,
            authorization,
            &local_data,
            program_proofs,
            &*self.ledger,
            rng,
        )?;

        Ok(transaction)
    }

    /// Generate a coinbase transaction given candidate block transactions
    #[allow(clippy::too_many_arguments)]
    pub fn create_coinbase_transaction<R: Rng + CryptoRng>(
        &self,
        block_num: u32,
        transactions: &Transactions<Testnet1Transaction>,
        old_programs: Vec<&dyn Program<Testnet1Parameters>>,
        new_programs: Vec<&dyn Program<Testnet1Parameters>>,
        recipient: Address<Testnet1Parameters>,
        rng: &mut R,
    ) -> Result<(Vec<Record<Testnet1Parameters>>, Testnet1Transaction), ConsensusError> {
        let mut total_value_balance = crate::get_block_reward(block_num);
        for transaction in transactions.iter() {
            let tx_value_balance = transaction.value_balance;
            if tx_value_balance.is_negative() {
                return Err(ConsensusError::CoinbaseTransactionAlreadyExists());
            }

            total_value_balance = total_value_balance.add(transaction.value_balance);
        }

        // Generate a new account that owns the dummy input records
        let new_account = Account::new(rng).unwrap();

        // Generate dummy input records having as address the genesis address.
        let private_keys = vec![new_account.private_key.clone(); Testnet1Parameters::NUM_INPUT_RECORDS];
        let mut old_records = Vec::with_capacity(Testnet1Parameters::NUM_INPUT_RECORDS);
        for i in 0..Testnet1Parameters::NUM_INPUT_RECORDS {
            let sn_nonce_input: [u8; 4] = rng.gen();

            let old_record = Record::new(
                old_programs[i],
                new_account.address.clone(),
                true, // The input record is dummy
                0,
                Payload::default(),
                <Testnet1Parameters as Parameters>::serial_number_nonce_crh().hash(&sn_nonce_input)?,
                rng,
            )?;

            old_records.push(old_record);
        }

        let new_is_dummy_flags = [vec![false], vec![true; Testnet1Parameters::NUM_OUTPUT_RECORDS - 1]].concat();
        let new_values = [vec![total_value_balance.0 as u64], vec![
            0;
            Testnet1Parameters::NUM_OUTPUT_RECORDS
                - 1
        ]]
        .concat();

        let mut joint_serial_numbers = vec![];
        for i in 0..Testnet1Parameters::NUM_INPUT_RECORDS {
            let (sn, _) = old_records[i].to_serial_number(&private_keys[i])?;
            joint_serial_numbers.extend_from_slice(&to_bytes_le![sn]?);
        }

        let mut new_records = vec![];
        for j in 0..Testnet1Parameters::NUM_OUTPUT_RECORDS {
            new_records.push(Record::new_full(
                new_programs[j],
                recipient.clone(),
                new_is_dummy_flags[j],
                new_values[j],
                Payload::default(),
                (Testnet1Parameters::NUM_INPUT_RECORDS + j) as u8,
                joint_serial_numbers.clone(),
                rng,
            )?);
        }

        let transaction = self.create_transaction(old_records, private_keys, new_records.clone(), None, rng)?;

        Ok((new_records, transaction))
    }

    fn get_canon_difficulty_from_height(&self, height: u32) -> Result<u128, StorageError> {
        let current_block_height = self.ledger.block_height();
        let path_size = current_block_height - height;
        let mut aggregate_difficulty = 0u128;

        for i in 0..path_size {
            let block_header = self
                .ledger
                .get_block_header(&self.ledger.get_block_hash(current_block_height - i)?)?;

            aggregate_difficulty += block_header.difficulty_target as u128;
        }

        Ok(aggregate_difficulty)
    }
}

fn get_delta_percentage(side_chain_diff: u128, canon_diff: u128) -> f64 {
    let delta = side_chain_diff - canon_diff;
    (delta as f64 / canon_diff as f64) * 100.0
}
