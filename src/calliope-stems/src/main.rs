//! calliope-stems: the stem separation server (API v1). See specs/gui-stem-extraction.plan.md.

mod cli;
mod config;
mod jobs;
mod separator;
mod server;
mod workdir;

use std::fmt::Arguments;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cli::Parsed;

/// Writes a log line to stderr (journald under systemd).
pub fn log(args: Arguments<'_>) {
    eprintln!("calliope-stems: {args}");
}

static TERMINATE: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    TERMINATE.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
    }
}

fn main() {
    let cfg = match cli::parse(std::env::args().skip(1), &cli::Env::from_process()) {
        Ok(Parsed::Help) => {
            print!("{}", cli::HELP);
            return;
        }
        Ok(Parsed::Version) => {
            println!("calliope-stems {}", cli::VERSION);
            return;
        }
        Ok(Parsed::Run(cfg)) => Arc::new(*cfg),
        Err(msg) => {
            eprintln!("calliope-stems: {msg}");
            std::process::exit(2);
        }
    };

    if let Err(e) = workdir::ensure(&cfg.work_dir) {
        eprintln!("calliope-stems: cannot create the work directory {}: {e}", cfg.work_dir.display());
        std::process::exit(1);
    }
    let removed = workdir::clean_start(&cfg.work_dir);
    log(format_args!("cleanup removed={removed}"));

    let http = match tiny_http::Server::http(cfg.listen) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("calliope-stems: cannot listen on {}: {e}", cfg.listen);
            std::process::exit(1);
        }
    };
    let addr = http.server_addr().to_ip().unwrap_or(cfg.listen);
    install_signal_handlers();

    let mgr = jobs::Manager::new(cfg.clone());
    let worker = {
        let m = mgr.clone();
        std::thread::spawn(move || m.run_worker())
    };
    {
        let m = mgr.clone();
        std::thread::spawn(move || m.run_janitor());
    }
    log(format_args!("listening addr={addr} model={} separator={}", cfg.model, cfg.separator.display()));

    while !TERMINATE.load(Ordering::SeqCst) {
        match http.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(req)) => {
                let (m, c) = (mgr.clone(), cfg.clone());
                std::thread::spawn(move || server::handle(&m, &c, req));
            }
            Ok(None) => {}
            Err(e) => {
                log(format_args!("accept failed: {e}"));
                break;
            }
        }
    }

    log(format_args!("stopping"));
    mgr.shutdown();
    mgr.wait_idle(Duration::from_secs(5));
    let _ = worker.join();
}
