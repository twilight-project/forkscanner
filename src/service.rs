use crate::{Block, Chaintip, Node, Transaction};
use diesel::prelude::PgConnection;
use jsonrpc_core::types::error::Error as JsonRpcError;
use jsonrpc_core::*;
use jsonrpc_http_server::*;
use r2d2::PooledConnection;
use r2d2_diesel::ConnectionManager;
use serde::Deserialize;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::str::FromStr;

type Conn = PooledConnection<ConnectionManager<diesel::PgConnection>>;

#[derive(Debug, Deserialize)]
struct NodeId {
    id: i64,
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

fn get_active_tips(conn: Conn, _params: Params) -> Result<Value> {
    if let Ok(tips) = Chaintip::list_active(&conn) {
        Ok(serde_json::Value::String(
            serde_json::to_string(&tips).unwrap(),
        ))
    } else {
        Err(JsonRpcError::internal_error())
    }
}

/// RPC service endpoints for users of forkscanner.
pub fn run_server(listen: &str, db_url: &str) {
    let manager = ConnectionManager::<PgConnection>::new(db_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Connection pool");

    let mut io = IoHandler::new();
    let p = pool.clone();
    io.add_sync_method("get_active_tips", move |params: Params| {
        let conn = p.get().unwrap();
        get_active_tips(conn, params)
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

    let server = ServerBuilder::new(io)
        .start_http(&SocketAddr::from_str(listen).unwrap())
        .expect("Failed to start RPC server");

    server.wait();
}
