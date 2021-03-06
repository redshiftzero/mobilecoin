// Copyright (c) 2018-2020 MobileCoin Inc.

//! MobileCoin Logging.
//!
//! The configuration of our logging is affected by environment variables. The following variables
//! are relevant:
//! - MC_LOG - Specifies the logging level (see
//! https://docs.rs/slog-envlogger/2.1.0/slog_envlogger/ for format)
//! - MC_LOG_GELF - When set to host:port, enables logging into a
//! [GELF](https://docs.graylog.org/en/3.0/pages/gelf.html) UDP receiver. Suitable for use with
//! [logstash](https://www.elastic.co/products/logstash).
//! - MC_LOG_UDP_JSON - When set to host:port, enables logging JSON log messages into a UDP socket.
//! Suitable for use with [filebeat](https://www.elastic.co/products/beats/filebeat).
//! - MC_LOG_EXTRA_CONTEXT - Adds an extra logging context (key=val, separated by comma).

/// Sets chan_size for stdout, gelf, and UDP loggers
const STDOUT_CHANNEL_SIZE: usize = 100_000;
const GELF_CHANNEL_SIZE: usize = 100_000;
const UDP_CHANNEL_SIZE: usize = 100_000;

/// Wrap calls to assert! macros to record an error message before panic
#[macro_export]
macro_rules! log_assert {
    ($logger:expr, $cond:expr) => ({
        if !$cond {
            let cond_str = stringify!($cond);
            log::crit!($logger, "assert!({}) failed", cond_str);
            std::thread::sleep(Duration::from_millis(500));
            panic!("assert!({}) failed", cond_str);
        }
    });
    ($logger:expr, $cond:expr,) => ({
        if !$cond {
            let cond_str = stringify!($cond);
            log::crit!($logger, "assert!({}) failed", cond_str);
            std::thread::sleep(Duration::from_millis(500));
            panic!("assert!({}) failed", cond_str);
        }
    });
    ($logger:expr, $cond:expr, $($arg:tt)+) => ({
        if !$cond {
            let m = format!($($arg)+);
            let cond_str = stringify!($cond);
            log::crit!($logger, "assert!({}) failed, {}", cond_str, m);
            std::thread::sleep(Duration::from_millis(500));
            panic!("assert!({}) failed, {}", cond_str, m);
        }
    })
}

/// Wrap calls to assert_eq! macros to record an error message before panic
#[macro_export]
macro_rules! log_assert_eq {
    ($logger:expr, $left:expr, $right:expr) => ({
        log_assert!($logger, ($left) == ($right));
    });
    ($logger:expr, $left:expr, $right:expr,) => ({
        log_assert!($logger, ($left) == ($right));
    });
    ($logger:expr, $left:expr, $right:expr, $($arg:tt)+) => ({
        let m = format!($($arg)+);
        log_assert!($logger, ($left) == ($right), "{}", m);
    })
}

/// Wrap calls to assert_ne! macros to record an error message before panic
#[macro_export]
macro_rules! log_assert_ne {
    ($logger:expr, $left:expr, $right:expr) => ({
        log_assert!($logger, ($left) != ($right));
    });
    ($logger:expr, $left:expr, $right:expr,) => ({
        log_assert!($logger, ($left) != ($right));
    });
    ($logger:expr, $left:expr, $right:expr, $($arg:tt)+) => ({
        let m = format!($($arg)+);
        log_assert!($logger, ($left) != ($right), "{}", m);
    })
}

/// Expose the standard crit! debug! error! etc macros from slog
/// (those are the ones that accept a Logger instance)
pub mod log {
    pub use slog::{crit, debug, error, info, trace, warn};
}

/// A global logger, for when passing a Logger instance is impractical.
pub mod global_log {
    pub use slog_scope::{crit, debug, error, info, trace, warn};
}

/// Macros to ease with tests/benches that require a Logger instance.
pub use mc_util_logger_macros::{bench_with_logger, test_with_logger};

