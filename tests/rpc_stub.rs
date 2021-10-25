use bitcoincore_rpc::Auth;
use crossbeam_channel::unbounded;
use forkscanner::ForkScanner;
use jsonrpc_core::*;
use jsonrpc_http_server::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

thread_local! {
    pub static BLOCKS: Vec<&'static str> = include!("./data/blocks.txt");
    pub static BLOCK_NUM: AtomicUsize = AtomicUsize::new(0);

    pub static HEADERS: Vec<&'static str> = include!("./data/headers.txt");
    pub static HEADER_NUM: AtomicUsize = AtomicUsize::new(0);
}

fn get_best_block_hash(_: Params) -> Result<Value> {
    let num = BLOCK_NUM.with(|bn| {
        bn.fetch_add(1, Ordering::SeqCst)
    });
    let response = BLOCKS.with(|v| v[num].to_string());

    Ok(Value::String(response))
}

fn get_block_header_info(_: Params) -> Result<Value> {
    let num = HEADER_NUM.with(|bn| {
        bn.fetch_add(1, Ordering::SeqCst)
    });
    let response = HEADERS.with(|v| v[num].to_string());

    Ok(response.into())
}

fn start_rpc_server() {
    let mut io = IoHandler::new();
    io.add_sync_method("getbestblockhash", get_best_block_hash);
    io.add_sync_method("getblockheader", get_block_header_info);

    let server = ServerBuilder::new(io)
        .start_http(&"127.0.0.1:8339".parse().unwrap())
        .expect("Failed to start RPC server");

    thread::spawn(move || server.wait());
}

#[test]
fn one_test() {
    dotenv::dotenv().expect("No env file");
    start_rpc_server();

    let (sender, receiver) = unbounded();
    let auth = Auth::UserPass("bitcoin".into(), "pass".into());
    let fork_watcher =
        ForkScanner::new("http://localhost:8339", auth)
        .expect("Bitcoin rpc client failed");

    let _client = thread::spawn(move || fork_watcher.run(sender));

    //match receiver.recv() {
    //    Ok(obj) => println!("TJDEBUG goodiegoodie! {:?}", obj),
    //    Err(e) => println!("TJDEBUG {:?}", e),
    //}

    assert!(false);
}
