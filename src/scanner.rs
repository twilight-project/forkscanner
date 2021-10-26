use bitcoincore_rpc::bitcoin as btc;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use crossbeam_channel::Sender;
use diesel::prelude::PgConnection;
use crate::Block;
use log::{debug, error, info};
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use thiserror::Error;

const MAX_BLOCK_DEPTH: usize = 10;

type ForkScannerResult<T> = Result<T, ForkScannerError>;

#[derive(Debug, Error)]
pub enum ForkScannerError {
    #[error("Failed establishing bitcoin RPC connection {0:?}")]
    RpcClientError(#[from] bitcoincore_rpc::Error),
    #[error("Failed establishing database connection {0:?}")]
    DbConnectionError(#[from] diesel::result::ConnectionError),
    #[error("Database query error {0:?}")]
    DatabaseError(#[from] diesel::result::Error),
    #[error("Env var missing  {0:?}")]
    VarError(#[from] std::env::VarError),
    #[error("Hash convert error {0:?}")]
    HexError(#[from] bitcoincore_rpc::bitcoin::hashes::hex::Error),
    #[error("Failed to fetch parent block.")]
    ParentBlockFetchError,
}

#[derive(Debug)]
pub enum ReorgMessage {
    TipUpdated(String),
    ReorgDetected(String, Vec<(i64, String)>),
}

fn create_block_and_ancestors(client: &Client, conn: &PgConnection, block_hash: btc::BlockHash) -> ForkScannerResult<()> {
    let mut hash = block_hash;

    for _ in 0..MAX_BLOCK_DEPTH {
        let bh = client.get_block_header_info(&hash)?;
        let block = Block::get_or_create(&conn, &bh)?;

        if block.connected {
            break;
        }

        match block.parent_hash {
            Some(h) => {
                hash = btc::BlockHash::from_str(&h)?;
            }
            None => break,
        }
    }

    Ok(())
}

pub struct ForkScanner {
    best_hash: btc::BlockHash,
    client: Client,
    db_conn: PgConnection,
    should_exit: Arc<AtomicBool>,
}

impl ForkScanner {
    pub fn new(db_conn: PgConnection, host: impl Into<String>, auth: Auth) -> ForkScannerResult<ForkScanner> {
        let client = Client::new(host.into(), auth)?;
        let best_hash = client.get_best_block_hash()?;
        let should_exit = Arc::new(AtomicBool::new(false));

        create_block_and_ancestors(&client, &db_conn, best_hash)?;

        Ok(ForkScanner {
            best_hash,
            client,
            db_conn,
            should_exit,
        })
    }

    // We initialized with get_best_block_hash, now we just poll continually
    // for new blocks, and fetch ancestors up to MAX_BLOCK_HEIGHT postgres
    // will do the rest for us.
    pub fn run(mut self, channel: Sender<ReorgMessage>) {
        loop {
            if self.should_exit.load(Ordering::Relaxed) {
                info!("Watcher leaving... goodbye!");
                break;
            }

            let latest = match self.client.wait_for_new_block(5000) {
                Ok(block) => block.hash,
                Err(e) => {
                    error!("Waiting for new block: {:#?}", e);
                    println!("TJDEBUUUUUG erreasdf");
                    continue;
                }
            };

            if latest == self.best_hash {
                debug!("Nothing changed");
                continue;
            }

            match create_block_and_ancestors(&self.client, &self.db_conn, latest) {
                Err(e) => error!("Failed fetching ancestors {:?}", e),
                _ => ()
            }

            self.best_hash = latest;

            match Block::find_fork(&self.db_conn) {
                Ok(result) => {
                    debug!("Got result {:?}", result);
                    if result.len() > 0 {
                        // find_fork filters on parent_hash not null, so unwrap is ok
                        let hash = &result[0].0.as_ref().unwrap();
                        let tips = Block::find_tips(&self.db_conn, &hash).expect("Database error.");

                        let message = ReorgMessage::ReorgDetected(hash.to_string(), tips);
                        channel.send(message).expect("broken channel");
                    } else {
                        let message = ReorgMessage::TipUpdated(latest.to_string());
                        channel.send(message).expect("broken channel");
                    }
                }
                Err(e) => error!("Fork query failed: {:?}", e),
            }
        }
    }
}
