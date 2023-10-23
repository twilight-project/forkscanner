use bitcoincore_rpc::Client;
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
    #[structopt(short = "a", long = "watch-addresses", default_value = "none", possible_values(&["none", "inputs", "outputs", "all"]))]
    watch_addresses: WatcherMode,
}

fn main() {
    dotenv::dotenv().expect("Failed loading dotenv");

    let log_dir = std::env::var("LOG_DIR").map_or("/var/log".to_string(), |v| v);
    let file_appender = tracing_appender::rolling::hourly(log_dir, "forkscanner.log").rolling(FixedWindowRoller::builder().build("forkscanner.log", 5).unwrap());
    tracing_subscriber::fmt::Subscriber::builder()
        .with_writer(file_appender)
        .with_env_filter(tracing_subscriber::filter::EnvFilter::from_default_env())
        .with_ansi(false)
        .with_level(true)
        .with_line_number(true)
        .init();

    let db_url = std::env::var("DATABASE_URL").expect("No DB url");
    let opt = Opt::from_args();

    let (scanner, receiver, command) =
        ForkScanner::<Client>::new(db_url.clone(), opt.watch_addresses)
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
