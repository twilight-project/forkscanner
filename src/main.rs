use bitcoincore_rpc::Client;
use diesel::prelude::PgConnection;
use diesel::Connection;
use forkscanner::run_server;
use forkscanner::{ForkScanner, WatcherMode};
use log::info;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "forkscanner", about = "A Bitcoin fork monitor.")]
struct Opt {
    /// Set rpc port
    #[structopt(short = "r", long = "rpc", default_value = "8339")]
    rpc: u16,

    /// Set ws port
    #[structopt(short = "w", long = "ws", default_value = "8340")]
    ws: u16,

    /// Enable address watcher
    #[structopt(short = "a", long = "watch-addresses", default_value = "none")]
    watch_addresses: WatcherMode,
}

fn main() {
    env_logger::init();
    dotenv::dotenv().expect("Failed loading dotenv");
    let db_url = std::env::var("DATABASE_URL").expect("No DB url");
    let db_conn = PgConnection::establish(&db_url).expect("Connection failed");
    let opt = Opt::from_args();

    let (scanner, receiver, command) = ForkScanner::<Client>::new(db_conn, opt.watch_addresses)
        .expect("Launching forkscanner failed");
    let duration = std::time::Duration::from_millis(10_000);

    let _handle = std::thread::spawn(move || loop {
        scanner.run();
        info!("Run finished, sleeping");
        std::thread::sleep(duration);
    });

    info!(
        "Starting RPC server on 127.0.0.1 rpc-port {} subscribe-port {}",
        opt.rpc, opt.ws
    );
    run_server(
        "0.0.0.0".into(),
        opt.rpc,
        opt.ws,
        db_url.into(),
        receiver,
        command,
    );
}
