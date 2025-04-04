// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

pub mod db_generator;
pub mod transaction_committer;
pub mod transaction_executor;
pub mod transaction_generator;

use crate::{
    transaction_committer::TransactionCommitter, transaction_executor::TransactionExecutor,
    transaction_generator::TransactionGenerator,
};
use aptos_config::config::{NodeConfig, RocksdbConfig, NO_OP_STORAGE_PRUNER_CONFIG};
use aptos_logger::prelude::*;

use aptos_vm::AptosVM;
use aptosdb::AptosDB;
use executor::block_executor::BlockExecutor;
use executor_types::BlockExecutorTrait;
use std::{
    fs,
    path::Path,
    sync::{mpsc, Arc},
};
use storage_interface::{DbReader, DbReaderWriter};

pub fn init_db_and_executor(config: &NodeConfig) -> (Arc<dyn DbReader>, BlockExecutor<AptosVM>) {
    let (db, dbrw) = DbReaderWriter::wrap(
        AptosDB::open(
            &config.storage.dir(),
            false,                       /* readonly */
            NO_OP_STORAGE_PRUNER_CONFIG, /* pruner */
            RocksdbConfig::default(),
        )
        .expect("DB should open."),
    );

    let executor = BlockExecutor::new(dbrw);

    (db, executor)
}

/// Runs the benchmark with given parameters.
pub fn run_benchmark(
    block_size: usize,
    num_transfer_blocks: usize,
    source_dir: impl AsRef<Path>,
    checkpoint_dir: impl AsRef<Path>,
    verify: bool,
) {
    // Create rocksdb checkpoint.
    if checkpoint_dir.as_ref().exists() {
        fs::remove_dir_all(checkpoint_dir.as_ref()).unwrap_or(());
    }
    std::fs::create_dir_all(checkpoint_dir.as_ref()).unwrap();

    AptosDB::open(
        &source_dir,
        true,                        /* readonly */
        NO_OP_STORAGE_PRUNER_CONFIG, /* pruner */
        RocksdbConfig::default(),
    )
    .expect("db open failure.")
    .create_checkpoint(checkpoint_dir.as_ref())
    .expect("db checkpoint creation fails.");

    let (mut config, genesis_key) = aptos_genesis_tool::test_config();
    config.storage.dir = checkpoint_dir.as_ref().to_path_buf();

    let (db, executor) = init_db_and_executor(&config);
    let start_version = db.get_latest_version().unwrap();
    let parent_block_id = executor.committed_block_id();
    let executor_1 = Arc::new(executor);
    let executor_2 = executor_1.clone();

    let (block_sender, block_receiver) = mpsc::sync_channel(50 /* bound */);
    let (commit_sender, commit_receiver) = mpsc::sync_channel(3 /* bound */);

    let mut generator = TransactionGenerator::new_with_existing_db(
        genesis_key,
        block_sender,
        source_dir,
        start_version,
    );
    let start_version = generator.version();

    // Spawn two threads to run transaction generator and executor separately.
    let gen_thread = std::thread::Builder::new()
        .name("txn_generator".to_string())
        .spawn(move || {
            generator.run_transfer(block_size, num_transfer_blocks);
            generator
        })
        .expect("Failed to spawn transaction generator thread.");
    let exe_thread = std::thread::Builder::new()
        .name("txn_executor".to_string())
        .spawn(move || {
            let mut exe = TransactionExecutor::new(
                executor_1,
                parent_block_id,
                start_version,
                Some(commit_sender),
            );
            while let Ok(transactions) = block_receiver.recv() {
                info!("Received block of size {:?} to execute", transactions.len());
                exe.execute_block(transactions);
            }
        })
        .expect("Failed to spawn transaction executor thread.");
    let commit_thread = std::thread::Builder::new()
        .name("txn_committer".to_string())
        .spawn(move || {
            let mut committer =
                TransactionCommitter::new(executor_2, start_version, commit_receiver);
            committer.run();
        })
        .expect("Failed to spawn transaction committer thread.");

    // Wait for generator to finish.
    let mut generator = gen_thread.join().unwrap();
    generator.drop_sender();
    // Wait until all transactions are committed.
    exe_thread.join().unwrap();
    commit_thread.join().unwrap();

    // Do a sanity check on the sequence number to make sure all transactions are committed.
    if verify {
        generator.verify_sequence_number(db.clone());
    }
}

#[cfg(test)]
mod tests {
    use aptos_config::config::NO_OP_STORAGE_PRUNER_CONFIG;
    use aptos_temppath::TempPath;

    #[test]
    fn test_benchmark() {
        let storage_dir = TempPath::new();
        let checkpoint_dir = TempPath::new();

        crate::db_generator::run(
            25,    /* num_accounts */
            10000, /* init_account_balance */
            5,     /* block_size */
            storage_dir.as_ref(),
            NO_OP_STORAGE_PRUNER_CONFIG, /* prune_window */
        );

        super::run_benchmark(
            5, /* block_size */
            5, /* num_transfer_blocks */
            storage_dir.as_ref(),
            checkpoint_dir,
            false,
        );
    }
}
