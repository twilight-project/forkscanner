use forkscanner::ForkScanner;
use bitcoincore_rpc::Auth;
use crossbeam_channel::unbounded;
use std::thread;


fn main() {
    dotenv::dotenv().expect("Failed loading dotenv");
    let rpc_user = std::env::var("RPC_USER").expect("No bitcoin rpc user");
    let rpc_pass = std::env::var("RPC_PASS").expect("No bitcoin rpc password");
    let auth = Auth::UserPass(rpc_user, rpc_pass);

    let (tx, rx) = unbounded();

    let scanner = ForkScanner::new("http://localhost:8332", auth)
        .expect("Launching forkscanner failed");

    let _handle = thread::spawn(|| {
        scanner.run(tx);
    });

    loop {
        match rx.recv() {
            Ok(msg) => println!("TJDEBUG got a fork message {:?}", msg),
            Err(e) => panic!("Error {:?}", e),
        }
    }

}
