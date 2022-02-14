use crate::{Block, Chaintip, Node, StaleCandidate, StaleCandidateChildren, Transaction};
use bitcoin::consensus::encode::serialize_hex;
use bitcoincore_rpc::bitcoin as btc;
use bitcoincore_rpc::bitcoincore_rpc_json::{
    GetBlockHeaderResult, GetChainTipsResultStatus, GetChainTipsResultTip, GetPeerInfoResult,
    GetRawTransactionResult,
};
use bitcoincore_rpc::Error as BitcoinRpcError;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use diesel::prelude::PgConnection;
use jsonrpc::error::Error as JsonRpcError;
use jsonrpc::error::RpcError;
use log::{debug, error, info};
#[cfg(test)]
use mockall::*;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    iter::FromIterator,
    str::FromStr,
};
use thiserror::Error;

const MAX_ANCESTRY_DEPTH: usize = 100;
const MAX_BLOCK_DEPTH: i64 = 10;
const BLOCK_NOT_FOUND: i32 = -5;
const STALE_WINDOW: i64 = 100;
const DOUBLE_SPEND_RANGE: i64 = 30;

type ForkScannerResult<T> = Result<T, ForkScannerError>;

#[derive(Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase")]
pub struct FullBlock {
    bits: serde_json::Value,
    chainwork: serde_json::Value,
    confirmations: serde_json::Value,
    difficulty: serde_json::Value,
    hash: serde_json::Value,
    height: serde_json::Value,
    mediantime: serde_json::Value,
    merkleroot: serde_json::Value,
    n_tx: serde_json::Value,
    nonce: serde_json::Value,
    previousblockhash: serde_json::Value,
    size: serde_json::Value,
    strippedsize: serde_json::Value,
    time: serde_json::Value,
    tx: Vec<JsonTransaction>,
    version: serde_json::Value,
    version_hex: serde_json::Value,
    weight: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct JsonTransaction {
    hash: String,
    hex: String,
    locktime: usize,
    size: usize,
    txid: String,
    version: usize,
    vin: Vec<Vin>,
    vout: Vec<Vout>,
    vsize: usize,
    weight: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vin {
    txid: Option<String>,
    vout: Option<usize>,
    script_sig: Option<ScriptSig>,
    sequence: usize,
    txinwitness: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vout {
    value: f64,
    n: usize,
    script_pub_key: ScriptPubKey,
}

#[derive(Debug, Deserialize)]
pub struct ScriptSig {
    asm: String,
    hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptPubKey {
    asm: String,
    hex: String,
    req_sigs: Option<usize>,
    r#type: String,
    addresses: Option<Vec<String>>,
}

#[cfg_attr(test, automock)]
pub trait BtcClient: Sized {
    fn new(host: &String, auth: Auth) -> ForkScannerResult<Self>;
    fn disconnect_node(&self, id: u64) -> Result<serde_json::Value, bitcoincore_rpc::Error>;
    fn get_chain_tips(&self) -> Result<Vec<GetChainTipsResultTip>, bitcoincore_rpc::Error>;
    fn get_block_from_peer(
        &self,
        hash: String,
        id: u64,
    ) -> Result<serde_json::Value, bitcoincore_rpc::Error>;
    fn get_block_header(
        &self,
        hash: &btc::BlockHash,
    ) -> Result<btc::BlockHeader, bitcoincore_rpc::Error>;
    fn get_block_verbose(&self, hash: String) -> Result<FullBlock, bitcoincore_rpc::Error>;
    fn get_block_header_info(
        &self,
        hash: &btc::BlockHash,
    ) -> Result<GetBlockHeaderResult, bitcoincore_rpc::Error>;
    fn get_block_hex(&self, hash: &btc::BlockHash) -> Result<String, bitcoincore_rpc::Error>;
    fn get_peer_info(&self) -> Result<Vec<GetPeerInfoResult>, bitcoincore_rpc::Error>;
    fn get_raw_transaction_info<'a>(
        &self,
        txid: &btc::Txid,
        block_hash: Option<&'a btc::BlockHash>,
    ) -> Result<GetRawTransactionResult, bitcoincore_rpc::Error>;
    fn set_network_active(&self, active: bool)
        -> Result<serde_json::Value, bitcoincore_rpc::Error>;
    fn submit_block(
        &self,
        hex: String,
        hash: &btc::BlockHash,
    ) -> Result<serde_json::Value, bitcoincore_rpc::Error>;
    fn submit_header(&self, header: String) -> Result<serde_json::Value, bitcoincore_rpc::Error>;
    fn invalidate_block(&self, hash: &btc::BlockHash) -> Result<(), bitcoincore_rpc::Error>;
    fn reconsider_block(&self, hash: &btc::BlockHash) -> Result<(), bitcoincore_rpc::Error>;
}

impl BtcClient for Client {
    fn new(host: &String, auth: Auth) -> ForkScannerResult<Client> {
        Ok(Client::new(host, auth)?)
    }

    fn disconnect_node(&self, id: u64) -> Result<serde_json::Value, bitcoincore_rpc::Error> {
        let peer_id = serde_json::Value::Number(serde_json::Number::from(id));
        RpcApi::call::<serde_json::Value>(
            self,
            "disconnectnode",
            &[serde_json::Value::String("".into()), peer_id],
        )
    }

    fn get_chain_tips(&self) -> Result<Vec<GetChainTipsResultTip>, bitcoincore_rpc::Error> {
        RpcApi::get_chain_tips(self)
    }

    fn get_block_from_peer(
        &self,
        hash: String,
        id: u64,
    ) -> Result<serde_json::Value, bitcoincore_rpc::Error> {
        let peer_id = serde_json::Value::Number(serde_json::Number::from(id));
        RpcApi::call::<serde_json::Value>(
            self,
            "getblockfrompeer",
            &[serde_json::Value::String(hash), peer_id],
        )
    }

    fn get_block_header(
        &self,
        hash: &btc::BlockHash,
    ) -> Result<btc::BlockHeader, bitcoincore_rpc::Error> {
        RpcApi::get_block_header(self, hash)
    }

    fn get_block_verbose(&self, hash: String) -> Result<FullBlock, bitcoincore_rpc::Error> {
        RpcApi::call::<FullBlock>(
            self,
            "getblock",
            &[
                serde_json::Value::String(hash),
                serde_json::Value::Number(serde_json::Number::from(2)),
            ],
        )
    }

    fn get_block_header_info(
        &self,
        hash: &btc::BlockHash,
    ) -> Result<GetBlockHeaderResult, bitcoincore_rpc::Error> {
        RpcApi::get_block_header_info(self, hash)
    }

    fn get_block_hex(&self, hash: &btc::BlockHash) -> Result<String, bitcoincore_rpc::Error> {
        RpcApi::get_block_hex(self, hash)
    }

    fn get_peer_info(&self) -> Result<Vec<GetPeerInfoResult>, bitcoincore_rpc::Error> {
        RpcApi::get_peer_info(self)
    }

    fn get_raw_transaction_info(
        &self,
        txid: &btc::Txid,
        block_hash: Option<&btc::BlockHash>,
    ) -> Result<GetRawTransactionResult, bitcoincore_rpc::Error> {
        RpcApi::get_raw_transaction_info(self, txid, block_hash)
    }

    fn set_network_active(
        &self,
        active: bool,
    ) -> Result<serde_json::Value, bitcoincore_rpc::Error> {
        RpcApi::call::<serde_json::Value>(self, "setnetworkactive", &[active.into()])
    }

    fn submit_block(
        &self,
        hex: String,
        hash: &btc::BlockHash,
    ) -> Result<serde_json::Value, bitcoincore_rpc::Error> {
        RpcApi::call::<serde_json::Value>(
            self,
            "submitblock",
            &[hex.into(), hash.to_string().into()],
        )
    }

    fn submit_header(&self, header: String) -> Result<serde_json::Value, bitcoincore_rpc::Error> {
        RpcApi::call::<serde_json::Value>(
            self,
            "submitheader",
            &[serde_json::Value::String(header)],
        )
    }

    fn invalidate_block(&self, hash: &btc::BlockHash) -> Result<(), bitcoincore_rpc::Error> {
        RpcApi::invalidate_block(self, hash)
    }

    fn reconsider_block(&self, hash: &btc::BlockHash) -> Result<(), bitcoincore_rpc::Error> {
        RpcApi::reconsider_block(self, hash)
    }
}

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

/// Once we have a block hash, we want to enter it into the database.
/// If the parent hash is not there, we walk up the block's ancestry
/// up to MAX_ANCESTRY_DEPTH and make entries for those blocks as well.
fn create_block_and_ancestors<BC: BtcClient>(
    client: &BC,
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

/// Holds connection info for a bitcoin node that forkscanner is
/// connected to.
pub struct ScannerClient<BC: BtcClient> {
    node_id: i64,
    client: BC,
    mirror: Option<BC>,
}

impl<BC: BtcClient> ScannerClient<BC> {
    pub fn new(
        node_id: i64,
        host: String,
        mirror: Option<String>,
        auth: Auth,
    ) -> ForkScannerResult<ScannerClient<BC>> {
        let client = BC::new(&host, auth.clone())?;
        let mirror = match mirror {
            Some(h) => Some(BC::new(&h, auth)?),
            None => None,
        };

        Ok(ScannerClient {
            node_id,
            client,
            mirror,
        })
    }

    pub fn client(&self) -> &BC {
        &self.client
    }

    pub fn mirror(&self) -> &Option<BC> {
        &self.mirror
    }
}

/// The main forkscanner struct. This maintains a list of bitcoin nodes to connect to,
/// and db connection to record chain info.
pub struct ForkScanner<BC: BtcClient> {
    node_list: Vec<Node>,
    clients: Vec<ScannerClient<BC>>,
    db_conn: PgConnection,
}

impl<BC: BtcClient> ForkScanner<BC> {
    pub fn new(db_conn: PgConnection) -> ForkScannerResult<ForkScanner<BC>> {
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

    // process chaintip entries for a client, log to database.
    fn process_client(&self, client: &BC, node: &Node) -> ForkScannerResult<()> {
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
        // start by purging chaintips, keeping only the previously 'active' chaintips.
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
        self.find_stale_candidates();

        // for 3 most recent stale candidates...
        self.process_stale_candidates()
    }

    fn process_stale_candidates(&self) {
        // Find top 3.
        let candidates = match StaleCandidate::top_n(&self.db_conn, 3) {
            Ok(c) => c,
            Err(e) => {
                error!("Database error {:?}", e);
                return;
            }
        };

        for mut candidate in candidates {
            let blocks = match Block::get_at_height(&self.db_conn, candidate.height) {
                Ok(b) => b,
                Err(e) => {
                    error!("Database error {:?}", e);
                    continue;
                }
            };

            for block in blocks {
                self.fetch_transactions(&block);
                let descendants = match block.descendants(&self.db_conn, Some(DOUBLE_SPEND_RANGE)) {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Database error {:?}", e);
                        continue;
                    }
                };

                for desc in descendants {
                    self.fetch_transactions(&desc);
                }
            }

            let tip_height = match Block::max_height(&self.db_conn) {
                Ok(Some(tip)) => tip,
                _ => continue,
            };

            let children = match candidate.children(&self.db_conn) {
                Ok(c) => c,
                Err(e) => {
                    error!("Database error {:?}", e);
                    continue;
                }
            };

            if children.len() == 0
                || candidate.height_processed.is_none()
                || (candidate.height_processed.unwrap() < tip_height
                    && candidate.height_processed.unwrap() <= candidate.height + DOUBLE_SPEND_RANGE)
            {
                self.set_children(&mut candidate);
                self.set_conflicting_txs(&mut candidate, tip_height);
            }
        }
    }

    fn set_children(&self, candidate: &mut StaleCandidate) {
        if let Err(e) = candidate.purge_children(&self.db_conn) {
            error!("Could not purge children! {:?}", e);
            return;
        };

        let blocks = match Block::get_at_height(&self.db_conn, candidate.height) {
            Ok(b) => b,
            Err(e) => {
                error!("Database error {:?}", e);
                return;
            }
        };

        for block in blocks {
            let descendants =
                match block.descendants_by_work(&self.db_conn, block.height + STALE_WINDOW) {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Database error {:?}", e);
                        continue;
                    }
                };

            match StaleCandidateChildren::create(
                &self.db_conn,
                &block,
                descendants.last().unwrap(),
                descendants.len() as i32,
            ) {
                Ok(_) => (),
                Err(e) => {
                    error!("Database error {:?}", e);
                }
            }
            candidate.n_children += 1;
        }

        if let Err(e) = candidate.update(&self.db_conn) {
            error!("Database error {:?}", e);
        };
    }

    fn set_conflicting_txs(&self, candidate: &mut StaleCandidate, tip_height: i64) {
        if let Some(confirmed_in_one) = self.get_confirmed_in_one_branch(candidate) {
            // TODO: this handles only 2 branches, shortest and longest.
            let client = self.clients.iter().next().unwrap().clone();

            fn map_key(tx_id: String, vout: u32) -> String {
                format!("{}##{}", tx_id, vout)
            }

            let confirmed_in_one_total = if confirmed_in_one.len() == 0 {
                0.0
            } else {
                Transaction::amount_for_txs(&self.db_conn, &confirmed_in_one).unwrap()
            };

            let children = match candidate.children(&self.db_conn) {
                Ok(c) => c,
                Err(e) => {
                    error!("Database error {:?}", e);
                    return;
                }
            };

            let block = match Block::get(&self.db_conn, &children[0].root_id) {
                Ok(b) => b,
                Err(e) => {
                    error!("Database error {:?}", e);
                    return;
                }
            };

            let short_txs: Vec<_> =
                match block.block_and_descendant_transactions(&self.db_conn, DOUBLE_SPEND_RANGE) {
                    Ok(txs) => txs,
                    Err(e) => {
                        error!("Database error {:?}", e);
                        return;
                    }
                };

            let mut short_map: HashMap<String, GetRawTransactionResult> = HashMap::new();

            for transaction in short_txs {
                let txid = btc::Txid::from_str(&transaction.txid).unwrap();
                let hash = btc::BlockHash::from_str(&transaction.block_id).unwrap();
                let tx = match client.client().get_raw_transaction_info(&txid, Some(&hash)) {
                    Ok(tx) => tx,
                    Err(e) => {
                        error!("RPC error {:?}", e);
                        return;
                    }
                };
                for input in &tx.vin {
                    short_map.insert(
                        map_key(input.txid.unwrap().to_string(), input.vout.unwrap()),
                        tx.clone(),
                    );
                }
            }

            let block = match Block::get(&self.db_conn, &children[1].root_id) {
                Ok(b) => b,
                Err(e) => {
                    error!("Database error {:?}", e);
                    return;
                }
            };

            let long_txs: Vec<_> =
                match block.block_and_descendant_transactions(&self.db_conn, DOUBLE_SPEND_RANGE) {
                    Ok(txs) => txs,
                    Err(e) => {
                        error!("Database error {:?}", e);
                        return;
                    }
                };

            let mut long_map: HashMap<String, GetRawTransactionResult> = HashMap::new();

            for transaction in long_txs {
                let txid = btc::Txid::from_str(&transaction.txid).unwrap();
                let hash = btc::BlockHash::from_str(&transaction.block_id).unwrap();
                let tx = match client.client().get_raw_transaction_info(&txid, Some(&hash)) {
                    Ok(tx) => tx,
                    Err(e) => {
                        error!("RPC error {:?}", e);
                        return;
                    }
                };
                for input in &tx.vin {
                    long_map.insert(
                        map_key(input.txid.unwrap().to_string(), input.vout.unwrap()),
                        tx.clone(),
                    );
                }
            }

            let double_spent: (f64, Vec<_>) = short_map
                .iter()
                .filter_map(|(txout, tx)| {
                    if long_map.contains_key(txout) && tx.txid != long_map.get(txout).unwrap().txid
                    {
                        Some((tx.clone(), long_map.get(txout).unwrap().clone()))
                    } else {
                        None
                    }
                })
                .fold((0.0, vec![]), |(mut amt, mut by), b| {
                    amt += b.0.vout.iter().fold(0.0, |a, b| a + b.value.as_btc());
                    by.push(b.1.txid.to_string());
                    (amt, by)
                });

            let rbf: (f64, Vec<_>) = short_map
                .iter()
                .filter_map(|(txout, tx)| {
                    if !long_map.contains_key(txout) || long_map.get(txout).unwrap().txid == tx.txid
                    {
                        None
                    } else if tx.vout.len() != long_map.get(txout).unwrap().vout.len() {
                        None
                    } else {
                        let mut txouts = tx.vout.clone();
                        let mut otherouts = long_map.get(txout).unwrap().vout.clone();

                        txouts.sort_by(|l, r| {
                            if l.script_pub_key.hex < r.script_pub_key.hex {
                                std::cmp::Ordering::Less
                            } else {
                                std::cmp::Ordering::Greater
                            }
                        });

                        otherouts.sort_by(|l, r| {
                            if l.script_pub_key.hex < r.script_pub_key.hex {
                                std::cmp::Ordering::Less
                            } else {
                                std::cmp::Ordering::Greater
                            }
                        });

                        let same = !txouts.iter().zip(otherouts).any(|(l, r)| {
                            l.script_pub_key != r.script_pub_key
                                || (l.value.as_btc() - r.value.as_btc()).abs() > 0.0001
                        });
                        if same {
                            Some((tx.clone(), long_map.get(txout).unwrap().clone()))
                        } else {
                            None
                        }
                    }
                })
                .fold((0.0, vec![]), |(mut amt, mut by), b| {
                    amt += b.0.vout.iter().fold(0.0, |a, b| a + b.value.as_btc());
                    by.push(b.1.txid.to_string());
                    (amt, by)
                });

            candidate.confirmed_in_one_branch_total = confirmed_in_one_total;
            candidate.double_spent_in_one_branch_total = double_spent.0;
            candidate.rbf_total = rbf.0;
            if let Err(e) = candidate.update_double_spent_by(&self.db_conn, &double_spent.1) {
                error!("Failed to update double spent by {:?}", e);
            }
            if let Err(e) = candidate.update_rbf_by(&self.db_conn, &rbf.1) {
                error!("Failed to update rbf by {:?}", e);
            }

            candidate.height_processed = Some(tip_height);
            if let Err(e) = candidate.update(&self.db_conn) {
                error!("Failed to update stale candidate {}", e);
            }
        }
    }

    fn get_confirmed_in_one_branch(&self, candidate: &StaleCandidate) -> Option<Vec<String>> {
        // TODO: handle more than two branches
        if candidate.n_children != 2 {
            return None;
        }

        let children = match candidate.children(&self.db_conn) {
            Ok(c) => c,
            Err(e) => {
                error!("Database error {:?}", e);
                return None;
            }
        };

        let headers_only = children
            .iter()
            .all(|c| match Block::get(&self.db_conn, &c.root_id) {
                Ok(b) => b.headers_only,
                _ => false,
            });

        if headers_only {
            return None;
        }

        let block = match Block::get(&self.db_conn, &children[0].root_id) {
            Ok(b) => b,
            Err(e) => {
                error!("Database error {:?}", e);
                return None;
            }
        };

        let mut short_txs = match block.num_transactions(&self.db_conn) {
            Ok(txs) => txs > 0,
            Err(e) => {
                error!("Database error {:?}", e);
                return None;
            }
        };

        let descendants = match block.descendants(&self.db_conn, Some(DOUBLE_SPEND_RANGE)) {
            Ok(d) => d,
            Err(e) => {
                error!("Database error {:?}", e);
                return None;
            }
        };

        for desc in descendants {
            short_txs &= match desc.num_transactions(&self.db_conn) {
                Ok(txs) => txs > 0,
                Err(e) => {
                    error!("Database error {:?}", e);
                    return None;
                }
            };
        }

        if !short_txs {
            return None;
        }

        let short_tx_ids: Vec<String> =
            match block.block_and_descendant_transactions(&self.db_conn, DOUBLE_SPEND_RANGE) {
                Ok(txs) => txs.into_iter().map(|t| t.txid).collect(),
                Err(e) => {
                    error!("Database error {:?}", e);
                    return None;
                }
            };

        let block = match Block::get(&self.db_conn, &children[1].root_id) {
            Ok(b) => b,
            Err(e) => {
                error!("Database error {:?}", e);
                return None;
            }
        };

        let mut long_txs = match block.num_transactions(&self.db_conn) {
            Ok(txs) => txs > 0,
            Err(e) => {
                error!("Database error {:?}", e);
                return None;
            }
        };

        let descendants = match block.descendants(&self.db_conn, Some(DOUBLE_SPEND_RANGE)) {
            Ok(d) => d,
            Err(e) => {
                error!("Database error {:?}", e);
                return None;
            }
        };

        for desc in descendants {
            long_txs &= match desc.num_transactions(&self.db_conn) {
                Ok(txs) => txs > 0,
                Err(e) => {
                    error!("Database error {:?}", e);
                    return None;
                }
            }
        }

        if !long_txs {
            return None;
        }

        let long_tx_ids: Vec<String> =
            match block.block_and_descendant_transactions(&self.db_conn, DOUBLE_SPEND_RANGE) {
                Ok(txs) => txs.into_iter().map(|t| t.txid).collect(),
                Err(e) => {
                    error!("Database error {:?}", e);
                    return None;
                }
            };

        let short = HashSet::<_>::from_iter(short_tx_ids.into_iter());
        let long = HashSet::<_>::from_iter(long_tx_ids.into_iter());

        if short.len() < long.len() {
            Some(short.difference(&long).cloned().collect())
        } else {
            Some(short.symmetric_difference(&long).cloned().collect())
        }
    }

    fn find_stale_candidates(&self) {
        let tip_height = match Block::max_height(&self.db_conn) {
            Ok(Some(tip)) => tip,
            _ => return,
        };

        let candidates =
            match Block::find_stale_candidates(&self.db_conn, tip_height - STALE_WINDOW) {
                Ok(candidates) if candidates.len() > 0 => candidates,
                _ => return,
            };

        for candidate in candidates.iter() {
            match Block::count_at_height(&self.db_conn, candidate.height - 1) {
                Ok(ct) if ct > 1 => continue,
                _ => (),
            }

            let blocks = match Block::get_at_height(&self.db_conn, candidate.height) {
                Ok(b) => b,
                Err(e) => {
                    error!("Database error {:?}", e);
                    continue;
                }
            };

            if let Err(e) =
                StaleCandidate::create(&self.db_conn, candidate.height, blocks.len() as i32)
            {
                error!("Database error {:?}", e);
                continue;
            }
        }
    }

    fn fetch_transactions(&self, block: &Block) {
        let node = self
            .clients
            .iter()
            .find(|c| c.node_id == block.first_seen_by)
            .unwrap()
            .clone();

        let block_info = match node.client().get_block_verbose(block.hash.clone()) {
            Ok(bi) => bi,
            Err(e) => {
                error!("RPC call failed {:?}", e);
                return;
            }
        };

        for (idx, tx) in block_info.tx.iter().enumerate() {
            let value = tx.vout.iter().fold(0., |a, amt| a + amt.value);
            if let Err(e) = Transaction::create(
                &self.db_conn,
                block.hash.to_string(),
                idx,
                &tx.txid,
                &tx.hex,
                value,
            ) {
                error!("Could not insert transaction {:?}", e);
            }
        }
    }

    // Rollback checks. Here we are looking to use the mirror node to try to set a 'valid-headers'
    // chaintip as the active one by invalidating the currently active chaintip. We briefly turn
    // off p2p on this node so the state doesn't change underneath us. Then, if the switch to new
    // chaintip is successful, we check if it would've been a valid tip.
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

            for tip in chaintips
                .iter()
                .filter(|tip| tip.status == GetChainTipsResultStatus::ValidHeaders)
            {
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
                if let Err(BitcoinRpcError::JsonRpc(JsonRpcError::Rpc(RpcError { code, .. }))) =
                    mirror.get_block_hex(&hash)
                {
                    if code == BLOCK_NOT_FOUND {
                        if let Ok(hex) = node.client().get_block_hex(&hash) {
                            match mirror.submit_block(hex, &hash) {
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
                if let Err(e) = mirror.set_network_active(false) {
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

                        let active = tips
                            .iter()
                            .find(|t| t.status == GetChainTipsResultStatus::Active)
                            .unwrap()
                            .clone();

                        if active.hash == tip.hash {
                            if let Err(e) =
                                Block::set_valid(&self.db_conn, &tip.hash.to_string(), node.node_id)
                            {
                                error!("Database error {:?}", e);
                                continue;
                            }

                            // Undo the rollback
                            for hash in invalidated_hashes {
                                let _ = mirror.reconsider_block(&hash);
                            }
                        } else {
                            for t in tips
                                .into_iter()
                                .filter(|t| t.status == GetChainTipsResultStatus::Invalid)
                            {
                                let _ = mirror.reconsider_block(&t.hash);
                            }
                        }

                        if let Err(e) = mirror.set_network_active(true) {
                            error!("Could not reactivate p2p {:?}", e);
                        }

                        // TODO: is tip still considered invalid? mark it invalid #442++
                    }
                    Err(_) => {
                        error!("Could not make tip active, restoring state...");
                        if let Err(e) = mirror.set_network_active(true) {
                            error!("Could not reactivate p2p {:?}", e);
                        }
                        let tips = match mirror.get_chain_tips() {
                            Ok(t) => t,
                            Err(e) => {
                                error!("Chain tips error {:?}", e);
                                continue;
                            }
                        };

                        for t in tips
                            .into_iter()
                            .filter(|t| t.status == GetChainTipsResultStatus::Invalid)
                        {
                            let _ = mirror.reconsider_block(&t.hash);
                        }
                    }
                }
            }
        }
    }

    // Find all the blocks that need to be invalidated on the mirror node in order to set a new
    // tip.
    fn set_tip_active(
        &self,
        mirror: &BC,
        tip: &GetChainTipsResultTip,
    ) -> ForkScannerResult<Vec<btc::BlockHash>> {
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

            let active = tips
                .iter()
                .find(|t| t.status == GetChainTipsResultStatus::Active)
                .unwrap();

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

    // Fin the point where the two tips branched from eachother by walking up the ancestry of on
    // tip and finding the earliest one that is also an ancestor of the other tip.
    fn find_branch_point(
        &self,
        active: &GetChainTipsResultTip,
        tip: &GetChainTipsResultTip,
    ) -> Option<btc::BlockHash> {
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

                let desc = block1.descendants(&self.db_conn, None).ok()?;

                let fork = desc.into_iter().find(|b| b.hash == tip.hash.to_string());

                if let Some(_) = fork {
                    let hash = btc::BlockHash::from_str(&block1.hash).unwrap();
                    return Some(hash);
                }
            }
        }
    }

    // Do we have any blocks that are 'headers-only'? If so, try to fetch the full body.
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

                    if let Err(e) = node.client().submit_block(b, &hash) {
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

                    if let Err(e) = mirror.submit_header(header) {
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
                if let Err(_) = mirror.get_block_from_peer(block.hash.clone(), peer.id) {
                    let _ = mirror.disconnect_node(peer.id);
                }
            }
            gbfp_blocks.push(block);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use diesel::{sql_query, Connection, RunQueryDsl};

    fn chaintips_setup() -> Vec<GetChainTipsResultTip> {
        vec![GetChainTipsResultTip {
            branch_length: 8,
            hash: btc::BlockHash::from_str(
                "0000000000000000000501b978d69da3d476ada6a41aba60a42612806204013a",
            )
            .expect("Bad hash"),
            height: 78000,
            status: GetChainTipsResultStatus::HeadersOnly,
        }]
    }

    fn blockheaders1() -> impl Iterator<Item = GetBlockHeaderResult> {
        let items: Vec<GetBlockHeaderResult> =
            serde_json::from_str(include_str!("data/blockheaders1.json")).expect("Bad JSON");
        items.into_iter()
    }

    fn setup_blocks1(conn: &PgConnection) {
        let queries = include_str!("data/blocks1.txt");
        sql_query(queries).execute(conn).expect("Query failed");
    }

    #[test]
    fn test_process_client() {
        let db_url = "postgres://forktester:forktester@localhost/forktester";
        let db_conn = PgConnection::establish(&db_url).expect("Connection failed");
        let test_conn = PgConnection::establish(&db_url).expect("Connection failed");

        let ctx = MockBtcClient::new_context();
        ctx.expect()
            .times(3)
            .returning(|_x, _y| Ok(MockBtcClient::default()));

        let mut scanner = ForkScanner::<MockBtcClient>::new(db_conn).expect("Client setup failed");
        scanner.clients[0]
            .client
            .expect_get_chain_tips()
            .returning(|| Ok(vec![]));
        {
            let node = &scanner.node_list[0];
            let client = &scanner.clients[0].client;
            let result = scanner.process_client(client, node);
            assert!(result.is_ok());
        }

        scanner.clients[0].client.checkpoint();

        let tips = chaintips_setup();

        scanner.clients[0]
            .client
            .expect_get_chain_tips()
            .return_once(move || Ok(tips));

        scanner.clients[0]
            .client
            .expect_get_block_header_info()
            .return_once(move |_| {
                Err(bitcoincore_rpc::Error::Io(std::io::Error::from(
                    std::io::ErrorKind::ConnectionRefused,
                )))
            });

        {
            let node = &scanner.node_list[0];
            let client = &scanner.clients[0].client;
            let result = scanner.process_client(client, node);
            assert!(result.is_err());
        }

        scanner.clients[0].client.checkpoint();

        let tips = chaintips_setup();
        let mut blockheaders = blockheaders1();

        {
            let t = test_conn
                .begin_test_transaction()
                .expect("Could not open test transaction");
            let node = &scanner.node_list[0];
            setup_blocks1(&test_conn);

            scanner.clients[0]
                .client
                .expect_get_chain_tips()
                .return_once(move || Ok(tips));

            scanner.clients[0]
                .client
                .expect_get_block_header_info()
                .times(1)
                .returning(move |_| Ok(blockheaders.next().expect("Out of headers")));

            let client = &scanner.clients[0].client;
            scanner
                .process_client(client, node)
                .expect("process_client failed");
        }

        test_conn.test_transaction(|| {
            let _db_setup = diesel::sql_query(include_str!("data/setup_match_children.sql"))
                .execute(&test_conn)
                .expect("DB query failed");
            let tip: Chaintip = serde_json::from_str(include_str!("data/match_children_tips.json"))
                .expect("Bad JSON");

            scanner
                .match_children(&tip)
                .expect("Match children call failed");

            let actives: Vec<_> = Chaintip::list_active(&test_conn)
                .expect("DB query failed")
                .into_iter()
                .map(|tip| (tip.id, tip.parent_chaintip))
                .collect();

            assert_eq!(
                actives,
                vec![(0, None), (1, Some(9)), (4, None), (5, Some(0))]
            );
            Ok::<(), ForkScannerError>(())
        });
    }
}
