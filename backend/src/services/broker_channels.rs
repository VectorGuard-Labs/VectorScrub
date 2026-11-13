//! The channels VectorScrub may use to send a deletion request to a company.
//!
//! A request is only ever sent to an address listed here, and an address is only listed with
//! evidence: the company's OWN published page, a short quote from it that contains the address,
//! and the date it was last checked. Nothing is guessed, taken from a third-party guide, or
//! taken from the California registry's contact column (which is the regulator's contact, and in
//! many cases an individual's address). Adding a channel is a data change to
//! `data/broker_channels.json`, and `validate` refuses an entry that lacks its evidence.
//!
//! `applies_to` lists the states whose residents the company's page offers the channel to. A
//! customer whose saved address is in another state, or whose state cannot be told, is not
//! offered it: the legal basis the request cites would not be theirs.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const CATALOG_JSON: &str = include_str!("../../data/broker_channels.json");

/// What a request may ask for.
pub const REQUEST_KINDS: [&str; 2] = ["delete", "opt_out_sale"];

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub company: String,
    pub registry_id: Option<String>,
    pub method: String,
    pub address: String,
    pub source_url: String,
    pub evidence: String,
    pub verified_on: String,
    pub applies_to: Vec<String>,
    pub legal_basis: String,
    pub requests: Vec<String>,
    pub agent_requirements: String,
    pub notes: String,
    /// A subject line the company's page asks for ("Privacy Rights Request"). Used as given.
    #[serde(default)]
    pub subject: Option<String>,
    /// The company's page asks for the person's address to find their records, so a customer
    /// who does not agree to share it cannot be sent to this company.
    #[serde(default)]
    pub requires_address: bool,
}

#[derive(Debug, Deserialize)]
struct Catalog {
    channels: Vec<Channel>,
}

/// The last two labels of a hostname ("privacy.whitepages.com" -> "whitepages.com").
fn registrable(host: &str) -> String {
    let labels: Vec<&str> = host.trim_end_matches('.').split('.').collect();
    labels[labels.len().saturating_sub(2)..].join(".")
}

fn valid_date(s: &str) -> bool {
    let p: Vec<&str> = s.split('-').collect();
    p.len() == 3
        && p[0].len() == 4
        && p[1].len() == 2
        && p[2].len() == 2
        && p.iter().all(|x| x.bytes().all(|b| b.is_ascii_digit()))
        && (1..=12).contains(&p[1].parse::<u32>().unwrap_or(0))
        && (1..=31).contains(&p[2].parse::<u32>().unwrap_or(0))
}

/// Why a catalog is not acceptable, or Ok. Run at startup and by tests.
pub fn validate(channels: &[Channel]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for c in channels {
        let who = &c.id;
        if c.id.is_empty() || c.id.len() > 40 || !c.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            return Err(format!("{who}: id must be lowercase letters, digits and hyphens"));
        }
        if !seen.insert(c.id.clone()) {
            return Err(format!("{who}: duplicate id"));
        }
        if c.company.trim().is_empty() || c.company.len() > 200 {
            return Err(format!("{who}: company name missing"));
        }
        if c.method != "email" {
            return Err(format!("{who}: only the email method is supported"));
        }
        let (local, domain) = c.address.split_once('@').ok_or_else(|| format!("{who}: address is not an email address"))?;
        if local.is_empty()
            || domain.split('.').count() < 2
            || c.address != c.address.to_ascii_lowercase()
            || c.address.bytes().any(|b| b.is_ascii_whitespace() || b == b',' || b == b';' || b == b'<' || b == b'>' || b == b'"')
            || c.address.matches('@').count() != 1
        {
            return Err(format!("{who}: address must be one plain lowercase email address"));
        }
        let source = c.source_url.strip_prefix("https://").ok_or_else(|| format!("{who}: source_url must be https"))?;
        let host = source.split(['/', '?', '#']).next().unwrap_or("");
        if host.is_empty() || host.contains('@') || host.contains(':') {
            return Err(format!("{who}: source_url has no plain host"));
        }
        if registrable(domain) != registrable(host) {
            return Err(format!("{who}: the address must be on the same site as the page that publishes it"));
        }
        if !c.evidence.to_ascii_lowercase().contains(&c.address) {
            return Err(format!("{who}: the quoted evidence must contain the address"));
        }
        if c.evidence.len() < c.address.len() + 20 || c.evidence.len() > 400 {
            return Err(format!("{who}: evidence must be a short quote from the page"));
        }
        if !valid_date(&c.verified_on) {
            return Err(format!("{who}: verified_on must be YYYY-MM-DD"));
        }
        if c.applies_to.iter().any(|s| s.len() != 2 || !s.bytes().all(|b| b.is_ascii_uppercase())) {
            return Err(format!("{who}: applies_to holds two-letter state codes"));
        }
        if c.requests.is_empty() || !c.requests.iter().all(|r| REQUEST_KINDS.contains(&r.as_str())) {
            return Err(format!("{who}: requests must be among {REQUEST_KINDS:?}"));
        }
        if c.legal_basis.trim().is_empty() || c.agent_requirements.trim().is_empty() {
            return Err(format!("{who}: legal_basis and agent_requirements are required"));
        }
        if let Some(subject) = &c.subject {
            if subject.trim().is_empty() || subject.len() > 100 || subject.chars().any(|ch| ch.is_control()) {
                return Err(format!("{who}: subject must be one short plain line"));
            }
        }
    }
    Ok(())
}

