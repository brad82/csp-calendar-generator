//! Sync shifts into an iCloud calendar over CalDAV (libdav).

use std::collections::{HashMap, HashSet};

use http::Uri;
use jiff::Timestamp;
use libdav::caldav::{FindCalendarHomeSet, FindCalendars, GetCalendarResources};
use libdav::dav::{Delete, GetProperty, ListResources, PutResource, WebDavClient};
use libdav::{CalDavClient, names};
use anyhow::{Context, Result, anyhow, bail};
use tower_http::auth::AddAuthorization;

use crate::roster::Shift;
use crate::{Cfg, Https, ical};

pub struct ICloud {
    dav: CalDavClient<AddAuthorization<Https>>,
}

const ICS: &str = "text/calendar; charset=utf-8";

impl ICloud {
    pub fn new(http: Https, user: &str, pass: &str) -> Self {
        let client = AddAuthorization::basic(http, user, pass);
        let dav = CalDavClient::new(WebDavClient::new(Uri::from_static("https://caldav.icloud.com/"), client));
        ICloud { dav }
    }

    /// (display name, href) for every calendar on the account.
    pub async fn calendars(&self) -> Result<Vec<(String, String)>> {
        let principal = self
            .dav
            .find_current_user_principal()
            .await
            .context("iCloud login failed (check ICLOUD_USER / ICLOUD_APP_PASSWORD)")?
            .ok_or_else(|| anyhow!("iCloud returned no user principal"))?;
        let homes = self.dav.request(FindCalendarHomeSet::new(principal.path())).await?.home_sets;
        let mut out = Vec::new();
        for home in homes {
            for cal in self.dav.request(FindCalendars::new(home.path())).await?.calendars {
                let name =
                    self.dav.request(GetProperty::new(&cal.href, &names::DISPLAY_NAME)).await?.value.unwrap_or_default();
                out.push((name, cal.href));
            }
        }
        Ok(out)
    }

    /// Make the calendar match the roster for this patroller: add new shifts, update
    /// changed ones, and delete future shifts that left the roster (past ones stay).
    pub async fn sync(&self, cfg: &Cfg, cal_name: &str, shifts: &[Shift], first_name: &str) -> Result<()> {
        let cal_href = self.find_calendar(cal_name).await?;

        // Our events are named csp-{void}-{date}-{woid}.ics
        let prefix = format!("csp-{}-", cfg.void_id);
        let existing = self.existing(&cal_href, &prefix).await?;

        let (mut added, mut updated, mut unchanged, mut removed, mut skipped) = (0, 0, 0, 0, 0);
        let mut wanted = HashSet::new();
        for s in shifts {
            let file = format!("{prefix}{}-{}.ics", s.date, s.woid);
            wanted.insert(file.clone());
            let (ics, hash) = ical::event(cfg, s, first_name)?;
            let result = match existing.get(&file) {
                Some(e) if e.hash == hash => {
                    unchanged += 1;
                    continue;
                }
                Some(e) => {
                    self.dav.request(PutResource::new(&e.href).update(ics, ICS, e.etag.clone())).await.map(|_| &mut updated)
                }
                None => {
                    let href = format!("{cal_href}{file}");
                    self.dav.request(PutResource::new(&href).create(ics, ICS)).await.map(|_| &mut added)
                }
            };
            match result {
                Ok(counter) => *counter += 1,
                Err(e) if format!("{e:?}").contains("PreconditionFailed") || e.to_string().contains("412") => {
                    // iCloud event IDs are unique per account: a copy exists in another calendar
                    eprintln!(
                        "skipped {}: event {} already exists in another iCloud calendar; delete that copy",
                        s.date,
                        ical::uid(cfg, s)
                    );
                    skipped += 1;
                }
                Err(e) => return Err(e.into()),
            }
        }

        let today = Timestamp::now().to_zoned_datetime(cfg.tz.clone()).strftime("%Y-%m-%d").to_string();
        for (file, e) in &existing {
            let date = file.get(prefix.len()..prefix.len() + 10).unwrap_or_default();
            if wanted.contains(file) || date < today.as_str() {
                continue;
            }
            self.dav.request(Delete::new(&e.href).force()).await?;
            removed += 1;
        }

        let mut msg = format!("iCloud {cal_name:?}: {added} added, {updated} updated, {unchanged} unchanged, {removed} removed");
        if skipped > 0 {
            msg.push_str(&format!(", {skipped} skipped"));
        }
        println!("{msg}");
        Ok(())
    }

    async fn find_calendar(&self, name: &str) -> Result<String> {
        let cals = self.calendars().await?;
        let Some((_, href)) = cals.iter().find(|(n, _)| n == name) else {
            let names: Vec<_> = cals.iter().map(|(n, _)| n.as_str()).collect();
            bail!("no iCloud calendar named {name:?} (found: {})", names.join(", "));
        };
        Ok(if href.ends_with('/') { href.clone() } else { format!("{href}/") })
    }

    /// Events this tool created earlier, keyed by file name.
    async fn existing(&self, cal_href: &str, prefix: &str) -> Result<HashMap<String, Existing>> {
        let ours: Vec<String> = self
            .dav
            .request(ListResources::new(cal_href))
            .await?
            .resources
            .into_iter()
            .filter(|r| !r.resource_type.is_collection)
            .map(|r| r.href)
            .filter(|h| file_name(h).starts_with(prefix) && h.ends_with(".ics"))
            .collect();

        let mut out = HashMap::new();
        if ours.is_empty() {
            return Ok(out);
        }
        for r in self.dav.request(GetCalendarResources::new(cal_href).with_hrefs(&ours)).await?.resources {
            if let Ok(content) = r.content {
                let hash = ical::stored_hash(&content.data).unwrap_or_default();
                out.insert(file_name(&r.href).to_string(), Existing { href: r.href.clone(), etag: content.etag, hash });
            }
        }
        Ok(out)
    }
}

struct Existing {
    href: String,
    etag: String,
    hash: String,
}

fn file_name(href: &str) -> &str {
    href.rsplit('/').next().unwrap_or(href)
}
