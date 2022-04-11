use crate::{Block, Chaintip, Node, ScannerMessage, Transaction};
use crossbeam::channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use diesel::prelude::PgConnection;
use jsonrpc_core::types::error::Error as JsonRpcError;
use jsonrpc_core::*;
use jsonrpc_http_server as hts;
use jsonrpc_pubsub::{PubSubHandler, Session, Sink, Subscriber, SubscriptionId};
use jsonrpc_ws_server as wss;
use log::{debug, error, info};
use r2d2::PooledConnection;
use r2d2_diesel::ConnectionManager;
use rand::Rng;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex, RwLock},
    thread, time,
};
use thiserror::Error;

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

fn get_block(conn: Conn, params: Params) -> Result<Value> {
    match params.parse::<BlockQuery>() {
        Ok(q) => match q {
            BlockQuery::Height(h) => {
                if let Ok(result) = Block::get_at_height(&conn, h) {
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
) {
    let manager = ConnectionManager::<PgConnection>::new(db_url);
    let tips = Arc::new(RwLock::new(vec![]));
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Connection pool");
    let pool2 = pool.clone();

    let tips1 = tips.clone();
    let l1 = listen.clone();
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

        let killer_clone = killers.clone();
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
                    killers
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
                if let Some(arc) = killer_clone.lock().expect("Lock poisoned").remove(&id) {
                    arc.store(true, Ordering::SeqCst);
                }
                Box::pin(futures::future::ok(Value::Bool(true)))
            }),
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