/// Expose slog, slog_scope, and select useful primitives.
pub use slog;
pub use slog::{o, FnValue, Logger, PushFnValue};
pub use slog_scope;

mod sentry_logger;
/// Internal modules/imports.
mod udp_writer;

use chrono;
use lazy_static::lazy_static;
use mc_util_build_info;
use sentry_logger::SentryLogger;
use slog::Drain;
use slog_gelf;
use slog_json;
use std::{env, io, sync::Mutex, time::Instant};

/// Custom timestamp function for use with slog-term
fn custom_timestamp(io: &mut dyn io::Write) -> io::Result<()> {
    write!(io, "{}", chrono::Utc::now())
}

/// Create a basic stdout logger.
fn create_stdout_logger() -> slog::Fuse<slog_async::Async> {
    let decorator = slog_term::TermDecorator::new().stdout().build();
    let drain = slog_envlogger::new(
        slog_term::FullFormat::new(decorator)
            .use_custom_timestamp(custom_timestamp)
            .build()
            .fuse(),
    );
    slog_async::Async::new(drain)
        .thread_name("slog-stdout".into())
        .chan_size(STDOUT_CHANNEL_SIZE)
        .build()
        .fuse()
}

/// Create a GELF (https://docs.graylog.org/en/3.0/pages/gelf.html) logger.
fn create_gelf_logger() -> Option<slog::Fuse<slog_async::Async>> {
    env::var("MC_LOG_GELF").ok().map(|remote_host_port| {
        let local_hostname = hostname::get_hostname().unwrap();

        let drain = slog_envlogger::new(
            slog_gelf::Gelf::new(&local_hostname, &remote_host_port[..])
                .expect("failed creating Gelf logger for")
                .fuse(),
        );

        slog_async::Async::new(drain)
            .thread_name("slog-gelf".into())
            .chan_size(GELF_CHANNEL_SIZE)
            .build()
            .fuse()
    })
}

/// Create a UDP JSON logger.
fn create_udp_json_logger() -> Option<slog::Fuse<slog_async::Async>> {
    env::var("MC_LOG_UDP_JSON").ok().map(|remote_host_port| {
        let drain = slog_json::Json::new(udp_writer::UdpWriter::new(remote_host_port))
            .set_newlines(false)
            .set_flush(true)
            .add_key_value(o!(
                    "ts" => PushFnValue(move |_, ser| {
                        ser.emit(chrono::Local::now().to_rfc3339())
                    }),
                    "level_str" => FnValue(move |record| {
                        record.level().as_short_str()
                    }),
                    "level"  => FnValue(move |record| {
                        record.level().as_usize()
                    }),
                    "message" => PushFnValue(move |record, ser| {
                        // Cap message at 65000 bytes to increase chances of it fitting in a UDP
                        // packet.
                        let mut msg = format!("{}", record.msg());
                        if msg.len() > 65000 {
                            msg = format!("{}... <trimmed>", &msg[0..65000]);
                        }
                        ser.emit(msg)
                    }),
            ))
            .build()
            .fuse();

        let drain = slog_envlogger::new(drain);

        slog_async::Async::new(drain)
            .thread_name("slog-udp".into())
            .chan_size(UDP_CHANNEL_SIZE)
            .build()
            .fuse()
    })
}

