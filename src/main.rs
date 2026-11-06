use clap::Parser;
use idx::cli::{self, Cli};

fn init_tracing(verbosity: u8, quiet: bool) {
    use tracing_subscriber::EnvFilter;
    let default = match verbosity {
        // --quiet: only errors, unless IDX_LOG asks for more.
        0 if quiet => "error",
        0 => "warn",
        1 => "idx=info,warn",
        2 => "idx=debug,info",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_env("IDX_LOG").unwrap_or_else(|_| EnvFilter::new(default));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(|| idx::progress::Stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_target(verbosity >= 2)
        .without_time()
        .init();
}

fn main() {
    let cli = Cli::parse();
    let quiet = idx::config::quiet_requested(cli.global.quiet, cli.global.verbose, &|k| {
        std::env::var(k).ok()
    });
    idx::output::set_quiet(quiet);
    init_tracing(cli.global.verbose, quiet);
    let global = cli.global.clone();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            cli::report_error(
                Some(&global),
                &idx::IdxError::Other(format!("runtime: {e}")),
            );
            std::process::exit(1);
        }
    };

    match runtime.block_on(cli::run(cli)) {
        Ok(()) => {}
        Err(err) => {
            cli::report_error(Some(&global), &err);
            std::process::exit(err.exit_code());
        }
    }
}
