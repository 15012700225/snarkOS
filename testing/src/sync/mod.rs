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

use snarkos_consensus::{ConsensusParameters, MerkleTreeLedger};
use snarkos_storage::LedgerStorage;
use snarkvm_algorithms::CRH;
use snarkvm_dpc::{testnet1::parameters::Testnet1Parameters, Network, Parameters, TransactionError, TransactionScheme};
use snarkvm_parameters::{testnet1::InnerSNARKVKParameters, Parameter};
use snarkvm_posw::PoswMarlin;
use snarkvm_utilities::{to_bytes_le, FromBytes, ToBytes};

use once_cell::sync::Lazy;
use std::{
    io::{Read, Result as IoResult, Write},
    sync::Arc,
};

mod data;
pub use data::*;

mod fixture;
pub use fixture::*;

pub static TEST_CONSENSUS_PARAMS: Lazy<ConsensusParameters> = Lazy::new(|| {
    let inner_snark_id = to_bytes_le![<Testnet1Parameters as Parameters>::inner_circuit_id_crh()
        .hash(&InnerSNARKVKParameters::load_bytes().unwrap())
        .unwrap()]
    .unwrap();

    ConsensusParameters {
        max_block_size: 1_000_000usize,
        max_nonce: u32::max_value(),
        target_block_time: 2i64, //unix seconds
        network_id: Network::Testnet1,
        verifier: PoswMarlin::verify_only().unwrap(),
        authorized_inner_snark_ids: vec![inner_snark_id],
    }
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestTestnet1Transaction;

impl TransactionScheme for TestTestnet1Transaction {
    type Commitment = [u8; 32];
    type Digest = [u8; 32];
    type EncryptedRecord = [u8; 32];
    type InnerCircuitID = [u8; 32];
    type Memorandum = [u8; 64];
    type SerialNumber = [u8; 32];
    type Signature = [u8; 32];
    type ValueBalance = i64;

    fn transaction_id(&self) -> Result<[u8; 32], TransactionError> {
        Ok([0u8; 32])
    }

    fn network_id(&self) -> u8 {
        0
    }

    fn ledger_digest(&self) -> &Self::Digest {
        &[0u8; 32]
    }

    fn inner_circuit_id(&self) -> &Self::InnerCircuitID {
        &[0u8; 32]
    }

    fn old_serial_numbers(&self) -> &[Self::SerialNumber] {
        &[[0u8; 32]; 2]
    }

    fn new_commitments(&self) -> &[Self::Commitment] {
        &[[0u8; 32]; 2]
    }

    fn value_balance(&self) -> i64 {
        0
    }

    fn memorandum(&self) -> &Self::Memorandum {
        &[0u8; 64]
    }

    fn signatures(&self) -> &[Self::Signature] {
        &[[0u8; 32]; 2]
    }

    fn encrypted_records(&self) -> &[Self::EncryptedRecord] {
        &[[0u8; 32]; 2]
    }

    fn size(&self) -> usize {
        0
    }
}

impl ToBytes for TestTestnet1Transaction {
    #[inline]
    fn write_le<W: Write>(&self, mut _writer: W) -> IoResult<()> {
        Ok(())
    }
}

impl FromBytes for TestTestnet1Transaction {
    #[inline]
    fn read_le<R: Read>(mut _reader: R) -> IoResult<Self> {
        Ok(Self)
    }
}

pub fn create_test_consensus() -> snarkos_consensus::Consensus<LedgerStorage> {
    create_test_consensus_from_ledger(Arc::new(FIXTURE_VK.ledger()))
}

pub fn create_test_consensus_from_ledger(
    ledger: Arc<MerkleTreeLedger<LedgerStorage>>,
) -> snarkos_consensus::Consensus<LedgerStorage> {
    snarkos_consensus::Consensus {
        ledger,
        memory_pool: Default::default(),
        parameters: TEST_CONSENSUS_PARAMS.clone(),
        dpc: FIXTURE.dpc.clone(),
    }
}
