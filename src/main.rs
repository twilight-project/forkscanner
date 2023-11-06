use bitcoincore_rpc::Client;
use forkscanner::run_server;
use forkscanner::{ForkScanner, WatcherMode};
use log::info;
use structopt::StructOpt;
use tracing_rolling_file::base::*;
use tracing_rolling_file::RollingFileAppender;

const MAX_FILE_COUNT: usize = 12;
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

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

    fn make_appender() -> RollingFileAppender<tracing_rolling_file::RollingConditionBase> {
        let log_prefix = std::env::var("LOG_PREFIX").map_or("/var/log/forkscanner".to_string(), |v| v);
        RollingFileAppenderBase::new(
            log_prefix,
            RollingConditionBase::new().hourly().max_size(MAX_FILE_SIZE),
            MAX_FILE_COUNT,
        )
        .expect("Could not create file appender")
    }

    tracing_subscriber::fmt::Subscriber::builder()
        .with_writer(make_appender)
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
