//! The services: what runs once the early start is done, in this order, and how each behaves. A service's `start`
//! looks at the settings and the command line when it is (re)started and says what to run, or why not, or when to try
//! again; the supervisor does the rest.

use super::say;
use super::start::Boot;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub struct Service {
    pub name: &'static str,
    pub about: &'static str,
    pub start: fn(&Ctx) -> Start,
    /// After its processes have ended (a stop or a restart): what it leaves behind, cleared.
    pub stop: Option<fn()>,
    pub lifetime: Lifetime,
    pub output: Output,
    /// The settings it uses: a change restarts it. "wifi." = every key that starts so.
    pub settings: &'static [&'static str],
    /// Whether it does its job while it runs, beyond its processes being there: a failed check restarts it.
    pub health: Option<Health>,
}

/// A running service's health check: asked every `every`, on init's thread, so a check answers within a few hundred
/// ms. `new` makes the one check the service has from boot on: what it learns (a back-off) outlives a restart.
pub struct Health {
    pub every: Duration,
    pub new: fn() -> Box<dyn Check>,
}

pub trait Check {
    /// The service has started (again): what the check saw of the run before is gone.
    fn started(&mut self);
    /// Healthy (None), or why the service should start again.
    fn check(&mut self, now: Instant) -> Option<String>;
}

pub enum Lifetime {
    /// Runs until stopped; started again `again_after` it ends (longer while it keeps ending soon after starting).
    Daemon { again_after: Duration },
    /// Runs to its end. `wait`: the services after it start once it has ended; `retry`: a failure is tried again.
    Once { wait: bool, retry: Option<Backoff> },
}

/// Retries: `first` apart `times` times, then `then` apart, for good.
pub struct Backoff {
    pub first: Duration,
    pub times: u32,
    pub then: Duration,
}

impl Backoff {
    pub fn after(&self, attempt: u32) -> Duration {
        if attempt < self.times { self.first } else { self.then }
    }
}

/// Where a service's output goes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// /var/log/NAME.log, through init; the older part in NAME.log.1: a log at 32 KB, and a daemon's when it ends
    /// (with how), become that.
    Log,
    /// The log and, line by line, the kernel's (the console: the MCU logs it).
    LogAndKernel,
    /// The console, as it is (the self test's report).
    Console,
    /// The console as a login shell's terminal: a session of its own, the console its controlling tty.
    Shell,
}

/// What a service's `start` gets: the boot, the settings as they are now, the debug services' mode from the MCU,
/// and how many times in a row it said [`Start::Later`].
pub struct Ctx<'a> {
    pub boot: &'a Boot,
    pub settings: BTreeMap<String, String>,
    pub debug: Option<bool>,
    pub attempt: u32,
}

impl Ctx<'_> {
    pub fn setting(&self, key: &str) -> String {
        zd1_settings::get_from(&self.settings, key).unwrap_or_default()
    }
}

pub enum Start {
    Run(Vec<Command>),
    Off(String),
    Later(Duration, String),
}

/// A program and its arguments; `arg0` if it must see another name (a login shell's "-sh").
pub struct Command {
    pub argv: Vec<String>,
    pub arg0: Option<&'static str>,
}

pub(super) fn run(argv: &[&str]) -> Start {
    Start::Run(vec![command(argv)])
}

pub(super) fn command(argv: &[&str]) -> Command {
    Command { argv: argv.iter().map(|s| s.to_string()).collect(), arg0: None }
}

pub(super) const fn daemon(again_after_ms: u64) -> Lifetime {
    Lifetime::Daemon { again_after: Duration::from_millis(again_after_ms) }
}

/// The services, in the order they start (system.rs).
pub fn table() -> &'static [Service] {
    super::system::SERVICES
}

/// Once every service has been started: what the emulator and the docs wait for.
pub fn up() {
    say("up");
}

/// A shell on the console.
pub(super) fn shell(_: &Ctx) -> Start {
    Start::Run(vec![Command { argv: vec!["/bin/sh".into()], arg0: Some("-sh") }])
}
