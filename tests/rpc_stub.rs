use bitcoincore_rpc::Auth;
use crossbeam_channel::unbounded;
use diesel::prelude::PgConnection;
use diesel::Connection;
use forkscanner::{ForkScanner, ReorgMessage};
use jsonrpc_core::*;
use jsonrpc_http_server::*;
use std::collections::HashMap;
use std::iter::FromIterator;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

thread_local! {
    pub static BLOCKS: Vec<&'static str> = include!("./data/blocks.txt");
    pub static BLOCK_NUM: AtomicUsize = AtomicUsize::new(0);

    pub static HEADERS: HashMap<&'static str, &'static str> = include!("./data/headers.txt");

    pub static REFS: Vec<&'static str> = include!("./data/blockref.txt");
    pub static REF_NUM: AtomicUsize = AtomicUsize::new(0);
}

fn get_best_block_hash(_: Params) -> Result<Value> {
    let num = BLOCK_NUM.with(|bn| bn.fetch_add(1, Ordering::SeqCst));
    let response = BLOCKS.with(|v| v[num].to_string());

    Ok(Value::String(response))
}

fn get_block_header_info(params: Params) -> Result<Value> {
    let hash = match params {
        Params::Array(array) => array[0].clone(),
        _ => panic!("Invalid paramss"),
    };
    let hash = match hash {
        Value::String(s) => s,
        _ => panic!("Expecting a string"),
    };
    let response = HEADERS.with(|v| v[&hash.as_str()].to_string());

    let response: Value = serde_json::from_str(&response).expect("JSON serde error");

    Ok(response.into())
}

fn wait_for_new_block(_: Params) -> Result<Value> {
    let num = REF_NUM.with(|bn| bn.fetch_add(1, Ordering::SeqCst));
    let response = REFS.with(|v| v[num].to_string());

    let response: Value = serde_json::from_str(&response).expect("JSON serde error");

    std::thread::sleep(Duration::from_millis(500));
    Ok(response.into())
}

fn start_rpc_server() {
    let mut io = IoHandler::new();
    io.add_sync_method("getbestblockhash", get_best_block_hash);
    io.add_sync_method("getblockheader", get_block_header_info);
    io.add_sync_method("waitfornewblock", wait_for_new_block);

    let server = ServerBuilder::new(io)
        .start_http(&"127.0.0.1:8339".parse().unwrap())
        .expect("Failed to start RPC server");

    thread::spawn(move || server.wait());
}

#[test]
fn one_test() {
    let db_url = "postgres://forktester:forktester@localhost/forktester";
    let db_conn = PgConnection::establish(&db_url).expect("Connection failed");
    let _trans = db_conn.begin_test_transaction();
    start_rpc_server();

    let (sender, receiver) = unbounded();
    let auth = Auth::UserPass("bitcoin".into(), "pass".into());
    let fork_watcher = ForkScanner::new(db_conn, "http://localhost:8339", auth)
        .expect("Bitcoin rpc client failed");

    let _client = thread::spawn(move || fork_watcher.run(sender));

    match receiver.recv() {
        Ok(ReorgMessage::TipUpdated(_obj)) => (),
        e => panic!("expected tipupdated {:?}", e),
    }

    match receiver.recv() {
        Ok(ReorgMessage::TipUpdated(_obj)) => (),
        e => panic!("expected tipupdated {:?}", e),
    }

    match receiver.recv() {
        Ok(ReorgMessage::ReorgDetected(_obj, _obj2)) => (),
        e => panic!("expected tipupdated {:?}", e),
    }
}
