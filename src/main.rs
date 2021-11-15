use crossbeam_channel::unbounded;
use diesel::prelude::PgConnection;
use diesel::Connection;
use forkscanner::ForkScanner;
use std::thread;

fn main() {
    dotenv::dotenv().expect("Failed loading dotenv");
    let db_url = std::env::var("DATABASE_URL").expect("No DB url");
    let db_conn = PgConnection::establish(&db_url).expect("Connection failed");

    let (tx, rx) = unbounded();
    let scanner = ForkScanner::new(db_conn).expect("Launching forkscanner failed");

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
