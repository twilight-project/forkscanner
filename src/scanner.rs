use crate::{Block, Chaintip, Node};
use bitcoincore_rpc::bitcoin as btc;
use bitcoincore_rpc::bitcoincore_rpc_json::GetChainTipsResultStatus;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use diesel::prelude::PgConnection;
use log::{debug, error, info};
use std::str::FromStr;
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
    node_id: i64,
    headers_only: bool,
    block_hash: &String,
) -> ForkScannerResult<()> {
    let mut hash = btc::BlockHash::from_str(block_hash)?;

    for _ in 0..MAX_BLOCK_DEPTH {
        let bh = client.get_block_header_info(&hash)?;
        let block = Block::get_or_create(&conn, node_id, headers_only, &bh)?;

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

        Ok(ForkScanner {
            node_list,
            clients,
            db_conn,
        })
    }

    fn process_client(&self, client: &Client, node: &Node) {
        let tips = match client.get_chain_tips() {
            Ok(tips) => tips,
            Err(e) => {
                error!("rpc error {:?}", e);
                return;
            }
        };

        for tip in tips {
            let hash = tip.hash.to_string();

            match tip.status {
                GetChainTipsResultStatus::HeadersOnly => {
                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, node.id, true, &hash) {
                        error!("Failed fetching ancestors {:?}", e);
                        break;
                    }
                }
                GetChainTipsResultStatus::ValidHeaders => {
                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, node.id, true, &hash) {
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

                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, node.id, false, &hash) {
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

                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, node.id, false, &hash) {
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

                    if let Err(e) = create_block_and_ancestors(client, &self.db_conn, node.id, false, &hash) {
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

    fn match_children(&self, tip: &Chaintip) {
        // Chaintips with a height less than current tip, see if they are an ancestor
        // of current.
        // If none or error, skip current node and go to next one.
        let candidate_tips =
            match Chaintip::list_active_lt(&self.db_conn, tip.height) {
                Ok(tips) => tips,
                Err(e) => {
                    error!("Chaintip query {:?}", e);
                    return;
                }
            };

        for mut candidate in candidate_tips {
            if candidate.parent_chaintip.is_some() {
                continue;
            }

            let mut block = match Block::get(&self.db_conn, &tip.block) {
                Ok(block) => block,
                Err(e) => {
                    error!("Block query: match children {:?}", e);
                    return;
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

                if invalid {
                    break;
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

    fn check_parent(&self, tip: &mut Chaintip) {
        if tip.parent_chaintip.is_none() {
            return;
        }

        // Chaintips with a height greater than current tip, see if they are a successor
        // of current. If so, disconnect parent pointer.
        let candidate_tips =
            match Chaintip::list_invalid_gt(&self.db_conn, tip.height) {
                Ok(tips) => tips,
                Err(e) => {
                    error!("Chaintip query {:?}", e);
                    return;
                }
            };

        for candidate in &candidate_tips {
            let mut block = match Block::get(&self.db_conn, &candidate.block) {
                Ok(block) => block,
                Err(e) => {
                    error!("Block query: match children {:?}", e);
                    return;
                }
            };

            loop {
                if tip.block == block.hash {
                    tip.parent_chaintip = None;
                    if let Err(e) = tip.update(&self.db_conn) {
                        error!("Chaintip update failed {:?}", e);
                        break;
                    }
                    return;
                }

                // This tip is not an ancestor of the other if the heights are equal at this
                // point.
                if block.parent_hash.is_none() || block.height == tip.height {
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

    fn match_parent(&self, tip: &mut Chaintip, node: &Node) {
        // we have a parent still, keep it.
        if tip.parent_chaintip.is_some() {
            return;
        }

        let candidate_tips =
            match Chaintip::list_active_gt(&self.db_conn, tip.height) {
                Ok(tips) => tips,
                Err(e) => {
                    error!("Chaintip query {:?}", e);
                    return;
                }
            };

        for candidate in &candidate_tips {
            let mut block = match Block::get(&self.db_conn, &candidate.block) {
                Ok(block) => block,
                Err(e) => {
                    error!("Block query: match children {:?}", e);
                    return;
                }
            };

            loop {
                // Don't attach as parent if current node or any chaintip has marked invalid.
                let invalid = match Block::marked_invalid_by(&self.db_conn, &block.hash, node.id) {
                    Ok(invalid) => invalid,
                    Err(e) => {
                        error!("Block query: match children {:?}", e);
                        break;
                    }
                };

                let invalid_tip = match Chaintip::get_invalid(&self.db_conn, &block.hash) {
                    Ok(_) => true,
                    Err(diesel::result::Error::NotFound) => false,
                    Err(e) => {
                        error!("Chaintips: match parent {:?}", e);
                        break;
                    }
                };

                if invalid || invalid_tip {
                    break;
                }

                if block.hash == tip.block {
                    tip.parent_chaintip = Some(candidate.id);
                    if let Err(e) = tip.update(&self.db_conn) {
                        error!("Chaintip update failed {:?}", e);
                        break;
                    }
                    return;
                }

                // This tip is not an ancestor of the other if the heights are equal at this
                // point.
                if block.parent_hash.is_none() || block.height == tip.height {
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

    // We initialized with get_best_block_hash, now we just poll continually
    // for new blocks, and fetch ancestors up to MAX_BLOCK_HEIGHT postgres
    // will do the rest for us.
    pub fn run(self) {
        if let Err(e) = Chaintip::purge(&self.db_conn) {
            error!("Error purging database {:?}", e);
            return;
        }

        for (client, node) in self.clients.iter().zip(&self.node_list) {
            self.process_client(client, node);
        }

        // For each node, start with their active chaintip and see if
        // other chaintips are behind this one. Link them via 'parent_chaintip'
        // if this one has not been marked invalid by some node.
        for node in &self.node_list {
            let mut tip = match Chaintip::get_active(&self.db_conn, node.id) {
                Ok(t) => t,
                Err(e) => {
                    error!("Query failed {:?}", e);
                    return;
                }
            };

            self.match_children(&tip);
            self.check_parent(&mut tip);
            self.match_parent(&mut tip, node);
        }
    }
}
