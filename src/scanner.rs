use crate::{Block, Chaintip, Node};
use bitcoin::consensus::encode::serialize_hex;
use bitcoincore_rpc::bitcoin as btc;
use bitcoincore_rpc::bitcoincore_rpc_json::{GetChainTipsResultTip, GetChainTipsResultStatus};
use bitcoincore_rpc::Error as BitcoinRpcError;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use diesel::prelude::PgConnection;
use jsonrpc::error::Error as JsonRpcError;
use jsonrpc::error::RpcError;
use log::{debug, error, info};
use std::str::FromStr;
use thiserror::Error;

const MAX_ANCESTRY_DEPTH: usize = 100;
const MAX_BLOCK_DEPTH: i64 = 10;
const BLOCK_NOT_FOUND: i32 = -5;

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
    #[error("Failed to roll back.")]
    FailedRollback,
}

#[derive(Debug)]
pub enum ReorgMessage {
    TipUpdated(String),
    ReorgDetected(String, Vec<(i64, String)>),
}

fn create_block_and_ancestors(
    client: &Client,
    conn: &PgConnection,
    headers_only: bool,
    block_hash: &String,
    node_id: i64,
) -> ForkScannerResult<()> {
    let mut hash = btc::BlockHash::from_str(block_hash)?;

    for _ in 0..MAX_ANCESTRY_DEPTH {
        let bh = client.get_block_header_info(&hash)?;
        let block = Block::get_or_create(&conn, headers_only, node_id, &bh)?;

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

pub struct ScannerClient {
    node_id: i64,
    client: Client,
    mirror: Option<Client>,
}

impl ScannerClient {
    pub fn new(
        node_id: i64,
        host: String,
        mirror: Option<String>,
        auth: Auth,
    ) -> ForkScannerResult<ScannerClient> {
        let client = Client::new(&host, auth.clone())?;
        let mirror = match mirror {
            Some(h) => Some(Client::new(&h, auth)?),
            None => None,
        };

        Ok(ScannerClient {
            node_id,
            client,
            mirror,
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn mirror(&self) -> &Option<Client> {
        &self.mirror
    }
}

pub struct ForkScanner {
    node_list: Vec<Node>,
    clients: Vec<ScannerClient>,
    db_conn: PgConnection,
}

impl ForkScanner {
    pub fn new(db_conn: PgConnection) -> ForkScannerResult<ForkScanner> {
        let node_list = Node::list(&db_conn)?;

        let mut clients = Vec::new();

        for node in &node_list {
            let host = format!("http://{}:{}", node.rpc_host, node.rpc_port);
            let auth = Auth::UserPass(node.rpc_user.clone(), node.rpc_pass.clone());
            let mirror_host = match node.mirror_rpc_port {
                Some(port) => Some(format!("http://{}:{}", node.rpc_host, port)),
                None => None,
            };
            let client = ScannerClient::new(node.id, host, mirror_host, auth)?;
            clients.push(client);
        }

        Ok(ForkScanner {
            node_list,
            clients,
            db_conn,
        })
    }

    fn process_client(&self, client: &Client, node: &Node) -> ForkScannerResult<()> {
        let tips = client.get_chain_tips()?;

        for tip in tips {
            let hash = tip.hash.to_string();

            match tip.status {
                GetChainTipsResultStatus::HeadersOnly => {
                    create_block_and_ancestors(client, &self.db_conn, true, &hash, node.id)?;
                }
                GetChainTipsResultStatus::ValidHeaders => {
                    create_block_and_ancestors(client, &self.db_conn, true, &hash, node.id)?;
                }
                GetChainTipsResultStatus::Invalid => {
                    Chaintip::set_invalid_fork(&self.db_conn, tip.height as i64, &hash, node.id)?;

                    create_block_and_ancestors(client, &self.db_conn, false, &hash, node.id)?;

                    Block::set_invalid(&self.db_conn, &hash, node.id)?;
                }
                GetChainTipsResultStatus::ValidFork => {
                    Chaintip::set_valid_fork(&self.db_conn, tip.height as i64, &hash, node.id)?;

                    create_block_and_ancestors(client, &self.db_conn, false, &hash, node.id)?;

                    Block::set_valid(&self.db_conn, &hash, node.id)?;
                }
                GetChainTipsResultStatus::Active => {
                    Chaintip::set_active_tip(&self.db_conn, tip.height as i64, &hash, node.id)?;

                    create_block_and_ancestors(client, &self.db_conn, false, &hash, node.id)?;

                    Block::set_valid(&self.db_conn, &hash, node.id)?;
                }
            }
        }
        Ok(())
    }

    fn match_children(&self, tip: &Chaintip) -> ForkScannerResult<()> {
        // Chaintips with a height less than current tip, see if they are an ancestor
        // of current.
        // If none or error, skip current node and go to next one.
        let candidate_tips = Chaintip::list_active_lt(&self.db_conn, tip.height)?;

        for mut candidate in candidate_tips {
            if candidate.parent_chaintip.is_some() {
                continue;
            }

            let mut block = Block::get(&self.db_conn, &tip.block)?;

            loop {
                // Break if this current block was marked invalid by someone.
                let invalid =
                    match Block::marked_invalid_by(&self.db_conn, &block.hash, candidate.node) {
                        Ok(v) => v,
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
                    return Ok(());
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
        Ok(())
    }

    fn check_parent(&self, tip: &mut Chaintip) -> ForkScannerResult<()> {
        if tip.parent_chaintip.is_none() {
            return Ok(());
        }

        // Chaintips with a height greater than current tip, see if they are a successor
        // of current. If so, disconnect parent pointer.
        let candidate_tips = Chaintip::list_invalid_gt(&self.db_conn, tip.height)?;

        for candidate in &candidate_tips {
            let mut block = Block::get(&self.db_conn, &candidate.block)?;

            loop {
                if tip.block == block.hash {
                    tip.parent_chaintip = None;
                    if let Err(e) = tip.update(&self.db_conn) {
                        error!("Chaintip update failed {:?}", e);
                        break;
                    }
                    return Ok(());
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
        Ok(())
    }

    fn match_parent(&self, tip: &mut Chaintip, node: &Node) -> ForkScannerResult<()> {
        // we have a parent still, keep it.
        if tip.parent_chaintip.is_some() {
            return Ok(());
        }

        let candidate_tips = Chaintip::list_active_gt(&self.db_conn, tip.height)?;

        for candidate in &candidate_tips {
            let mut block = Block::get(&self.db_conn, &candidate.block)?;

            loop {
                // Don't attach as parent if current node or any chaintip has marked invalid.
                let invalid = match Block::marked_invalid_by(&self.db_conn, &block.hash, node.id) {
                    Ok(invalid) => invalid,
                    Err(e) => {
                        error!("Block query: match children {:?}", e);
                        break;
                    }
                };

                if invalid {
                    break;
                }

                if block.hash == tip.block {
                    tip.parent_chaintip = Some(candidate.id);
                    if let Err(e) = tip.update(&self.db_conn) {
                        error!("Chaintip update failed {:?}", e);
                        break;
                    }
                    return Ok(());
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
        Ok(())
    }

    // We initialized with get_best_block_hash, now we just poll continually
    // for new blocks, and fetch ancestors up to MAX_BLOCK_HEIGHT postgres
    // will do the rest for us.
    pub fn run(&self) {
        if let Err(e) = Chaintip::purge(&self.db_conn) {
            error!("Error purging database {:?}", e);
            return;
        }

        for (client, node) in self.clients.iter().zip(&self.node_list) {
            if let Err(e) = self.process_client(client.client(), node) {
                error!("Error processing client {:?}", e);
                continue;
            }
        }

        // For each node, start with their active chaintip and see if
        // other chaintips are behind this one. Link them via 'parent_chaintip'
        // if this one has not been marked invalid by some node.
        for node in &self.node_list {
            let mut tip = match Chaintip::get_active(&self.db_conn, node.id) {
                Ok(t) => t,
                Err(e) => {
                    error!("Query failed {:?}", e);
                    continue;
                }
            };

            if let Err(e) = self.match_children(&tip) {
                error!("Match children failed {:?}", e);
                continue;
            }

            if let Err(e) = self.check_parent(&mut tip) {
                error!("Checking parent failed {:?}", e);
                continue;
            }

            if let Err(e) = self.match_parent(&mut tip, node) {
                error!("Match parent failed {:?}", e);
                continue;
            }
        }

        // Now try to fill in missing blocks.
        self.find_missing_blocks();
        self.rollback_checks();
    }

    fn rollback_checks(&self) {
        for node in self.clients.iter().filter(|c| c.mirror().is_some()) {
            let mirror = node.mirror().as_ref().unwrap();
            let chaintips = match mirror.get_chain_tips() {
                Ok(tips) => tips,
                Err(e) => {
                    error!("RPC Error {:?}", e);
                    continue;
                }
            };

            let active_height = chaintips
                .iter()
                .filter(|tip| tip.status == GetChainTipsResultStatus::Active)
                .next()
                .unwrap()
                .height;

            for tip in chaintips.iter().filter(|tip| tip.status == GetChainTipsResultStatus::ValidHeaders) {
                if tip.height < active_height - MAX_BLOCK_DEPTH as u64 {
                    continue;
                }

                let block = match Block::get(&self.db_conn, &tip.hash.to_string()) {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                let is_valid = Block::marked_valid_by(&self.db_conn, &block.hash, node.node_id);
                let is_invalid = Block::marked_invalid_by(&self.db_conn, &block.hash, node.node_id);

                if is_valid.unwrap_or(true) || is_invalid.unwrap_or(true) {
                    continue;
                }

                let hash = btc::BlockHash::from_str(&block.hash).unwrap();
                if let Err(BitcoinRpcError::JsonRpc(JsonRpcError::Rpc(RpcError { code, .. }))) = mirror.get_block_hex(&hash) {
                    if code == BLOCK_NOT_FOUND {
                        if let Ok(hex) = node.client().get_block_hex(&hash) {
                            match mirror.call::<serde_json::Value>(
                                "submitblock",
                                &[hex.into(), hash.to_string().into()],
                            ) {
                                Ok(_) => (),
                                Err(e) => {
                                    error!("Could not submit block {:?}", e);
                                    continue;
                                }
                            }
                        } else {
                            error!("Could not fetch block");
                            continue;
                        }
                    }
                }

                // Validate fork
                if let Err(e) = mirror.call::<serde_json::Value>("setnetworkactive", &[false.into()]) {
                    error!("Could not disable p2p {:?}", e);
                    continue;
                }
                match self.set_tip_active(mirror, tip) {
                    Ok(invalidated_hashes) => {
                        let tips = match mirror.get_chain_tips() {
                            Ok(t) => t,
                            Err(e) => {
                                error!("Chain tips error {:?}", e);
                                continue;
                            }
                        };

                        let active = tips.iter().find(|t| t.status == GetChainTipsResultStatus::Active).unwrap().clone();

                        if active.hash == tip.hash {
                            if let Err(e) = Block::set_valid(&self.db_conn, &tip.hash.to_string(), node.node_id) {
                                error!("Database error {:?}", e);
                                continue;
                            }

                            // Undo the rollback
                            for hash in invalidated_hashes {
                                let _ = mirror.reconsider_block(&hash);
                            }
                        } else {
                            for t in tips.into_iter().filter(|t| t.status == GetChainTipsResultStatus::Invalid) {
                                let _ = mirror.reconsider_block(&t.hash);
                            }
                        }

                        if let Err(e) = mirror.call::<serde_json::Value>("setnetworkactive", &[true.into()]) {
                            error!("Could not reactivate p2p {:?}", e);
                        }

                        // TODO: is tip still considered invalid? mark it invalid #442++
                    }
                    Err(_) => {
                        error!("Could not make tip active, restoring state...");
                        if let Err(e) = mirror.call::<serde_json::Value>("setnetworkactive", &[true.into()]) {
                            error!("Could not reactivate p2p {:?}", e);
                        }
                        let tips = match mirror.get_chain_tips() {
                            Ok(t) => t,
                            Err(e) => {
                                error!("Chain tips error {:?}", e);
                                continue;
                            }
                        };

                        for t in tips.into_iter().filter(|t| t.status == GetChainTipsResultStatus::Invalid) {
                            let _ = mirror.reconsider_block(&t.hash);
                        }
                    }
                }
            }
        }
    }

    fn set_tip_active(&self, mirror: &Client, tip: &GetChainTipsResultTip) -> ForkScannerResult<Vec<btc::BlockHash>> {
        let mut invalidated_hashes = Vec::new();
        let mut retry_count = 0;

        loop {
            let tips = match mirror.get_chain_tips() {
                Ok(t) => t,
                Err(e) => {
                    error!("Chain tips error {:?}", e);
                    continue;
                }
            };

            let active = tips.iter().find(|t| t.status == GetChainTipsResultStatus::Active).unwrap();

            if active.hash == tip.hash {
                break;
            }

            if retry_count > 100 {
                return Err(ForkScannerError::FailedRollback);
            }

            let mut blocks_to_invalidate = Vec::new();

            if active.height == tip.height {
                blocks_to_invalidate.push(active.hash);
            } else {
                if let Some(branch) = self.find_branch_point(active, tip) {
                    blocks_to_invalidate.push(branch);
                }

                let children = match Block::children(&self.db_conn, &tip.hash.to_string()) {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Children fetch failed {:?}", e);
                        continue;
                    }
                };

                for child in children {
                    let hash = btc::BlockHash::from_str(&child.hash).unwrap();
                    blocks_to_invalidate.push(hash);
                }
            }

            for block in blocks_to_invalidate {
                let _ = mirror.invalidate_block(&block);
                invalidated_hashes.push(block);
            }

            retry_count += 1;
        }
        Ok(invalidated_hashes)
    }

    fn find_branch_point(&self, active: &GetChainTipsResultTip, tip: &GetChainTipsResultTip) -> Option<btc::BlockHash> {
        if active.height <= tip.height {
            return None;
        }

        let mut block1 = Block::get(&self.db_conn, &active.hash.to_string()).ok()?;

        while block1.height > tip.height as i64 {
            block1 = block1.parent(&self.db_conn).ok()?;
        }

        if block1.hash == tip.hash.to_string() {
            None
        } else {
            loop {
                block1 = block1.parent(&self.db_conn).ok()?;

                let desc = block1.descendants(&self.db_conn).ok()?;

                let fork = desc.into_iter().find(|b| b.hash == tip.hash.to_string());

                if let Some(_) = fork {
                    let hash = btc::BlockHash::from_str(&block1.hash).unwrap();
                    return Some(hash);
                }
            }
        }
    }

    fn find_missing_blocks(&self) {
        let tip_height = match Block::max_height(&self.db_conn) {
            Ok(Some(h)) => h,
            Ok(_) => {
                info!("No blocks in database");
                return;
            }
            Err(e) => {
                error!("Query failed {:?}", e);
                return;
            }
        };

        let mut headers_only_blocks = match Block::headers_only(&self.db_conn, tip_height - 40_000)
        {
            Ok(blocks) => blocks,
            Err(e) => {
                error!("Header query failed {:?}", e);
                return;
            }
        };

        let mut gbfp_blocks = vec![];
        for mut block in headers_only_blocks.drain(..) {
            let originally_seen = block.first_seen_by;

            let mut raw_block = None;
            if tip_height - block.height < MAX_BLOCK_DEPTH {
                let hash = btc::BlockHash::from_str(&block.hash).unwrap();
                for client in &self.clients {
                    match client.client().get_block_hex(&hash) {
                        Ok(block_hex) => {
                            block.headers_only = false;

                            if let Err(e) = block.update(&self.db_conn) {
                                error!("Could not clear headers flag {:?}", e);
                            }

                            raw_block = Some(block_hex);
                            break;
                        }
                        _ => continue,
                    }
                }

                if raw_block.is_some() {
                    let b = raw_block.clone().unwrap();
                    let node = self
                        .clients
                        .iter()
                        .find(|c| c.node_id == originally_seen)
                        .unwrap();

                    if let Err(e) = node.client().call::<serde_json::Value>(
                        "submitblock",
                        &[b.into(), hash.to_string().into()],
                    ) {
                        error!("Could not submit block {:?}", e);
                        continue;
                    }
                }
            }

            if raw_block.is_some() {
                continue;
            }

            let client = self.clients.iter().filter(|c| c.mirror().is_some()).next();
            if client.is_none() {
                error!("No mirror nodes");
                continue;
            }

            let hash = btc::BlockHash::from_str(&block.hash).unwrap();
            let mirror = client.unwrap().mirror().as_ref().unwrap();
            match mirror.get_block_header(&hash) {
                Ok(_) => (),
                Err(BitcoinRpcError::JsonRpc(JsonRpcError::Rpc(RpcError { code, .. })))
                    if code == BLOCK_NOT_FOUND =>
                {
                    debug!("Header not found");
                    let node = self
                        .clients
                        .iter()
                        .find(|c| c.node_id == originally_seen)
                        .unwrap();
                    let header = match node.client().get_block_header(&hash) {
                        Ok(block_header) => serialize_hex(&block_header),
                        Err(e) => {
                            error!("Could not fetch header from originally seen {:?}", e);
                            continue;
                        }
                    };

                    if let Err(e) = mirror.call::<serde_json::Value>(
                        "submitheader",
                        &[serde_json::Value::String(header)],
                    ) {
                        error!("Could not submit block {:?}", e);
                        continue;
                    }
                }
                Err(e) => {
                    error!("Client connection error {:?}", e);
                    continue;
                }
            };

            let peers = match mirror.get_peer_info() {
                Ok(p) => p,
                Err(_) => {
                    error!("No peers");
                    continue;
                }
            };

            for peer in peers {
                let peer_id = serde_json::Value::Number(serde_json::Number::from(peer.id));
                if let Err(_) = mirror.call::<serde_json::Value>(
                    "getblockfrompeer",
                    &[
                        serde_json::Value::String(block.hash.clone()),
                        peer_id.clone(),
                    ],
                ) {
                    let _ = mirror.call::<serde_json::Value>(
                        "disconnectnode",
                        &[serde_json::Value::String("".into()), peer_id],
                    );
                }
            }
            gbfp_blocks.push(block);
        }
    }
}
