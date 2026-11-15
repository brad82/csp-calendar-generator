//! Logged-in session on the CSP Roster site. ureq keeps the PHPSESSID cookie
//! and follows redirects.

use anyhow::{Result, bail};
use ureq::Agent;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) csp-ical";

pub struct Session {
    agent: Agent,
    base: String,
}

pub fn is_login_page(html: &str) -> bool {
    html.contains(r#"name="inm""#)
}

impl Session {
    pub fn login(base: &str, user: &str, pass: &str) -> Result<Self> {
        let agent: Agent = Agent::config_builder().user_agent(UA).build().into();
        let s = Session { agent, base: base.to_string() };
        s.get("login.php")?; // start a session
        let html = s
            .agent
            .post(&s.url("login_.php"))
            .send_form([("login", user), ("cboid", "8"), ("inm", pass), ("submitbutton", "  Login  ")])?
            .body_mut()
            .read_to_string()?;
        if is_login_page(&html) {
            bail!("login failed: check CSP_LOGIN / CSP_PASSWORD");
        }
        Ok(s)
    }

    pub fn get(&self, path: &str) -> Result<String> {
        Ok(self.agent.get(&self.url(path)).call()?.body_mut().read_to_string()?)
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base)
    }
}
