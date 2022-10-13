use crate::{
    scanner::BtcClient, serde_bigdecimal, Block, Chaintip, ConflictingBlock, Lags, Node, Peer,
    ScannerCommand, ScannerMessage, StaleCandidate, Transaction, Watched,
};
use bigdecimal::BigDecimal;
use bitcoin::consensus::encode::serialize_hex;
use bitcoincore_rpc::bitcoin::Block as BitcoinBlock;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use chrono::prelude::*;
use crossbeam::channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use diesel::prelude::PgConnection;
use hex::ToHex;
use jsonrpc_core::types::error::Error as JsonRpcError;
use jsonrpc_core::*;
use jsonrpc_http_server as hts;
use jsonrpc_pubsub::{PubSubHandler, Session, Sink, Subscriber, SubscriptionId};
use jsonrpc_ws_server as wss;
use log::{debug, error, info};
use r2d2::PooledConnection;
use r2d2_diesel::ConnectionManager;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex, RwLock},
    thread, time,
};
use thiserror::Error;

const BLOCK_WINDOW: i64 = 10;

type Conn = PooledConnection<ConnectionManager<diesel::PgConnection>>;
type ManagedPool = r2d2::Pool<ConnectionManager<PgConnection>>;

#[derive(Debug, Error)]
pub enum WsError {
    #[error("Diesel query error {0:?}")]
    DieselError(#[from] diesel::result::Error),
    #[error("Connection pool error {0:?}")]
    R2D2Error(#[from] r2d2::Error),
    #[error("Serde error {0:?}")]
    SerdeError(#[from] serde_json::Error),
    #[error("Sink error {0:?}")]
    SinkError(#[from] futures::channel::mpsc::TrySendError<std::string::String>),
}

// https://docs.rs/bitcoin/0.27.1/bitcoin/blockdata/block/struct.Block.html
#[derive(Debug, Deserialize)]
struct BlockUpload {
    node_id: i64,
    block: BitcoinBlock,
}

#[derive(Debug, Deserialize)]
struct NodeId {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct GetBlockFromPeer {
    node_id: i64,
    hash: String,
    peer_id: u64,
}

#[derive(Debug, Deserialize)]
struct TipArgs {
    active_only: bool,
}

#[derive(Debug, Deserialize)]
struct TxId {
    id: String,
}

#[derive(Debug, Deserialize)]
struct NodeArgs {
    name: String,
    rpc_host: String,
    rpc_port: i32,
    mirror_rpc_port: Option<i32>,
    user: String,
    pass: String,
    mirror_host: Option<String>,
    archive: bool,
}

#[derive(Debug, Deserialize)]
struct WatchedAddressUpdate {
    remove: Vec<String>,
	add: Vec<(String, DateTime<Utc>)>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct WatchAddress {
    watch: Vec<String>,
    watch_until: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BlockQuery {
    Height(i64),
    Hash(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct SetTipQuery {
    node_id: i64,
    hash: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
struct ValidationCheck {
    tip: String,
    tip_height: i64,
    stale_height: i64,
    height_difference: i64,
    stale_timestamp: DateTime<Utc>,
    stale_branch_len: i64,
    stale_branch_root: String,
}

impl ValidationCheck {
    pub fn new_candidate(
        conn: &Conn,
        tip: &Chaintip,
        candidate: &StaleCandidate,
        candidate_hash: String,
    ) -> Option<ValidationCheck> {
        if tip.height <= candidate.height {
            return None;
        }

        let mut block1 = Block::get(conn, &tip.block.to_string()).ok()?;

        while block1.height > candidate.height {
            block1 = block1.parent(conn).ok()?;
        }

        let (stale_branch_len, stale_branch_root) = if &block1.hash == &candidate_hash {
            (0, candidate_hash.clone())
        } else {
            loop {
                block1 = block1.parent(conn).ok()?;

                let desc = block1.descendants(conn, None).ok()?;

                let fork = desc.into_iter().find(|b| &b.hash == &candidate_hash);

                if let Some(_) = fork {
                    break (candidate.height - block1.height, block1.hash.clone());
                }
            }
        };

        Some(ValidationCheck {
            tip: tip.block.clone(),
            tip_height: tip.height,
            height_difference: tip.height - candidate.height,
            stale_height: candidate.height,
            stale_timestamp: candidate.created_at,
            stale_branch_len,
            stale_branch_root,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct BlockArg {
    max_height: i64,
}

#[derive(Debug, Serialize)]
struct BlockResult {
    pub hash: String,
    pub height: i64,
    pub parent_hash: Option<String>,
    pub connected: bool,
    pub first_seen_by: i64,
    pub headers_only: bool,
    pub work: String,
    pub txids: Option<Vec<String>>,
    pub txids_added: Option<Vec<String>>,
    pub txids_omitted: Option<Vec<String>>,
    pub pool_name: Option<String>,
    #[serde(serialize_with = "serde_bigdecimal")]
    pub template_txs_fee_diff: Option<BigDecimal>,
    #[serde(serialize_with = "serde_bigdecimal")]
    pub tx_omitted_fee_rates: Option<BigDecimal>,
    #[serde(serialize_with = "serde_bigdecimal")]
    pub lowest_template_fee_rate: Option<BigDecimal>,
    #[serde(serialize_with = "serde_bigdecimal")]
    pub total_fee: Option<BigDecimal>,
    pub coinbase_message: Option<Vec<u8>>,
}

fn txid_bytes_to_hex(txids: Option<Vec<u8>>) -> Option<Vec<String>> {
    txids.map(|txs| {
        txs.chunks(32)
            .map(|chunk| chunk.encode_hex::<String>())
            .collect()
    })
}

impl BlockResult {
    pub fn from_block(block: Block) -> BlockResult {
        BlockResult {
            hash: block.hash,
            height: block.height,
            parent_hash: block.parent_hash,
            connected: block.connected,
            first_seen_by: block.first_seen_by,
            headers_only: block.headers_only,
            work: block.work,
            txids: txid_bytes_to_hex(block.txids),
            txids_added: txid_bytes_to_hex(block.txids_added),
            txids_omitted: txid_bytes_to_hex(block.txids_omitted),
            pool_name: block.pool_name,
            template_txs_fee_diff: block.template_txs_fee_diff,
            tx_omitted_fee_rates: block.tx_omitted_fee_rates,
            lowest_template_fee_rate: block.lowest_template_fee_rate,
            total_fee: block.total_fee,
            coinbase_message: block.coinbase_message,
        }
    }
}

fn validation_checks(conn: Conn, window: i64) -> Result<Value> {
    let tips = Chaintip::list_active(&conn);

    if tips.is_err() {
        return Err(JsonRpcError::internal_error());
    }

    let tips = tips.unwrap();

    let checks: Vec<ValidationCheck> = tips
        .into_iter()
        .flat_map(|tip| {
            let candidates =
                StaleCandidate::list_ge(&conn, tip.height - window).unwrap_or_default();
            candidates
                .into_iter()
                .flat_map(|candidate| {
                    let blocks = Block::get_at_height(&conn, candidate.height).unwrap_or_default();

                    blocks
                        .into_iter()
                        .filter_map(|b| {
                            ValidationCheck::new_candidate(&conn, &tip, &candidate, b.hash)
                        })
                        .collect::<Vec<ValidationCheck>>()
                })
                .collect::<Vec<ValidationCheck>>()
        })
        .collect();

    info!("{} stale candidates in window {}", checks.len(), window);
    match serde_json::to_value(checks) {
        Ok(t) => Ok(t),
        Err(_) => Err(JsonRpcError::internal_error()),
    }
}

// check if tx is in active tip
fn tx_is_active(conn: Conn, params: Params) -> Result<Value> {
    match params.parse::<TxId>() {
        Ok(id) => {
            let tips = Chaintip::list_active(&conn);

            if tips.is_err() {
                return Err(JsonRpcError::internal_error());
            }
            let tips: HashSet<_> = tips.unwrap().into_iter().map(|t| t.block).collect();

            match Transaction::tx_block_and_descendants(&conn, id.id) {
                Ok(blocks) => {
                    let is_active = blocks.into_iter().any(|b| tips.contains(&b.hash));

                    Ok(is_active.into())
                }
                Err(_) => Err(JsonRpcError::internal_error()),
            }
        }
        Err(args) => {
            let err = JsonRpcError::invalid_params(format!("Invalid parameters, {:?}", args));
            Err(err)
        }
    }
}

// update watched addresses
fn update_watched_addresses(conn: Conn, params: Params) -> Result<Value> {
    match params.parse::<WatchedAddressUpdate>() {
	    Ok(updates) => {
		    let WatchedAddressUpdate { remove, add } = updates;

		    if let Err(_) = Watched::remove(&conn, remove) {
                return Err(JsonRpcError::internal_error());
			};

            if let Err(_) = Watched::insert(&conn, add) {
                return Err(JsonRpcError::internal_error());
			}

		    Ok("OK".into())
		}
        Err(args) => {
            let err = JsonRpcError::invalid_params(format!("Invalid parameters, {:?}", args));
            Err(err)
        }
	}
}

fn submit_block(conn: Conn, params: Params) -> Result<Value> {
    match params.parse::<BlockUpload>() {
        Ok(upload) => match Node::get(&conn, upload.node_id) {
            Ok(node) => {
                let auth = Auth::UserPass(node.rpc_user.clone(), node.rpc_pass.clone());

                if let Ok(client) = Client::new(&node.rpc_host, auth) {
                    let hash = upload.block.block_hash();
                    let block_hex = serialize_hex(&upload.block);

                    match client.submit_block(block_hex, &hash) {
                        Ok(_) => Ok("OK".into()),
                        Err(e) => {
                            let errmsg = format!("Call to submit block failed. {:?}", e);
                            return Err(JsonRpcError::invalid_params(errmsg));
                        }
                    }
                } else {
                    let err = JsonRpcError::invalid_params(format!(
                        "Failed to establish a node connection."
                    ));
                    return Err(err);
                }
            }
            Err(e) => {
                let err = JsonRpcError::invalid_params(format!("Node not found, {:?}", e));
                Err(err)
            }
        },
        Err(args) => {
            let err = JsonRpcError::invalid_params(format!("Invalid parameters, {:?}", args));
            Err(err)
        }
    }
}

// get peer list for a node
fn get_peers(conn: Conn, params: Params) -> Result<Value> {
    match params.parse::<NodeId>() {
        Ok(id) => match Peer::list(&conn, id.id) {
            Ok(peers) => match serde_json::to_value(peers) {
                Ok(value) => Ok(value),
                Err(_) => Err(JsonRpcError::internal_error()),
            },
            Err(_) => Err(JsonRpcError::internal_error()),
        },
        Err(args) => {
            let err = JsonRpcError::invalid_params(format!("Invalid parameters, {:?}", args));
            Err(err)
        }
    }
}

// updated chaintip to the provided block
fn set_tip(conn: Conn, cmd: Sender<ScannerCommand>, params: Params) -> Result<Value> {
    match params.parse::<SetTipQuery>() {
        Ok(query) => {
            match Node::list(&conn) {
                Ok(nodes) => {
                    let n = nodes.iter().find(|n| n.id == query.node_id);

                    if n.is_none() {
                        let err = JsonRpcError::invalid_params(format!(
                            "Node not found: {:?}",
                            query.node_id
                        ));
                        return Err(err);
                    }
                }
                Err(e) => {
                    let err =
                        JsonRpcError::invalid_params(format!("Could not get node list, {:?}", e));
                    return Err(err);
                }
            }

            let c = ScannerCommand::SetTip {
                node_id: query.node_id,
                hash: query.hash,
            };
            cmd.send(c).expect("Command channel broke");
            Ok("success".into())
        }
        Err(e) => {
            let err = JsonRpcError::invalid_params(format!("Invalid parameters, {:?}", e));
            Err(err)
        }
    }
}

// get a block from a connected peer
fn get_block_from_peer(conn: Conn, params: Params) -> Result<Value> {
    match params.parse::<GetBlockFromPeer>() {
        Ok(query) => match Node::list(&conn) {
            Ok(nodes) => {
                let n = nodes.iter().find(|n| n.id == query.node_id);

                if n.is_none() {
                    let err = JsonRpcError::invalid_params(format!(
                        "Node not found: {:?}",
                        query.node_id
                    ));
                    return Err(err);
                }

                let node = n.unwrap();

                let auth = Auth::UserPass(node.rpc_user.clone(), node.rpc_pass.clone());
                if let Ok(client) = Client::new(&node.rpc_host, auth) {
                    let peer_id =
                        serde_json::Value::Number(serde_json::Number::from(query.peer_id));
                    let result = RpcApi::call::<serde_json::Value>(
                        &client,
                        "getblockfrompeer",
                        &[serde_json::Value::String(query.hash), peer_id],
                    );

                    match result {
                        Ok(result) => Ok(result),
                        Err(e) => {
                            let errmsg = format!("Call to get blocks failed. {:?}", e);
                            return Err(JsonRpcError::invalid_params(errmsg));
                        }
                    }
                } else {
                    let err = JsonRpcError::invalid_params(format!(
                        "Failed to establish a node connection."
                    ));
                    return Err(err);
                }
            }
            Err(e) => {
                let err = JsonRpcError::invalid_params(format!("Could not get node list, {:?}", e));
                return Err(err);
            }
        },
        Err(e) => {
            let err = JsonRpcError::invalid_params(format!("Invalid parameters, {:?}", e));
            Err(err)
        }
    }
}

fn get_block(conn: Conn, params: Params) -> Result<Value> {
    match params.parse::<BlockQuery>() {
        Ok(q) => match q {
            BlockQuery::Height(h) => {
                if let Ok(result) = Block::get_at_height(&conn, h) {
                    let result: Vec<_> = result
                        .into_iter()
                        .map(|block| BlockResult::from_block(block))
                        .collect();

                    match serde_json::to_value(result) {
                        Ok(s) => Ok(s),
                        Err(_) => Err(JsonRpcError::internal_error()),
                    }
                } else {
                    Err(JsonRpcError::internal_error())
                }
            }
            BlockQuery::Hash(h) => {
                if let Ok(result) = Block::get(&conn, &h) {
                    let result = BlockResult::from_block(result);

                    match serde_json::to_value(vec![result]) {
                        Ok(s) => Ok(s),
                        Err(_) => Err(JsonRpcError::internal_error()),
                    }
                } else {
                    Err(JsonRpcError::internal_error())
                }
            }
        },
        Err(args) => {
            let err = JsonRpcError::invalid_params(format!("Invalid parameters, {:?}", args));
            Err(err)
        }
    }
}

// add a new node to forkscanner
fn add_node(conn: Conn, params: Params) -> Result<Value> {
    match params.parse::<NodeArgs>() {
        Ok(args) => {
            if let Ok(n) = Node::insert(
                &conn,
                args.name,
                args.rpc_host,
                args.rpc_port,
                args.mirror_rpc_port,
                args.user,
                args.pass,
                args.mirror_host,
                args.archive,
            ) {
                Ok(n.id.into())
            } else {
                Err(JsonRpcError::internal_error())
            }
        }
        Err(args) => {
            let err = JsonRpcError::invalid_params(format!("Invalid parameters, {:?}", args));
            Err(err)
        }
    }
}

// get nodes from database
fn get_nodes(conn: Conn, params: Params) -> Result<Value> {
	if params != Params::None {
		let err = JsonRpcError::invalid_params(format!("Unexpected parameters to get_nodes."));
		Err(err)
	} else {
		if let Ok(nodes) = Node::list(&conn) {
		    let nodes: Vec<_> = nodes.into_iter().map(|mut n| {
			    n.rpc_pass = "xxx".to_string();
				n
			}).collect();

		    match serde_json::to_value(&nodes) {
			    Ok(n) => Ok(n),
				Err(_) => Err(JsonRpcError::internal_error()),
			}
		} else {
			Err(JsonRpcError::internal_error())
		}
	}
}

// remove node from database
fn remove_node(conn: Conn, params: Params) -> Result<Value> {
    match params.parse::<NodeId>() {
        Ok(id) => {
            if let Ok(_) = Node::remove(&conn, id.id) {
                Ok("OK".into())
            } else {
                Err(JsonRpcError::internal_error())
            }
        }
        Err(args) => {
            let err = JsonRpcError::invalid_params(format!("Invalid parameters, {:?}", args));
            Err(err)
        }
    }
}

// fetch currently active chaintips
fn get_tips(params: Params, conn: Conn) -> Result<Value> {
    match params.parse::<TipArgs>() {
        Ok(t) => {
            let chaintips = if t.active_only {
                Chaintip::list_active(&conn)
            } else {
                Chaintip::list(&conn)
            };

            if let Err(e) = chaintips {
                let err =
                    JsonRpcError::invalid_params(format!("Couldn't fetch chaintips, {:?}", e));
                return Err(err);
            }

            match serde_json::to_value(chaintips.unwrap()) {
                Ok(t) => Ok(t),
                Err(_) => Err(JsonRpcError::internal_error()),
            }
        }
        Err(args) => {
            let err = JsonRpcError::invalid_params(format!("Invalid parameters, {:?}", args));
            Err(err)
        }
    }
}

// validation endpoint subscription handler
fn handle_validation_subscribe(
    exit: Arc<AtomicBool>,
    receiver: Receiver<ScannerMessage>,
    pool: ManagedPool,
    window: i64,
    sink: Sink,
) {
    info!("New subscription");
    let send_update = move |pool: &ManagedPool, sink: &Sink| -> std::result::Result<(), WsError> {
        let conn = pool.get()?;
        match validation_checks(conn, window) {
            Ok(resp) => Ok(sink.notify(Params::Array(vec![resp]))?),
            Err(_) => Ok(sink.notify(Params::Array(vec![
                "Failed to update validation checks".into()
            ]))?),
        }
    };

    thread::spawn(move || {
        if let Err(e) = send_update(&pool, &sink) {
            error!(
                "Error sending validation checks to initialize client {:?}",
                e
            );
        }

        loop {
            if exit.load(Ordering::SeqCst) {
                break;
            }

            match receiver.recv_timeout(time::Duration::from_millis(5000)) {
                Ok(ScannerMessage::StaleCandidateUpdate) => {
                    if let Err(e) = send_update(&pool, &sink) {
                        error!("Error sending chaintips to client {:?}", e);
                    }
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    info!("No chaintip updates");
                }
                Err(e) => {
                    error!("Error! {:?}", e);
                }
            }
        }
    });
}

// Notify of watched address activity
fn handle_watched_addresses(
    exit: Arc<AtomicBool>,
    receiver: Receiver<ScannerMessage>,
    watch: Vec<String>,
    watch_until: DateTime<Utc>,
    pool: ManagedPool,
    sink: Sink,
) {
    let conn = pool.get().expect("Connection pool failure");

    let watches: Vec<_> = watch.into_iter().map(|w| (w, watch_until.clone())).collect();
    Watched::insert(&conn, watches).expect("Could not insert watchlist!");

    info!("New address activity");
    let send_update =
        move |transactions: Vec<Transaction>, sink: &Sink| -> std::result::Result<(), WsError> {
            let resp = transactions
                .into_iter()
                .map(|tx| serde_json::to_value(tx).expect("Could not serialize transaction"))
                .collect();
            Ok(sink.notify(Params::Array(resp))?)
        };

    thread::spawn(move || loop {
        if exit.load(Ordering::SeqCst) {
            break;
        }

        match receiver.recv_timeout(time::Duration::from_millis(5000)) {
            Ok(ScannerMessage::WatchedAddress(transactions)) => {
                if let Err(e) = send_update(transactions, &sink) {
                    error!("Error sending watched activity to client {:?}", e);
                }
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                info!("No lagging node updates");
            }
            Err(e) => {
                error!("Error! {:?}", e);
            }
        }
    });
}

// Notify of lagging nodes
fn handle_lagging_nodes_subscribe(
    exit: Arc<AtomicBool>,
    receiver: Receiver<ScannerMessage>,
    sink: Sink,
) {
    info!("New subscription");
    let send_update = move |lags: Vec<Lags>, sink: &Sink| -> std::result::Result<(), WsError> {
        let resp = lags
            .into_iter()
            .map(|conf| serde_json::to_value(conf).expect("Could not serialize lagging node"))
            .collect();
        Ok(sink.notify(Params::Array(resp))?)
    };

    thread::spawn(move || loop {
        if exit.load(Ordering::SeqCst) {
            break;
        }

        match receiver.recv_timeout(time::Duration::from_millis(5000)) {
            Ok(ScannerMessage::LaggingNodes(lags)) => {
                if let Err(e) = send_update(lags, &sink) {
                    error!("Error sending lagging nodes to client {:?}", e);
                }
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                info!("No lagging node updates");
            }
            Err(e) => {
                error!("Error! {:?}", e);
            }
        }
    });
}

// invalid block endpoint subscription handler
fn handle_invalid_block_subscribe(
    exit: Arc<AtomicBool>,
    receiver: Receiver<ScannerMessage>,
    sink: Sink,
) {
    info!("New subscription");
    let send_update = move |blocks: Vec<ConflictingBlock>,
                            sink: &Sink|
          -> std::result::Result<(), WsError> {
        let resp = blocks
            .into_iter()
            .map(|conf| serde_json::to_value(conf).expect("Could not serialize conflicting block"))
            .collect();
        Ok(sink.notify(Params::Array(resp))?)
    };

    thread::spawn(move || loop {
        if exit.load(Ordering::SeqCst) {
            break;
        }

        match receiver.recv_timeout(time::Duration::from_millis(5000)) {
            Ok(ScannerMessage::NewBlockConflicts(conflicts)) => {
                if let Err(e) = send_update(conflicts, &sink) {
                    error!("Error sending block conflicts to client {:?}", e);
                }
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                info!("No block conflict updates");
            }
            Err(e) => {
                error!("Error! {:?}", e);
            }
        }
    });
}

fn handle_subscribe_forks(
    exit: Arc<AtomicBool>,
    pool: ManagedPool,
    receiver: Receiver<ScannerMessage>,
    _: Params,
    sink: Sink,
) {
    info!("New subscription");
    fn send_update(tips: Vec<Chaintip>, sink: &Sink) -> std::result::Result<(), WsError> {
        let tips: Vec<_> = tips
            .into_iter()
            .map(|tip| serde_json::to_value(tip).expect("JSON serde failed"))
            .collect();

        Ok(sink.notify(Params::Array(tips))?)
    }

    thread::spawn(move || {
        let conn = pool.get().expect("Could not get pooled connection!");
        match Chaintip::list_active(&conn) {
            Ok(tips) => {
                if let Err(e) = send_update(tips, &sink) {
                    error!("Error sending chaintips to initialize client {:?}", e);
                }
            }
            Err(e) => {
                error!("Database error {:?}", e);
            }
        }

        loop {
            if exit.load(Ordering::SeqCst) {
                break;
            }

            match receiver.recv_timeout(time::Duration::from_millis(5000)) {
                Ok(ScannerMessage::NewChaintip) => {
                    let conn = pool.get().expect("Could not get pooled connection!");
                    let tips = match Chaintip::list_active(&conn) {
                        Ok(tips) => tips,
                        Err(e) => {
                            error!("Database error {:?}", e);
                            continue;
                        }
                    };

                    if let Err(e) = send_update(tips, &sink) {
                        error!("Error sending chaintips to client {:?}", e);
                    }
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    info!("No chaintip updates");
                }
                Err(e) => {
                    error!("Error! {:?}", e);
                }
            }
        }
    });
}

// handles subscriptions for chaintip updates
fn handle_subscribe(
    exit: Arc<AtomicBool>,
    receiver: Receiver<ScannerMessage>,
    tips: Arc<RwLock<Vec<Chaintip>>>,
    _: Params,
    sink: Sink,
) {
    info!("New subscription");
    fn send_update(
        tips: &Arc<RwLock<Vec<Chaintip>>>,
        sink: &Sink,
    ) -> std::result::Result<(), WsError> {
        let values = tips.read().expect("Lock poisoned").clone();
        let tips: Vec<_> = values
            .into_iter()
            .map(|tip| serde_json::to_value(tip).expect("JSON serde failed"))
            .collect();

        Ok(sink.notify(Params::Array(tips))?)
    }

    thread::spawn(move || {
        if let Err(e) = send_update(&tips, &sink) {
            error!("Error sending chaintips to initialize client {:?}", e);
        }

        loop {
            if exit.load(Ordering::SeqCst) {
                break;
            }

            match receiver.recv_timeout(time::Duration::from_millis(5000)) {
                Ok(ScannerMessage::NewChaintip) => {
                    if let Err(e) = send_update(&tips, &sink) {
                        error!("Error sending chaintips to client {:?}", e);
                    }
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    info!("No chaintip updates");
                }
                Err(e) => {
                    error!("Error! {:?}", e);
                }
            }
        }
    });
}

fn session_meta(context: &wss::RequestContext) -> Option<Arc<Session>> {
    debug!("Request context {:#?}", context);
    Some(Arc::new(Session::new(context.sender())))
}

/// RPC service endpoints for users of forkscanner.
pub fn run_server(
    listen: String,
    rpc: u16,
    subs: u16,
    db_url: String,
    receiver: Receiver<ScannerMessage>,
    command: Sender<ScannerCommand>,
) {
    let manager = ConnectionManager::<PgConnection>::new(db_url);
    let tips = Arc::new(RwLock::new(vec![]));
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Connection pool");
    let pool2 = pool.clone();

    let tips1 = tips.clone();
    let l1 = listen.clone();

    // set up some rpc endpoints
    let t1 = thread::spawn(move || {
        let mut io = IoHandler::new();
        let p = pool.clone();
        io.add_sync_method("get_tips", move |params: Params| {
            let conn = p.get().unwrap();
            get_tips(params, conn)
        });

        let p = pool.clone();
        io.add_sync_method("add_node", move |params: Params| {
            let conn = p.get().unwrap();
            add_node(conn, params)
        });

        let p = pool.clone();
        io.add_sync_method("remove_node", move |params: Params| {
            let conn = p.get().unwrap();
            remove_node(conn, params)
        });

        let p = pool.clone();
        io.add_sync_method("get_nodes", move |params: Params| {
            let conn = p.get().unwrap();
            get_nodes(conn, params)
        });

        let p = pool.clone();
        io.add_sync_method("get_block", move |params: Params| {
            let conn = p.get().unwrap();
            get_block(conn, params)
        });

        let p = pool.clone();
        io.add_sync_method("get_block_from_peer", move |params: Params| {
            let conn = p.get().unwrap();
            get_block_from_peer(conn, params)
        });

        let p = pool.clone();
        let cmd = command.clone();
        io.add_sync_method("set_tip", move |params: Params| {
            let conn = p.get().unwrap();
            let c = cmd.clone();
            set_tip(conn, c, params)
        });

        let p = pool.clone();
        io.add_sync_method("tx_is_active", move |params: Params| {
            let conn = p.get().unwrap();
            tx_is_active(conn, params)
        });

        let p = pool.clone();
        io.add_sync_method("get_peers", move |params: Params| {
            let conn = p.get().unwrap();
            get_peers(conn, params)
        });

        let p = pool.clone();
        io.add_sync_method("submit_block", move |params: Params| {
            let conn = p.get().unwrap();
            submit_block(conn, params)
        });

        let p = pool.clone();
        io.add_sync_method("update_watched_addresses", move |params: Params| {
            let conn = p.get().unwrap();
            update_watched_addresses(conn, params)
        });

        let server = hts::ServerBuilder::new(io)
            .start_http(&SocketAddr::from((l1.parse::<IpAddr>().unwrap(), rpc)))
            .expect("Failed to start RPC server");

        server.wait();
    });

    let subscriptions = Arc::new(Mutex::new(
        HashMap::<&str, Vec<Sender<ScannerMessage>>>::default(),
    ));
    let subscriptions2 = subscriptions.clone();
    let subscriptions3 = subscriptions.clone();
    let subscriptions4 = subscriptions.clone();
    let subscriptions5 = subscriptions.clone();
    // listener thread for notifications from forkscanner
    let t2 = thread::spawn(move || loop {
        match receiver.recv() {
            Ok(ScannerMessage::NewChaintip) => {
                debug!("New chaintip updates");
                if let Some(subs) = subscriptions2
                    .lock()
                    .expect("Lock poisoned")
                    .get_mut("active_fork")
                {
                    subs.retain(|sub| sub.send(ScannerMessage::NewChaintip).is_ok());
                }
                if let Some(subs) = subscriptions2
                    .lock()
                    .expect("Lock poisoned")
                    .get_mut("forks")
                {
                    subs.retain(|sub| sub.send(ScannerMessage::NewChaintip).is_ok());
                }
            }
            Ok(ScannerMessage::LaggingNodes(lags)) => {
                debug!("New lagging nodes updates");
                if let Some(subs) = subscriptions2
                    .lock()
                    .expect("Lock poisoned")
                    .get_mut("lagging_nodes")
                {
                    subs.retain(|sub| sub.send(ScannerMessage::LaggingNodes(lags.clone())).is_ok());
                }
            }
            Ok(ScannerMessage::NewBlockConflicts(conflicts)) => {
                debug!("New block conflict updates");
                if let Some(subs) = subscriptions2
                    .lock()
                    .expect("Lock poisoned")
                    .get_mut("invalid_block_checks")
                {
                    subs.retain(|sub| {
                        sub.send(ScannerMessage::NewBlockConflicts(conflicts.clone()))
                            .is_ok()
                    });
                }
            }
            Ok(ScannerMessage::TipUpdated(invalidated_hashes)) => {
                debug!("New chaintip updates");
                if let Some(subs) = subscriptions2
                    .lock()
                    .expect("Lock poisoned")
                    .get_mut("active_fork")
                {
                    subs.retain(|sub| {
                        sub.send(ScannerMessage::TipUpdated(invalidated_hashes.clone()))
                            .is_ok()
                    });
                }
            }
            Ok(ScannerMessage::TipUpdateFailed(err)) => {
                debug!("New chaintip updates");
                if let Some(subs) = subscriptions2
                    .lock()
                    .expect("Lock poisoned")
                    .get_mut("active_fork")
                {
                    subs.retain(|sub| {
                        sub.send(ScannerMessage::TipUpdateFailed(err.clone()))
                            .is_ok()
                    });
                }
            }
            Ok(ScannerMessage::WatchedAddress(txs)) => {
                debug!("New watched address activity");
                if let Some(subs) = subscriptions2
                    .lock()
                    .expect("Lock poisoned")
                    .get_mut("watched_addresses")
                {
                    debug!(
                        "New watched address activity: updating {} subscriptions",
                        subs.len()
                    );
                    subs.retain(|sub| {
                        sub.send(ScannerMessage::WatchedAddress(txs.clone()))
                            .is_ok()
                    });
                }
            }
            Ok(ScannerMessage::StaleCandidateUpdate) => {
                debug!("New stale candidate updates");
                if let Some(subs) = subscriptions2
                    .lock()
                    .expect("Lock poisoned")
                    .get_mut("validation_checks")
                {
                    debug!(
                        "New stale candidates: updating {} subscriptions",
                        subs.len()
                    );
					subs.retain(|sub| sub.send(ScannerMessage::StaleCandidateUpdate).is_ok());
                }
            }
            Ok(ScannerMessage::AllChaintips(mut t)) => {
                debug!("New chaintips {:?}", t);
                std::mem::swap(&mut t, &mut tips.write().expect("Lock poisoned"));
                if let Some(subs) = subscriptions2
                    .lock()
                    .expect("Lock poisoned")
                    .get_mut("active_fork")
                {
                    subs.retain(|sub| sub.send(ScannerMessage::NewChaintip).is_ok());
                }
            }
            Err(e) => {
                error!("Channel broke {:?}", e);
                break;
            }
        }
    });

    let t3 = thread::spawn(move || {
        let killers = Arc::new(Mutex::new(
            HashMap::<SubscriptionId, Arc<AtomicBool>>::default(),
        ));

        let mut io = PubSubHandler::new(MetaIoHandler::default());
        io.add_sync_method("ping", |_: Params| Ok(Value::String("pong".into())));

        let killer_clone1 = killers.clone();
        let killer_clone2 = killers.clone();
        let killer_clone3 = killers.clone();
        let killer_clone4 = killers.clone();
        let killer_clone5 = killers.clone();
        let killer_clone6 = killers.clone();
        let killer_clone7 = killers.clone();
        let killer_clone8 = killers.clone();
        let killer_clone9 = killers.clone();
        let killer_clone10 = killers.clone();
        let killer_clone11 = killers.clone();
        let pool3 = pool2.clone();
        let pool4 = pool2.clone();
        let pool5 = pool2.clone();
        let subscriptions2 = subscriptions.clone();
        let subscriptions6 = subscriptions.clone();
        // ws subscription endpoint for fork notifications
        io.add_subscription(
            "active_fork",
            (
                "subscribe_active_fork",
                move |params: Params, _, subscriber: Subscriber| {
                    info!("Subscribe to active fork");
                    let mut rng = rand::rngs::OsRng::default();
                    if params != Params::None {
                        subscriber
                            .reject(Error {
                                code: ErrorCode::ParseError,
                                message: "Invalid parameters. Subscription rejected.".into(),
                                data: None,
                            })
                            .unwrap();
                        return;
                    }

                    let kill_switch = Arc::new(AtomicBool::new(false));
                    let sub_id = SubscriptionId::Number(rng.gen());
                    let sink = subscriber.assign_id(sub_id.clone()).unwrap();
                    killer_clone1
                        .lock()
                        .expect("Lock poisoned")
                        .insert(sub_id, kill_switch.clone());
                    let (notify_tx, notify_rx) = unbounded();
                    {
                        let mut sub_lock = subscriptions.lock().expect("Lock poisoned");
                        sub_lock
                            .entry("active_fork")
                            .or_insert(vec![])
                            .push(notify_tx);
                    }

                    handle_subscribe(kill_switch, notify_rx, tips1.clone(), params, sink)
                },
            ),
            ("unsubscribe_active_fork", move |id: SubscriptionId, _| {
                if let Some(arc) = killer_clone2.lock().expect("Lock poisoned").remove(&id) {
                    arc.store(true, Ordering::SeqCst);
                }
                Box::pin(futures::future::ok(Value::Bool(true)))
            }),
        );

        // ws subscription endpoint for fork notifications
        io.add_subscription(
            "forks",
            (
                "subscribe_forks",
                move |params: Params, _, subscriber: Subscriber| {
                    info!("Subscribe to forks");
                    let mut rng = rand::rngs::OsRng::default();
                    if params != Params::None {
                        subscriber
                            .reject(Error {
                                code: ErrorCode::ParseError,
                                message: "Invalid parameters. Subscription rejected.".into(),
                                data: None,
                            })
                            .unwrap();
                        return;
                    }

                    let kill_switch = Arc::new(AtomicBool::new(false));
                    let sub_id = SubscriptionId::Number(rng.gen());
                    let sink = subscriber.assign_id(sub_id.clone()).unwrap();
                    killer_clone10
                        .lock()
                        .expect("Lock poisoned")
                        .insert(sub_id, kill_switch.clone());
                    let (notify_tx, notify_rx) = unbounded();
                    {
                        let mut sub_lock = subscriptions6.lock().expect("Lock poisoned");
                        sub_lock.entry("forks").or_insert(vec![]).push(notify_tx);
                    }

                    handle_subscribe_forks(kill_switch, pool5.clone(), notify_rx, params, sink)
                },
            ),
            ("unsubscribe_forks", move |id: SubscriptionId, _| {
                if let Some(arc) = killer_clone11.lock().expect("Lock poisoned").remove(&id) {
                    arc.store(true, Ordering::SeqCst);
                }
                Box::pin(futures::future::ok(Value::Bool(true)))
            }),
        );
        // subscription endpoint for giving diff between tip height and stale block heights
        io.add_subscription(
            "validation_checks",
            (
                "validation_checks",
                move |params: Params, _, subscriber: Subscriber| {
                    info!("Subscribe to validation checks");
                    let mut rng = rand::rngs::OsRng::default();

                    let block_window = if let Params::None = params {
                        BLOCK_WINDOW
                    } else {
                        let BlockArg { max_height } = if let Ok(parm) = params.parse() {
                            parm
                        } else {
                            subscriber
                                .reject(Error {
                                    code: ErrorCode::ParseError,
                                    message:
                                        "Invalid parameters. Expected None, or max_height: i64"
                                            .into(),
                                    data: None,
                                })
                                .unwrap();
                            return;
                        };
                        max_height
                    };

                    let kill_switch = Arc::new(AtomicBool::new(false));
                    let sub_id = SubscriptionId::Number(rng.gen());
                    let sink = subscriber.assign_id(sub_id.clone()).unwrap();
                    killer_clone3
                        .lock()
                        .expect("Lock poisoned")
                        .insert(sub_id, kill_switch.clone());
                    let (notify_tx, notify_rx) = unbounded();
                    {
                        let mut sub_lock = subscriptions2.lock().expect("Lock poisoned");
                        sub_lock
                            .entry("validation_checks")
                            .or_insert(vec![])
                            .push(notify_tx);
                    }

                    handle_validation_subscribe(
                        kill_switch,
                        notify_rx,
                        pool3.clone(),
                        block_window,
                        sink,
                    )
                },
            ),
            (
                "unsubscribe_validation_checks",
                move |id: SubscriptionId, _| {
                    if let Some(arc) = killer_clone4.lock().expect("Lock poisoned").remove(&id) {
                        arc.store(true, Ordering::SeqCst);
                    }
                    Box::pin(futures::future::ok(Value::Bool(true)))
                },
            ),
        );

        io.add_subscription(
            "invalid_block_checks",
            (
                "invalid_block_checks",
                move |params: Params, _, subscriber: Subscriber| {
                    info!("Subscribe to invalid block checks");
                    let mut rng = rand::rngs::OsRng::default();

                    if params != Params::None {
                        subscriber
                            .reject(Error {
                                code: ErrorCode::ParseError,
                                message: "Invalid parameters. Subscription rejected.".into(),
                                data: None,
                            })
                            .unwrap();
                        return;
                    }

                    let kill_switch = Arc::new(AtomicBool::new(false));
                    let sub_id = SubscriptionId::Number(rng.gen());
                    let sink = subscriber.assign_id(sub_id.clone()).unwrap();
                    killers
                        .lock()
                        .expect("Lock poisoned")
                        .insert(sub_id, kill_switch.clone());
                    let (notify_tx, notify_rx) = unbounded();
                    {
                        let mut sub_lock = subscriptions3.lock().expect("Lock poisoned");
                        sub_lock
                            .entry("invalid_block_checks")
                            .or_insert(vec![])
                            .push(notify_tx);
                    }

                    handle_invalid_block_subscribe(kill_switch, notify_rx, sink)
                },
            ),
            (
                "unsubscribe_invalid_block_checks",
                move |id: SubscriptionId, _| {
                    if let Some(arc) = killer_clone5.lock().expect("Lock poisoned").remove(&id) {
                        arc.store(true, Ordering::SeqCst);
                    }
                    Box::pin(futures::future::ok(Value::Bool(true)))
                },
            ),
        );

        io.add_subscription(
            "lagging_nodes_checks",
            (
                "lagging_nodes_checks",
                move |params: Params, _, subscriber: Subscriber| {
                    info!("Subscribe to lagging nodes checks");
                    let mut rng = rand::rngs::OsRng::default();

                    if params != Params::None {
                        subscriber
                            .reject(Error {
                                code: ErrorCode::ParseError,
                                message: "Invalid parameters. Subscription rejected.".into(),
                                data: None,
                            })
                            .unwrap();
                        return;
                    }

                    let kill_switch = Arc::new(AtomicBool::new(false));
                    let sub_id = SubscriptionId::Number(rng.gen());
                    let sink = subscriber.assign_id(sub_id.clone()).unwrap();
                    killer_clone6
                        .lock()
                        .expect("Lock poisoned")
                        .insert(sub_id, kill_switch.clone());
                    let (notify_tx, notify_rx) = unbounded();
                    {
                        let mut sub_lock = subscriptions4.lock().expect("Lock poisoned");
                        sub_lock
                            .entry("lagging_nodes")
                            .or_insert(vec![])
                            .push(notify_tx);
                    }

                    handle_lagging_nodes_subscribe(kill_switch, notify_rx, sink)
                },
            ),
            (
                "unsubscribe_lagging_nodes_checks",
                move |id: SubscriptionId, _| {
                    if let Some(arc) = killer_clone7.lock().expect("Lock poisoned").remove(&id) {
                        arc.store(true, Ordering::SeqCst);
                    }
                    Box::pin(futures::future::ok(Value::Bool(true)))
                },
            ),
        );

        io.add_subscription(
            "watched_address_checks",
            (
                "watched_address_checks",
                move |params: Params, _, subscriber: Subscriber| {
                    info!("Subscribe to watched address checks");
                    let mut rng = rand::rngs::OsRng::default();

                    let WatchAddress { watch, watch_until } = match params.parse() {
					    Ok(parm) => parm,
						Err(e) => {
							subscriber
								.reject(Error {
									code: ErrorCode::ParseError,
									message: format!("Invalid parameters. Expected list of addresses to watch. {:?}", e)
										.into(),
									data: None,
								})
								.unwrap();
							return;
						}
                    };

                    let kill_switch = Arc::new(AtomicBool::new(false));
                    let sub_id = SubscriptionId::Number(rng.gen());
                    let sink = subscriber.assign_id(sub_id.clone()).unwrap();
                    killer_clone8
                        .lock()
                        .expect("Lock poisoned")
                        .insert(sub_id, kill_switch.clone());
                    let (notify_tx, notify_rx) = unbounded();
                    {
                        let mut sub_lock = subscriptions5.lock().expect("Lock poisoned");
                        sub_lock
                            .entry("watched_addresses")
                            .or_insert(vec![])
                            .push(notify_tx);
                    }

                    handle_watched_addresses(
                        kill_switch,
                        notify_rx,
                        watch,
                        watch_until,
                        pool4.clone(),
                        sink,
                    )
                },
            ),
            (
                "unsubscribe_watched_address_checks",
                move |id: SubscriptionId, _| {
                    if let Some(arc) = killer_clone9.lock().expect("Lock poisoned").remove(&id) {
                        arc.store(true, Ordering::SeqCst);
                    }
                    Box::pin(futures::future::ok(Value::Bool(true)))
                },
            ),
        );

        info!("Coming up on {} {}", listen, subs);
        let server = wss::ServerBuilder::with_meta_extractor(io, session_meta)
            .start(&SocketAddr::from((listen.parse::<IpAddr>().unwrap(), subs)))
            .expect("Failed to start sub server");

        server.wait().expect("WS server crashed");
        info!("WS service is exiting");
    });

    t1.join().expect("Thread join");
    t2.join().expect("Thread join");
    t3.join().expect("Thread join");
}
