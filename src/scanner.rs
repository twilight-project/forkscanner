use bitcoincore_rpc::bitcoin as btc;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use bitcoincore_rpc::bitcoincore_rpc_json::GetChainTipsResultStatus;
use crossbeam_channel::Sender;
use diesel::prelude::PgConnection;
use crate::{Block, Chaintip};
use log::{debug, error, info};
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use std::thread::sleep;
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

fn create_block_and_ancestors(client: &Client, conn: &PgConnection, block_hash: &String) -> ForkScannerResult<()> {
    let mut hash = btc::BlockHash::from_str(block_hash)?;

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
    host: String,
    clients: Vec<Client>,
    db_conn: PgConnection,
    should_exit: Arc<AtomicBool>,
}

impl ForkScanner {
    pub fn new(db_conn: PgConnection, host: &str, auth: Auth) -> ForkScannerResult<ForkScanner> {
        let client = Client::new(host, auth)?;
        let should_exit = Arc::new(AtomicBool::new(false));

        Ok(ForkScanner {
            host: host.to_string(),
            clients: vec![client],
            db_conn,
            should_exit,
        })
    }

    fn process_client(&self, client: &Client) {
        let tips = match client.get_chain_tips() {
            Ok(tips) => tips,
            Err(e) => {
                error!("rpc error {:?}", e);
                sleep(Duration::from_millis(500));
                return;
            }
        };

        if let Err(e) = Chaintip::purge(&self.db_conn) {
            error!("Error purging database {:?}", e);
            sleep(Duration::from_millis(500));
            return;
        }

        for tip in tips {
            let hash = tip.hash.to_string();

            match tip.status {
                GetChainTipsResultStatus::HeadersOnly => {
                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, &hash) {
                        error!("Failed fetching ancestors {:?}", e);
                        break;
                    }
                }
                GetChainTipsResultStatus::ValidHeaders => {
                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, &hash) {
                        error!("Failed fetching ancestors {:?}", e);
                        break;
                    }
                }
                GetChainTipsResultStatus::Invalid => {
                    if let Err(e) = Chaintip::set_invalid_fork(&self.db_conn, tip.height as i64, &hash, &self.host) {
                        error!("Failed setting chaintip {:?}", e);
                        break;
                    }

                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, &hash) {
                        error!("Failed fetching ancestors {:?}", e);
                        break;
                    }

                    if let Err(e) = Block::set_invalid(&self.db_conn, &hash, &self.host) {
                        error!("Failed setting valid {:?}", e);
                        break;
                    }
                }
                GetChainTipsResultStatus::ValidFork => {
                    if let Err(e) = Chaintip::set_valid_fork(&self.db_conn, tip.height as i64, &hash, &self.host) {
                        error!("Failed setting chaintip {:?}", e);
                        break;
                    }

                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, &hash) {
                        error!("Failed fetching ancestors {:?}", e);
                        break;
                    }

                    if let Err(e) = Block::set_valid(&self.db_conn, &hash, &self.host) {
                        error!("Failed setting valid {:?}", e);
                        break;
                    }
                }
                GetChainTipsResultStatus::Active => {
                    if let Err(e) = Chaintip::set_active_tip(&self.db_conn, tip.height as i64, &hash, &self.host) {
                        error!("Failed setting chaintip {:?}", e);
                        break;
                    }

                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, &hash) {
                        error!("Failed fetching ancestors {:?}", e);
                        break;
                    }

                    if let Err(e) = Block::set_valid(&self.db_conn, &hash, &self.host) {
                        error!("Failed setting valid {:?}", e);
                        break;
                    }
                }
            }
        }
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

            for client in &self.clients {
                self.process_client(&client);
            }


            // TODO: check ancestries....
            sleep(Duration::from_millis(10000));
        }
    }
}
