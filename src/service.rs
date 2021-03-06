use crate::{
    serde_bigdecimal, Block, Chaintip, Node, ScannerCommand, ScannerMessage, StaleCandidate,
    Transaction,
};
use bigdecimal::BigDecimal;
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

    debug!("{} stale candidates in window {}", checks.len(), window);
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
fn get_tips(params: Params, tips: Vec<Chaintip>) -> Result<Value> {
    match params.parse::<TipArgs>() {
        Ok(t) => {
            let chaintips = if t.active_only {
                tips.iter()
                    .filter(|t| t.status == "active")
                    .cloned()
                    .collect::<Vec<Chaintip>>()
            } else {
                tips.to_vec()
            };

            match serde_json::to_value(chaintips) {
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

// handles subscriptions for chaintip updates
fn handle_subscribe(
    exit: Arc<AtomicBool>,
    receiver: Receiver<ScannerMessage>,
    pool: ManagedPool,
    _: Params,
    sink: Sink,
) {
    info!("New subscription");
    fn send_update(pool: &ManagedPool, sink: &Sink) -> std::result::Result<(), WsError> {
        let conn = pool.get()?;
        let values = Chaintip::list(&conn)?;
        let tips = serde_json::to_value(values)?;

        Ok(sink.notify(Params::Array(vec![tips]))?)
    }

    thread::spawn(move || {
        if let Err(e) = send_update(&pool, &sink) {
            error!("Error sending chaintips to initialize client {:?}", e);
        }

        loop {
            if exit.load(Ordering::SeqCst) {
                break;
            }

            match receiver.recv_timeout(time::Duration::from_millis(5000)) {
                Ok(ScannerMessage::NewChaintip) => {
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
        io.add_sync_method("get_tips", move |params: Params| {
            get_tips(params, tips1.read().expect("RwLock failed").clone())
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

        let server = hts::ServerBuilder::new(io)
            .start_http(&SocketAddr::from((l1.parse::<IpAddr>().unwrap(), rpc)))
            .expect("Failed to start RPC server");

        server.wait();
    });

    let subscriptions = Arc::new(Mutex::new(
        HashMap::<&str, Vec<Sender<ScannerMessage>>>::default(),
    ));
    let subscriptions2 = subscriptions.clone();
    // listener thread for notifications from forkscanner
    let t2 = thread::spawn(move || loop {
        match receiver.recv() {
            Ok(ScannerMessage::NewChaintip) => {
                debug!("New chaintip updates");
                if let Some(subs) = subscriptions2.lock().expect("Lock poisoned").get("forks") {
                    for sub in subs {
                        sub.send(ScannerMessage::NewChaintip)
                            .expect("Channel broke");
                    }
                }
            }
            Ok(ScannerMessage::TipUpdated(invalidated_hashes)) => {
                debug!("New chaintip updates");
                if let Some(subs) = subscriptions2.lock().expect("Lock poisoned").get("forks") {
                    for sub in subs {
                        sub.send(ScannerMessage::TipUpdated(invalidated_hashes.clone()))
                            .expect("Channel broke");
                    }
                }
            }
            Ok(ScannerMessage::TipUpdateFailed(err)) => {
                debug!("New chaintip updates");
                if let Some(subs) = subscriptions2.lock().expect("Lock poisoned").get("forks") {
                    for sub in subs {
                        sub.send(ScannerMessage::TipUpdateFailed(err.clone()))
                            .expect("Channel broke");
                    }
                }
            }
            Ok(ScannerMessage::StaleCandidateUpdate) => {
                debug!("New stale candidate updates");
                if let Some(subs) = subscriptions2
                    .lock()
                    .expect("Lock poisoned")
                    .get("validation_checks")
                {
                    debug!(
                        "New stale candidates: updating {} subscriptions",
                        subs.len()
                    );
                    for sub in subs {
                        sub.send(ScannerMessage::StaleCandidateUpdate)
                            .expect("Channel broke");
                    }
                }
            }
            Ok(ScannerMessage::AllChaintips(mut t)) => {
                debug!("New chaintips {:?}", t);
                std::mem::swap(&mut t, &mut tips.write().expect("Lock poisoned"));
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
        let pool3 = pool2.clone();
        let subscriptions2 = subscriptions.clone();
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
                    killer_clone1
                        .lock()
                        .expect("Lock poisoned")
                        .insert(sub_id, kill_switch.clone());
                    let (notify_tx, notify_rx) = unbounded();
                    {
                        let mut sub_lock = subscriptions.lock().expect("Lock poisoned");
                        sub_lock.entry("forks").or_insert(vec![]).push(notify_tx);
                    }

                    handle_subscribe(kill_switch, notify_rx, pool2.clone(), params, sink)
                },
            ),
            ("unsubscribe_forks", move |id: SubscriptionId, _| {
                if let Some(arc) = killer_clone2.lock().expect("Lock poisoned").remove(&id) {
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
                    killers
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
                    if let Some(arc) = killer_clone3.lock().expect("Lock poisoned").remove(&id) {
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
