//! csp-ical: sync a CSP Roster patroller's shifts into an iCloud calendar.
//!
//! Usage: csp-ical [--list-calendars] <env-file>...
//! Example: csp-ical .env.icloud .env.patroller

mod ical;
mod icloud;
mod login;
mod roster;

use std::env;

use anyhow::{Context, Result, anyhow, bail};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::TokioExecutor;
use jiff::Timestamp;
use jiff::tz::{TimeZone, TimeZoneDatabase};

use icloud::ICloud;
use login::Session;

pub type Https = Client<HttpsConnector<HttpConnector>, String>;

const USAGE: &str = "usage: csp-ical [--list-calendars] <env-file>...";

pub struct Cfg {
    pub base: String,
    pub void_id: String,
    pub location: String,
    pub title_prefix: String,
    pub tz: TimeZone,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("[{}] {e:#}", Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ"));
        std::process::exit(1);
    }
}

/// Env var, or `default` when unset or empty.
pub fn var(k: &str, default: &str) -> String {
    env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

fn required(k: &str) -> Result<String> {
    env::var(k).ok().filter(|v| !v.is_empty()).ok_or_else(|| anyhow!("set {k} in an env file"))
}

async fn run() -> Result<()> {
    let mut list_calendars = false;
    let mut files = 0;
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--list-calendars" | "-list-calendars" => list_calendars = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            file => {
                // later files override earlier ones
                dotenvy::from_path_override(file).with_context(|| file.to_string())?;
                files += 1;
            }
        }
    }
    if files == 0 {
        bail!(USAGE);
    }

    let cfg = Cfg {
        base: var("CSP_BASE", "https://lakelouise.csproster.ca"),
        void_id: var("CSP_VOID", "1848"),
        location: var("LOCATION", "1 Whitehorn Road, Lake Louise, Alberta, Canada, T0L 1E0"),
        title_prefix: var("TITLE_PREFIX", "⛑️"),
        // Always use the bundled zone data so results don't depend on the server's tzdata
        tz: TimeZoneDatabase::bundled().get(&var("TZ_NAME", "America/Edmonton")).context("TZ_NAME")?,
    };

    let icloud = ICloud::new(https_client(), &required("ICLOUD_USER")?, &required("ICLOUD_APP_PASSWORD")?);

    if list_calendars {
        for (name, _) in icloud.calendars().await? {
            println!("{name}");
        }
        return Ok(());
    }
    let cal_name = required("ICLOUD_CALENDAR")?;

    let session = Session::login(&cfg.base, &required("CSP_LOGIN")?, &required("CSP_PASSWORD")?)?;
    let roster = roster::fetch(&session, &cfg.void_id)?;
    icloud.sync(&cfg, &cal_name, &roster.shifts, &roster.first_name).await
}

/// HTTPS client for libdav, with built-in root certificates (no system CA files needed).
fn https_client() -> Https {
    let connector = HttpsConnectorBuilder::new().with_webpki_roots().https_only().enable_http1().build();
    Client::builder(TokioExecutor::new()).build(connector)
}