fn load() -> Result<Vec<Channel>, String> {
    let catalog: Catalog = serde_json::from_str(CATALOG_JSON).map_err(|e| format!("broker_channels.json: {e}"))?;
    validate(&catalog.channels)?;
    Ok(catalog.channels)
}

/// Check the embedded catalog; called at startup so a bad data file stops the deploy.
pub fn check_embedded() -> Result<usize, String> {
    load().map(|c| c.len())
}

pub fn catalog() -> &'static [Channel] {
    static CATALOG: OnceLock<Vec<Channel>> = OnceLock::new();
    CATALOG.get_or_init(|| load().expect("data/broker_channels.json is invalid (checked at startup)"))
}

pub fn find(id: &str) -> Option<&'static Channel> {
    catalog().iter().find(|c| c.id == id)
}

/// Whether a customer whose saved address is in `state` may be offered this channel.
pub fn eligible(channel: &Channel, state: Option<&str>) -> bool {
    channel.applies_to.is_empty() || state.is_some_and(|s| channel.applies_to.iter().any(|a| a.eq_ignore_ascii_case(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Channel {
        catalog()[0].clone()
    }

    #[test]
    fn the_embedded_catalog_is_valid_and_not_empty() {
        assert!(check_embedded().unwrap() >= 1);
        for c in catalog() {
            assert_eq!(c.method, "email");
            assert!(c.evidence.to_ascii_lowercase().contains(&c.address), "{}", c.id);
        }
    }

    #[test]
    fn find_and_eligibility() {
        let w = find("whitepages").expect("whitepages");
        assert_eq!(w.address, "privacyrequest@whitepages.com");
        assert!(find("nope").is_none());
        assert!(eligible(w, Some("CA")) && eligible(w, Some("ca")));
        assert!(!eligible(w, Some("TX")), "the page offers this route to California residents");
        assert!(!eligible(w, None), "an unknown state is not guessed to be California");
        let mut open = good();
        open.applies_to.clear();
        assert!(eligible(&open, None) && eligible(&open, Some("TX")));
    }

    #[test]
    fn every_kind_of_incomplete_or_unsafe_entry_is_refused() {
        type Change = Box<dyn Fn(&mut Channel)>;
        let cases: Vec<(&str, Change)> = vec![
            ("address not in the evidence", Box::new(|c| c.evidence = "email us at someone.else@whitepages.com please do".into())),
            ("address on a different site", Box::new(|c| { c.address = "privacy@other-company.com".into(); c.evidence = "email privacy@other-company.com for deletion requests".into(); })),
            ("subdomain trick", Box::new(|c| { c.address = "x@whitepages.com.evil.net".into(); c.evidence = "email x@whitepages.com.evil.net for deletion requests".into(); })),
            ("not https", Box::new(|c| c.source_url = "http://www.whitepages.com/privacy".into())),
            ("source with credentials", Box::new(|c| c.source_url = "https://user@whitepages.com/p".into())),
            ("two addresses", Box::new(|c| { c.address = "a@whitepages.com,b@whitepages.com".into(); c.evidence = "email a@whitepages.com,b@whitepages.com for requests".into(); })),
            ("uppercase", Box::new(|c| c.address = "PrivacyRequest@whitepages.com".into())),
            ("whitespace", Box::new(|c| c.address = "privacy request@whitepages.com".into())),
            ("no at sign", Box::new(|c| c.address = "privacyrequest.whitepages.com".into())),
            ("method not supported", Box::new(|c| c.method = "portal".into())),
            ("bad id", Box::new(|c| c.id = "Has Spaces".into())),
            ("empty id", Box::new(|c| c.id = String::new())),
            ("bad date", Box::new(|c| c.verified_on = "yesterday".into())),
            ("impossible month", Box::new(|c| c.verified_on = "2026-13-01".into())),
            ("lowercase state", Box::new(|c| c.applies_to = vec!["ca".into()])),
            ("long state", Box::new(|c| c.applies_to = vec!["CAL".into()])),
            ("unknown request kind", Box::new(|c| c.requests = vec!["sell_my_data".into()])),
            ("no requests", Box::new(|c| c.requests.clear())),
            ("no legal basis", Box::new(|c| c.legal_basis = "  ".into())),
            ("no agent requirements", Box::new(|c| c.agent_requirements.clear())),
            ("evidence is a whole page", Box::new(|c| c.evidence = format!("{} {}", "x".repeat(500), c.address))),
            ("no company", Box::new(|c| c.company.clear())),
            ("multi-line subject", Box::new(|c| c.subject = Some("Privacy\r\nBcc: x@y.example".into()))),
            ("empty subject", Box::new(|c| c.subject = Some("  ".into()))),
            ("very long subject", Box::new(|c| c.subject = Some("s".repeat(101)))),
        ];
        for (what, change) in cases {
            let mut c = good();
            change(&mut c);
            assert!(validate(&[c]).is_err(), "must be refused: {what}");
        }
        assert!(validate(&[good(), good()]).is_err(), "duplicate ids");
        assert!(validate(&[good()]).is_ok());
    }

    #[test]
    fn the_catalog_holds_the_channels_found_on_the_companies_own_pages() {
        let ids: Vec<&str> = catalog().iter().map(|c| c.id.as_str()).collect();
        for expected in ["whitepages", "stirista", "realsource", "dealmachine", "recruitbot", "snovio"] {
            assert!(ids.contains(&expected), "{expected}");
        }
        // general channels (no state restriction) vs the two that are California-only
        for id in ["stirista", "realsource", "dealmachine"] {
            assert!(find(id).unwrap().applies_to.is_empty(), "{id} is offered to everyone by its page");
        }
        for id in ["whitepages", "recruitbot", "snovio"] {
            assert_eq!(find(id).unwrap().applies_to, vec!["CA".to_string()], "{id}");
        }
        assert_eq!(find("dealmachine").unwrap().subject.as_deref(), Some("Privacy Rights Request"));
        assert!(find("stirista").unwrap().requires_address);
        assert!(!find("realsource").unwrap().requires_address);
    }

    #[test]
    fn channels_the_pages_do_not_support_are_not_in_the_catalog() {
        // Found on the companies' pages and rejected on reading: not a general deletion channel.
        for (id, address) in [
            ("recordsfinder", "privacy@recordsfinder.com"),   // judges and law-enforcement families only
            ("staterecords", "privacy@staterecords.org"),     // the same
            ("tapad", "privacy@tapad.com"),                   // "for inquiries not related to" deletion requests
            ("spokeo", "privacy@spokeo.com"),                 // its EU data-protection contact
        ] {
            assert!(find(id).is_none(), "{id}");
            assert!(catalog().iter().all(|c| c.address != address), "{address}");
        }
    }

    #[test]
    fn registrable_domains() {
        assert_eq!(registrable("privacy.whitepages.com"), "whitepages.com");
        assert_eq!(registrable("whitepages.com"), "whitepages.com");
        assert_ne!(registrable("whitepages.com.evil.net"), "whitepages.com");
    }
}
