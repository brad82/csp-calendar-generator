//! Build the iCalendar file for one shift.

use anyhow::Result;
use icalendar::{Calendar, Component, Event, EventLike, EventStatus};
use jiff::civil::DateTime;
use sha1::{Digest, Sha1};

use crate::Cfg;
use crate::roster::Shift;

/// Custom property holding a hash of the event's content, so unchanged events are skipped.
pub const HASH_PROP: &str = "X-CSP-HASH";

/// Stable event ID for a shift, so updates replace the same event.
pub fn uid(cfg: &Cfg, s: &Shift) -> String {
    let host = cfg.base.split("://").nth(1).unwrap_or(&cfg.base).split('/').next().unwrap_or_default();
    format!("woid-{}-{}@{host}", s.woid, cfg.void_id)
}

/// One-event calendar and its content hash.
pub fn event(cfg: &Cfg, s: &Shift, first_name: &str) -> Result<(String, String)> {
    let mut what = if s.kind.is_empty() { "Patrol Day".to_string() } else { s.kind.clone() };
    if !s.area.is_empty() {
        what = format!("{what} / {}", s.area);
    }
    let mut summary = if first_name.is_empty() { what } else { format!("{first_name} - {what}") };
    if !cfg.title_prefix.is_empty() {
        summary = format!("{} {summary}", cfg.title_prefix);
    }
    let link = format!("{}/patroldaydetail.php?woid={}", cfg.base, s.woid);
    let mut desc = format!("Status: {}", s.status);
    if !s.area.is_empty() {
        desc.push_str(&format!("\nDuty: {}", s.area));
    }
    desc.push_str(&format!("\n{link}"));
    let cancelled = s.status.to_lowercase().contains("cancel");
    let (uid, start, end) = (uid(cfg, s), utc(cfg, &s.date, &s.start)?, utc(cfg, &s.date, &s.end)?);

    let fields = [uid.as_str(), &start, &end, &summary, &cfg.location, &desc, &link, if cancelled { "C" } else { "" }];
    let hash = hex::encode(Sha1::digest(fields.join("\n").as_bytes()));

    let event = Event::new()
        .uid(&uid)
        .add_property("DTSTART", &start)
        .add_property("DTEND", &end)
        .summary(&summary)
        .location(&cfg.location)
        .description(&desc)
        .url(&link)
        .status(if cancelled { EventStatus::Cancelled } else { EventStatus::Confirmed })
        .add_property(HASH_PROP, &hash)
        .done();
    Ok((Calendar::new().push(event).done().to_string(), hash))
}

/// The stored hash in an event fetched back from the server, if any.
pub fn stored_hash(ics: &str) -> Option<String> {
    let cal: Calendar = ics.parse().ok()?;
    cal.components.iter().filter_map(|c| c.as_event()).find_map(|e| e.property_value(HASH_PROP)).map(String::from)
}

/// Local wall time in the configured zone -> UTC "YYYYMMDDTHHMMSSZ".
fn utc(cfg: &Cfg, date: &str, hm: &str) -> Result<String> {
    let local: DateTime = format!("{date}T{hm}").parse()?;
    Ok(local.to_zoned_datetime(cfg.tz.clone())?.timestamp().strftime("%Y%m%dT%H%M%SZ").to_string())
}
