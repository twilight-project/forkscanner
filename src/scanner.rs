use crate::{
    Block, BlockTemplate, Chaintip, ConflictingBlock, FeeRate, InflatedBlock, InvalidBlock, Lags,
    NewPeer, Node, Peer, Pool, SoftForks, StaleCandidate, StaleCandidateChildren, Transaction,
    TxOutset, Watched,
};
use bigdecimal::{BigDecimal, FromPrimitive};
use bitcoin::{consensus::encode::serialize_hex, util::amount::Amount};
use bitcoin_hashes::{sha256d, Hash};
use bitcoincore_rpc::bitcoin as btc;
use bitcoincore_rpc::bitcoincore_rpc_json::{
    GetBlockHeaderResult, GetBlockResult, GetBlockTemplateCapabilities, GetBlockTemplateModes,
    GetBlockTemplateResult, GetBlockTemplateRules, GetBlockchainInfoResult,
    GetChainTipsResultStatus, GetChainTipsResultTip, GetPeerInfoResultConnectionType,
    GetPeerInfoResultNetwork, GetRawTransactionResult, GetTxOutSetInfoResult,
};
use bitcoincore_rpc::Error as BitcoinRpcError;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use chrono::prelude::*;
use crossbeam::channel::{unbounded, Receiver, Sender, TryRecvError};
use diesel::prelude::PgConnection;
use diesel::Connection;
use jsonrpc::error::Error as JsonRpcError;
use jsonrpc::error::RpcError;
use log::{debug, error, info, warn};
#[cfg(test)]
use mockall::*;
use rayon::prelude::*;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    iter::{once, FromIterator},
    str::FromStr,
};
use thiserror::Error;

const NULL_DATA_INDEX: usize = 1;
const SCRIPT_HASH_INDEX: usize = 1;
const MAX_ANCESTRY_DEPTH: usize = 100;
const MAX_BLOCK_DEPTH: i64 = 10;
const BLOCK_NOT_FOUND: i32 = -5;
const BLOCK_NOT_ON_DISK: i32 = -1;
const STALE_WINDOW: i64 = 100;
const DOUBLE_SPEND_RANGE: i64 = 30;
const REACHABLE_CHECK_INTERVAL: i64 = 10;
const MINER_POOL_INFO: &str =
    "https://raw.githubusercontent.com/0xB10C/known-mining-pools/master/pools.json";
const SATOSHI_TO_BTC: i64 = 100_000_000;

type ForkScannerResult<T> = Result<T, ForkScannerError>;

/// Types for the pool info fetched from MINER_POOL_INFO.
#[derive(Debug, Deserialize)]
pub struct MinerPool {
    pub name: String,
    pub link: String,
}

#[derive(Debug, Deserialize)]
pub struct MinerPoolInfo {
    pub coinbase_tags: HashMap<String, MinerPool>,
    pub payout_addresses: HashMap<String, MinerPool>,
}

/// Notifications from forkscanner to the api server.
pub enum ScannerMessage {
    LaggingNodes(Vec<Lags>),
    NewChaintip,
    NewBlockConflicts(Vec<ConflictingBlock>),
    AllChaintips(Vec<Chaintip>),
    StaleCandidateUpdate,
    TipUpdateFailed(String),
    TipUpdated(Vec<String>),
    WatchedAddress(Vec<Transaction>),
}

/// Command types from api to forkscanner.
pub enum ScannerCommand {
    SetTip { node_id: i64, hash: String },
}

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
#[allow(unused)]
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
#[allow(unused)]
pub struct Vin {
    txid: Option<String>,
    vout: Option<usize>,
    script_sig: Option<ScriptSig>,
    sequence: usize,
    txinwitness: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct Vout {
    value: f64,
    n: usize,
    script_pub_key: ScriptPubKey,
}

#[derive(Debug, Deserialize)]
#[allow(unused)]
pub struct ScriptSig {
    asm: String,
    hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct ScriptPubKey {
    asm: String,
    hex: String,
    req_sigs: Option<usize>,
    r#type: String,
    addresses: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct PeerInfo {
    pub id: u64,
    pub addr: String,
    pub addrbind: String,
    pub addrlocal: Option<String>,
    pub network: Option<GetPeerInfoResultNetwork>,
    pub services: String,
    pub relaytxes: bool,
    pub lastsend: u64,
    pub lastrecv: u64,
    pub last_transaction: Option<u64>,
    pub last_block: Option<u64>,
    pub bytessent: u64,
    pub bytesrecv: u64,
    pub conntime: u64,
    pub timeoffset: i64,
    pub pingtime: Option<f64>,
    pub minping: Option<f64>,
    pub pingwait: Option<f64>,
    pub version: u64,
    pub subver: String,
    pub inbound: bool,
    pub addnode: Option<bool>,
    pub startingheight: Option<i64>,
    pub banscore: Option<i64>,
    pub synced_headers: Option<i64>,
    pub synced_blocks: Option<i64>,
    pub inflight: Option<Vec<u64>>,
    pub whitelisted: Option<bool>,
    #[serde(
        rename = "minfeefilter",
        default,
        with = "bitcoin::util::amount::serde::as_btc::opt"
    )]
    pub min_fee_filter: Option<Amount>,
    pub bytessent_per_msg: Option<HashMap<String, u64>>,
    pub bytesrecv_per_msg: Option<HashMap<String, u64>>,
    pub connection_type: Option<GetPeerInfoResultConnectionType>,
}

/// Trait defining interface to bitcoin RPC API
#[cfg_attr(test, automock)]
pub trait BtcClient: Sized {
    fn new(host: &String, auth: Auth) -> ForkScannerResult<Self>;
    fn disconnect_node(&self, id: u64) -> Result<serde_json::Value, bitcoincore_rpc::Error>;
    fn get_blockchain_info(&self) -> Result<GetBlockchainInfoResult, bitcoincore_rpc::Error>;
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
    fn get_block_info(
        &self,
        hash: &btc::BlockHash,
    ) -> Result<GetBlockResult, bitcoincore_rpc::Error>;
    fn get_block_template(
        &self,
        mode: GetBlockTemplateModes,
        rules: &[GetBlockTemplateRules],
        capabilities: &[GetBlockTemplateCapabilities],
    ) -> Result<GetBlockTemplateResult, bitcoincore_rpc::Error>;
    fn get_block_verbose(&self, hash: String) -> Result<FullBlock, bitcoincore_rpc::Error>;
    fn get_block_header_info(
        &self,
        hash: &btc::BlockHash,
    ) -> Result<GetBlockHeaderResult, bitcoincore_rpc::Error>;
    fn get_block_hex(&self, hash: &btc::BlockHash) -> Result<String, bitcoincore_rpc::Error>;
    fn get_peer_info(&self) -> Result<Vec<PeerInfo>, bitcoincore_rpc::Error>;
    fn get_raw_transaction_info<'a>(
        &self,
        txid: &btc::Txid,
        block_hash: Option<&'a btc::BlockHash>,
    ) -> Result<GetRawTransactionResult, bitcoincore_rpc::Error>;
    fn get_tx_out_set_info(&self) -> Result<GetTxOutSetInfoResult, bitcoincore_rpc::Error>;
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

