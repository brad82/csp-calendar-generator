//! Scrape a patroller's shifts from their roster page.

use std::collections::HashMap;

use anyhow::{Result, bail};
use regex::Regex;

use crate::login::{Session, is_login_page};
use crate::var;

pub struct Shift {
    pub woid: String,
    pub date: String,
    pub kind: String,
    pub status: String,
    pub area: String,
    pub start: String,
    pub end: String,
}

pub struct Roster {
    pub first_name: String,
    pub shifts: Vec<Shift>,
}

/// Fetch the patroller's page and return their first name and season shifts with times.
pub fn fetch(session: &Session, void_id: &str) -> Result<Roster> {
    let html = session.get(&format!("userdetail.php?void={void_id}"))?;
    if is_login_page(&html) {
        bail!("session not accepted: redirected to login");
    }

    let first_name = Some(var("FIRST_NAME", "")).filter(|v| !v.is_empty()).unwrap_or_else(|| {
        Regex::new(r"User : ([^<\s]+)").unwrap().captures(&html).map(|c| c[1].to_string()).unwrap_or_default()
    });

    let mut shifts = Vec::new();
    let (def_start, def_end) = (var("SHIFT_START", "08:00"), var("SHIFT_END", "16:00"));
    for (mut s, cal_id) in parse_shifts(&html) {
        let times = match cal_id {
            Some(id) => times(session, void_id, &id),
            None => None,
        };
        (s.start, s.end) = times.unwrap_or((def_start.clone(), def_end.clone()));
        shifts.push(s);
    }
    if shifts.is_empty() {
        bail!("no shifts found; page layout may have changed");
    }
    Ok(Roster { first_name, shifts })
}

/// Start/end times from the site's own Outlook export.
fn times(session: &Session, void_id: &str, cal_id: &str) -> Option<(String, String)> {
    let txt = session.get(&format!("calendar_add.php?void={void_id}&i1oid={cal_id}")).ok()?;
    let t = |prop: &str| {
        Regex::new(&format!(r"{prop}:\d{{4}}-\d{{2}}-\d{{2}} (\d{{2}}:\d{{2}})"))
            .unwrap()
            .captures(&txt)
            .map(|c| c[1].to_string())
    };
    Some((t("DTSTART")?, t("DTEND")?))
}

fn strip(s: &str) -> String {
    let s = Regex::new(r"(?i)<br\s*/?>").unwrap().replace_all(s, " ");
    let s = Regex::new(r"<[^>]+>").unwrap().replace_all(&s, "");
    let s = s.replace("&nbsp;", " ").replace("&amp;", "&");
    Regex::new(r"\s+").unwrap().replace_all(&s, " ").trim().to_string()
}

/// Shifts (without times) plus the calendar-export id for upcoming ones.
fn parse_shifts(html: &str) -> Vec<(Shift, Option<String>)> {
    let mut dates = HashMap::new();
    let mut cal_ids = HashMap::new();
    // woid -> full date, from the stats image map
    let woid_re = Regex::new(r"woid=(\d+)").unwrap();
    for c in Regex::new(r#"<area[^>]*title="Patrol Day (\d{4}-\d{2}-\d{2})"[^>]*>"#).unwrap().captures_iter(html) {
        if let Some(w) = woid_re.captures(&c[0]) {
            dates.insert(w[1].to_string(), c[1].to_string());
        }
    }
    // woid -> full date + calendar id, from "Upcoming Schedule"
    for c in Regex::new(r#"woid=(\d+)">(\d{4}-\d{2}-\d{2})</a>[\s\S]*?i1oid=(\d+)"#).unwrap().captures_iter(html) {
        dates.insert(c[1].to_string(), c[2].to_string());
        cal_ids.insert(c[1].to_string(), c[3].to_string());
    }

    // Season schedule rows: <tr><td>Type</td><td><a woid>Sat, Nov 14</a></td><td>Status<br> Area</td></tr>
    let section = html.split_once("<h3>Schedule for").map_or("", |(_, s)| s);
    let row = Regex::new(
        r#"<tr><td>([^<]*)</td><td><a href="patroldaydetail\.php\?woid=(\d+)">([^<]*)</a></td><td>([\s\S]*?)</td></tr>"#,
    )
    .unwrap();
    let br = Regex::new(r"(?i)<br\s*/?>").unwrap();
    let mut out = Vec::new();
    for c in row.captures_iter(section) {
        let woid = c[2].to_string();
        let Some(date) = dates.get(&woid) else {
            eprintln!("skipping woid {woid} ({}): no full date found", &c[3]);
            continue;
        };
        let mut parts = br.split(&c[4]);
        let status = strip(parts.next().unwrap_or_default());
        let area = parts.map(strip).collect::<Vec<_>>().join(" ").trim().to_string();
        let cal_id = cal_ids.get(&woid).cloned();
        let shift = Shift {
            date: date.clone(),
            kind: strip(&c[1]),
            status,
            area,
            woid,
            start: String::new(),
            end: String::new(),
        };
        out.push((shift, cal_id));
    }
    out
}
