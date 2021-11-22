use diesel::prelude::PgConnection;
use diesel::Connection;
use forkscanner::ForkScanner;

fn main() {
    dotenv::dotenv().expect("Failed loading dotenv");
    let db_url = std::env::var("DATABASE_URL").expect("No DB url");
    let db_conn = PgConnection::establish(&db_url).expect("Connection failed");

    let scanner = ForkScanner::new(db_conn).expect("Launching forkscanner failed");
    scanner.run();
}