/// Create the root logger, which logs to stdout and optionally a GELF endpoint
/// (if the `MC_LOG_GELF` environment variable is set) or a UDP JSON endpoint (if the
/// `MC_LOG_UDP_JSON` environment variable is set).
pub fn create_root_logger() -> Logger {
    // Support MC_LOG in addition to RUST_LOG. This makes allows us to not affect cargo's logs when
    // doing stuff like MC_LOG=trace cargo test -p ...
    if env::var("RUST_LOG").is_err() && env::var("MC_LOG").is_ok() {
        env::set_var("RUST_LOG", env::var("MC_LOG").unwrap());
    }

    // Default to INFO log level for everything if we do not have an explicit setting.
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }

    // Create our loggers.
    let network_logger = match (create_gelf_logger(), create_udp_json_logger()) {
        (None, None) => None,
        (Some(gelf), None) => Some(gelf),
        (None, Some(udp_json)) => Some(udp_json),
        (Some(_), Some(_)) => panic!("MC_LOG_GELF and MC_LOG_UDP_JSON are mutually exclusive!"),
    };
    let stdout_logger = create_stdout_logger();

    // Extra context that always gets added to each log message.
    let extra_kv = o!(
        "mc.src" => MaybeMcSrcValue {},
        "mc.module" => MaybeMcModuleValue {},
    );

    // Create root logger.
    let mut root_logger = if let Some(network_logger) = network_logger {
        Logger::root(
            slog::Duplicate(stdout_logger, network_logger).fuse(),
            extra_kv,
        )
    } else {
        Logger::root(stdout_logger, extra_kv)
    };

    // Add extra context if it is available.
    // (Format we're parsing is key1=val1,key2=val2,... a trailing comma is allowed)
    if let Ok(mc_log_extra) = env::var("MC_LOG_EXTRA_CONTEXT") {
        for key_val_str in mc_log_extra.split(',') {
            if !key_val_str.is_empty() {
                let key_val = key_val_str.split('=').collect::<Vec<&str>>();
                if key_val.len() != 2 {
                    panic!("invalid MC_LOG_EXTRA key/val: {}", key_val_str)
                }

                let k = key_val[0].to_string();
                let v = key_val[1].to_string();

                root_logger = root_logger.new(o!(k => v));
            }
        }
    }

    // Return
    root_logger
}

/// Create a logger that is suitable for use during test execution.
pub fn create_test_logger(test_name: String) -> Logger {
    create_root_logger().new(o!(
        "mc.test_name" => test_name,
    ))
}

/// Create a logger that discards everything.
pub fn create_null_logger() -> Logger {
    Logger::root(slog::Discard, o!())
}

/// Switchable app logger support.
lazy_static! {
    static ref SWITCHABLE_APP_LOGGER: slog_atomic::AtomicSwitchCtrl<(), io::Error> =
        slog_atomic::AtomicSwitch::new(
            slog::Discard.map_err(|_| io::Error::new(io::ErrorKind::Other, "should not happen"))
        )
        .ctrl();
}

/// Create an application logger (to be used by our binary crates).
pub fn create_app_logger<T: slog::SendSyncRefUnwindSafeKV + 'static>(
    values: slog::OwnedKV<T>,
) -> (Logger, slog_scope::GlobalLoggerGuard) {
    // Put a root logger in the slog-atomic object
    SWITCHABLE_APP_LOGGER.set(
        Mutex::new(create_root_logger())
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "mutex error")),
    );

    // Get the root logger
    let root_logger = Logger::root(SWITCHABLE_APP_LOGGER.drain().fuse(), o!());

    // Wrap root logger in a SentryLogger so that error and critical messages get forwarded to Sentry.
    let root_logger = SentryLogger::wrap(root_logger);

    // App-specific logging context and slog-scope initialization.
    let current_exe = std::env::current_exe()
        .expect("failed getting current exe")
        .file_name()
        .expect("failed getting current exe filename")
        .to_str()
        .expect("to_str failed")
        .to_string();

    let app_logger = root_logger
        .new(o!(
            "mc.app" => current_exe.clone(),
        ))
        .new(values);
    let guard = slog_scope::set_global_logger(app_logger.clone());
    slog_stdlog::init().expect("slog_stdlog::init failed");

    {
        let mut buf = String::new();
        mc_util_build_info::write_report(&mut buf).expect("Getting build_info report failed");
        log::info!(app_logger, "{} started: {}", current_exe, buf);
    }

    (app_logger, guard)
}

