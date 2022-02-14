use bitcoincore_rpc::Client;
use diesel::prelude::PgConnection;
use diesel::Connection;
use forkscanner::run_server;
use forkscanner::ForkScanner;
use log::info;

fn main() {
    dotenv::dotenv().expect("Failed loading dotenv");
    let db_url = std::env::var("DATABASE_URL").expect("No DB url");
    let db_conn = PgConnection::establish(&db_url).expect("Connection failed");

    let scanner = ForkScanner::<Client>::new(db_conn).expect("Launching forkscanner failed");
    let duration = std::time::Duration::from_millis(10_000);

    let _handle = std::thread::spawn(move || loop {
        scanner.run();
        info!("Run finished, sleeping");
        std::thread::sleep(duration);
    });

    info!("Starting RPC server on 127.0.0.1:8339");
    run_server("127.0.0.1:8339", &db_url);
}