    fn get_blockchain_info(&self) -> Result<GetBlockchainInfoResult, bitcoincore_rpc::Error> {
        RpcApi::get_blockchain_info(self)
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

    fn get_block_info(
        &self,
        hash: &btc::BlockHash,
    ) -> Result<GetBlockResult, bitcoincore_rpc::Error> {
        RpcApi::get_block_info(self, hash)
    }

    fn get_block_template(
        &self,
        mode: GetBlockTemplateModes,
        rules: &[GetBlockTemplateRules],
        capabilities: &[GetBlockTemplateCapabilities],
    ) -> Result<GetBlockTemplateResult, bitcoincore_rpc::Error> {
        RpcApi::get_block_template(self, mode, rules, capabilities)
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

    fn get_peer_info(&self) -> Result<Vec<PeerInfo>, bitcoincore_rpc::Error> {
        RpcApi::call(self, "getpeerinfo", &[])
    }

    fn get_raw_transaction_info(
        &self,
        txid: &btc::Txid,
        block_hash: Option<&btc::BlockHash>,
    ) -> Result<GetRawTransactionResult, bitcoincore_rpc::Error> {
        RpcApi::get_raw_transaction_info(self, txid, block_hash)
    }

    fn get_tx_out_set_info(&self) -> Result<GetTxOutSetInfoResult, bitcoincore_rpc::Error> {
        RpcApi::get_tx_out_set_info(self)
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
    #[error("Invalid coinbase")]
    InvalidCoinbase,
}

fn calc_max_inflation(height: i64) -> Option<BigDecimal> {
    let interval = height as usize / 210_000;
    let reward = 50 * SATOSHI_TO_BTC;
    BigDecimal::from_i64(reward >> interval)
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
        let mut block = Block::get_or_create(&conn, headers_only, node_id, &bh)?;

        if block.connected {
            break;
        }

        // working with a pruned node, we'll get a BLOCK_NOT_ON_DISK message, this is okay.
        let GetBlockResult { tx, .. } = match client.get_block_info(&hash) {
            Ok(block) => block,
            Err(BitcoinRpcError::JsonRpc(JsonRpcError::Rpc(RpcError { code, .. })))
                if code == BLOCK_NOT_ON_DISK =>
            {
                return Ok(());
            }
            Err(e @ _) => return Err(e.into()),
        };
        if let Some((coinbase_tx, rest_txs)) = tx.split_first() {
            let coinbase_info = client.get_raw_transaction_info(&coinbase_tx, Some(&hash))?;

            let hash_bytes: Vec<u8> = once(coinbase_tx)
                .chain(rest_txs.iter())
                .flat_map(|tx| tx.as_hash().as_ref().to_vec())
                .collect();

            if coinbase_info.vin.len() == 0 {
                error!("Invalid coinbase!");
                return Err(ForkScannerError::InvalidCoinbase);
            }

            let mut pool = None;
            let mut coinbase_message = None;

            for vin in coinbase_info.vin.into_iter() {
                if let Some(cb) = vin.coinbase {
                    coinbase_message = Some(cb.clone());
                    let pool_tag = String::from_utf8_lossy(&cb).to_string();

                    for p in Pool::list(&conn)? {
                        if let Some(_) = pool_tag.find(&p.tag) {
                            pool = Some(p);
                            break;
                        }
                    }

                    if pool.is_some() {
                        break;
                    }
                }
            }

            let pool_name = if pool.is_none() {
                let cbm = coinbase_message.clone().unwrap_or(b"NONE".to_vec());
                let name = format!("{:X?}", cbm);
                warn!("Missing coinbase info! Your mining pool info may be out of date. Coinbase message is {:?}", name);
                name
            } else {
                pool.unwrap().name
            };

            let mut amount = 0;
            for vout in coinbase_info.vout.iter() {
                amount += vout.value.as_sat();
            }

            let max_inflation =
                calc_max_inflation(block.height).expect("Could not calculate inflation");
            let total_fee = (BigDecimal::from(amount) - max_inflation) / SATOSHI_TO_BTC;

            block.txids = Some(hash_bytes);
            block.pool_name = Some(pool_name);
            block.total_fee = Some(total_fee);
            block.coinbase_message = coinbase_message;
            if let Err(e) = block.update(&conn) {
                error!("DB update failed for block fees {e:?}");
            }
        } else {
            warn!("No coinbase tx in block!");
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

/// Find fork point between given block and the current active block,
/// then invalidate up to the fork point, and set the given block as active tip.
fn make_block_active<BC: BtcClient>(
    client: &BC,
    db_conn: &PgConnection,
    block: &Block,
) -> ForkScannerResult<Vec<btc::BlockHash>> {
    let mut invalidated_hashes = Vec::new();
    let mut retry_count = 0;

    loop {
        let tips = match client.get_chain_tips() {
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

        let active_hash = active.hash.to_string();
        if active_hash == block.hash {
            break;
        }

        if retry_count > 100 {
            return Err(ForkScannerError::FailedRollback);
        }

        let mut blocks_to_invalidate = Vec::new();

        if active.height as i64 == block.height {
            blocks_to_invalidate.push(active.hash);
        } else {
            if let Some(branch) = find_fork_point(db_conn, active, block) {
                blocks_to_invalidate.push(branch);
            }

            let children = match Block::children(db_conn, &block.hash.to_string()) {
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

        for b in blocks_to_invalidate {
            let _ = client.invalidate_block(&b);
            invalidated_hashes.push(b);
        }

        retry_count += 1;
    }
    Ok(invalidated_hashes)
}

/// Find the fork point between given block and the active tip.
fn find_fork_point(
    db_conn: &PgConnection,
    active: &GetChainTipsResultTip,
    block: &Block,
) -> Option<btc::BlockHash> {
    if active.height as i64 <= block.height {
        return None;
    }

    let mut block1 = Block::get(&db_conn, &active.hash.to_string()).ok()?;

    while block1.height > block.height {
        block1 = block1.parent(db_conn).ok()?;
    }

    if block1.hash == block.hash {
        None
    } else {
        loop {
            block1 = block1.parent(db_conn).ok()?;

            let desc = block1.descendants(db_conn, None).ok()?;

            let fork = desc.into_iter().find(|b| b.hash == block.hash);

            if let Some(_) = fork {
                let hash = btc::BlockHash::from_str(&block1.hash).unwrap();
                return Some(hash);
            }
        }
    }
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
pub struct ForkScanner<BC: BtcClient + std::fmt::Debug> {
    node_list: Vec<Node>,
    archive_node: ScannerClient<BC>,
    clients: Vec<ScannerClient<BC>>,
    db_conn: PgConnection,
    notify_tx: Sender<ScannerMessage>,
    command: Receiver<ScannerCommand>,
}

impl<BC: BtcClient + std::fmt::Debug> ForkScanner<BC> {
    pub fn new(
        db_conn: PgConnection,
    ) -> ForkScannerResult<(
        ForkScanner<BC>,
        Receiver<ScannerMessage>,
        Sender<ScannerCommand>,
    )> {
        let node_list = Node::list(&db_conn)?;

        let mut clients = Vec::new();
        let mut archive_node = None;
        let mut found_archive = false;

        for node in &node_list {
            let host = format!("http://{}:{}", node.rpc_host, node.rpc_port);
            let auth = Auth::UserPass(node.rpc_user.clone(), node.rpc_pass.clone());

            let mirror_host = match node.mirror_rpc_port {
                Some(port) => Some(format!("http://{}:{}", node.rpc_host, port)),
                None => None,
            };
            info!(
                "Connecting to bitcoin client: {}, Mirror: {:?}",
                host, mirror_host
            );

            if archive_node.is_none() {
                let client = ScannerClient::new(node.id, host.clone(), None, auth.clone())?;
                archive_node = Some(client);
            } else if node.archive {
                let client = ScannerClient::new(node.id, host.clone(), None, auth.clone())?;
                archive_node = Some(client);
                found_archive = true;
            }

            let client = ScannerClient::new(node.id, host, mirror_host, auth)?;
            clients.push(client);
        }

        let (notify_tx, notify_rx) = unbounded();
        let (cmd_tx, cmd_rx) = unbounded();

        if !found_archive {
            warn!("No archive node was found, using first node as fallback!");
        }

        Ok((
            ForkScanner {
                archive_node: archive_node.unwrap(),
                node_list,
                clients,
                db_conn,
                notify_tx,
                command: cmd_rx,
            },
            notify_rx,
            cmd_tx,
        ))
    }

    // fetch block templates and calculate fee rates.
    fn fetch_block_templates(&self, client: &BC, node: &Node) {
        info!("Block templates from {}", node.id);
        match client.get_block_template(
            GetBlockTemplateModes::Template,
            &[GetBlockTemplateRules::SegWit],
            &[],
        ) {
            Ok(template) => {
                let parent = template.previous_block_hash.to_string();
                let height = template.height as i64;
                let n_txs = template.transactions.len() as i32;
                let tx_ids = template
                    .transactions
                    .iter()
                    .flat_map(|tx| tx.txid.as_hash().as_ref().to_vec())
                    .collect();
                let rates = template
                    .transactions
                    .iter()
                    .map(|tx| tx.fee.as_sat() as i32 / (tx.weight as i32 / 4))
                    .collect();

                let total = BigDecimal::from(template.coinbase_value.as_sat())
                    - calc_max_inflation(height).expect("Could not get max_inflation")
                        / SATOSHI_TO_BTC;

                // Create new db entry for the template
                if let Err(e) = BlockTemplate::create(
                    &self.db_conn,
                    parent,
                    node.id,
                    total,
                    height,
                    n_txs,
                    tx_ids,
                    rates,
                ) {
                    error!("Failed to create template entry {e:?}");
                }
            }
            Err(e) => {
                error!("Error fetching block templates! {e:?}");
            }
        }
    }

    // process chaintip entries for a client, log to database.
    fn process_client(&self, client: &BC, node: &Node) -> ForkScannerResult<bool> {
        let tips = client.get_chain_tips()?;

        let mut changed = false;
        info!("Node {} has {} chaintips to process", node.id, tips.len());
        for tip in tips {
            let hash = tip.hash.to_string();

            // In all cases, try to fetch ancestor blocks as well.
            match tip.status {
                GetChainTipsResultStatus::HeadersOnly => {
                    match create_block_and_ancestors(client, &self.db_conn, true, &hash, node.id) {
                        Err(ForkScannerError::RpcClientError(e)) => {
                            if let BitcoinRpcError::JsonRpc(JsonRpcError::Rpc(RpcError {
                                code,
                                ..
                            })) = e
                            {
                                if code != BLOCK_NOT_ON_DISK {
                                    return Err(ForkScannerError::RpcClientError(e));
                                }
                            } else {
                                return Err(ForkScannerError::RpcClientError(e));
                            }
                        }
                        Err(e) => return Err(e),
                        _ => {}
                    }
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
                    let rows =
                        Chaintip::set_active_tip(&self.db_conn, tip.height as i64, &hash, node.id)?;

                    create_block_and_ancestors(client, &self.db_conn, false, &hash, node.id)?;

                    Block::set_valid(&self.db_conn, &hash, node.id)?;
                    changed |= rows > 0;
                }
            }

            if let Ok(block) = Block::get(&self.db_conn, &hash) {
                self.fetch_transactions(&block);
            }
        }
        Ok(changed)
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

    // get raw transaction hex for each watched address tx
    fn watched_address_checks(&self) -> Vec<Transaction> {
        // Clear expired watch entries
        if let Err(e) = Watched::clear(&self.db_conn) {
            error!("Watchlist query error {:?}", e);
            return vec![];
        }

        match Watched::fetch(&self.db_conn) {
            Ok(transactions) => transactions,
            Err(e) => {
                error!("An error occured fetching watch list {:?})", e);
                vec![]
            }
        }
    }

    fn lag_checks(&self) -> Vec<Lags> {
        if let Err(e) = Lags::purge(&self.db_conn) {
            error!("Purge lag tables failed {:?}", e);
        }

        match Chaintip::list_active(&self.db_conn) {
            Ok(tips) => {
                let max_height = tips.iter().map(|t| t.height).max().unwrap();
                let blocks: Vec<_> = tips
                    .iter()
                    .filter_map(|t| match Block::get(&self.db_conn, &t.block) {
                        Ok(b) => Some(b),
                        Err(e) => {
                            error!("Database error checking block work: {:?}", e);
                            None
                        }
                    })
                    .collect();
                let max_work = blocks.iter().map(|b| b.work.clone()).max().unwrap();

                for tip in tips {
                    let block = blocks.iter().find(|b| b.hash == tip.block).unwrap();

                    // If it's 2 blocks behind or work is less, consider it lagging
                    if tip.height < max_height - 1 || block.work < max_work {
                        if let Err(e) = Lags::insert(&self.db_conn, tip.node) {
                            error!("Node lag update failed: {:?}", e);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Lag checks failed {:?}", e);
            }
        }

        match Lags::list(&self.db_conn) {
            Ok(lags) => lags,
            Err(e) => {
                error!("Error fetching lagging nodes: {:?}", e);
                vec![]
            }
        }
    }

    // We initialized with get_best_block_hash, now we just poll continually
    // for new blocks, and fetch ancestors up to MAX_BLOCK_HEIGHT postgres
    // will do the rest for us.
    pub fn run(&self) {
        // update the miner pools info
        match ureq::get(MINER_POOL_INFO).call() {
            Ok(info) => {
                match info.into_string() {
                    Ok(info) => {
                        if let Ok::<MinerPoolInfo, _>(pool_info) = serde_json::from_str(&info) {
                            if let Err(e) = Pool::create_or_update_batch(&self.db_conn, pool_info) {
                                error!("Failed to update miner pool info! {e:?}");
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to fetch miner pool info! {e:?}");
                    }
                };
            }
            Err(e) => {
                warn!("Could not fetch miner pool info! {e:?}");
            }
        };
        // start by purging chaintips, keeping only the previously 'active' chaintips.
        if let Err(e) = Chaintip::purge(&self.db_conn) {
            error!("Error purging database {:?}", e);
            return;
        }

        // purge block templates as well.
        if let Err(e) = BlockTemplate::purge(&self.db_conn) {
            error!("Error purging database {:?}", e);
            return;
        }

        // check for requests from the api server
        while self.command.len() > 0 {
            match self.command.try_recv() {
                Ok(msg) => match msg {
                    ScannerCommand::SetTip { node_id, hash } => {
                        let node = self
                            .clients
                            .iter()
                            .find(|c| c.node_id == node_id)
                            .expect("Node not found!");

                        let block = match Block::get(&self.db_conn, &hash) {
                            Ok(b) => b,
                            Err(e) => {
                                error!("Could not fetch block from db!");
                                self.notify_tx
                                    .send(ScannerMessage::TipUpdateFailed(e.to_string()))
                                    .expect("Notify channel broken");
                                continue;
                            }
                        };

                        match self.set_tip_active(node.client(), block.hash, block.height as u64) {
                            Ok(invalidated_hashes) => {
                                let hashes = invalidated_hashes
                                    .into_iter()
                                    .map(|h| h.to_string())
                                    .collect();
                                let update = ScannerMessage::TipUpdated(hashes);
                                self.notify_tx.send(update).expect("Notify channel broken");
                            }
                            Err(e) => {
                                error!("Could not set chaintip for node {}!", node_id);
                                self.notify_tx
                                    .send(ScannerMessage::TipUpdateFailed(e.to_string()))
                                    .expect("Notify channel broken");
                            }
                        }
                    }
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    error!("Command channel disconnected!");
                    return;
                }
            }
        }

        let mut changed = false;
        for (client, node) in self.clients.iter().zip(&self.node_list) {
            if let Ok(peers) = client.client().get_peer_info() {
                let peers = peers
                    .into_iter()
                    .map(|p| NewPeer {
                        node_id: node.id,
                        peer_id: p.id as i64,
                        address: p.addr,
                        version: p.version as i64,
                    })
                    .collect();

                if let Err(e) = Peer::update_peers(&self.db_conn, node.id, peers) {
                    error!("Peer list update failed! {:?}", e);
                }
            } else {
                error!("RPC get peers failed!");
            }

            if let Ok(info) = client.client().get_blockchain_info() {
                info!("Got blockchain info");
                if let Err(e) =
                    SoftForks::update_or_insert(&self.db_conn, client.node_id, info.softforks)
                {
                    error!("Softfork update failed: {:?}", e);
                }
            } else {
                error!(
                    "Failed to fetch blockchain info from {:?}!",
                    client.client()
                );
                continue;
            }

            self.fetch_block_templates(client.client(), node);

            // process new chaintip entries from each client.
            changed |= match self.process_client(client.client(), node) {
                Ok(changed) => changed,
                Err(e) => {
                    error!("Error processing client {:?}", e);
                    continue;
                }
            };
        }

        // We have up to date chaintips, check for lags
        let lags = self.lag_checks();

        if lags.len() > 0 {
            info!("We have {} lagging nodes", lags.len());
            self.notify_tx
                .send(ScannerMessage::LaggingNodes(lags))
                .expect("Channel closed");
        }

        // Check watched addresses
        let addresses = self.watched_address_checks();

        if addresses.len() > 0 {
            info!("We have {} watched address activity", addresses.len());
            self.notify_tx
                .send(ScannerMessage::WatchedAddress(addresses))
                .expect("Channel closed");
        }

        // update the API server of chaintip updates
        if changed {
            info!("Sending chaintip notifications");
            self.notify_tx
                .send(ScannerMessage::NewChaintip)
                .expect("Channel closed");
        }

        match InvalidBlock::get_recent_conflicts(&self.db_conn) {
            Ok(conflicts) if conflicts.len() > 0 => {
                self.notify_tx
                    .send(ScannerMessage::NewBlockConflicts(conflicts))
                    .expect("Channel closed");
            }
            Ok(_) => {}
            Err(e) => {
                error!("Error querying database for block conflicts! {:?}", e);
            }
        }

        // get min height block template, and blocks with no fee diffs yet.
        info!("Fecthing block templates");
        match BlockTemplate::get_min(&self.db_conn) {
            Ok(Some(min_template)) => {
                if let Ok(blocks) = Block::get_with_fee_no_diffs(&self.db_conn, min_template) {
                    for mut block in blocks {
                        if block.txids.is_none() {
                            continue;
                        }

                        let latest_template =
                            match BlockTemplate::get_with_txs(&self.db_conn, block.height) {
                                Ok(lb) => lb,
                                Err(e) => {
                                    error!("Could not fetch latest template! {e:?}");
                                    continue;
                                }
                            };

                        if latest_template.tx_ids.len() == 0 {
                            continue;
                        }

                        let template_txids: Vec<_> = (latest_template.tx_ids)
                            .chunks(32)
                            .map(|chunk| sha256d::Hash::from_slice(chunk).expect("Bad hash value"))
                            .collect();
                        let block_txids =
                            HashSet::<_>::from_iter((block.txids.unwrap()).chunks(32).map(
                                |chunk| sha256d::Hash::from_slice(chunk).expect("Bad hash value"),
                            ));

                        let tx_pos_omitted =
                            template_txids.iter().enumerate().filter_map(|(idx, txid)| {
                                if block_txids.contains(txid) {
                                    None
                                } else {
                                    Some(idx)
                                }
                            });
                        let tx_template = HashSet::<_>::from_iter(template_txids.iter().cloned());

                        let total_fee = block.total_fee.unwrap();
                        let added = block_txids.difference(&tx_template);
                        let omitted = tx_template.difference(&block_txids);
                        match FeeRate::list_by(
                            &self.db_conn,
                            latest_template.parent_block_hash,
                            latest_template.node_id,
                        ) {
                            Ok(fee_rates) => {
                                for mut fee_rate in
                                    tx_pos_omitted.into_iter().map(|i| fee_rates[i].clone())
                                {
                                    fee_rate.omitted = true;
                                    if let Err(e) = fee_rate.update(&self.db_conn) {
                                        error!("Fee rate update failed {e:?}");
                                        return;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Could not fetch fee rates {e:?}");
                                return;
                            }
                        };

                        block.txids_added =
                            Some(added.into_iter().flat_map(|a| a.to_vec()).collect());
                        block.txids_omitted =
                            Some(omitted.into_iter().flat_map(|a| a.to_vec()).collect());
                        block.lowest_template_fee_rate =
                            Some(BigDecimal::from(latest_template.lowest_fee_rate));
                        block.template_txs_fee_diff = Some(total_fee - latest_template.fee_total);
                    }
                }
            }
            Ok(None) => warn!("No block templates!"),
            Err(e) => {
                error!("Error fetching min template! {e:?}");
                return;
            }
        };

        match Chaintip::list_non_lagging(&self.db_conn) {
            Ok(tips) => {
                // Get the most frequent tip
                let counts = tips
                    .iter()
                    .map(|tip| tip.block.clone())
                    .collect::<counter::Counter<_>>();

                if let Some((top, _)) = counts.most_common().into_iter().next() {
                    let tip = tips.into_iter().find(|item| item.block == top).unwrap();

                    self.notify_tx
                        .send(ScannerMessage::AllChaintips(vec![tip]))
                        .expect("Channel closed");
                }
            }
            Err(e) => error!("Database error: {:?}", e),
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

        // Now try to fill in missing blocks,
        // check inflation, do rollbacks, and stale candidates.
        self.find_missing_blocks();
        self.inflation_checks();
        self.rollback_checks();
        self.find_stale_candidates();

        // for 3 most recent stale candidates...
        self.process_stale_candidates();
        self.notify_tx
            .send(ScannerMessage::StaleCandidateUpdate)
            .expect("Channel closed");
    }

    fn inflation_checks(&self) {
        let mirrors = match Node::get_mirrors(&self.db_conn) {
            Ok(nodes) => nodes,
            Err(e) => {
                error!("RPC Error {e:?}");
                return;
            }
        };

        info!("Checking mirror node reachability");
        for mut mirror in mirrors {
            if let Some(_ts) = mirror.unreachable_since {
                let last_poll = mirror.last_polled.expect("No last_polled");
                let elapsed = Utc::now().signed_duration_since(last_poll);
                if elapsed.num_minutes() > REACHABLE_CHECK_INTERVAL {
                    mirror.last_polled = Some(Utc::now());
                    let node = self
                        .clients
                        .iter()
                        .find(|c| c.node_id == mirror.id)
                        .unwrap()
                        .mirror()
                        .as_ref()
                        .unwrap();

                    match node.get_blockchain_info() {
                        Ok(info) => {
                            mirror.initial_block_download = info.initial_block_download;
                            mirror.unreachable_since = None;
                            mirror.last_polled = None;
                        }
                        Err(e) => debug!("Could not reach mirror on reachable check {e:?}"),
                    }

                    if let Err(e) = mirror.update(&self.db_conn) {
                        error!("Updating mirror info failed {e:?}");
                    }
                }
            }
        }

        let mirrors = match Node::get_active_reachable(&self.db_conn) {
            Ok(m) => m,
            Err(e) => {
                error!("Could not connect to database {e:?}");
                return;
            }
        };

        info!("Inflation checks for {} nodes", mirrors.len());
        mirrors.par_iter().for_each(|mirror| {
            let host = format!(
                "http://{}:{}",
                mirror.rpc_host,
                mirror.mirror_rpc_port.expect("No mirror port")
            );
            let auth = Auth::UserPass(mirror.rpc_user.clone(), mirror.rpc_pass.clone());
            let client = BC::new(&host, auth).expect("Create client failed");

            let db_url = std::env::var("DATABASE_URL").expect("No DB url");
            let db_conn = PgConnection::establish(&db_url).expect("Connection failed");

            // stop p2p traffic so nothing changes underneath us.
            if let Err(e) = client.set_network_active(false) {
                error!("RPC call failed: {e:?}");
                return;
            };

            let latest = match client.get_blockchain_info() {
                Ok(info) => {
                    let hash = info.best_block_hash.to_string();

                    create_block_and_ancestors(&client, &db_conn, true, &hash, mirror.id)
                        .expect("Fetching blocks for inflation checks failed");

                    // if we have one, we're done here.
                    match TxOutset::get(&db_conn, &hash, mirror.id) {
                        Ok(Some(_)) => {
                            let _ = client
                                .set_network_active(true)
                                .expect("Could not re-enable network");
                            return;
                        }
                        Err(e) => {
                            error!("Database error {e:?}");
                            let _ = client
                                .set_network_active(true)
                                .expect("Could not re-enable network");
                            return;
                        }
                        _ => {}
                    };

                    hash
                }
                Err(e) => {
                    error!("RPC call failed {e:?}");
                    let _ = client
                        .set_network_active(true)
                        .expect("Could not re-enable network");
                    return;
                }
            };

            let block = match Block::get(&db_conn, &latest) {
                Ok(block) => block,
                Err(e) => {
                    error!("Block not found {e:?}");
                    let _ = client
                        .set_network_active(true)
                        .expect("Could not re-enable network");
                    return;
                }
            };

            let mut blocks_to_check = vec![block.clone()];
            let mut comparison_block = block.clone();

            loop {
                if block.height - comparison_block.height >= MAX_BLOCK_DEPTH {
                    break;
                }

                comparison_block = match comparison_block.parent(&db_conn) {
                    Ok(block) => block,
                    Err(e) => {
                        error!("Could not fetch parent block {e:?}");
                        let _ = client
                            .set_network_active(true)
                            .expect("Could not re-enable network");
                        return;
                    }
                };

                let comparison_tx_outset =
                    match TxOutset::get(&db_conn, &comparison_block.hash, mirror.id) {
                        Ok(outset) => outset,
                        Err(e) => {
                            error!("Database error {e:?}");
                            let _ = client
                                .set_network_active(true)
                                .expect("Could not re-enable network");
                            return;
                        }
                    };

                if comparison_tx_outset.is_some() {
                    break;
                }

                blocks_to_check.push(comparison_block.clone());
            }

            // Go through all blocks to check, and make each one active one by one fetching tx outset info for each.
            for block in blocks_to_check.iter().rev() {
                match make_block_active(&client, &db_conn, block) {
                    Ok(invalidated_hashes) => {
                        let tx_outset_info = match client.get_tx_out_set_info() {
                            Ok(o) => o,
                            Err(e) => {
                                error!("TX outset info call failed {e:?}");
                                let _ = client
                                    .set_network_active(true)
                                    .expect("Could not re-enable network");
                                return;
                            }
                        };

                        // Undo the rollback
                        for hash in invalidated_hashes {
                            let _ = client.reconsider_block(&hash);
                        }

                        let amount = BigDecimal::from_str(&tx_outset_info.total_amount.to_string())
                            .expect("BigDecimal parsing failed");
                        let mut outset = match TxOutset::create(
                            &db_conn,
                            tx_outset_info.tx_outs,
                            amount,
                            &block.hash,
                            mirror.id,
                        ) {
                            Ok(outset) => outset,
                            Err(e) => {
                                error!("Database connection failed {e:?}");
                                let _ = client
                                    .set_network_active(true)
                                    .expect("Could not re-enable network");
                                return;
                            }
                        };

                        let prev_block = match block.parent(&db_conn) {
                            Ok(b) => b,
                            Err(e) => {
                                error!("Could not fetch parent for fee tx outsets {e:?}");
                                let _ = client
                                    .set_network_active(true)
                                    .expect("Could not re-enable network");
                                return;
                            }
                        };

                        // if we have one, we're done here.
                        let prev_outset = match TxOutset::get(&db_conn, &prev_block.hash, mirror.id)
                        {
                            Ok(Some(os)) => os,
                            Ok(None) => {
                                error!("No previous outset to compare against!");
                                let _ = client
                                    .set_network_active(true)
                                    .expect("Could not re-enable network");
                                return;
                            }
                            Err(e) => {
                                error!("Database error {e:?}");
                                let _ = client
                                    .set_network_active(true)
                                    .expect("Could not re-enable network");
                                return;
                            }
                        };

                        let inflation = outset.total_amount.clone() - prev_outset.total_amount;

                        let max_inflation = calc_max_inflation(block.height)
                            .expect("Could not calculate inflation");

                        if inflation > max_inflation {
                            outset.inflated = true;
                            if let Err(e) = outset.update(&db_conn) {
                                error!("Could not update inflation status for block {e:?}");
                                let _ = client
                                    .set_network_active(true)
                                    .expect("Could not re-enable network");
                                return;
                            }

                            if let Err(e) = InflatedBlock::create(
                                &db_conn,
                                outset.node_id,
                                block,
                                max_inflation,
                                inflation,
                            ) {
                                error!("Could not insert inflated block {e:?}");
                                let _ = client
                                    .set_network_active(true)
                                    .expect("Could not re-enable network");
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        error!("Make block active failed {e:?}");
                        let _ = client
                            .set_network_active(true)
                            .expect("Could not re-enable network");
                        return;
                    }
                }
            }

            // restart p2p traffic
            if let Err(e) = client.set_network_active(true) {
                error!("RPC call failed: {e:?}");
                return;
            };
        });
    }

    fn process_stale_candidates(&self) {
        info!("Processing stale candidates");
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

    // point this candidate at its children.
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

    // find conflicting transactions.
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

    // Get transactions that might be in one branch but not in the other.
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

        info!("Checking TX differences");
        if short.len() < long.len() {
            Some(short.difference(&long).cloned().collect())
        } else {
            Some(short.symmetric_difference(&long).cloned().collect())
        }
    }

    // find blocks at same height, within a window, and mark them as possibly stale.
    fn find_stale_candidates(&self) {
        info!("Stale candidate checks");
        let tip_height = match Block::max_height(&self.db_conn) {
            Ok(Some(tip)) => tip,
            _ => return,
        };

        let candidates =
            match Block::find_stale_candidates(&self.db_conn, tip_height - STALE_WINDOW) {
                Ok(candidates) if candidates.len() > 0 => candidates,
                _ => return,
            };

        info!("{} stale candidates", candidates.len());
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

    // get transactions for a block and save info to database.
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
            let inputs = self.get_input_addrs(tx);
            let mut swept = false;
            let mut address = String::from("NO_ADDRESS");

            for vout in &tx.vout {
                match vout.script_pub_key.r#type.as_str() {
                    "nulldata" => {
                        address = vout
                            .script_pub_key
                            .asm
                            .split(' ')
                            .nth(NULL_DATA_INDEX)
                            .unwrap_or("NO_ADDRESS")
                            .into();
                        debug!("Hashes cannot be converted to addresses, skipping")
                    }
                    "scripthash" => {
                        address = vout
                            .script_pub_key
                            .asm
                            .split(' ')
                            .nth(SCRIPT_HASH_INDEX)
                            .unwrap_or("NO_ADDRESS")
                            .into();
                        debug!("Hashes cannot be converted to addresses, skipping")
                    }
                    "witness_v1_taproot" | "witness_v0_keyhash" | "witness_v0_scripthash" => {
                        address = vout
                            .script_pub_key
                            .asm
                            .split(' ')
                            .last()
                            .unwrap_or("NO_ADDRESS")
                            .into();
                        debug!("Address hashes cannot be converted to addresses, skipping")
                    }
                    "pubkeyhash" => {
                        match &vout.script_pub_key.addresses {
                            Some(addrs) => {
                                address = addrs.iter().next().unwrap().clone();
                                swept |=
                                    !inputs.contains(&btc::Address::from_str(&address).unwrap());
                            }
                            None => {
                                address = vout
                                    .script_pub_key
                                    .asm
                                    .split(' ')
                                    .nth(2)
                                    .unwrap_or("NO_ADDRESS")
                                    .into();
                                debug!("No address in transaction! {:?}", vout);
                            }
                        };
                    }
                    o => {
                        error!("No handler for output type: {} {:?}", o, vout)
                    }
                }
            }

            let value = tx.vout.iter().fold(0., |a, amt| a + amt.value);
            if let Err(e) = Transaction::create(
                &self.db_conn,
                address,
                swept,
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

    fn get_input_addrs(&self, tx: &JsonTransaction) -> HashSet<btc::Address> {
        // find the input amount for the tx
        let mut input_amounts = HashSet::default();

        for txin in tx.vin.iter() {
            if let Some(txid) = &txin.txid {
                let txid = btc::Txid::from_str(&txid).unwrap();
                match self
                    .archive_node
                    .client()
                    .get_raw_transaction_info(&txid, None)
                {
                    Ok(tx) => {
                        for vout in tx.vout.iter() {
                            if let Some(addrs) = &vout.script_pub_key.addresses {
                                input_amounts.extend(addrs.iter().cloned());
                            }
                        }
                    }
                    Err(_) => {
                        // This is very noisy when you don't have an archive node.
                        debug!("Could not fetch transaction info! {:?}", txid);
                        continue;
                    }
                }
            }
        }

        input_amounts
    }

    // Rollback checks. Here we are looking to use the mirror node to try to set a 'valid-headers'
    // chaintip as the active one by invalidating the currently active chaintip. We briefly turn
    // off p2p on this node so the state doesn't change underneath us. Then, if the switch to new
    // chaintip is successful, we check if it would've been a valid tip.
    fn rollback_checks(&self) {
        info!("Running rollback checks");
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
                match self.set_tip_active(mirror, tip.hash.to_string(), tip.height) {
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
        tip_hash: String,
        tip_height: u64,
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

            if active.hash.to_string() == tip_hash {
                break;
            }

            if retry_count > 100 {
                return Err(ForkScannerError::FailedRollback);
            }

            let mut blocks_to_invalidate = Vec::new();

            if active.height == tip_height {
                blocks_to_invalidate.push(active.hash);
            } else {
                if let Some(branch) = self.find_branch_point(active, &tip_hash, tip_height) {
                    blocks_to_invalidate.push(branch);
                }

                let children = match Block::children(&self.db_conn, &tip_hash) {
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
        tip_hash: &String,
        tip_height: u64,
    ) -> Option<btc::BlockHash> {
        if active.height <= tip_height {
            return None;
        }

        let mut block1 = Block::get(&self.db_conn, &active.hash.to_string()).ok()?;

        while block1.height as u64 > tip_height {
            block1 = block1.parent(&self.db_conn).ok()?;
        }

        if &block1.hash == tip_hash {
            None
        } else {
            loop {
                block1 = block1.parent(&self.db_conn).ok()?;

                let desc = block1.descendants(&self.db_conn, None).ok()?;

                let fork = desc.into_iter().find(|b| &b.hash == tip_hash);

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
            Ok(blocks) if blocks.len() == 0 => return,
            Ok(blocks) => blocks,
            Err(e) => {
                error!("Header query failed {:?}", e);
                return;
            }
        };

        info!(
            "There are {} headers only blocks to fetch",
            headers_only_blocks.len()
        );
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
                Err(e) => {
                    error!("No peers: {e:?}");
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

        let client = self
            .clients
            .iter()
            .filter(|c| c.mirror().is_some())
            .next()
            .unwrap();
        let mirror = client.mirror().as_ref().unwrap();

        let mut found_block = false;
        for mut block in gbfp_blocks.into_iter() {
            let hash = btc::BlockHash::from_str(&block.hash).unwrap();
            match mirror.get_block_hex(&hash) {
                Ok(block_hex) => {
                    found_block = true;
                    match mirror.get_block_header_info(&hash) {
                        Ok(info) => {
                            block.headers_only = false;
                            block.work = hex::encode(info.chainwork);
                            if let Err(e) = block.update(&self.db_conn) {
                                error!("Could not clear headers flag {:?}", e);
                            }
                        }
                        Err(e) => {
                            error!("Error fetching block info! {:?}", e);
                        }
                    };

                    match self
                        .clients
                        .iter()
                        .filter(|c| c.node_id == block.first_seen_by)
                        .next()
                    {
                        Some(client) => {
                            if let Err(e) = client.client().submit_block(block_hex, &hash) {
                                error!("Could not submit block to client! {:?}", e);
                                continue;
                            }
                        }
                        None => {
                            error!("Could not find client that saw this block!");
                            continue;
                        }
                    }
                }
                _ => continue,
            }
        }

        if !found_block {
            // disconnect all peers
            match mirror.get_peer_info() {
                Ok(peers) => {
                    for peer in peers {
                        let _ = mirror.disconnect_node(peer.id);
                    }
                }
                Err(e) => {
                    error!("No peers: {e:?}");
                }
            }
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
            //let t = test_conn
            //    .begin_test_transaction()
            //    .expect("Could not open test transaction");
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

        //test_conn.test_transaction(|| {
        //    let _db_setup = diesel::sql_query(include_str!("data/setup_match_children.sql"))
        //        .execute(&test_conn)
        //        .expect("DB query failed");
        //    let tip: Chaintip = serde_json::from_str(include_str!("data/match_children_tips.json"))
        //        .expect("Bad JSON");

        //    scanner
        //        .match_children(&tip)
        //        .expect("Match children call failed");

        //    let actives: Vec<_> = Chaintip::list_active(&test_conn)
        //        .expect("DB query failed")
        //        .into_iter()
        //        .map(|tip| (tip.id, tip.parent_chaintip))
        //        .collect();

        //    assert_eq!(
        //        actives,
        //        vec![(0, None), (1, Some(9)), (4, None), (5, Some(0))]
        //    );
        //    Ok::<(), ForkScannerError>(())
        //});
    }
}
