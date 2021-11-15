use crate::{Block, Chaintip, Node};
use bitcoincore_rpc::bitcoin as btc;
use bitcoincore_rpc::bitcoincore_rpc_json::GetChainTipsResultStatus;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use crossbeam_channel::Sender;
use diesel::prelude::PgConnection;
use log::{debug, error, info};
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::sleep;
use std::time::Duration;
use thiserror::Error;

const MAX_BLOCK_DEPTH: usize = 100;

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

fn create_block_and_ancestors(
    client: &Client,
    conn: &PgConnection,
    block_hash: &String,
) -> ForkScannerResult<()> {
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
    node_list: Vec<Node>,
    clients: Vec<Client>,
    db_conn: PgConnection,
    should_exit: Arc<AtomicBool>,
}

impl ForkScanner {
    pub fn new(db_conn: PgConnection) -> ForkScannerResult<ForkScanner> {
        let node_list = Node::list(&db_conn)?;

        let mut clients = Vec::new();

        for node in &node_list {
            let host = format!("http://{}:{}", node.rpc_host, node.rpc_port);
            let auth = Auth::UserPass(node.rpc_user.clone(), node.rpc_pass.clone());
            let client = Client::new(&host, auth)?;
            clients.push(client);
        }

        let should_exit = Arc::new(AtomicBool::new(false));

        Ok(ForkScanner {
            node_list,
            clients,
            db_conn,
            should_exit,
        })
    }

    fn process_client(&self, client: &Client, node: &Node) {
        let tips = match client.get_chain_tips() {
            Ok(tips) => tips,
            Err(e) => {
                error!("rpc error {:?}", e);
                sleep(Duration::from_millis(500));
                return;
            }
        };

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
                    if let Err(e) =
                        Chaintip::set_invalid_fork(&self.db_conn, tip.height as i64, &hash, node.id)
                    {
                        error!("Failed setting chaintip {:?}", e);
                        break;
                    }

                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, &hash) {
                        error!("Failed fetching ancestors {:?}", e);
                        break;
                    }

                    if let Err(e) = Block::set_invalid(&self.db_conn, &hash, node.id) {
                        error!("Failed setting valid {:?}", e);
                        break;
                    }
                }
                GetChainTipsResultStatus::ValidFork => {
                    if let Err(e) =
                        Chaintip::set_valid_fork(&self.db_conn, tip.height as i64, &hash, node.id)
                    {
                        error!("Failed setting chaintip {:?}", e);
                        break;
                    }

                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, &hash) {
                        error!("Failed fetching ancestors {:?}", e);
                        break;
                    }

                    if let Err(e) = Block::set_valid(&self.db_conn, &hash, node.id) {
                        error!("Failed setting valid {:?}", e);
                        break;
                    }
                }
                GetChainTipsResultStatus::Active => {
                    if let Err(e) =
                        Chaintip::set_active_tip(&self.db_conn, tip.height as i64, &hash, node.id)
                    {
                        error!("Failed setting chaintip {:?}", e);
                        break;
                    }

                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, &hash) {
                        error!("Failed fetching ancestors {:?}", e);
                        break;
                    }

                    if let Err(e) = Block::set_valid(&self.db_conn, &hash, node.id) {
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

            if let Err(e) = Chaintip::purge(&self.db_conn) {
                error!("Error purging database {:?}", e);
                sleep(Duration::from_millis(500));
                return;
            }

            for (client, node) in self.clients.iter().zip(&self.node_list) {
                self.process_client(client, node);
            }

            // For each node, start with their active chaintip and see if
            // other chaintips are behind this one. Link them via 'parent_chaintip'
            // if this one has not been marked invalid by some node.
            for node in &self.node_list {
                let tip = match Chaintip::get_active(&self.db_conn, node.id) {
                    Ok(t) => t,
                    Err(e) => {
                        error!("Query failed {:?}", e);
                        continue;
                    }
                };

                // Chaintips with a height less than current tip, see if they are an ancestor
                // of current.
                // If none or error, skip current node and go to next one.
                let candidate_tips =
                    match Chaintip::list_active_no_parent(&self.db_conn, tip.height) {
                        Ok(tips) => tips,
                        Err(e) => {
                            error!("Chaintip query {:?}", e);
                            continue;
                        }
                    };

                'outer: for mut candidate in candidate_tips {
                    let mut block = match Block::get(&self.db_conn, &tip.block) {
                        Ok(block) => block,
                        Err(e) => {
                            error!("Block query: match children {:?}", e);
                            continue;
                        }
                    };

                    loop {
                        // Break if this current block was marked invalid by someone.
                        let invalid = match Block::marked_invalid(&self.db_conn, &block.hash) {
                            Ok(v) => v > 0,
                            Err(e) => {
                                error!("BlockInvalid query {:?}", e);
                                break;
                            }
                        };

                        // TODO: might need to mark parents invalid here.
                        if invalid {
                            break 'outer;
                        }

                        if block.hash == candidate.block {
                            candidate.parent_chaintip = Some(tip.id);
                            if let Err(e) = candidate.update(&self.db_conn) {
                                error!("Chaintip update failed {:?}", e);
                                break;
                            }
                        }

                        // This tip is not an ancestor of the other if the heights are equal at this
                        // point.
                        if block.parent_hash.is_none() || block.height == candidate.height {
                            break;
                        }

                        block = match Block::get(&self.db_conn, &block.parent_hash.unwrap()) {
                            Ok(b) => b,
                            Err(e) => {
                                error!("Get parent block failed {:?}", e);
                                break;
                            }
                        };
                    }
                }
            }

            sleep(Duration::from_millis(10000));
        }
    }
}