/// The hack that re-initializes the app logger.
pub fn recreate_app_logger() {
    SWITCHABLE_APP_LOGGER.set(
        Mutex::new(create_root_logger())
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "mutex error")),
    );
}

/// Get the global Logger instance, managed by `slog_scope`.
pub fn global_logger() -> Logger {
    slog_scope::logger()
}

/// Convenience wrapper around `slog_scope::scope`.
pub fn scoped_global_logger<F, R>(logger: &Logger, f: F) -> R
where
    F: FnOnce(&Logger) -> R,
{
    slog_scope::scope(&logger, || f(&logger))
}

/// Simple time measurement utility, based on the [measure_time](https://docs.rs/measure_time/) crate.
/// Note that even though the macro lives inside the `logger` module, it needs to be imported by
/// `use mc_common::trace_time`, since Rust exports all macros at the crate level :/
#[macro_export]
macro_rules! trace_time {
    ($logger:expr, $($arg:tt)+) => (
        let _trace_time = $crate::logger::TraceTime::new($logger.clone(), $crate::logger::slog::record_static!($crate::logger::slog::Level::Trace, ""), format!($($arg)+));
    )
}

pub struct TraceTime<'a> {
    logger: Logger,
    rstatic: slog::RecordStatic<'a>,
    msg: String,
    start: Instant,
}

impl<'a> TraceTime<'a> {
    pub fn new(logger: Logger, rstatic: slog::RecordStatic<'a>, msg: String) -> Self {
        let start = Instant::now();
        Self {
            logger,
            rstatic,
            msg,
            start,
        }
    }
}

impl<'a> Drop for TraceTime<'a> {
    fn drop(&mut self) {
        let time_in_ms = (self.start.elapsed().as_secs() as f64 * 1_000.0)
            + (self.start.elapsed().subsec_nanos() as f64 / 1_000_000.0);

        let time = match time_in_ms as u64 {
            0..=3000 => format!("{}ms", time_in_ms),
            3001..=60000 => format!("{:.2}s", time_in_ms / 1000.0),
            _ => format!("{:.2}m", time_in_ms / 1000.0 / 60.0),
        };

        self.logger.log(&slog::Record::new(
            &self.rstatic,
            &format_args!("{}: took {}", self.msg, time),
            slog::b!("duration_ms" => time_in_ms),
        ));
    }
}

#[cfg(test)]
mod trace_time_tests {
    use super::*;

    #[test]
    fn basic_trace_time() {
        let logger = create_test_logger("basic_trace_time".to_string());

        slog_scope::scope(&logger.clone(), || {
            trace_time!(global_logger(), "test global");

            {
                trace_time!(logger, "test inner");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            std::thread::sleep(std::time::Duration::from_millis(10));
        });
    }
}

// `MaybeMcSrcValue` allows us to selectively include "mc.src" in our logging context.
// We want to only include it for log messages that did not originate from inside an enclave,
// since enclave logging context already includes this information (see mc_sgx_urts::enclave_log).
// Doing it this way is necessary due due to how `slog` works.
struct MaybeMcSrcValue;
impl slog::Value for MaybeMcSrcValue {
    fn serialize(
        &self,
        record: &slog::Record,
        key: slog::Key,
        serializer: &mut dyn slog::Serializer,
    ) -> slog::Result {
        if record.file() != "<enclave>" {
            serializer.emit_str(key, &format!("{}:{}", record.file(), record.line()))?;
        }
        Ok(())
    }
}

// See `MaybeMcSrcValue` above.
struct MaybeMcModuleValue;
impl slog::Value for MaybeMcModuleValue {
    fn serialize(
        &self,
        record: &slog::Record,
        key: slog::Key,
        serializer: &mut dyn slog::Serializer,
    ) -> slog::Result {
        if record.file() != "<enclave>" {
            serializer.emit_str(key, record.module())?;
        }
        Ok(())
    }
}
